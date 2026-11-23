//! Paying: the subscription and the usage packs, through Stripe (`billing.rs`).
//!
//! The browser is sent to Stripe to pay and comes back to `/plans`. What was paid is
//! learnt from Stripe twice over: from its webhook, and from the browser coming back
//! with the session's id (`confirm`), whichever is first. Both read the session or the
//! subscription from Stripe itself and store what Stripe says, so doing it twice, or in
//! any order, ends in the same state.

use super::{parse_json, ApiResult, Shared};
use crate::auth::User;
use crate::billing::{self, Billing, Session, StripeError, SUBSCRIPTION};
use crate::error::ApiError;
use crate::worker::AppState;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use lilanaya_engine::Issue;
use serde::Deserialize;
use serde_json::{json, Value};

fn switched_off() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "payment_unavailable", "Payment is not set up on this server.")
}

/// Stripe could not be used. The detail goes to the log; the user is told to try again.
fn unreachable(what: &str, e: StripeError) -> ApiError {
    eprintln!("billing: {what}: {e}");
    ApiError::new(StatusCode::BAD_GATEWAY, "payment_unavailable", "The payment service could not be reached. Nothing was charged. Try again in a moment.")
}

fn billing(state: &AppState) -> Result<&Billing, ApiError> {
    state.billing.as_ref().ok_or_else(switched_off)
}

/// What the signed-in user has paid for: their subscription, what is left of their packs, and what they bought.
pub async fn status(State(state): Shared, user: User) -> ApiResult {
    let subscription = state.store.subscription(user.id).await?;
    let usage = state.store.usage(user.id).await?;
    let purchases = state.store.purchases(user.id).await?;
    Ok(Json(json!({
        "enabled": state.billing.is_some(),
        "plan": user.plan,
        // Null for someone who has never subscribed. `paid` is what makes them a subscriber.
        "subscription": subscription.map(|s| json!({ "status": s.status, "paid": s.paid, "period_start": s.period_start, "period_end": s.period_end, "cancel_at_period_end": s.cancel_at_period_end })),
        "packs": { "strategies_left": usage.pack_strategies_left, "runs_left": usage.pack_runs_left },
        "purchases": purchases.iter().map(|p| json!({ "pack": p.pack, "strategies": p.strategies, "runs": p.runs, "amount": p.amount, "currency": p.currency, "created_at": p.created_at, "refunded": p.refunded })).collect::<Vec<_>>(),
        "can_manage": state.billing.is_some() && state.store.customer(user.id).await?.is_some(),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckoutBody {
    /// `subscription` or a pack's id.
    item: String,
    /// The buyer ticked the box: they accept the terms, ask for access to start now, and know that what they have used is not refunded if they withdraw.
    #[serde(default)]
    agree: bool,
}

/// Starts Stripe's Checkout page for the subscription or a pack and answers with its address.
pub async fn checkout(State(state): Shared, user: User, body: Result<Bytes, BytesRejection>) -> ApiResult {
    let req: CheckoutBody = parse_json(body)?;
    let billing = billing(&state)?;
    if req.item != SUBSCRIPTION && billing::pack(&req.item).is_none() {
        return Err(ApiError::invalid("That is not for sale", vec![Issue::new("item", "must be subscription, small, medium or large")]));
    }
    if !billing.offers(&req.item) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_for_sale", "That pack is not for sale at the moment."));
    }
    if !req.agree {
        return Err(ApiError::invalid("Agree to the terms to go on", vec![Issue::new("agree", "you must accept the terms, and that the service starts at once, before paying")]));
    }
    if req.item == SUBSCRIPTION && state.store.subscription(user.id).await?.is_some_and(|s| matches!(s.status.as_str(), "active" | "past_due" | "trialing" | "incomplete")) {
        return Err(ApiError::new(StatusCode::CONFLICT, "already_subscribed", "You already have a subscription. Manage it from your profile."));
    }
    let customer = match state.store.customer(user.id).await? {
        Some(customer) => customer,
        None => {
            let made = billing.create_customer(&user.email, user.id).await.map_err(|e| unreachable("create customer", e))?;
            state.store.keep_customer(user.id, &made).await?
        }
    };
    state.store.accept_terms(user.id).await?;
    let url = billing.checkout(&req.item, &customer, user.id, &state.config.public_url).await.map_err(|e| unreachable("start checkout", e))?;
    Ok(Json(json!({ "url": url })))
}

/// Starts Stripe's customer portal (change card, cancel, invoices) and answers with its address.
pub async fn portal(State(state): Shared, user: User) -> ApiResult {
    let billing = billing(&state)?;
    let Some(customer) = state.store.customer(user.id).await? else {
        return Err(ApiError::new(StatusCode::CONFLICT, "nothing_to_manage", "You have not bought anything yet."));
    };
    let url = billing.portal(&customer, &state.config.public_url).await.map_err(|e| unreachable("start portal", e))?;
    Ok(Json(json!({ "url": url })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmBody {
    session_id: String,
}

/// The browser is back from Stripe with a session's id: read the session from Stripe and store what was paid.
/// Somebody else's session, or one Stripe does not know, is "not found".
pub async fn confirm(State(state): Shared, user: User, body: Result<Bytes, BytesRejection>) -> ApiResult {
    let req: ConfirmBody = parse_json(body)?;
    let billing = billing(&state)?;
    let session = billing.session(req.session_id.trim()).await.map_err(|e| unreachable("read session", e))?;
    let Some(session) = session.filter(|s| s.user_id == Some(user.id)) else {
        return Err(ApiError::not_found("That payment"));
    };
    let applied = apply_session(&state, billing, &session).await?;
    Ok(Json(json!({ "paid": session.paid && applied, "item": session.item })))
}

/// Stores what a Checkout session paid for. False if it paid for nothing we can give: not paid yet,
/// not ours, or not the customer we made for this user.
async fn apply_session(state: &AppState, billing: &Billing, session: &Session) -> Result<bool, ApiError> {
    let (Some(user_id), Some(item)) = (session.user_id, session.item.as_deref()) else {
        return Ok(false);
    };
    if !session.paid {
        return Ok(false);
    }
    // The session must have been paid by the customer record we hold for this user.
    let customer = state.store.customer(user_id).await?;
    if customer.is_none() || customer != session.customer {
        eprintln!("billing: session {} names user {user_id} but another customer; ignored", session.id);
        return Ok(false);
    }
    if item == SUBSCRIPTION {
        let Some(id) = &session.subscription else {
            return Ok(false);
        };
        return sync_subscription(state, billing, user_id, id).await;
    }
    let Some(pack) = billing::pack(item) else {
        return Ok(false);
    };
    if state.store.record_purchase(user_id, &pack, session).await? {
        println!("billing: user {user_id} bought the {} pack", pack.id);
    }
    Ok(true)
}

/// Reads a subscription from Stripe and stores it for the user, if it is that user's.
async fn sync_subscription(state: &AppState, billing: &Billing, user_id: i64, id: &str) -> Result<bool, ApiError> {
    let Some(sub) = billing.subscription(id).await.map_err(|e| unreachable("read subscription", e))? else {
        return Ok(false);
    };
    let customer = state.store.customer(user_id).await?;
    if customer.is_none() || customer != sub.customer {
        eprintln!("billing: subscription {id} is not user {user_id}'s; ignored");
        return Ok(false);
    }
    state.store.save_subscription(user_id, &sub).await?;
    Ok(true)
}

/// Stripe's messages. Each is checked against Stripe's signature and its age, and acted on once.
/// An answer other than 2xx makes Stripe send the message again later, which is what we want
/// when Stripe or the database could not be reached.
pub async fn webhook(State(state): Shared, headers: HeaderMap, body: Bytes) -> ApiResult {
    let billing = billing(&state)?;
    let signature = headers.get("stripe-signature").and_then(|v| v.to_str().ok()).unwrap_or("");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    if let Err(why) = billing.verify(signature, &body, now) {
        eprintln!("billing: webhook refused: {why}");
        return Err(ApiError::bad_request("invalid_signature", "This message is not signed by Stripe"));
    }
    let event: Value = serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("invalid_json", "The message is not valid JSON"))?;
    let (Some(id), Some(kind)) = (event["id"].as_str(), event["type"].as_str()) else {
        return Err(ApiError::bad_request("invalid_event", "The message has no id or type"));
    };
    if state.store.event_seen(id).await? {
        return Ok(Json(json!({ "received": true, "duplicate": true })));
    }
    let object = &event["data"]["object"];
    let object_id = object["id"].as_str().unwrap_or("");
    // Whose it is: every object we care about names the customer.
    let user = match object["customer"].as_str() {
        Some(customer) => state.store.customer_user(customer).await?,
        None => None,
    };
    match kind {
        "checkout.session.completed" | "checkout.session.async_payment_succeeded" => {
            if let Some(session) = billing.session(object_id).await.map_err(|e| unreachable("read session", e))? {
                apply_session(&state, billing, &session).await?;
            }
        }
        "customer.subscription.created" | "customer.subscription.updated" | "customer.subscription.deleted" | "customer.subscription.paused" | "customer.subscription.resumed" => {
            if let Some(user_id) = user {
                sync_subscription(&state, billing, user_id, object_id).await?;
            }
        }
        // A renewal was paid, or was not: read the subscription again.
        "invoice.paid" | "invoice.payment_succeeded" | "invoice.payment_failed" => {
            if let Some(user_id) = user {
                if let Some(known) = state.store.subscription(user_id).await? {
                    sync_subscription(&state, billing, user_id, &known.stripe_id).await?;
                }
            }
        }
        // Money paid back for a pack takes the same share of the pack back; a payment disputed with the bank takes all of it.
        "charge.refunded" | "charge.dispute.created" => {
            let share = if kind == "charge.dispute.created" || object["refunded"] == true {
                1.0
            } else {
                match (object["amount_refunded"].as_f64(), object["amount"].as_f64()) {
                    (Some(back), Some(paid)) if paid > 0.0 => back / paid,
                    _ => 0.0,
                }
            };
            if let (true, Some(payment)) = (share > 0.0, object["payment_intent"].as_str()) {
                if state.store.refund_purchase(payment, share).await? {
                    println!("billing: {:.0}% of the payment {payment} for a pack was paid back, and that share of the pack with it", share * 100.0);
                }
            }
        }
        _ => {}
    }
    state.store.record_event(id, kind).await?;
    Ok(Json(json!({ "received": true })))
}
