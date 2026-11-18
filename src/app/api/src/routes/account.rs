//! Sign-in by emailed link, the session, and what the signed-in user is entitled to.

use super::{parse_json, ApiResult, Shared};
use crate::auth::{hash_token, new_token, session_cookie, session_token, User, Viewer, SIGN_IN_LINK_MINUTES};
use crate::error::ApiError;
use crate::worker::AppState;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::State;
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_DISPLAY_NAME_CHARS: usize = 60;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailBody {
    email: String,
}

/// Lower-cased address if it looks like one. The real check is that the link arrives.
fn normalise_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    let (local, domain) = email.split_once('@')?;
    let plausible = !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && email.len() <= 254
        && !email.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"'))
        && !domain.contains('@');
    plausible.then_some(email)
}

/// Emails a one-time sign-in link. Answers 202 whether or not the address has an account.
pub async fn request_link(State(state): Shared, body: Result<Bytes, BytesRejection>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let req: EmailBody = parse_json(body)?;
    let Some(email) = normalise_email(&req.email) else {
        return Err(ApiError::invalid("Enter a valid email address", vec![lilanaya_engine::Issue::new("email", "is not a valid email address")]));
    };
    let token = new_token();
    if !state.store.create_sign_in_token(&email, &hash_token(&token)).await? {
        return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", "Too many sign-in links were asked for this address. Try again in 15 minutes."));
    }
    let link = format!("{}/sign-in/confirm?token={token}", state.config.public_url);
    let text = format!(
        "Open this link to sign in to Lilanaya:\n\n{link}\n\nIt works once and for {SIGN_IN_LINK_MINUTES} minutes. If you did not ask for it, ignore this email: nobody can sign in without the link.\n"
    );
    state.mailer.send(&email, "Your Lilanaya sign-in link", &text).await.map_err(|e| ApiError::internal(format!("send sign-in email: {e}")))?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "sent": true, "expires_in_minutes": SIGN_IN_LINK_MINUTES }))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmBody {
    token: String,
}

/// Trades the token from the emailed link for a session cookie; creates the user on first sign-in.
pub async fn confirm_link(State(state): Shared, body: Result<Bytes, BytesRejection>) -> Result<Response, ApiError> {
    let req: ConfirmBody = parse_json(body)?;
    let Some(email) = state.store.consume_sign_in_token(&hash_token(req.token.trim())).await? else {
        return Err(ApiError::bad_request("invalid_token", "This sign-in link has expired or was already used. Ask for a new one."));
    };
    let default_name: String = email.split('@').next().unwrap_or("player").chars().take(MAX_DISPLAY_NAME_CHARS).collect();
    let user = state.store.upsert_user(&email, &default_name).await?;
    let session = new_token();
    state.store.create_session(user.id, &hash_token(&session)).await?;
    let cookie = session_cookie(Some(&session), state.config.secure_cookies());
    Ok(([(header::SET_COOKIE, cookie)], Json(me_json(&state, &user).await?)).into_response())
}

/// Ends the session and clears the cookie. Fine to call when not signed in.
pub async fn sign_out(State(state): Shared, parts: Parts) -> Result<Response, ApiError> {
    if let Some(token) = session_token(&parts) {
        state.store.delete_session(&hash_token(&token)).await?;
    }
    let cookie = session_cookie(None, state.config.secure_cookies());
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "signed_out": true }))).into_response())
}

/// Who is signed in, for the front end: `{"user": {…}}`, or `{"user": null}` (200, not 401) when nobody is.
pub async fn session(State(state): Shared, viewer: Viewer) -> ApiResult {
    let user = match &viewer.0 {
        Some(user) => me_json(&state, user).await?,
        None => Value::Null,
    };
    Ok(Json(json!({ "user": user })))
}

pub async fn me(State(state): Shared, user: User) -> ApiResult {
    Ok(Json(me_json(&state, &user).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileBody {
    display_name: String,
}

pub async fn update_me(State(state): Shared, user: User, body: Result<Bytes, BytesRejection>) -> ApiResult {
    let req: ProfileBody = parse_json(body)?;
    let name = req.display_name.trim();
    if name.is_empty() || name.chars().count() > MAX_DISPLAY_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            "The display name is not valid",
            vec![lilanaya_engine::Issue::new("display_name", format!("must be 1 to {MAX_DISPLAY_NAME_CHARS} characters"))],
        ));
    }
    let user = state.store.rename_user(user.id, name).await?;
    Ok(Json(me_json(&state, &user).await?))
}

pub async fn entitlements(user: User) -> ApiResult {
    Ok(Json(entitlements_json(&user)))
}

pub async fn usage(State(state): Shared, user: User) -> ApiResult {
    Ok(Json(usage_json(&state, &user).await?))
}

/// What the plan unlocks. No billing yet: the plan is set by hand in the database.
fn entitlements_json(user: &User) -> Value {
    let subscriber = user.plan == "subscriber";
    json!({ "plan": user.plan, "analytics_access": subscriber, "quick_test_access": subscriber, "private_worlds": subscriber })
}

/// Runs started this calendar month against the plan's allowance. Shown only; nothing is refused yet.
async fn usage_json(state: &AppState, user: &User) -> Result<Value, ApiError> {
    let (period, used) = state.store.runs_this_month(user.id).await?;
    let monthly = state.config.monthly_runs(&user.plan);
    Ok(json!({ "period": period, "monthly_runs": monthly, "used": used, "remaining": (monthly - used).max(0), "enforced": false }))
}

async fn me_json(state: &AppState, user: &User) -> Result<Value, ApiError> {
    Ok(json!({
        "id": user.id,
        "email": user.email,
        "display_name": user.display_name,
        "roles": user.roles,
        "plan": user.plan,
        "entitlements": entitlements_json(user),
        "usage": usage_json(state, user).await?,
        "features": { "compile": state.llm.is_some() },
    }))
}

#[cfg(test)]
mod tests {
    use super::normalise_email;

    #[test]
    fn addresses_are_lower_cased_and_roughly_checked() {
        assert_eq!(normalise_email("  Ada@Example.COM "), Some("ada@example.com".into()));
        for bad in ["", "ada", "ada@", "@example.com", "ada@example", "a b@example.com", "ada@@example.com", "ada@.com", "ada <a@example.com>"] {
            assert_eq!(normalise_email(bad), None, "{bad}");
        }
    }
}
