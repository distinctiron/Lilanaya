//! Sign-in through Google, Apple, Facebook or GitHub (see `oauth.rs`).
//!
//! These routes are visited by the browser itself, not called by the front end,
//! so they answer with redirects: on to the provider, then back to a page of the
//! site. A failure lands on `/sign-in?error=<code>`, which explains it.
//!
//! The random `state` sent to the provider is also kept in a short-lived cookie,
//! and the two must match when the browser comes back: that ties the answer to
//! the browser that asked, so nobody can be signed in to an account they did
//! not choose by being sent a ready-made callback link.

use super::account::{normalise_email, MAX_DISPLAY_NAME_CHARS};
use super::Shared;
use crate::auth::{cookie, hash_token, new_token, session_cookie, User};
use crate::oauth::Failure;
use crate::worker::AppState;
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{AppendHeaders, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

const STATE_COOKIE: &str = "lilanaya_oauth";
/// How long someone has to finish at the provider.
const STATE_MINUTES: i64 = 10;
const MAX_NEXT_CHARS: usize = 512;

fn encode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn decode(value: &str) -> String {
    form_urlencoded::parse(format!("v={value}").as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
}

/// The `Set-Cookie` value that stores (or, with `None`, clears) the sign-in in progress.
fn state_cookie(value: Option<&str>, secure: bool) -> String {
    let (value, max_age) = match value {
        Some(value) => (value, STATE_MINUTES * 60),
        None => ("", 0),
    };
    format!("{STATE_COOKIE}={value}; Path=/api/auth; Max-Age={max_age}; HttpOnly; SameSite=Lax{}", if secure { "; Secure" } else { "" })
}

/// Only ever send people back to a page of this site.
fn safe_next(raw: Option<&str>) -> Option<String> {
    let next = raw?;
    let local = next.starts_with('/') && !next.starts_with("//") && !next.contains('\\') && !next.chars().any(char::is_control) && next.chars().count() <= MAX_NEXT_CHARS;
    local.then(|| next.to_string())
}

fn redirect(to: &str, cookies: Vec<String>) -> Response {
    let cookies = cookies.into_iter().map(|c| (header::SET_COOKIE, c));
    (StatusCode::SEE_OTHER, AppendHeaders(cookies), [(header::LOCATION, to.to_string()), (header::CACHE_CONTROL, "no-store".to_string())]).into_response()
}

/// Back to the sign-in page, which explains what `code` means.
fn failed(state: &AppState, code: &str, next: Option<&str>) -> Response {
    let next = next.map(|n| format!("&next={}", encode(n))).unwrap_or_default();
    redirect(&format!("/log-in?error={code}{next}"), vec![state_cookie(None, state.config.secure_cookies())])
}

fn redirect_uri(state: &AppState, provider: &str) -> String {
    format!("{}/api/auth/{provider}/callback", state.config.public_url)
}

/// The providers that can be used, for the sign-in page: `{"providers": [{"id", "name"}]}`.
pub async fn providers(State(state): Shared) -> Json<Value> {
    let list: Vec<Value> = state.oauth.providers().iter().map(|p| json!({ "id": p.id, "name": p.name })).collect();
    Json(json!({ "providers": list }))
}

#[derive(Deserialize)]
pub struct StartQuery {
    next: Option<String>,
}

/// Sends the browser to the provider to approve the sign-in.
pub async fn start(State(state): Shared, Path(provider): Path<String>, Query(query): Query<StartQuery>) -> Response {
    let next = safe_next(query.next.as_deref());
    let Some(provider) = state.oauth.provider(&provider) else {
        return failed(&state, "provider_unavailable", next.as_deref());
    };
    let token = new_token();
    let remembered = format!("{token}.{}.{}", provider.id, encode(next.as_deref().unwrap_or("")));
    redirect(&provider.authorize_url(&token, &redirect_uri(&state, provider.id)), vec![state_cookie(Some(&remembered), state.config.secure_cookies())])
}

/// What the provider sends back with the browser.
#[derive(Deserialize)]
pub struct Answer {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    /// The person's name, when Apple gave it (see `callback_form`).
    name: Option<String>,
    /// Apple, first sign-in only: `{"name": {"firstName", "lastName"}, "email"}` as JSON text.
    user: Option<String>,
}

/// Apple posts its answer as a form from its own site, and browsers leave `SameSite=Lax`
/// cookies out of such a request. Turn it into the ordinary GET, which does carry them.
pub async fn callback_form(State(state): Shared, Path(provider): Path<String>, Form(answer): Form<Answer>) -> Response {
    let Some(provider) = state.oauth.provider(&provider) else {
        return failed(&state, "provider_unavailable", None);
    };
    let name = answer.user.as_deref().and_then(|raw| serde_json::from_str::<Value>(raw).ok()).map(|user| {
        let part = |key: &str| user["name"][key].as_str().unwrap_or("").trim().to_string();
        format!("{} {}", part("firstName"), part("lastName")).trim().to_string()
    });
    let mut query = form_urlencoded::Serializer::new(String::new());
    for (key, value) in [("code", answer.code), ("state", answer.state), ("error", answer.error), ("name", name)] {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            query.append_pair(key, &value);
        }
    }
    redirect(&format!("/api/auth/{}/callback?{}", provider.id, query.finish()), Vec::new())
}

/// Where the provider sends the browser back to. Signs the person in and moves on to the page they wanted.
pub async fn callback(State(state): Shared, Path(provider): Path<String>, headers: HeaderMap, Query(answer): Query<Answer>) -> Response {
    // What `start` remembered in this browser: the state, the provider and where to go afterwards.
    let remembered = cookie(&headers, STATE_COOKIE).unwrap_or_default();
    let mut parts = remembered.splitn(3, '.');
    let (expected_state, expected_provider) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let next = safe_next(parts.next().map(decode).as_deref());

    let Some(provider) = state.oauth.provider(&provider) else {
        return failed(&state, "provider_unavailable", next.as_deref());
    };
    if expected_state.is_empty() || answer.state.as_deref() != Some(expected_state) || expected_provider != provider.id {
        return failed(&state, "state_mismatch", None);
    }
    if let Some(error) = &answer.error {
        // `access_denied` (`user_cancelled_authorize` at Apple) is the person pressing Cancel.
        let cancelled = matches!(error.as_str(), "access_denied" | "user_cancelled_authorize");
        if !cancelled {
            eprintln!("sign-in with {}: the provider answered error={error}", provider.id);
        }
        return failed(&state, if cancelled { "cancelled" } else { "provider_error" }, next.as_deref());
    }
    let Some(code) = answer.code.as_deref().filter(|c| !c.is_empty()) else {
        return failed(&state, "provider_error", next.as_deref());
    };

    let identity = match state.oauth.identify(provider, code, &redirect_uri(&state, provider.id)).await {
        Ok(identity) => identity,
        Err(Failure::Provider(detail)) => {
            eprintln!("sign-in with {}: {detail}", provider.id);
            return failed(&state, "provider_error", next.as_deref());
        }
    };
    let user = match signed_in_user(&state, provider.id, &identity.subject, identity.email.as_deref(), identity.name.or(answer.name).as_deref()).await {
        Ok(Some(user)) => user,
        Ok(None) => return failed(&state, "no_email", next.as_deref()),
        Err(e) => {
            eprintln!("sign-in with {}: database: {e}", provider.id);
            return failed(&state, "server_error", next.as_deref());
        }
    };
    let session = new_token();
    if let Err(e) = state.store.create_session(user.id, &hash_token(&session)).await {
        eprintln!("sign-in with {}: database: {e}", provider.id);
        return failed(&state, "server_error", next.as_deref());
    }
    let secure = state.config.secure_cookies();
    redirect(next.as_deref().unwrap_or("/profile"), vec![state_cookie(None, secure), session_cookie(Some(&session), secure)])
}

/// The user an outside account signs in as. An account seen before keeps its user, whatever
/// address it has now. A new one joins the user with the same address (the provider has
/// verified it, which is the same proof the emailed link asks for) or becomes a new user.
/// `None`: the provider gave no verified address, so there is nothing to tell the person apart by.
async fn signed_in_user(state: &AppState, provider: &str, subject: &str, email: Option<&str>, name: Option<&str>) -> Result<Option<User>, sqlx::Error> {
    if let Some(user) = state.store.identity_user(provider, subject).await? {
        return Ok(Some(user));
    }
    let Some(email) = email.and_then(normalise_email) else {
        return Ok(None);
    };
    let given: String = name.unwrap_or("").chars().filter(|c| !c.is_control()).take(MAX_DISPLAY_NAME_CHARS).collect();
    let default_name: String = match given.trim() {
        "" => email.split('@').next().unwrap_or("player").chars().take(MAX_DISPLAY_NAME_CHARS).collect(),
        given => given.to_string(),
    };
    let user = state.store.upsert_user(&email, &default_name).await?;
    state.store.link_identity(provider, subject, user.id).await?;
    Ok(Some(user))
}

#[cfg(test)]
mod tests {
    use super::safe_next;

    #[test]
    fn only_pages_of_this_site_are_accepted_as_next() {
        assert_eq!(safe_next(Some("/runs?mine=1")), Some("/runs?mine=1".into()));
        for bad in ["", "runs", "//evil.example", "/\\evil.example", "https://evil.example", "/a\nb"] {
            assert_eq!(safe_next(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(safe_next(None), None);
    }
}
