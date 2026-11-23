//! Handler tests. They need Postgres (`DATABASE_URL`, set by compose): each
//! test gets its own schema and its own worker, so tests do not see each other.
//! Runs and previews start child processes (`isolate.rs`), as they do in the real program.

use super::*;
use crate::billing::tests::{signature, FakeStripe, TEST_WEBHOOK_SECRET};
use crate::billing::Billing;
use crate::config::{Allowance, Config};
use crate::llm::tests::{completion, verdict, FakeApi};
use crate::llm::Llm;
use crate::mail::{Mailer, Sent};
use crate::oauth::tests::{id_token, TEST_CLIENT_ID, TEST_SECRET};
use crate::oauth::OAuth;
use crate::store::{CompileGate, CompileLimits, Store};
use crate::worker;
use axum::body::Body;
use axum::http::{Method, Request};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tower::ServiceExt;

/// The API with its own schema and worker, seen through one
/// browser: `cookie` is that browser's session. `TestApp::new` starts signed in
/// as ada@example.com; `anonymous` and `signed_in_as` give other browsers.
#[derive(Clone)]
struct TestApp {
    router: Router,
    store: Store,
    schema: String,
    mail: Arc<Mutex<Vec<Sent>>>,
    cookie: Option<String>,
}

impl TestApp {
    async fn new() -> Self {
        Self::with_llm(None).await
    }

    async fn with_llm(llm: Option<Llm>) -> Self {
        Self::build(llm, OAuth::none(), None, Config::for_tests()).await
    }

    async fn with_config(config: Config) -> Self {
        Self::build(None, OAuth::none(), None, config).await
    }

    async fn build(llm: Option<Llm>, oauth: OAuth, billing: Option<Billing>, config: Config) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set: run the tests with `docker compose run --rm api cargo test`");
        // The random part keeps a schema left behind by an interrupted test run from ever being picked up again.
        let schema = format!("test_{}_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst), &crate::auth::new_token()[..8]);
        let store = Store::connect(&url, Some(&schema)).await.expect("connect to the test database");
        let mail = Arc::new(Mutex::new(Vec::new()));
        let state = AppState::new(store.clone(), Mailer::Memory(mail.clone()), llm, oauth, billing, config);
        worker::spawn(state.clone());
        let app = TestApp { router: app(state), store, schema, mail, cookie: None };
        app.signed_in_as("ada@example.com").await
    }

    async fn close(self) {
        self.store.drop_schema(&self.schema).await;
    }

    /// The same API seen by a browser with no session.
    fn anonymous(&self) -> Self {
        TestApp { cookie: None, ..self.clone() }
    }

    /// The token in the latest sign-in email.
    fn emailed_token(&self) -> String {
        let mail = self.mail.lock().unwrap();
        let body = &mail.last().expect("an email was sent").body;
        let start = body.find("token=").expect("the email has a link") + "token=".len();
        body[start..].chars().take_while(|c| c.is_ascii_hexdigit()).collect()
    }

    /// Goes through the emailed-link sign-in and returns a browser holding the session.
    async fn signed_in_as(&self, email: &str) -> Self {
        let (status, body) = self.anonymous().post("/api/auth/email", json!({ "email": email })).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let (status, headers, body) = self.anonymous().send(Method::POST, "/api/auth/email/confirm", Some(json!({ "token": self.emailed_token() }))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let set_cookie = headers["set-cookie"].to_str().unwrap();
        assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Lax"), "{set_cookie}");
        TestApp { cookie: Some(set_cookie.split(';').next().unwrap().to_string()), ..self.clone() }
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, axum::http::HeaderMap, Value) {
        self.raw(method, path, None, body.map(|v| v.to_string())).await
    }

    /// Any request; an empty answer (a redirect) reads as `null`.
    async fn raw(&self, method: Method, path: &str, content_type: Option<&str>, body: Option<String>) -> (StatusCode, axum::http::HeaderMap, Value) {
        let body = body.map_or_else(Body::empty, Body::from);
        let mut req = Request::builder().method(method).uri(path);
        if let Some(cookie) = &self.cookie {
            req = req.header("cookie", cookie);
        }
        if let Some(content_type) = content_type {
            req = req.header("content-type", content_type);
        }
        let res = self.router.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{path}: body is not JSON ({e}): {bytes:?}")) };
        (status, headers, value)
    }

    /// A message to the webhook, as Stripe sends it: the body with a `Stripe-Signature` header (or none).
    async fn webhook_raw(&self, body: &str, signature: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(Method::POST).uri("/api/billing/webhook").header("content-type", "application/json");
        if let Some(signature) = signature {
            req = req.header("stripe-signature", signature);
        }
        let res = self.router.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// A message from Stripe, properly signed just now: event `id` of `kind` about `object`.
    async fn stripe_says(&self, id: &str, kind: &str, object: Value) -> (StatusCode, Value) {
        let body = json!({ "id": id, "type": kind, "data": { "object": object } }).to_string();
        self.webhook_raw(&body, Some(&signature(TEST_WEBHOOK_SECRET, &body, unix_now()))).await
    }

    /// Presses "Subscribe" or "Buy": returns the id of the Checkout session Stripe started.
    async fn start_checkout(&self, item: &str) -> String {
        let (status, body) = self.post("/api/billing/checkout", json!({ "item": item, "agree": true })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["url"].as_str().unwrap().rsplit('/').next().unwrap().to_string()
    }

    async fn my_id(&self) -> i64 {
        self.get("/api/me").await.1["id"].as_i64().unwrap()
    }

    /// Buys a pack all the way: Checkout, payment, and Stripe's message about it. `n` keeps the ids apart.
    async fn buy_pack(&self, stripe: &FakeStripe, pack: &str, n: u32) -> String {
        let session = self.start_checkout(pack).await;
        stripe.pay(&session, None, Some(&format!("pi_{n}")), 599);
        let customer = format!("cus_{}", self.my_id().await);
        let (status, body) = self.stripe_says(&format!("evt_pack_{n}"), "checkout.session.completed", json!({ "id": session, "object": "checkout.session", "customer": customer })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        session
    }

    /// Subscribes all the way, paid from `start` to `end` (Unix seconds). The subscription at Stripe is `sub`.
    async fn subscribe(&self, stripe: &FakeStripe, sub: &str, start: i64, end: i64) {
        let session = self.start_checkout("subscription").await;
        let customer = format!("cus_{}", self.my_id().await);
        stripe.pay(&session, Some(sub), None, 899);
        stripe.put_subscription(sub, &customer, "active", start, end, "paid");
        let (status, body) = self.stripe_says(&format!("evt_{sub}"), "checkout.session.completed", json!({ "id": session, "object": "checkout.session", "customer": customer })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, _, value) = self.send(method, path, body).await;
        (status, value)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.call(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, path, Some(body)).await
    }

    /// Queues a run and waits for the worker to finish it; returns the finished run.
    async fn run(&self, body: Value) -> Value {
        let (status, queued) = self.post("/api/runs", body).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
        assert!(queued["status"] == "queued", "{queued}");
        let id = queued["id"].as_i64().unwrap();
        for _ in 0..1000 {
            let (_, run) = self.get(&format!("/api/runs/{id}")).await;
            match run["status"].as_str() {
                Some("completed") | Some("failed") => return run,
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        panic!("run {id} did not finish");
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

const DAY: i64 = 24 * 60 * 60;

/// The API with payment on, against a stand-in for Stripe, and optionally with the stand-in AI.
async fn app_with_fake_stripe(llm: Option<Llm>, config: Config) -> (TestApp, FakeStripe) {
    let stripe = FakeStripe::default();
    let billing = stripe.billing().await;
    (TestApp::build(llm, OAuth::none(), Some(billing), config).await, stripe)
}

fn pd(extra: Value) -> Value {
    let mut body = json!({ "world_id": "prisoners-dilemma" });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    body
}

/// Stands in for all four providers. `person` is whoever "approves" the next sign-in:
/// `{"sub", "email", "email_verified", "name"}`. `seen` keeps every token request (provider, form).
#[derive(Clone, Default)]
struct FakeProviders {
    person: Arc<Mutex<Value>>,
    seen: Arc<Mutex<Vec<(String, BTreeMap<String, String>)>>>,
}

impl FakeProviders {
    fn approve_as(&self, person: Value) {
        *self.person.lock().unwrap() = person;
    }

    /// Starts listening on a free local port; returns the base URL.
    async fn start(&self) -> String {
        async fn token(State(fake): State<FakeProviders>, Path(provider): Path<String>, body: String) -> (StatusCode, Json<Value>) {
            let form: BTreeMap<String, String> = form_urlencoded::parse(body.as_bytes()).into_owned().collect();
            fake.seen.lock().unwrap().push((provider.clone(), form.clone()));
            if form["code"] != "good-code" {
                // GitHub refuses with 200 and an error in the body; the others with 400.
                let status = if provider == "github" { StatusCode::OK } else { StatusCode::BAD_REQUEST };
                return (status, Json(json!({ "error": "invalid_grant" })));
            }
            let mut claims = fake.person.lock().unwrap().clone();
            claims["iss"] = json!(if provider == "apple" { "https://appleid.apple.com" } else { "https://accounts.google.com" });
            claims["aud"] = json!(form["client_id"]);
            claims["exp"] = json!(4_000_000_000u64);
            (StatusCode::OK, Json(json!({ "access_token": "access-1", "id_token": id_token(&claims) })))
        }
        fn authorised(headers: &axum::http::HeaderMap) -> bool {
            headers.get("authorization").is_some_and(|v| v == "Bearer access-1") && headers.contains_key("user-agent")
        }
        async fn github_user(State(fake): State<FakeProviders>, headers: axum::http::HeaderMap) -> (StatusCode, Json<Value>) {
            if !authorised(&headers) {
                return (StatusCode::UNAUTHORIZED, Json(json!({ "message": "Requires authentication" })));
            }
            let person = fake.person.lock().unwrap().clone();
            (StatusCode::OK, Json(json!({ "id": person["sub"].as_str().unwrap().parse::<i64>().unwrap(), "login": "octo", "name": person["name"] })))
        }
        async fn github_emails(State(fake): State<FakeProviders>, headers: axum::http::HeaderMap) -> (StatusCode, Json<Value>) {
            if !authorised(&headers) {
                return (StatusCode::UNAUTHORIZED, Json(json!({ "message": "Requires authentication" })));
            }
            let person = fake.person.lock().unwrap().clone();
            (StatusCode::OK, Json(json!([
                { "email": "octo@users.noreply.github.com", "primary": false, "verified": true },
                { "email": person["email"], "primary": true, "verified": person["email_verified"] },
            ])))
        }
        async fn facebook_me(State(fake): State<FakeProviders>, headers: axum::http::HeaderMap, Query(query): Query<BTreeMap<String, String>>) -> (StatusCode, Json<Value>) {
            if !authorised(&headers) || query.get("fields").map(String::as_str) != Some("id,name,email") {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": { "message": "bad request" } })));
            }
            let person = fake.person.lock().unwrap().clone();
            let mut me = json!({ "id": person["sub"], "name": person["name"] });
            if person["email_verified"] == true {
                me["email"] = person["email"].clone();
            }
            (StatusCode::OK, Json(me))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/{provider}/token", post(token))
            .route("/github/user", get(github_user))
            .route("/github/user/emails", get(github_emails))
            .route("/facebook/me", get(facebook_me))
            .with_state(self.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }
}

async fn app_with_fake_providers() -> (TestApp, FakeProviders, String) {
    let fake = FakeProviders::default();
    let base = fake.start().await;
    (TestApp::build(None, OAuth::for_tests(&base), None, Config::for_tests()).await, fake, base)
}

fn location(headers: &axum::http::HeaderMap) -> String {
    headers["location"].to_str().unwrap().to_string()
}

fn query_of(url: &str) -> BTreeMap<String, String> {
    form_urlencoded::parse(url.split_once('?').map_or("", |(_, q)| q).as_bytes()).into_owned().collect()
}

/// The cookie called `name` that the answer sets, as a browser would send it back.
fn cookie_set(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string()).find(|c| c.starts_with(&format!("{name}=")) && !c.ends_with('='))
}

impl TestApp {
    /// A fresh browser presses "Continue with …": returns it holding the cookie that remembers the attempt, and the `state` sent to the provider.
    async fn start_federated(&self, provider: &str, query: &str) -> (TestApp, String, String) {
        let (status, headers, _) = self.anonymous().send(Method::GET, &format!("/api/auth/{provider}/start{query}"), None).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let to = location(&headers);
        let cookie = cookie_set(&headers, "lilanaya_oauth").expect("the attempt is remembered in a cookie");
        let set = headers["set-cookie"].to_str().unwrap();
        assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains("Path=/api/auth") && set.contains("Max-Age=600"), "{set}");
        let state = query_of(&to)["state"].clone();
        (TestApp { cookie: Some(cookie), ..self.clone() }, state, to)
    }

    /// The whole round trip with the provider approving. Returns where the browser ends up and, if signed in, the browser.
    async fn federated(&self, provider: &str, query: &str) -> (String, Option<TestApp>) {
        let (browser, state, _) = self.start_federated(provider, query).await;
        let (status, headers, _) = browser.send(Method::GET, &format!("/api/auth/{provider}/callback?code=good-code&state={state}"), None).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert!(headers.get_all("set-cookie").iter().any(|c| c.to_str().unwrap().starts_with("lilanaya_oauth=;")), "the attempt is forgotten");
        (location(&headers), cookie_set(&headers, "lilanaya_session").map(|cookie| TestApp { cookie: Some(cookie), ..self.clone() }))
    }
}

fn grudger(extra: Value) -> Value {
    let mut body = json!({
        "name": "Grudger", "description": "Cooperates until betrayed, then never forgives.", "mode": "rules",
        "states": ["NICE", "GRIM"], "initial_state": "NICE", "action_map": { "NICE": "C", "GRIM": "D" },
        "transitions": [{ "from": "NICE", "to": "GRIM", "guard": { "op": "last_opponent_action", "is": "D" } }],
    });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    body
}

fn ids(list: &Value) -> Vec<String> {
    list["strategies"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect()
}

const GRIM_JSON: &str = r#"{"name":"Grim","description":"Cooperates until betrayed.","states":["NICE","GRIM"],"initial_state":"NICE","action_map":{"NICE":"C","GRIM":"D"},"transitions":[{"from":"NICE","to":"GRIM","guard":{"op":"last_opponent_action","is":"D"}}],"rationale":"Two states."}"#;

async fn app_with_fake_llm() -> (TestApp, FakeApi) {
    let api = FakeApi::default();
    let llm = api.llm().await;
    (TestApp::with_llm(Some(llm)).await, api)
}

mod access;
mod accounts;
mod authoring;
mod billing;
mod compile;
mod contract;
mod federated;
mod general;
mod runs;
mod safety;
mod worlds;
