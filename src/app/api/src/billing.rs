//! Payment through Stripe. No card data comes near this program: paying happens on
//! Stripe's own Checkout page, and changing a card or cancelling on Stripe's customer
//! portal. What we do is start those pages, and learn from Stripe what was paid.
//!
//! Nothing Stripe tells us in a webhook is taken at its word beyond "look at this
//! object": the object is read again from Stripe with our own key. That makes the
//! order messages arrive in, and the shape Stripe gives them, not matter.
//!
//! Settings (see `.env.example`):
//! - `STRIPE_SECRET_KEY`: unset switches payment off (nothing is offered for sale).
//! - `STRIPE_WEBHOOK_SECRET`: the signing secret of the webhook endpoint `{PUBLIC_URL}/api/billing/webhook`.
//! - `STRIPE_SUBSCRIPTION_PRICE`: the id of the monthly price (`price_…`).
//! - `STRIPE_PACK_SMALL_PRICE`, `STRIPE_PACK_MEDIUM_PRICE`, `STRIPE_PACK_LARGE_PRICE`: the price of each
//!   pack; one that is unset is not offered.
//! - `STRIPE_TAX`: `on` (default) has Stripe Tax add VAT at checkout; `off` charges the price as it is.
//! - `STRIPE_URL`: default `https://api.stripe.com`.
//!
//! The amounts are not in this program: they are the prices in Stripe, read from there to be shown.

use aws_lc_rs::hmac;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_URL: &str = "https://api.stripe.com";
/// The version of Stripe's API every request asks for, so answers keep the shape this code reads.
const API_VERSION: &str = "2024-06-20";
/// A webhook message older (or newer) than this is refused: a recorded one cannot be played back later.
pub const WEBHOOK_TOLERANCE_SECONDS: i64 = 300;
/// How long prices read from Stripe are shown before they are read again.
const PRICE_CACHE: Duration = Duration::from_secs(600);

/// A usage pack: strategies the AI may write and runs, added to what the plan gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pack {
    pub id: &'static str,
    pub strategies: i64,
    pub runs: i64,
}

/// The catalog (user, 2026-11-23). What a pack costs is its price in Stripe.
pub const CATALOG: [Pack; 3] = [
    Pack { id: "small", strategies: 10, runs: 100 },
    Pack { id: "medium", strategies: 20, runs: 200 },
    Pack { id: "large", strategies: 50, runs: 500 },
];

pub fn pack(id: &str) -> Option<Pack> {
    CATALOG.iter().copied().find(|p| p.id == id)
}

/// The word for the subscription wherever a thing for sale is named.
pub const SUBSCRIPTION: &str = "subscription";

#[derive(Debug)]
pub struct StripeError {
    /// Stripe's status, if it answered at all.
    pub status: Option<u16>,
    pub detail: String,
}

impl std::fmt::Display for StripeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(status) => write!(f, "Stripe answered {status}: {}", self.detail),
            None => write!(f, "Stripe could not be reached: {}", self.detail),
        }
    }
}

/// A Checkout session as read from Stripe.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    /// Our user, as we wrote it on the session when we made it.
    pub user_id: Option<i64>,
    pub customer: Option<String>,
    /// What was bought: `subscription` or a pack's id.
    pub item: Option<String>,
    pub paid: bool,
    pub subscription: Option<String>,
    pub payment_intent: Option<String>,
    pub amount: i64,
    pub currency: String,
}

/// A subscription as read from Stripe.
#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    pub id: String,
    pub customer: Option<String>,
    pub status: String,
    /// Unix seconds.
    pub period_start: i64,
    pub period_end: i64,
    pub cancel_at_period_end: bool,
    /// The invoice for this period has been paid.
    pub paid: bool,
}

#[derive(Clone)]
pub struct Billing {
    http: reqwest::Client,
    base: String,
    key: String,
    webhook_secret: String,
    subscription_price: String,
    /// Pack id and its price at Stripe, for the packs that are offered.
    pack_prices: Vec<(&'static str, String)>,
    tax: bool,
    prices: Arc<Mutex<Option<(Instant, Value)>>>,
}

/// Only letters, digits and underscores: what Stripe's ids are made of. Anything else is not put in an address.
fn is_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 255 && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        // An expanded object: its id.
        Value::Object(o) => o.get("id").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

impl Billing {
    /// `Ok(None)` when no key is set: payment is switched off.
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::read(|name| std::env::var(name).ok())
    }

    fn read(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let var = |name: &str| lookup(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let Some(key) = var("STRIPE_SECRET_KEY") else {
            return Ok(None);
        };
        let need = |name: &str| var(name).ok_or_else(|| format!("{name} must be set when STRIPE_SECRET_KEY is (see .env.example)"));
        let price = |name: &str, value: String| if is_id(&value) && value.starts_with("price_") { Ok(value) } else { Err(format!("{name} must be the id of a price at Stripe (price_…), got \"{value}\"")) };
        let webhook_secret = need("STRIPE_WEBHOOK_SECRET")?;
        let subscription_price = price("STRIPE_SUBSCRIPTION_PRICE", need("STRIPE_SUBSCRIPTION_PRICE")?)?;
        let mut pack_prices = Vec::new();
        for (pack, name) in [("small", "STRIPE_PACK_SMALL_PRICE"), ("medium", "STRIPE_PACK_MEDIUM_PRICE"), ("large", "STRIPE_PACK_LARGE_PRICE")] {
            if let Some(value) = var(name) {
                pack_prices.push((pack, price(name, value)?));
            }
        }
        let tax = match var("STRIPE_TAX").map(|v| v.to_lowercase()).as_deref() {
            None | Some("on") | Some("true") | Some("1") => true,
            Some("off") | Some("false") | Some("0") => false,
            Some(other) => return Err(format!("STRIPE_TAX must be on or off, got \"{other}\"")),
        };
        Self::new(&var("STRIPE_URL").unwrap_or_else(|| DEFAULT_URL.into()), key, webhook_secret, subscription_price, pack_prices, tax).map(Some)
    }

    pub fn new(base: &str, key: String, webhook_secret: String, subscription_price: String, pack_prices: Vec<(&'static str, String)>, tax: bool) -> Result<Self, String> {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| format!("could not set up the HTTP client for Stripe: {e}"))?;
        Ok(Billing { http, base: base.trim_end_matches('/').to_string(), key, webhook_secret, subscription_price, pack_prices, tax, prices: Arc::default() })
    }

    pub fn describe(&self) -> String {
        let packs: Vec<&str> = self.pack_prices.iter().map(|(pack, _)| *pack).collect();
        format!(
            "payment is on: Stripe ({}), the subscription and {}, VAT {}",
            if self.key.starts_with("sk_live_") { "live" } else { "test mode" },
            if packs.is_empty() { "no packs".to_string() } else { format!("the packs {}", packs.join(", ")) },
            if self.tax { "added by Stripe Tax" } else { "not added" }
        )
    }

    pub fn tax(&self) -> bool {
        self.tax
    }

    /// The price at Stripe of `item` (`subscription` or a pack's id), if it is offered.
    fn price_of(&self, item: &str) -> Option<&str> {
        if item == SUBSCRIPTION {
            return Some(&self.subscription_price);
        }
        self.pack_prices.iter().find(|(pack, _)| *pack == item).map(|(_, price)| price.as_str())
    }

    pub fn offers(&self, item: &str) -> bool {
        self.price_of(item).is_some()
    }

    async fn call(&self, method: reqwest::Method, path: &str, form: &[(&str, String)]) -> Result<Value, StripeError> {
        let encoded: String = form_urlencoded::Serializer::new(String::new()).extend_pairs(form.iter().map(|(k, v)| (*k, v.as_str()))).finish();
        let get = method == reqwest::Method::GET;
        let address = if get && !encoded.is_empty() { format!("{}{path}?{encoded}", self.base) } else { format!("{}{path}", self.base) };
        let mut request = self.http.request(method, address).bearer_auth(&self.key).header("Stripe-Version", API_VERSION);
        if !get {
            request = request.header("content-type", "application/x-www-form-urlencoded").body(encoded);
        }
        let response = request.send().await.map_err(|e| StripeError { status: None, detail: e.to_string() })?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            // Stripe's own words for what was wrong; never the request, which carries nothing secret but the key in a header.
            let message = serde_json::from_str::<Value>(&body).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_string)).unwrap_or_else(|| body.chars().take(300).collect());
            return Err(StripeError { status: Some(status.as_u16()), detail: message });
        }
        serde_json::from_str(&body).map_err(|e| StripeError { status: Some(status.as_u16()), detail: format!("the answer is not JSON: {e}") })
    }

    /// Makes the customer record a user pays as.
    pub async fn create_customer(&self, email: &str, user_id: i64) -> Result<String, StripeError> {
        let made = self.call(reqwest::Method::POST, "/v1/customers", &[("email", email.to_string()), ("metadata[user_id]", user_id.to_string())]).await?;
        text(&made["id"]).ok_or_else(|| StripeError { status: Some(200), detail: "the customer has no id".into() })
    }

    /// Starts a Checkout page for `item` and returns its address. The page comes back to `/plans`.
    pub async fn checkout(&self, item: &str, customer: &str, user_id: i64, public_url: &str) -> Result<String, StripeError> {
        let price = self.price_of(item).ok_or_else(|| StripeError { status: None, detail: format!("\"{item}\" is not for sale") })?;
        let subscription = item == SUBSCRIPTION;
        let mut form: Vec<(&str, String)> = vec![
            ("mode", if subscription { "subscription" } else { "payment" }.to_string()),
            ("customer", customer.to_string()),
            ("client_reference_id", user_id.to_string()),
            ("line_items[0][price]", price.to_string()),
            ("line_items[0][quantity]", "1".to_string()),
            // Stripe puts the session's id where the braces are.
            ("success_url", format!("{public_url}/plans?paid={{CHECKOUT_SESSION_ID}}")),
            ("cancel_url", format!("{public_url}/plans")),
            ("metadata[item]", item.to_string()),
            ("metadata[user_id]", user_id.to_string()),
            // Charge the price as it is shown on the site. Left on, Stripe converts it to the buyer's own currency and adds a fee.
            ("adaptive_pricing[enabled]", "false".to_string()),
            // Said again where the money is paid: the buyer asks for access now, and what is used by then is not refunded on withdrawal.
            ("custom_text[submit][message]", "You get access straight away. If you use your 14-day right of withdrawal, what you have already used is not refunded.".to_string()),
        ];
        if subscription {
            form.push(("subscription_data[metadata][user_id]", user_id.to_string()));
        } else {
            form.push(("payment_intent_data[metadata][user_id]", user_id.to_string()));
            // A proper invoice for a one-off payment too, for the buyer's and our own books.
            form.push(("invoice_creation[enabled]", "true".to_string()));
        }
        if self.tax {
            form.push(("automatic_tax[enabled]", "true".to_string()));
            form.push(("billing_address_collection", "required".to_string()));
            form.push(("customer_update[address]", "auto".to_string()));
            form.push(("customer_update[name]", "auto".to_string()));
            form.push(("tax_id_collection[enabled]", "true".to_string()));
        }
        let session = self.call(reqwest::Method::POST, "/v1/checkout/sessions", &form).await?;
        text(&session["url"]).ok_or_else(|| StripeError { status: Some(200), detail: "the Checkout session has no address".into() })
    }

    /// Starts the customer portal (change card, cancel, invoices) and returns its address.
    pub async fn portal(&self, customer: &str, public_url: &str) -> Result<String, StripeError> {
        let session = self.call(reqwest::Method::POST, "/v1/billing_portal/sessions", &[("customer", customer.to_string()), ("return_url", format!("{public_url}/profile"))]).await?;
        text(&session["url"]).ok_or_else(|| StripeError { status: Some(200), detail: "the portal session has no address".into() })
    }

    /// Reads a Checkout session. `Ok(None)` if Stripe knows of none with this id.
    pub async fn session(&self, id: &str) -> Result<Option<Session>, StripeError> {
        if !is_id(id) {
            return Ok(None);
        }
        let s = match self.call(reqwest::Method::GET, &format!("/v1/checkout/sessions/{id}"), &[]).await {
            Ok(s) => s,
            Err(StripeError { status: Some(404), .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(Session {
            id: id.to_string(),
            user_id: s["client_reference_id"].as_str().and_then(|v| v.parse().ok()),
            customer: text(&s["customer"]),
            item: text(&s["metadata"]["item"]),
            // `no_payment_required`: a coupon we gave took the whole price.
            paid: s["status"] == "complete" && matches!(s["payment_status"].as_str(), Some("paid" | "no_payment_required")),
            subscription: text(&s["subscription"]),
            payment_intent: text(&s["payment_intent"]),
            amount: s["amount_total"].as_i64().unwrap_or(0),
            currency: s["currency"].as_str().unwrap_or("").to_string(),
        }))
    }

    /// Reads a subscription. `Ok(None)` if Stripe knows of none with this id.
    pub async fn subscription(&self, id: &str) -> Result<Option<Subscription>, StripeError> {
        if !is_id(id) {
            return Ok(None);
        }
        let s = match self.call(reqwest::Method::GET, &format!("/v1/subscriptions/{id}"), &[("expand[]", "latest_invoice".to_string())]).await {
            Ok(s) => s,
            Err(StripeError { status: Some(404), .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        // Newer versions of Stripe's API keep the period on the subscription's item.
        let period = |name: &str| s[name].as_i64().or_else(|| s["items"]["data"][0][name].as_i64()).unwrap_or(0);
        Ok(Some(Subscription {
            id: id.to_string(),
            customer: text(&s["customer"]),
            status: s["status"].as_str().unwrap_or("unknown").to_string(),
            period_start: period("current_period_start"),
            period_end: period("current_period_end"),
            cancel_at_period_end: s["cancel_at_period_end"].as_bool().unwrap_or(false) || s["cancel_at"].is_i64(),
            paid: s["latest_invoice"]["status"] == "paid",
        }))
    }

    /// Ends a subscription now (used when an account is deleted). One that is already gone is fine.
    pub async fn cancel_subscription(&self, id: &str) -> Result<(), StripeError> {
        if !is_id(id) {
            return Ok(());
        }
        match self.call(reqwest::Method::DELETE, &format!("/v1/subscriptions/{id}"), &[]).await {
            Ok(_) | Err(StripeError { status: Some(404), .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// What is for sale and what it costs, as the prices stand at Stripe:
    /// `{"subscription": {"amount", "currency", "interval"}, "packs": [{"id", "strategies", "runs", "amount", "currency"}]}`.
    /// Amounts are in the currency's smallest unit. `None` if Stripe cannot be read and nothing is remembered.
    pub async fn prices(&self) -> Option<Value> {
        let remembered = self.prices.lock().unwrap().clone();
        if let Some((at, prices)) = &remembered {
            if at.elapsed() < PRICE_CACHE {
                return Some(prices.clone());
            }
        }
        match self.read_prices().await {
            Ok(prices) => {
                *self.prices.lock().unwrap() = Some((Instant::now(), prices.clone()));
                Some(prices)
            }
            Err(e) => {
                eprintln!("billing: could not read the prices: {e}");
                // Better yesterday's price list than none.
                remembered.map(|(_, prices)| prices)
            }
        }
    }

    async fn read_prices(&self) -> Result<Value, StripeError> {
        let read = |id: String| async move {
            let price = self.call(reqwest::Method::GET, &format!("/v1/prices/{id}"), &[]).await?;
            let amount = price["unit_amount"].as_i64().ok_or_else(|| StripeError { status: Some(200), detail: format!("the price {id} has no fixed amount") })?;
            Ok::<_, StripeError>(json!({ "amount": amount, "currency": price["currency"].as_str().unwrap_or(""), "interval": price["recurring"]["interval"] }))
        };
        let subscription = read(self.subscription_price.clone()).await?;
        if subscription["interval"].is_null() {
            return Err(StripeError { status: Some(200), detail: "STRIPE_SUBSCRIPTION_PRICE is a one-off price; it must be a recurring one".into() });
        }
        let mut packs = Vec::new();
        for (id, price) in &self.pack_prices {
            let pack = pack(id).expect("offered packs are in the catalog");
            let price = read(price.clone()).await?;
            if !price["interval"].is_null() {
                return Err(StripeError { status: Some(200), detail: format!("the price of the {id} pack is a recurring one; it must be one-off") });
            }
            packs.push(json!({ "id": pack.id, "strategies": pack.strategies, "runs": pack.runs, "amount": price["amount"], "currency": price["currency"] }));
        }
        Ok(json!({ "subscription": subscription, "packs": packs }))
    }

    /// Whether `body` was signed by Stripe with our webhook secret at about `now` (Unix seconds).
    /// `header` is the `Stripe-Signature` header: `t=<time>,v1=<signature>[,v1=…]`.
    pub fn verify(&self, header: &str, body: &[u8], now: i64) -> Result<(), &'static str> {
        let mut time = None;
        let mut signatures = Vec::new();
        for part in header.split(',') {
            match part.trim().split_once('=') {
                Some(("t", value)) => time = value.parse::<i64>().ok(),
                Some(("v1", value)) => signatures.push(value),
                _ => {}
            }
        }
        let time = time.ok_or("the signature has no time")?;
        if (now - time).abs() > WEBHOOK_TOLERANCE_SECONDS {
            return Err("the message is too old");
        }
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.webhook_secret.as_bytes());
        let mut signed = format!("{time}.").into_bytes();
        signed.extend_from_slice(body);
        let matches = signatures.iter().filter_map(|s| unhex(s)).any(|tag| hmac::verify(&key, &signed, &tag).is_ok());
        if matches {
            Ok(())
        } else {
            Err("the signature does not match")
        }
    }
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::collections::BTreeMap;

    pub const TEST_WEBHOOK_SECRET: &str = "whsec_test";

    /// The `Stripe-Signature` header Stripe would send for `body` at `time`, signed with `secret`.
    pub fn signature(secret: &str, body: &str, time: i64) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let tag = hmac::sign(&key, format!("{time}.{body}").as_bytes());
        format!("t={time},v1={}", tag.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    /// A stand-in for Stripe. It keeps the objects tests put in it and hands them out as
    /// Stripe would; what the program sends to make a customer, a Checkout page or a portal
    /// page is recorded in `seen`.
    #[derive(Clone, Default)]
    pub struct FakeStripe {
        pub sessions: Arc<Mutex<BTreeMap<String, Value>>>,
        pub subscriptions: Arc<Mutex<BTreeMap<String, Value>>>,
        /// (path, form) of every POST and DELETE.
        pub seen: Arc<Mutex<Vec<(String, BTreeMap<String, String>)>>>,
        /// Answer everything with this status instead (an outage).
        pub broken: Arc<Mutex<Option<u16>>>,
    }

    type Answer = (StatusCode, Json<Value>);

    fn missing() -> Answer {
        (StatusCode::NOT_FOUND, Json(json!({ "error": { "message": "No such object" } })))
    }

    impl FakeStripe {
        fn down(&self, headers: &HeaderMap) -> Option<Answer> {
            if headers.get("authorization").is_none_or(|v| v != "Bearer sk_test_key") || headers.get("stripe-version").is_none() {
                return Some((StatusCode::UNAUTHORIZED, Json(json!({ "error": { "message": "Invalid API Key provided" } }))));
            }
            self.broken.lock().unwrap().map(|status| (StatusCode::from_u16(status).unwrap(), Json(json!({ "error": { "message": "Stripe is having a bad day" } }))))
        }

        fn record(&self, path: &str, body: &str) -> BTreeMap<String, String> {
            let form: BTreeMap<String, String> = form_urlencoded::parse(body.as_bytes()).into_owned().collect();
            self.seen.lock().unwrap().push((path.to_string(), form.clone()));
            form
        }

        /// The forms posted to `path`, oldest first.
        pub fn posted(&self, path: &str) -> Vec<BTreeMap<String, String>> {
            self.seen.lock().unwrap().iter().filter(|(p, _)| p == path).map(|(_, form)| form.clone()).collect()
        }

        /// Puts a subscription in Stripe's books. `invoice` is the status of its latest invoice.
        pub fn put_subscription(&self, id: &str, customer: &str, status: &str, start: i64, end: i64, invoice: &str) {
            self.subscriptions.lock().unwrap().insert(
                id.to_string(),
                json!({ "id": id, "object": "subscription", "customer": customer, "status": status, "current_period_start": start, "current_period_end": end, "cancel_at_period_end": false, "latest_invoice": { "id": "in_1", "status": invoice } }),
            );
        }

        /// The buyer pays on Stripe's page: the session is complete and paid. A subscription's session
        /// names the subscription; a pack's names the payment.
        pub fn pay(&self, session: &str, subscription: Option<&str>, payment_intent: Option<&str>, amount: i64) {
            let mut sessions = self.sessions.lock().unwrap();
            let s = sessions.get_mut(session).expect("the session was started");
            s["status"] = json!("complete");
            s["payment_status"] = json!("paid");
            s["subscription"] = json!(subscription);
            s["payment_intent"] = json!(payment_intent);
            s["amount_total"] = json!(amount);
        }

        /// Starts listening on a free local port; returns the base URL.
        pub async fn start(&self) -> String {
            async fn customers(State(fake): State<FakeStripe>, headers: HeaderMap, body: String) -> Answer {
                if let Some(answer) = fake.down(&headers) {
                    return answer;
                }
                let form = fake.record("/v1/customers", &body);
                (StatusCode::OK, Json(json!({ "id": format!("cus_{}", form["metadata[user_id]"]), "object": "customer" })))
            }
            async fn checkout(State(fake): State<FakeStripe>, headers: HeaderMap, body: String) -> Answer {
                if let Some(answer) = fake.down(&headers) {
                    return answer;
                }
                let form = fake.record("/v1/checkout/sessions", &body);
                let id = format!("cs_test_{}", fake.sessions.lock().unwrap().len() + 1);
                // An open session, as Stripe makes it: nothing is paid yet.
                let session = json!({
                    "id": id, "object": "checkout.session", "status": "open", "payment_status": "unpaid", "mode": form["mode"],
                    "customer": form["customer"], "client_reference_id": form["client_reference_id"], "metadata": { "item": form["metadata[item]"], "user_id": form["metadata[user_id]"] },
                    "subscription": null, "payment_intent": null, "amount_total": 0, "currency": "eur", "url": format!("https://checkout.stripe.test/{id}"),
                });
                fake.sessions.lock().unwrap().insert(id, session.clone());
                (StatusCode::OK, Json(session))
            }
            async fn session(State(fake): State<FakeStripe>, headers: HeaderMap, Path(id): Path<String>) -> Answer {
                if let Some(answer) = fake.down(&headers) {
                    return answer;
                }
                fake.sessions.lock().unwrap().get(&id).map_or_else(missing, |s| (StatusCode::OK, Json(s.clone())))
            }
            async fn subscription(State(fake): State<FakeStripe>, headers: HeaderMap, Path(id): Path<String>) -> Answer {
                if let Some(answer) = fake.down(&headers) {
                    return answer;
                }
                fake.subscriptions.lock().unwrap().get(&id).map_or_else(missing, |s| (StatusCode::OK, Json(s.clone())))
            }
            async fn cancel(State(fake): State<FakeStripe>, headers: HeaderMap, Path(id): Path<String>) -> Answer {
                if let Some(answer) = fake.down(&headers) {
                    return answer;
                }
                fake.record(&format!("DELETE /v1/subscriptions/{id}"), "");
                match fake.subscriptions.lock().unwrap().get_mut(&id) {
                    Some(s) => {
                        s["status"] = json!("canceled");
                        (StatusCode::OK, Json(s.clone()))
                    }
                    None => missing(),
                }
            }
            async fn portal(State(fake): State<FakeStripe>, headers: HeaderMap, body: String) -> Answer {
                if let Some(answer) = fake.down(&headers) {
                    return answer;
                }
                let form = fake.record("/v1/billing_portal/sessions", &body);
                (StatusCode::OK, Json(json!({ "id": "bps_1", "url": format!("https://billing.stripe.test/{}", form["customer"]) })))
            }
            async fn price(State(fake): State<FakeStripe>, headers: HeaderMap, Path(id): Path<String>) -> Answer {
                if let Some(answer) = fake.down(&headers) {
                    return answer;
                }
                let (amount, recurring) = match id.as_str() {
                    "price_sub" => (899, json!({ "interval": "month" })),
                    "price_small" => (599, Value::Null),
                    "price_medium" => (999, Value::Null),
                    "price_large" => (2199, Value::Null),
                    _ => return missing(),
                };
                (StatusCode::OK, Json(json!({ "id": id, "object": "price", "unit_amount": amount, "currency": "eur", "recurring": recurring })))
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new()
                .route("/v1/customers", post(customers))
                .route("/v1/checkout/sessions", post(checkout))
                .route("/v1/checkout/sessions/{id}", get(session))
                .route("/v1/subscriptions/{id}", get(subscription).delete(cancel))
                .route("/v1/billing_portal/sessions", post(portal))
                .route("/v1/prices/{id}", get(price))
                .with_state(self.clone());
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            format!("http://{address}")
        }

        /// A client of this stand-in, with the subscription and all three packs for sale.
        pub async fn billing(&self) -> Billing {
            let packs = vec![("small", "price_small".to_string()), ("medium", "price_medium".to_string()), ("large", "price_large".to_string())];
            Billing::new(&self.start().await, "sk_test_key".into(), TEST_WEBHOOK_SECRET.into(), "price_sub".into(), packs, true).unwrap()
        }
    }

    fn read(settings: &[(&str, &str)]) -> Result<Option<Billing>, String> {
        Billing::read(|name| settings.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string()))
    }

    fn plain() -> Billing {
        Billing::new("http://stripe.invalid/", "sk_test_key".into(), TEST_WEBHOOK_SECRET.into(), "price_sub".into(), vec![("small", "price_small".into())], false).unwrap()
    }

    #[test]
    fn the_catalog_is_the_three_packs_agreed() {
        assert_eq!(CATALOG.map(|p| (p.id, p.strategies, p.runs)), [("small", 10, 100), ("medium", 20, 200), ("large", 50, 500)]);
        assert_eq!(pack("medium"), Some(CATALOG[1]));
        assert_eq!(pack("subscription"), None);
        assert_eq!(pack("huge"), None);
    }

    #[test]
    fn without_a_key_payment_is_off_and_with_one_everything_it_needs_must_be_there() {
        assert!(read(&[]).unwrap().is_none());
        // Compose passes unset settings as empty strings.
        assert!(read(&[("STRIPE_SECRET_KEY", " "), ("STRIPE_WEBHOOK_SECRET", "whsec_x")]).unwrap().is_none());
        let key = ("STRIPE_SECRET_KEY", "sk_test_1");
        assert!(read(&[key]).err().unwrap().contains("STRIPE_WEBHOOK_SECRET"));
        assert!(read(&[key, ("STRIPE_WEBHOOK_SECRET", "whsec_x")]).err().unwrap().contains("STRIPE_SUBSCRIPTION_PRICE"));
        let base = [key, ("STRIPE_WEBHOOK_SECRET", "whsec_x"), ("STRIPE_SUBSCRIPTION_PRICE", "price_1")];
        let billing = read(&base).unwrap().unwrap();
        assert!(billing.tax(), "VAT is added unless it is switched off");
        assert!(billing.offers("subscription") && !billing.offers("small") && !billing.offers("nonsense"));
        assert_eq!(billing.base, "https://api.stripe.com");

        let with = |extra: (&'static str, &'static str)| read(&[base[0], base[1], base[2], extra]);
        let billing = with(("STRIPE_PACK_MEDIUM_PRICE", "price_m")).unwrap().unwrap();
        assert!(billing.offers("medium") && !billing.offers("small") && !billing.offers("large"));
        assert!(!with(("STRIPE_TAX", "off")).unwrap().unwrap().tax());
        assert!(with(("STRIPE_TAX", "maybe")).err().unwrap().contains("STRIPE_TAX"));
        // A product's id, or an amount, where a price's id belongs.
        assert!(with(("STRIPE_PACK_SMALL_PRICE", "prod_1")).err().unwrap().contains("STRIPE_PACK_SMALL_PRICE"));
        assert!(with(("STRIPE_PACK_LARGE_PRICE", "21.99")).err().unwrap().contains("STRIPE_PACK_LARGE_PRICE"));
        assert!(read(&[key, ("STRIPE_WEBHOOK_SECRET", "whsec_x"), ("STRIPE_SUBSCRIPTION_PRICE", "8.99")]).err().unwrap().contains("STRIPE_SUBSCRIPTION_PRICE"));
    }

    #[test]
    fn says_whether_it_is_live_without_saying_the_key() {
        let said = plain().describe();
        assert!(said.contains("test mode") && said.contains("small") && said.contains("not added"), "{said}");
        assert!(!said.contains("sk_test_key"));
    }

    #[test]
    fn a_message_is_accepted_only_with_stripe_s_signature_over_exactly_this_body_and_time() {
        let billing = plain();
        let body = r#"{"id":"evt_1","type":"checkout.session.completed"}"#;
        let now = 1_800_000_000;
        let good = signature(TEST_WEBHOOK_SECRET, body, now);
        assert_eq!(billing.verify(&good, body.as_bytes(), now), Ok(()));
        // Stripe sends two signatures while a secret is being changed: either one will do.
        let rolled = format!("{},v1={}", signature("whsec_old", body, now), good.split_once("v1=").unwrap().1);
        assert_eq!(billing.verify(&rolled, body.as_bytes(), now), Ok(()));

        let no = "the signature does not match";
        // Signed with another secret, over another body, or for another time.
        assert_eq!(billing.verify(&signature("whsec_other", body, now), body.as_bytes(), now), Err(no));
        assert_eq!(billing.verify(&good, body.replace("evt_1", "evt_2").as_bytes(), now), Err(no));
        assert_eq!(billing.verify(&good.replace(&format!("t={now}"), &format!("t={}", now + 1)), body.as_bytes(), now), Err(no));
        // Only the old scheme's signature, nonsense, or nothing.
        assert_eq!(billing.verify(&good.replace("v1=", "v0="), body.as_bytes(), now), Err(no));
        assert_eq!(billing.verify(&format!("t={now},v1=zz"), body.as_bytes(), now), Err(no));
        assert_eq!(billing.verify(&format!("t={now},v1="), body.as_bytes(), now), Err(no));
        assert_eq!(billing.verify(&format!("t={now}"), body.as_bytes(), now), Err(no));
        assert_eq!(billing.verify("", body.as_bytes(), now), Err("the signature has no time"));
        assert_eq!(billing.verify("v1=abcd", body.as_bytes(), now), Err("the signature has no time"));
    }

    #[test]
    fn a_recorded_message_cannot_be_played_back_later() {
        let billing = plain();
        let body = "{}";
        let sent = 1_800_000_000;
        let header = signature(TEST_WEBHOOK_SECRET, body, sent);
        assert_eq!(WEBHOOK_TOLERANCE_SECONDS, 300);
        assert_eq!(billing.verify(&header, body.as_bytes(), sent + 300), Ok(()));
        assert_eq!(billing.verify(&header, body.as_bytes(), sent + 301), Err("the message is too old"));
        // Nor one dated in the future.
        assert_eq!(billing.verify(&header, body.as_bytes(), sent - 300), Ok(()));
        assert_eq!(billing.verify(&header, body.as_bytes(), sent - 301), Err("the message is too old"));
    }

    #[test]
    fn only_what_looks_like_an_id_is_put_in_an_address() {
        for good in ["cs_test_a1B2", "sub_1", "price_1Abc"] {
            assert!(is_id(good), "{good}");
        }
        for bad in ["", "cs_1/../../v1/customers", "cs 1", "cs_1?expand=x", "cs_1#", "sub_é", &"a".repeat(256)] {
            assert!(!is_id(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn what_is_read_from_stripe_is_what_stripe_holds() {
        let fake = FakeStripe::default();
        let billing = fake.billing().await;
        // A customer and a Checkout page for the subscription.
        let customer = billing.create_customer("ada@example.com", 7).await.unwrap();
        assert_eq!(customer, "cus_7");
        let url = billing.checkout("subscription", &customer, 7, "https://lilanaya.example").await.unwrap();
        assert_eq!(url, "https://checkout.stripe.test/cs_test_1");
        let form = &fake.posted("/v1/checkout/sessions")[0];
        assert_eq!((form["mode"].as_str(), form["customer"].as_str(), form["client_reference_id"].as_str()), ("subscription", "cus_7", "7"));
        assert_eq!((form["line_items[0][price]"].as_str(), form["line_items[0][quantity]"].as_str()), ("price_sub", "1"));
        assert_eq!(form["success_url"], "https://lilanaya.example/plans?paid={CHECKOUT_SESSION_ID}");
        assert_eq!(form["cancel_url"], "https://lilanaya.example/plans");
        assert_eq!((form["metadata[item]"].as_str(), form["automatic_tax[enabled]"].as_str()), ("subscription", "true"));
        assert!(form["custom_text[submit][message]"].contains("right of withdrawal"));
        assert_eq!(form["adaptive_pricing[enabled]"], "false");
        assert!(!form.contains_key("invoice_creation[enabled]"));

        // A pack is a one-off payment with an invoice.
        billing.checkout("large", &customer, 7, "https://lilanaya.example").await.unwrap();
        let form = &fake.posted("/v1/checkout/sessions")[1];
        assert_eq!((form["mode"].as_str(), form["line_items[0][price]"].as_str(), form["invoice_creation[enabled]"].as_str()), ("payment", "price_large", "true"));
        assert_eq!(form["adaptive_pricing[enabled]"], "false");
        assert!(billing.checkout("huge", &customer, 7, "https://lilanaya.example").await.is_err());

        // The session as it stands: open, not paid.
        let session = billing.session("cs_test_1").await.unwrap().unwrap();
        assert_eq!((session.user_id, session.customer.as_deref(), session.item.as_deref(), session.paid), (Some(7), Some("cus_7"), Some("subscription"), false));
        assert_eq!(billing.session("cs_test_99").await.unwrap(), None);
        assert_eq!(billing.session("../customers").await.unwrap(), None);

        fake.put_subscription("sub_1", "cus_7", "active", 100, 200, "paid");
        let sub = billing.subscription("sub_1").await.unwrap().unwrap();
        assert_eq!((sub.status.as_str(), sub.period_start, sub.period_end, sub.paid, sub.cancel_at_period_end), ("active", 100, 200, true, false));
        assert_eq!(billing.subscription("sub_2").await.unwrap(), None);
        billing.cancel_subscription("sub_1").await.unwrap();
        assert_eq!(billing.subscription("sub_1").await.unwrap().unwrap().status, "canceled");
        billing.cancel_subscription("sub_gone").await.unwrap();

        assert_eq!(billing.portal("cus_7", "https://lilanaya.example").await.unwrap(), "https://billing.stripe.test/cus_7");
        assert_eq!(fake.posted("/v1/billing_portal/sessions")[0]["return_url"], "https://lilanaya.example/profile");
    }

    #[tokio::test]
    async fn the_period_is_read_from_the_item_when_stripe_keeps_it_there() {
        let fake = FakeStripe::default();
        let billing = fake.billing().await;
        fake.subscriptions.lock().unwrap().insert("sub_new".into(), json!({ "id": "sub_new", "customer": { "id": "cus_1" }, "status": "active", "cancel_at": 500, "items": { "data": [{ "current_period_start": 300, "current_period_end": 400 }] }, "latest_invoice": { "status": "open" } }));
        let sub = billing.subscription("sub_new").await.unwrap().unwrap();
        assert_eq!((sub.customer.as_deref(), sub.period_start, sub.period_end, sub.paid, sub.cancel_at_period_end), (Some("cus_1"), 300, 400, false, true));
    }

    #[tokio::test]
    async fn prices_come_from_stripe_and_are_remembered_through_an_outage() {
        let fake = FakeStripe::default();
        let billing = fake.billing().await;
        *fake.broken.lock().unwrap() = Some(503);
        assert_eq!(billing.prices().await, None, "nothing to show before Stripe has been read once");
        *fake.broken.lock().unwrap() = None;
        let prices = billing.prices().await.unwrap();
        assert_eq!(prices["subscription"], json!({ "amount": 899, "currency": "eur", "interval": "month" }));
        assert_eq!(prices["packs"], json!([
            { "id": "small", "strategies": 10, "runs": 100, "amount": 599, "currency": "eur" },
            { "id": "medium", "strategies": 20, "runs": 200, "amount": 999, "currency": "eur" },
            { "id": "large", "strategies": 50, "runs": 500, "amount": 2199, "currency": "eur" },
        ]));
        *fake.broken.lock().unwrap() = Some(503);
        assert_eq!(billing.prices().await, Some(prices));
    }

    #[tokio::test]
    async fn a_one_off_price_cannot_stand_in_for_the_subscription_nor_the_other_way_round() {
        let fake = FakeStripe::default();
        let base = fake.start().await;
        let wrong = Billing::new(&base, "sk_test_key".into(), TEST_WEBHOOK_SECRET.into(), "price_small".into(), vec![], true).unwrap();
        assert_eq!(wrong.prices().await, None);
        let wrong = Billing::new(&base, "sk_test_key".into(), TEST_WEBHOOK_SECRET.into(), "price_sub".into(), vec![("small", "price_sub".into())], true).unwrap();
        assert_eq!(wrong.prices().await, None);
    }

    #[tokio::test]
    async fn a_refusal_from_stripe_is_told_in_stripe_s_words_and_an_outage_as_one() {
        let fake = FakeStripe::default();
        let base = fake.start().await;
        let wrong_key = Billing::new(&base, "sk_test_other".into(), TEST_WEBHOOK_SECRET.into(), "price_sub".into(), vec![], true).unwrap();
        let error = wrong_key.create_customer("ada@example.com", 1).await.unwrap_err();
        assert_eq!((error.status, error.detail.as_str()), (Some(401), "Invalid API Key provided"));
        let nowhere = Billing::new("http://127.0.0.1:9", "sk_test_key".into(), TEST_WEBHOOK_SECRET.into(), "price_sub".into(), vec![], true).unwrap();
        let error = nowhere.create_customer("ada@example.com", 1).await.unwrap_err();
        assert_eq!(error.status, None);
        assert!(error.to_string().starts_with("Stripe could not be reached"));
    }
}
