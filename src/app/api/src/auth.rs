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
