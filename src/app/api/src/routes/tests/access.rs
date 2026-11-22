//! Who may call what. One table: every route of the API, asked by a signed-out
//! visitor, by the owner of what is asked for, and by another signed-in user.
//!
//! A route added to `routes.rs` without a row here fails
//! `every_route_is_in_the_table`, so a new route cannot go out without saying
//! who may use it.

use super::*;

const WHO: [&str; 3] = ["a signed-out visitor", "the owner", "another user"];

struct Row {
    method: Method,
    /// The route as `routes.rs` writes it.
    route: &'static str,
    /// The address asked for.
    path: String,
    body: Option<Value>,
    /// The body goes as a web form (Apple's callback) instead of JSON.
    form: bool,
    /// What each of [`WHO`] is answered.
    expect: [u16; 3],
}

fn row(method: Method, route: &'static str, path: impl Into<String>, body: Option<Value>, expect: [u16; 3]) -> Row {
    Row { method, route, path: path.into(), body, form: false, expect }
}

const OPEN: [u16; 3] = [200, 200, 200];
/// Needs a session, and then answers anyone who has one.
const SIGNED_IN: [u16; 3] = [401, 200, 200];
/// The owner's private thing: to everyone else it does not exist.
const OWNERS_ONLY: [u16; 3] = [404, 200, 404];
/// Browser redirects: sign-in through a provider always sends the browser somewhere.
const REDIRECT: [u16; 3] = [303, 303, 303];

/// The table. `p` and `q` are the owner's private and public strategy; `r` and `s` their private and public run.
fn table(p: &str, q: &str, r: i64, s: i64, world: &Value) -> Vec<Row> {
    use Method as M;
    let get = |route, path: String, expect| row(M::GET, route, path, None, expect);
    let pd_world = "/api/worlds/prisoners-dilemma";
    let private_edit = grudger(json!({}));
    let public_edit = grudger(json!({ "visibility": "public" }));
    let draft = json!({ "strategy": grudger(json!({})) });
    vec![
        get("/api/health", "/api/health".into(), OPEN),
        get("/api/hello", "/api/hello".into(), OPEN),
        // Worlds and their analytics are public.
        get("/api/worlds", "/api/worlds".into(), OPEN),
        row(M::POST, "/api/worlds/validate", "/api/worlds/validate", Some(world.clone()), OPEN),
        get("/api/worlds/{id}", pd_world.into(), OPEN),
        get("/api/worlds/{id}/leaderboard", format!("{pd_world}/leaderboard"), OPEN),
        get("/api/worlds/{id}/pairwise", format!("{pd_world}/pairwise"), OPEN),
        get("/api/worlds/{id}/kpis", format!("{pd_world}/kpis"), OPEN),
        // Asking for a private run's analytics by number gives nothing away, not even to its owner: analytics are public runs only.
        get("/api/worlds/{id}/pairwise", format!("{pd_world}/pairwise?run_id={r}"), [404, 404, 404]),
        // Signing in is for people who are not signed in.
        row(M::POST, "/api/auth/email", "/api/auth/email", Some(json!({ "email": "someone@example.com" })), [202, 202, 202]),
        row(M::POST, "/api/auth/email/confirm", "/api/auth/email/confirm", Some(json!({ "token": "0".repeat(64) })), [400, 400, 400]),
        get("/api/auth/providers", "/api/auth/providers".into(), OPEN),
        get("/api/auth/{provider}/start", "/api/auth/google/start".into(), REDIRECT),
        get("/api/auth/{provider}/callback", "/api/auth/google/callback?code=x&state=y".into(), REDIRECT),
        Row { form: true, ..row(M::POST, "/api/auth/{provider}/callback", "/api/auth/apple/callback", Some(json!("code=x&state=y")), REDIRECT) },
        get("/api/plans", "/api/plans".into(), OPEN),
        get("/api/session", "/api/session".into(), OPEN),
        // The account.
        get("/api/me", "/api/me".into(), SIGNED_IN),
        row(M::PATCH, "/api/me", "/api/me", Some(json!({ "display_name": "Renamed" })), SIGNED_IN),
        get("/api/me/entitlements", "/api/me/entitlements".into(), SIGNED_IN),
        get("/api/usage", "/api/usage".into(), SIGNED_IN),
        // Strategies: reading is open, a private one exists only for its owner, and only the owner changes one.
        get("/api/strategies", "/api/strategies".into(), OPEN),
        row(M::POST, "/api/strategies", "/api/strategies", Some(grudger(json!({}))), [401, 201, 201]),
        row(M::POST, "/api/strategies/preview", "/api/strategies/preview", Some(draft), SIGNED_IN),
        row(M::POST, "/api/strategies/compile", "/api/strategies/compile", Some(json!({ "prompt": "Cooperate until betrayed, then defect for ever." })), SIGNED_IN),
        get("/api/strategies/{id}", format!("/api/strategies/{p}"), OWNERS_ONLY),
        get("/api/strategies/{id}", format!("/api/strategies/{q}"), OPEN),
        get("/api/strategies/{id}", "/api/strategies/tit_for_tat".into(), OPEN),
        row(M::PUT, "/api/strategies/{id}", format!("/api/strategies/{p}"), Some(private_edit.clone()), [401, 200, 404]),
        row(M::PUT, "/api/strategies/{id}", format!("/api/strategies/{q}"), Some(public_edit), [401, 200, 403]),
        row(M::PUT, "/api/strategies/{id}", "/api/strategies/tit_for_tat", Some(private_edit), [401, 403, 403]),
        // Runs: starting one needs a session; a private one exists only for its owner, on every route that reaches it.
        get("/api/runs", "/api/runs".into(), OPEN),
        row(M::POST, "/api/runs", "/api/runs", Some(pd(json!({ "iterations": 5 }))), [401, 202, 202]),
        get("/api/runs/{id}", format!("/api/runs/{r}"), OWNERS_ONLY),
        get("/api/runs/{id}", format!("/api/runs/{s}"), OPEN),
        get("/api/runs/{id}/matches/{a}/{b}", format!("/api/runs/{r}/matches/always_defect/{p}"), OWNERS_ONLY),
        get("/api/runs/{id}/matches/{a}/{b}", format!("/api/runs/{s}/matches/always_cooperate/always_defect"), OPEN),
        get("/api/runs/{id}/race", format!("/api/runs/{r}/race"), OWNERS_ONLY),
        get("/api/runs/{id}/race", format!("/api/runs/{s}/race"), OPEN),
        get("/api/runs/{id}/artifacts", format!("/api/runs/{r}/artifacts"), OWNERS_ONLY),
        get("/api/runs/{id}/artifacts", format!("/api/runs/{s}/artifacts"), OPEN),
        get("/api/runs/{id}/artifacts/{*name}", format!("/api/runs/{r}/artifacts/manifest.json"), OWNERS_ONLY),
        get("/api/runs/{id}/artifacts/{*name}", format!("/api/runs/{r}/artifacts/traces/always_defect__{p}.json"), OWNERS_ONLY),
        get("/api/runs/{id}/artifacts/{*name}", format!("/api/runs/{s}/artifacts/manifest.json"), OPEN),
        // Last, because they take things away: deleting, and signing out.
        row(M::DELETE, "/api/strategies/{id}", "/api/strategies/tit_for_tat", None, [401, 403, 403]),
        row(M::DELETE, "/api/strategies/{id}", format!("/api/strategies/{p}"), None, [401, 200, 404]),
        row(M::DELETE, "/api/strategies/{id}", format!("/api/strategies/{q}"), None, [401, 200, 403]),
        row(M::POST, "/api/auth/sign-out", "/api/auth/sign-out", None, OPEN),
    ]
}

/// Every `(method, route)` that `routes.rs` registers, read from its source.
fn registered() -> Vec<(Method, String)> {
    let mut routes = Vec::new();
    for line in include_str!("../../routes.rs").lines().filter(|l| l.trim_start().starts_with(".route(\"")) {
        let route = line.split('"').nth(1).unwrap().to_string();
        let handlers = line.split_once(',').unwrap().1;
        for (name, method) in [("get(", Method::GET), ("post(", Method::POST), ("put(", Method::PUT), ("patch(", Method::PATCH), ("delete(", Method::DELETE)] {
            // `get(` at the start, or after a dot: not the end of a longer name.
            if handlers.trim_start().starts_with(name) || handlers.contains(&format!(".{name}")) {
                routes.push((method, route.clone()));
            }
        }
    }
    routes
}

#[test]
fn every_route_is_in_the_table() {
    let world = json!({});
    let rows = table("p", "q", 1, 2, &world);
    let listed: Vec<(Method, String)> = rows.iter().map(|r| (r.method.clone(), r.route.to_string())).collect();
    let registered = registered();
    assert!(registered.len() >= 35, "routes.rs is no longer read correctly: found only {} routes", registered.len());
    for route in &registered {
        assert!(listed.contains(route), "{} {} is a route of the API but has no row in tests/access.rs: add one saying who may call it", route.0, route.1);
    }
    for route in &listed {
        assert!(registered.contains(route), "{} {} has a row in tests/access.rs but is no longer a route of the API", route.0, route.1);
    }
}

#[tokio::test]
async fn every_route_answers_each_kind_of_caller_as_the_table_says() {
    let (owner, api) = app_with_fake_llm().await;
    // One compile each for the owner and the other user.
    api.say(GRIM_JSON).say(GRIM_JSON);
    let other = owner.signed_in_as("bob@example.com").await;
    let anonymous = owner.anonymous();

    let (_, world) = owner.get("/api/worlds/prisoners-dilemma").await;
    let (status, private) = owner.post("/api/strategies", grudger(json!({}))).await;
    assert_eq!(status, StatusCode::CREATED, "{private}");
    let (status, public) = owner.post("/api/strategies", grudger(json!({ "visibility": "public" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{public}");
    let (p, q) = (private["id"].as_str().unwrap(), public["id"].as_str().unwrap());
    let private_run = owner.run(pd(json!({ "actors": [p, "always_defect"], "iterations": 5 }))).await;
    let public_run = owner.run(pd(json!({ "iterations": 5 }))).await;
    assert_eq!((private_run["private"].as_bool(), public_run["private"].as_bool()), (Some(true), Some(false)));

    // Asked in this order, so that the owner's delete comes after the others were refused theirs.
    let callers = [(0, &anonymous), (2, &other), (1, &owner)];
    for row in table(p, q, private_run["id"].as_i64().unwrap(), public_run["id"].as_i64().unwrap(), &world) {
        for (who, caller) in callers {
            let (status, _, body) = if row.form {
                caller.raw(row.method.clone(), &row.path, Some("application/x-www-form-urlencoded"), row.body.as_ref().map(|b| b.as_str().unwrap().to_string())).await
            } else {
                caller.send(row.method.clone(), &row.path, row.body.clone()).await
            };
            let what = format!("{} {} asked by {}", row.method, row.path, WHO[who]);
            assert_eq!(status.as_u16(), row.expect[who], "{what}: {body}");
            if status.is_client_error() || status.is_server_error() {
                assert!(body["error"]["code"].is_string() && body["error"]["message"].is_string(), "{what}: an error must have the one error shape, got {body}");
            }
            if status == StatusCode::UNAUTHORIZED {
                assert_eq!(body["error"]["code"], "unauthenticated", "{what}");
            }
            if status == StatusCode::NOT_FOUND {
                // "Not there" and "not yours" must read the same.
                assert_eq!(body["error"]["code"], "not_found", "{what}");
            }
        }
    }
    owner.close().await;
}

#[tokio::test]
async fn a_private_thing_and_a_missing_thing_are_answered_word_for_word_alike() {
    let owner = TestApp::new().await;
    let other = owner.signed_in_as("bob@example.com").await;
    let (_, private) = owner.post("/api/strategies", grudger(json!({}))).await;
    let p = private["id"].as_str().unwrap();
    let run = owner.run(pd(json!({ "actors": [p, "always_defect"], "iterations": 5 }))).await;
    let r = run["id"].as_i64().unwrap();

    // The same request for something that was never there: only the name in the message may differ.
    let missing_strategy = "grudger-000000";
    let missing_run = r + 1000;
    for (real, made_up) in [
        (format!("/api/strategies/{p}"), format!("/api/strategies/{missing_strategy}")),
        (format!("/api/runs/{r}"), format!("/api/runs/{missing_run}")),
        (format!("/api/runs/{r}/race"), format!("/api/runs/{missing_run}/race")),
        (format!("/api/runs/{r}/artifacts"), format!("/api/runs/{missing_run}/artifacts")),
        (format!("/api/runs/{r}/artifacts/manifest.json"), format!("/api/runs/{missing_run}/artifacts/manifest.json")),
        (format!("/api/runs/{r}/matches/always_defect/{p}"), format!("/api/runs/{missing_run}/matches/always_defect/{p}")),
    ] {
        for caller in [&other, &owner.anonymous()] {
            let (status, hidden) = caller.get(&real).await;
            let (_, absent) = caller.get(&made_up).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{real}");
            let same = hidden.to_string().replace(p, "X").replace(&r.to_string(), "N");
            let other = absent.to_string().replace(missing_strategy, "X").replace(p, "X").replace(&missing_run.to_string(), "N");
            assert_eq!(same, other, "{real} gives away that it exists");
        }
    }
    owner.close().await;
}
