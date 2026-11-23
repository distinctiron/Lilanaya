//! Sign-in by emailed link, the session, and what the signed-in user is entitled to.

use super::{parse_json, ApiResult, Shared};
use crate::config::Allowance;
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

pub(super) const MAX_DISPLAY_NAME_CHARS: usize = 60;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailBody {
    email: String,
    /// What the visitor asked for. Left out, it is a sign-up.
    #[serde(default)]
    intent: Intent,
}

/// Signing up makes the account if there is none; logging in never makes one.
#[derive(Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Intent {
    #[default]
    SignUp,
    LogIn,
}

/// Lower-cased address if it looks like one. The real check is that the link arrives.
pub(super) fn normalise_email(raw: &str) -> Option<String> {
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

/// Emails a one-time link that logs the visitor in. Answers 202 whether or not the address has an account:
/// someone who asks to log in with an address that has none is told so in the email, not on the page,
/// so the page cannot be used to find out who has an account.
pub async fn request_link(State(state): Shared, body: Result<Bytes, BytesRejection>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let req: EmailBody = parse_json(body)?;
    let Some(email) = normalise_email(&req.email) else {
        return Err(ApiError::invalid("Enter a valid email address", vec![lilanaya_engine::Issue::new("email", "is not a valid email address")]));
    };
    let token = new_token();
    // Counted against the address's limit whichever email goes out.
    if !state.store.create_sign_in_token(&email, &hash_token(&token)).await? {
        return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", "Too many links were asked for this address. Try again in 15 minutes."));
    }
    let site = &state.config.public_url;
    let known = state.store.user_exists(&email).await?;
    let (subject, text) = if req.intent == Intent::LogIn && !known {
        (
            "No Lilanaya account for this address yet",
            format!("Someone asked to log in to Lilanaya with this address, but it has no account.\n\nTo make one, sign up here:\n\n{site}/sign-up\n\nIf that was not you, ignore this email: nothing was changed.\n"),
        )
    } else {
        let link = format!("{site}/log-in/confirm?token={token}");
        let (subject, what) = if known { ("Your Lilanaya log-in link", "log in to Lilanaya") } else { ("Finish signing up to Lilanaya", "finish signing up to Lilanaya") };
        (subject, format!("Open this link to {what}:\n\n{link}\n\nIt works once and for {SIGN_IN_LINK_MINUTES} minutes. If you did not ask for it, ignore this email: nobody can get in without the link.\n"))
    };
    state.mailer.send(&email, subject, &text).await.map_err(|e| ApiError::internal(format!("send log-in email: {e}")))?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "sent": true, "expires_in_minutes": SIGN_IN_LINK_MINUTES }))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmBody {
    token: String,
}

/// Trades the token from the emailed link for a session cookie; creates the user if this is a sign-up.
pub async fn confirm_link(State(state): Shared, body: Result<Bytes, BytesRejection>) -> Result<Response, ApiError> {
    let req: ConfirmBody = parse_json(body)?;
    let Some(email) = state.store.consume_sign_in_token(&hash_token(req.token.trim())).await? else {
        return Err(ApiError::bad_request("invalid_token", "This link has expired or was already used. Ask for a new one."));
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

/// Everything we hold about the signed-in user, as a file to keep.
pub async fn export(State(state): Shared, user: User) -> Result<Response, ApiError> {
    let data = state.store.export_user(user.id).await?;
    let headers = [(header::CONTENT_TYPE, "application/json"), (header::CONTENT_DISPOSITION, "attachment; filename=\"lilanaya-my-data.json\""), (header::CACHE_CONTROL, "no-store")];
    Ok((headers, serde_json::to_string_pretty(&data).unwrap_or_default()).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteBody {
    /// The account's own address, typed out: deleting cannot happen by a slip of the hand.
    email: String,
}

/// Deletes the signed-in user's account for good. A running subscription is ended at Stripe first,
/// so nobody is charged for an account that is gone; if that cannot be done, nothing is deleted.
pub async fn delete_me(State(state): Shared, user: User, body: Result<Bytes, BytesRejection>) -> Result<Response, ApiError> {
    let req: DeleteBody = parse_json(body)?;
    if normalise_email(&req.email).as_deref() != Some(user.email.as_str()) {
        return Err(ApiError::invalid("Type your own email address to delete the account", vec![lilanaya_engine::Issue::new("email", "is not this account's address")]));
    }
    if let Some(sub) = state.store.subscription(user.id).await?.filter(|s| !matches!(s.status.as_str(), "canceled" | "incomplete_expired")) {
        let Some(billing) = &state.billing else {
            return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "payment_unavailable", "Your subscription could not be ended, so the account was not deleted. Try again later."));
        };
        if let Err(e) = billing.cancel_subscription(&sub.stripe_id).await {
            eprintln!("billing: could not end subscription of user {} before deleting the account: {e}", user.id);
            return Err(ApiError::new(StatusCode::BAD_GATEWAY, "payment_unavailable", "Your subscription could not be ended, so the account was not deleted. Try again in a moment."));
        }
    }
    state.store.delete_user(user.id).await?;
    let cookie = session_cookie(None, state.config.secure_cookies());
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "deleted": true }))).into_response())
}

pub async fn entitlements(user: User) -> ApiResult {
    Ok(Json(entitlements_json(&user)))
}

pub async fn usage(State(state): Shared, user: User) -> ApiResult {
    Ok(Json(usage_json(&state, &user).await?))
}

/// What each plan includes, for the sign-up page. Public: it says nothing about any user.
pub async fn plans(State(state): Shared) -> ApiResult {
    let config = &state.config;
    let plan = |name: &str| {
        let (limit, per) = match config.ai_allowance(name) {
            Allowance::PerWorld(n) => (n, "world"),
            Allowance::PerMonth(n) => (n, "month"),
        };
        json!({ "ai_requests": { "limit": limit, "per": per }, "monthly_runs": config.monthly_runs(name), "max_active_runs": config.max_active_runs })
    };
    // What is for sale and at what price, as it stands at Stripe. Null when payment is off or Stripe cannot be read.
    let (prices, tax) = match &state.billing {
        Some(billing) => (billing.prices().await, billing.tax()),
        None => (None, false),
    };
    Ok(Json(json!({
        "free": plan("free"), "subscriber": plan("subscriber"),
        "billing": prices.map(|p| json!({ "subscription": p["subscription"], "packs": p["packs"], "tax_added": tax })),
    })))
}

/// What the plan unlocks.
fn entitlements_json(user: &User) -> Value {
    let subscriber = user.plan == "subscriber";
    json!({ "plan": user.plan, "analytics_access": subscriber, "quick_test_access": subscriber, "private_worlds": subscriber })
}

/// Runs started in the current period against the plan's allowance, and how many may wait or run at once.
/// The period is the paid month for a paying subscriber and the calendar month for everyone else.
async fn usage_json(state: &AppState, user: &User) -> Result<Value, ApiError> {
    let usage = state.store.usage(user.id).await?;
    let (period, used) = (&usage.period, usage.runs_used);
    let monthly = state.config.monthly_runs(&user.plan);
    // AI requests: `per` is "world" (that many in each world, for good) or "month" (that many a month, across worlds).
    let (by_world, this_month) = state.store.ai_requests_used(user.id).await?;
    let (limit, per) = match state.config.ai_allowance(&user.plan) {
        Allowance::PerWorld(n) => (n, "world"),
        Allowance::PerMonth(n) => (n, "month"),
    };
    Ok(json!({
        "period": period, "period_start": usage.period_start, "period_end": usage.period_end,
        "monthly_runs": monthly, "used": used, "remaining": (monthly - used).max(0), "enforced": true, "max_active_runs": state.config.max_active_runs,
        "ai_requests": { "limit": limit, "per": per, "used_by_world": by_world, "used_this_month": this_month },
        // What is left of the packs the user bought: used when the plan's own are used up.
        "packs": { "strategies_left": usage.pack_strategies_left, "runs_left": usage.pack_runs_left },
    }))
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
        "features": { "compile": state.llm.is_some(), "billing": state.billing.is_some() },
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
