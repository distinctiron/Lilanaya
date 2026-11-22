//! Sign-in through an account somewhere else: Google, Apple, Facebook or GitHub.
//!
//! All four use the OAuth 2 authorization-code flow, run entirely on the server:
//! the browser is sent to the provider with a random `state`, comes back with a
//! one-time `code`, and the server trades the code (with the client secret) for
//! who the person is.
//!
//! Each provider has its own flag (`on` or `off`) and is off unless the flag says `on`:
//! `GOOGLE_SIGN_IN`, `APPLE_SIGN_IN`, `FACEBOOK_SIGN_IN`, `GITHUB_SIGN_IN`. A provider that is
//! off is not offered, whatever credentials are set; one that is on must have its credentials.
//!
//! | Provider | Settings                                                          |
//! |----------|-------------------------------------------------------------------|
//! | Google   | `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`                        |
//! | Apple    | `APPLE_CLIENT_ID` (the Services ID), `APPLE_TEAM_ID`, `APPLE_KEY_ID`, `APPLE_PRIVATE_KEY` (the .p8 key) |
//! | Facebook | `FACEBOOK_CLIENT_ID` (the app ID), `FACEBOOK_CLIENT_SECRET`       |
//! | GitHub   | `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`                        |
//!
//! Each provider must be told the redirect address `{PUBLIC_URL}/api/auth/{provider}/callback`.

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const APPLE_AUDIENCE: &str = "https://appleid.apple.com";
/// How long the signed client secret made for one exchange with Apple is valid.
const APPLE_SECRET_SECONDS: u64 = 300;

/// Who the provider says has signed in.
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    /// The provider's own, stable id for the account.
    pub subject: String,
    /// An address the provider vouches for; `None` when it gave none or has not verified it.
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Failure {
    /// The provider answered something we cannot use. The detail is for the log.
    Provider(String),
}

enum Secret {
    Fixed(String),
    /// Apple wants a short-lived token signed with the developer key instead of a fixed secret.
    AppleKey { team_id: String, key_id: String, key: EcdsaKeyPair },
}

/// Where the identity comes from once the code has been traded in.
enum Profile {
    /// An OpenID Connect ID token in the token response (Google, Apple).
    IdToken { issuers: &'static [&'static str] },
    GitHub { api: String },
    Facebook { me_url: String },
}

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    client_id: String,
    secret: Secret,
    authorize_url: String,
    /// Extra query parameters for the authorize address, after the standard ones.
    authorize_extra: &'static str,
    token_url: String,
    scope: &'static str,
    profile: Profile,
}

pub struct OAuth {
    providers: Vec<Provider>,
    http: reqwest::Client,
}

/// Reads one provider's flag (say, `GOOGLE_SIGN_IN`). Unset means off.
fn flag_is_on(name: &str, raw: Option<&str>) -> Result<bool, String> {
    match raw.map(|v| v.trim().to_lowercase()).as_deref() {
        None | Some("") | Some("off") | Some("false") | Some("0") => Ok(false),
        Some("on") | Some("true") | Some("1") => Ok(true),
        Some(other) => Err(format!("{name} must be on or off, got \"{other}\"")),
    }
}

fn encode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Reads the key Apple hands out as a .p8 file. Accepts the file's text with real
/// line breaks or with `\n` written out (handy on one line of a .env file), or just the base64 body.
fn apple_key(raw: &str) -> Result<EcdsaKeyPair, String> {
    let body: String = raw.replace("\\n", "\n").lines().filter(|line| !line.starts_with("-----")).flat_map(|line| line.chars()).filter(|c| !c.is_whitespace()).collect();
    let der = STANDARD.decode(body).map_err(|e| format!("APPLE_PRIVATE_KEY is not a .p8 key (base64: {e})"))?;
    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der).map_err(|e| format!("APPLE_PRIVATE_KEY is not a P-256 private key in PKCS#8 form: {e}"))
}

/// The client secret Apple expects: a JWT signed (ES256) with the developer key.
fn apple_client_secret(client_id: &str, team_id: &str, key_id: &str, key: &EcdsaKeyPair, now: u64) -> Result<String, String> {
    let part = |value: Value| URL_SAFE_NO_PAD.encode(value.to_string());
    let header = part(json!({ "alg": "ES256", "kid": key_id }));
    let claims = part(json!({ "iss": team_id, "iat": now, "exp": now + APPLE_SECRET_SECONDS, "aud": APPLE_AUDIENCE, "sub": client_id }));
    let signed = format!("{header}.{claims}");
    let signature = key.sign(&SystemRandom::new(), signed.as_bytes()).map_err(|_| "could not sign the Apple client secret".to_string())?;
    Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref())))
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Reads the claims of an ID token that came straight from the provider's token
/// endpoint over TLS. That channel is what vouches for it (OpenID Connect Core
/// 3.1.3.7 allows this in place of checking the signature), so only the claims
/// are checked: who issued it, that it is for us, and that it has not expired.
fn id_token_identity(token: &str, issuers: &[&str], client_id: &str, now: u64) -> Result<Identity, String> {
    let payload = token.split('.').nth(1).ok_or("the ID token is not a JWT")?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|e| format!("the ID token is not base64url: {e}"))?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|e| format!("the ID token's claims are not JSON: {e}"))?;
    let issuer = claims["iss"].as_str().unwrap_or("");
    if !issuers.contains(&issuer) {
        return Err(format!("the ID token was issued by \"{issuer}\""));
    }
    let for_us = match &claims["aud"] {
        Value::String(aud) => aud == client_id,
        Value::Array(list) => list.iter().any(|aud| aud == client_id),
        _ => false,
    };
    if !for_us {
        return Err("the ID token is for another client".into());
    }
    if claims["exp"].as_u64().is_none_or(|exp| exp <= now) {
        return Err("the ID token has expired".into());
    }
    let subject = text(&claims["sub"]).ok_or("the ID token names nobody (no sub)")?;
    // Apple writes the flag as the string "true"; Google as a boolean.
    let verified = claims["email_verified"] == true || claims["email_verified"] == "true";
    Ok(Identity { subject, email: text(&claims["email"]).filter(|_| verified), name: text(&claims["name"]) })
}

impl Provider {
    fn google(client_id: String, secret: String) -> Self {
        Provider {
            id: "google",
            name: "Google",
            client_id,
            secret: Secret::Fixed(secret),
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            authorize_extra: "",
            token_url: "https://oauth2.googleapis.com/token".into(),
            scope: "openid email profile",
            profile: Profile::IdToken { issuers: &["https://accounts.google.com", "accounts.google.com"] },
        }
    }

    fn apple(client_id: String, team_id: String, key_id: String, key: EcdsaKeyPair) -> Self {
        Provider {
            id: "apple",
            name: "Apple",
            client_id,
            secret: Secret::AppleKey { team_id, key_id, key },
            authorize_url: "https://appleid.apple.com/auth/authorize".into(),
            // Apple only gives the name and address when it may post the answer back as a form.
            authorize_extra: "&response_mode=form_post",
            token_url: "https://appleid.apple.com/auth/token".into(),
            scope: "name email",
            profile: Profile::IdToken { issuers: &[APPLE_AUDIENCE] },
        }
    }

    fn facebook(client_id: String, secret: String) -> Self {
        Provider {
            id: "facebook",
            name: "Facebook",
            client_id,
            secret: Secret::Fixed(secret),
            // No Graph API version in the addresses: Facebook then uses the oldest version
            // the app may still call, so these do not stop working when a version is retired.
            authorize_url: "https://www.facebook.com/dialog/oauth".into(),
            authorize_extra: "",
            token_url: "https://graph.facebook.com/oauth/access_token".into(),
            scope: "email,public_profile",
            profile: Profile::Facebook { me_url: "https://graph.facebook.com/me".into() },
        }
    }

    fn github(client_id: String, secret: String) -> Self {
        Provider {
            id: "github",
            name: "GitHub",
            client_id,
            secret: Secret::Fixed(secret),
            authorize_url: "https://github.com/login/oauth/authorize".into(),
            authorize_extra: "",
            token_url: "https://github.com/login/oauth/access_token".into(),
            scope: "read:user user:email",
            profile: Profile::GitHub { api: "https://api.github.com".into() },
        }
    }

    /// Where to send the browser so the person can approve the sign-in.
    pub fn authorize_url(&self, state: &str, redirect_uri: &str) -> String {
        format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}{}",
            self.authorize_url,
            encode(&self.client_id),
            encode(redirect_uri),
            encode(self.scope),
            encode(state),
            self.authorize_extra
        )
    }

    fn client_secret(&self) -> Result<String, Failure> {
        match &self.secret {
            Secret::Fixed(secret) => Ok(secret.clone()),
            Secret::AppleKey { team_id, key_id, key } => apple_client_secret(&self.client_id, team_id, key_id, key, unix_now()).map_err(Failure::Provider),
        }
    }
}

impl OAuth {
    fn with(providers: Vec<Provider>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("lilanaya") // GitHub's API refuses requests without one
            .build()
            .map_err(|e| format!("could not set up the HTTP client for sign-in providers: {e}"))?;
        Ok(OAuth { providers, http })
    }

    pub fn from_env() -> Result<Self, String> {
        Self::from_settings(|name| std::env::var(name).ok())
    }

    /// `setting` looks a name up in the environment.
    fn from_settings(setting: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        // Compose passes unset settings as empty strings; treat those as unset.
        let var = |name: &str| setting(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        // A provider's credentials if its flag is on; `None` if it is off (its credentials are then not looked at).
        // On without all of its credentials is a mistake worth stopping for.
        let switched_on = |flag: &str, names: &[&str]| -> Result<Option<Vec<String>>, String> {
            if !flag_is_on(flag, var(flag).as_deref())? {
                return Ok(None);
            }
            let values: Vec<Option<String>> = names.iter().map(|name| var(name)).collect();
            let missing: Vec<&str> = names.iter().zip(&values).filter(|(_, v)| v.is_none()).map(|(name, _)| *name).collect();
            if !missing.is_empty() {
                return Err(format!("{flag} is on, but {} {} not set", missing.join(", "), if missing.len() == 1 { "is" } else { "are" }));
            }
            Ok(Some(values.into_iter().flatten().collect()))
        };
        let mut providers = Vec::new();
        if let Some(v) = switched_on("GOOGLE_SIGN_IN", &["GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET"])? {
            providers.push(Provider::google(v[0].clone(), v[1].clone()));
        }
        if let Some(v) = switched_on("APPLE_SIGN_IN", &["APPLE_CLIENT_ID", "APPLE_TEAM_ID", "APPLE_KEY_ID", "APPLE_PRIVATE_KEY"])? {
            providers.push(Provider::apple(v[0].clone(), v[1].clone(), v[2].clone(), apple_key(&v[3])?));
        }
        if let Some(v) = switched_on("FACEBOOK_SIGN_IN", &["FACEBOOK_CLIENT_ID", "FACEBOOK_CLIENT_SECRET"])? {
            providers.push(Provider::facebook(v[0].clone(), v[1].clone()));
        }
        if let Some(v) = switched_on("GITHUB_SIGN_IN", &["GITHUB_CLIENT_ID", "GITHUB_CLIENT_SECRET"])? {
            providers.push(Provider::github(v[0].clone(), v[1].clone()));
        }
        Self::with(providers)
    }

    /// The providers that are switched on, in the order they are offered.
    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    pub fn provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn describe(&self) -> String {
        match self.providers.len() {
            0 => "sign-in through Google, Apple, Facebook and GitHub is switched off (GOOGLE_SIGN_IN, APPLE_SIGN_IN, FACEBOOK_SIGN_IN, GITHUB_SIGN_IN; see .env.example): sign-in is by emailed link only".into(),
            _ => format!("sign-in is also offered through {}", self.providers.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")),
        }
    }

    /// Trades the one-time code for who signed in.
    pub async fn identify(&self, provider: &Provider, code: &str, redirect_uri: &str) -> Result<Identity, Failure> {
        let form = format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&client_secret={}",
            encode(code),
            encode(redirect_uri),
            encode(&provider.client_id),
            encode(&provider.client_secret()?)
        );
        let request = self.http.post(&provider.token_url).header("content-type", "application/x-www-form-urlencoded").header("accept", "application/json").body(form);
        let tokens = Self::json(request, "token").await?;
        // GitHub reports a refused code with status 200 and an `error` field.
        if let Some(error) = tokens["error"].as_str() {
            return Err(Failure::Provider(format!("{}: token exchange refused: {error} {}", provider.id, tokens["error_description"].as_str().unwrap_or(""))));
        }
        let access_token = || tokens["access_token"].as_str().ok_or_else(|| Failure::Provider(format!("{}: the token answer has no access_token", provider.id)));
        match &provider.profile {
            Profile::IdToken { issuers } => {
                let token = tokens["id_token"].as_str().ok_or_else(|| Failure::Provider(format!("{}: the token answer has no id_token", provider.id)))?;
                id_token_identity(token, issuers, &provider.client_id, unix_now()).map_err(|e| Failure::Provider(format!("{}: {e}", provider.id)))
            }
            Profile::GitHub { api } => {
                let token = access_token()?;
                let get = |path: &str| self.http.get(format!("{api}{path}")).bearer_auth(token).header("accept", "application/vnd.github+json");
                let user = Self::json(get("/user"), "github user").await?;
                let subject = user["id"].as_i64().ok_or_else(|| Failure::Provider("github: the user has no id".into()))?.to_string();
                let emails = Self::json(get("/user/emails"), "github emails").await?;
                // Only the primary address: the others can be GitHub's own no-reply addresses, which receive nothing.
                let email = emails.as_array().into_iter().flatten().find(|e| e["verified"] == true && e["primary"] == true).and_then(|e| text(&e["email"]));
                Ok(Identity { subject, email, name: text(&user["name"]).or_else(|| text(&user["login"])) })
            }
            Profile::Facebook { me_url } => {
                let me = Self::json(self.http.get(format!("{me_url}?fields=id,name,email")).bearer_auth(access_token()?), "facebook profile").await?;
                let subject = text(&me["id"]).ok_or_else(|| Failure::Provider("facebook: the profile has no id".into()))?;
                // Facebook has no "verified" flag: it only returns an address the person has confirmed.
                Ok(Identity { subject, email: text(&me["email"]), name: text(&me["name"]) })
            }
        }
    }

    async fn json(request: reqwest::RequestBuilder, what: &str) -> Result<Value, Failure> {
        let response = request.send().await.map_err(|e| Failure::Provider(format!("{what}: {e}")))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| Failure::Provider(format!("{what}: {e}")))?;
        let value: Value = serde_json::from_str(&body).map_err(|_| Failure::Provider(format!("{what}: answered {status}, not JSON")))?;
        // GitHub answers a refused code with 200 and an `error` field; the caller reports that one.
        if !status.is_success() {
            return Err(Failure::Provider(format!("{what}: answered {status}: {}", value.get("error").unwrap_or(&Value::Null))));
        }
        Ok(value)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use aws_lc_rs::signature::{KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};

    pub const TEST_CLIENT_ID: &str = "lilanaya-client";
    pub const TEST_SECRET: &str = "shh";

    impl OAuth {
        pub fn none() -> Self {
            Self::with(Vec::new()).unwrap()
        }

        /// All four providers, pointed at a stand-in server at `base`.
        pub fn for_tests(base: &str) -> Self {
            let key = EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap();
            let mut providers = vec![
                Provider::google(TEST_CLIENT_ID.into(), TEST_SECRET.into()),
                Provider::apple(TEST_CLIENT_ID.into(), "TEAM123".into(), "KEY456".into(), key),
                Provider::facebook(TEST_CLIENT_ID.into(), TEST_SECRET.into()),
                Provider::github(TEST_CLIENT_ID.into(), TEST_SECRET.into()),
            ];
            for p in &mut providers {
                p.authorize_url = format!("{base}/{}/authorize", p.id);
                p.token_url = format!("{base}/{}/token", p.id);
                match &mut p.profile {
                    Profile::GitHub { api } => *api = format!("{base}/github"),
                    Profile::Facebook { me_url } => *me_url = format!("{base}/facebook/me"),
                    Profile::IdToken { .. } => {}
                }
            }
            Self::with(providers).unwrap()
        }
    }

    /// An unsigned ID token carrying `claims`, as a stand-in provider would return it.
    pub fn id_token(claims: &Value) -> String {
        format!("{}.{}.c2ln", URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256"}"#), URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    fn decode(part: &str) -> Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
    }

    fn from(settings: &[(&str, &str)]) -> Result<Vec<&'static str>, String> {
        let lookup = |name: &str| settings.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string());
        OAuth::from_settings(lookup).map(|oauth| oauth.providers().iter().map(|p| p.id).collect())
    }

    #[test]
    fn a_flag_is_off_unless_it_says_on() {
        for off in [None, Some(""), Some("off"), Some(" OFF "), Some("false"), Some("0")] {
            assert_eq!(flag_is_on("GOOGLE_SIGN_IN", off), Ok(false), "{off:?}");
        }
        for on in ["on", "On", "true", "1"] {
            assert_eq!(flag_is_on("GOOGLE_SIGN_IN", Some(on)), Ok(true), "{on}");
        }
        assert_eq!(flag_is_on("GOOGLE_SIGN_IN", Some("maybe")), Err("GOOGLE_SIGN_IN must be on or off, got \"maybe\"".into()));
    }

    #[test]
    fn each_provider_is_switched_on_by_its_own_flag() {
        let credentials = [("GOOGLE_CLIENT_ID", "g"), ("GOOGLE_CLIENT_SECRET", "s"), ("FACEBOOK_CLIENT_ID", "f"), ("FACEBOOK_CLIENT_SECRET", "s"), ("GITHUB_CLIENT_ID", "h"), ("GITHUB_CLIENT_SECRET", "s")];
        let with = |flags: &[(&'static str, &'static str)]| from(&[&credentials[..], flags].concat());

        // Credentials alone switch nothing on.
        assert_eq!(with(&[]), Ok(vec![]));
        assert!(OAuth::from_settings(|_| None).unwrap().describe().contains("switched off"));
        // Google on; Facebook off and Apple and GitHub not mentioned.
        assert_eq!(with(&[("GOOGLE_SIGN_IN", "on"), ("FACEBOOK_SIGN_IN", "off"), ("APPLE_SIGN_IN", "")]), Ok(vec!["google"]));
        assert_eq!(with(&[("GITHUB_SIGN_IN", "on"), ("GOOGLE_SIGN_IN", "on"), ("FACEBOOK_SIGN_IN", "on")]), Ok(vec!["google", "facebook", "github"]));
        // A provider that is off may be half configured; one that is on may not.
        assert_eq!(from(&[("APPLE_CLIENT_ID", "com.example.web"), ("GITHUB_SIGN_IN", "off")]), Ok(vec![]));
        assert_eq!(with(&[("APPLE_SIGN_IN", "on"), ("APPLE_TEAM_ID", "T")]), Err("APPLE_SIGN_IN is on, but APPLE_CLIENT_ID, APPLE_KEY_ID, APPLE_PRIVATE_KEY are not set".into()));
        assert_eq!(from(&[("GOOGLE_SIGN_IN", "on"), ("GOOGLE_CLIENT_ID", "g")]), Err("GOOGLE_SIGN_IN is on, but GOOGLE_CLIENT_SECRET is not set".into()));
        assert!(with(&[("FACEBOOK_SIGN_IN", "sometimes")]).is_err());
    }

    #[test]
    fn id_tokens_are_checked_for_issuer_audience_and_expiry() {
        let issuers = &["https://accounts.google.com"];
        let claims = |change: Value| {
            let mut claims = json!({ "iss": "https://accounts.google.com", "aud": "me", "exp": 2000, "sub": "g-1", "email": "ada@example.com", "email_verified": true, "name": "Ada L." });
            claims.as_object_mut().unwrap().extend(change.as_object().unwrap().clone());
            id_token(&claims)
        };
        let ada = Identity { subject: "g-1".into(), email: Some("ada@example.com".into()), name: Some("Ada L.".into()) };
        assert_eq!(id_token_identity(&claims(json!({})), issuers, "me", 1000), Ok(ada.clone()));
        assert_eq!(id_token_identity(&claims(json!({ "aud": ["other", "me"], "email_verified": "true" })), issuers, "me", 1000), Ok(ada.clone()));
        // An address the provider has not verified is not passed on.
        for unverified in [json!(false), json!("false"), Value::Null] {
            assert_eq!(id_token_identity(&claims(json!({ "email_verified": unverified })), issuers, "me", 1000), Ok(Identity { email: None, ..ada.clone() }));
        }
        for bad in [json!({ "iss": "https://evil.example" }), json!({ "aud": "someone-else" }), json!({ "exp": 1000 }), json!({ "exp": null }), json!({ "sub": "" })] {
            assert!(id_token_identity(&claims(bad.clone()), issuers, "me", 1000).is_err(), "{bad}");
        }
        assert!(id_token_identity("not-a-token", issuers, "me", 1000).is_err());
    }

    #[test]
    fn the_apple_client_secret_is_a_jwt_signed_with_the_developer_key() {
        let generated = EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap();
        // The key arrives the way a .env file holds a .p8 file: PEM on one line, breaks written as \n.
        let pem = format!("-----BEGIN PRIVATE KEY-----\\n{}\\n-----END PRIVATE KEY-----", STANDARD.encode(generated.to_pkcs8v1().unwrap().as_ref()));
        let key = apple_key(&pem).unwrap();
        let secret = apple_client_secret("com.example.web", "TEAM123", "KEY456", &key, 1_000).unwrap();

        let parts: Vec<&str> = secret.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(decode(parts[0]), json!({ "alg": "ES256", "kid": "KEY456" }));
        assert_eq!(decode(parts[1]), json!({ "iss": "TEAM123", "iat": 1000, "exp": 1300, "aud": "https://appleid.apple.com", "sub": "com.example.web" }));
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        assert_eq!(signature.len(), 64, "r and s side by side, as JWT wants");
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key.public_key().as_ref())
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .expect("the signature checks out against the public key");

        assert!(apple_key("not a key").is_err());
    }

    #[test]
    fn the_authorize_address_carries_the_client_the_state_and_where_to_come_back() {
        let oauth = OAuth::for_tests("http://idp.test");
        let url = oauth.provider("apple").unwrap().authorize_url("s t", "http://localhost:3000/api/auth/apple/callback");
        assert_eq!(
            url,
            "http://idp.test/apple/authorize?response_type=code&client_id=lilanaya-client&redirect_uri=http%3A%2F%2Flocalhost%3A3000%2Fapi%2Fauth%2Fapple%2Fcallback&scope=name+email&state=s+t&response_mode=form_post"
        );
        assert!(oauth.provider("github").unwrap().authorize_url("s", "r").ends_with("&scope=read%3Auser+user%3Aemail&state=s"));
        assert!(oauth.provider("myspace").is_none());
    }
}
