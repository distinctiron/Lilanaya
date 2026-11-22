//! Sessions: who is asking. A signed-in browser carries an opaque random
//! token in an HttpOnly cookie; the database keeps only its SHA-256.

use crate::error::ApiError;
use crate::worker::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const SESSION_COOKIE: &str = "lilanaya_session";
pub const SESSION_DAYS: i64 = 30;
pub const SIGN_IN_LINK_MINUTES: i64 = 15;

#[derive(Clone, Debug)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub display_name: String,
    pub roles: Vec<String>,
    pub plan: String,
}

/// 256 random bits as 64 hex digits.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system's random source");
    hex(&bytes)
}

pub fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The `Set-Cookie` value that stores (or, with `None`, clears) the session.
pub fn session_cookie(token: Option<&str>, secure: bool) -> String {
    let (value, max_age) = match token {
        Some(token) => (token, SESSION_DAYS * 24 * 60 * 60),
        None => ("", 0),
    };
    format!("{SESSION_COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{}", if secure { "; Secure" } else { "" })
}

/// The value of the cookie called `name`, if the request carries one.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, value)| *n == name && !value.is_empty())
        .map(|(_, value)| value.to_string())
}

/// The session token the request carries, if any.
pub fn session_token(parts: &Parts) -> Option<String> {
    cookie(&parts.headers, SESSION_COOKIE)
}

/// Whoever is asking: a signed-in user, or nobody.
pub struct Viewer(pub Option<User>);

impl Viewer {
    pub fn id(&self) -> Option<i64> {
        self.0.as_ref().map(|u| u.id)
    }
}

impl FromRequestParts<Arc<AppState>> for Viewer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, ApiError> {
        match session_token(parts) {
            Some(token) => Ok(Viewer(state.store.session_user(&hash_token(&token)).await?)),
            None => Ok(Viewer(None)),
        }
    }
}

/// A signed-in user; the request is answered 401 without one.
impl FromRequestParts<Arc<AppState>> for User {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, ApiError> {
        Viewer::from_request_parts(parts, state)
            .await?
            .0
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthenticated", "Log in to do this"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_256_random_bits_as_hex() {
        let tokens: Vec<String> = (0..200).map(|_| new_token()).collect();
        for token in &tokens {
            assert!(token.len() == 64 && token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{token}");
        }
        let mut unique = tokens.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), tokens.len(), "two tokens were the same");
        // Not a counter or a clock: every digit takes many values across tokens.
        for position in 0..64 {
            let mut seen: Vec<u8> = tokens.iter().map(|t| t.as_bytes()[position]).collect();
            seen.sort();
            seen.dedup();
            assert!(seen.len() >= 8, "digit {position} takes only {} values", seen.len());
        }
    }

    #[test]
    fn only_the_sha_256_of_a_token_is_kept() {
        // The standard's own examples.
        assert_eq!(hash_token(""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hash_token("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let token = new_token();
        assert_eq!(hash_token(&token), hash_token(&token));
        assert_ne!(hash_token(&token), token);
        assert_ne!(hash_token(&token), hash_token(&token.to_uppercase()));
    }

    #[test]
    fn the_session_cookie_is_set_for_thirty_days_and_cleared_with_the_same_attributes() {
        assert_eq!(session_cookie(Some("abc"), false), "lilanaya_session=abc; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax");
        assert_eq!(session_cookie(Some("abc"), true), "lilanaya_session=abc; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax; Secure");
        assert_eq!(session_cookie(None, false), "lilanaya_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax");
        assert_eq!(session_cookie(None, true), "lilanaya_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax; Secure");
        assert_eq!((SESSION_DAYS, SIGN_IN_LINK_MINUTES), (30, 15));
    }

    #[test]
    fn a_cookie_is_found_by_its_exact_name_among_whatever_else_the_browser_sends() {
        let headers = |values: &[&str]| {
            let mut map = HeaderMap::new();
            for value in values {
                map.append(header::COOKIE, value.parse().unwrap());
            }
            map
        };
        let find = |values: &[&str]| cookie(&headers(values), SESSION_COOKIE);
        assert_eq!(find(&["lilanaya_session=abc"]), Some("abc".into()));
        assert_eq!(find(&["theme=dark; lilanaya_session=abc; other=1"]), Some("abc".into()));
        assert_eq!(find(&["theme=dark;lilanaya_session=abc"]), Some("abc".into()), "no space after the semicolon");
        assert_eq!(find(&["theme=dark", "lilanaya_session=abc"]), Some("abc".into()), "in a header of its own");
        assert_eq!(find(&["lilanaya_session=a=b=c"]), Some("a=b=c".into()), "only the first = separates name from value");
        // A cleared cookie, a look-alike name, or none at all.
        assert_eq!(find(&["lilanaya_session="]), None);
        assert_eq!(find(&["lilanaya_session_old=abc; xlilanaya_session=abc; LILANAYA_SESSION=abc"]), None);
        assert_eq!(find(&["lilanaya_session"]), None);
        assert_eq!(find(&[]), None);
        // An empty one first does not hide a real one after it.
        assert_eq!(find(&["lilanaya_session=; lilanaya_session=abc"]), Some("abc".into()));
    }
}
