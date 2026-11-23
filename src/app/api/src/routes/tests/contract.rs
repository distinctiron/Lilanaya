//! What the front end reads. Each answer the pages use is checked for the
//! fields they read and the kind of value in each, so a renamed or dropped
//! field fails here and not in a browser.
//!
//! A shape is written as JSON: `"string"`, `"number"`, `"bool"`, `"array"`,
//! `"object"` or `"any"` for a value of that kind (with `?` when it may also
//! be null), an object for an object that has at least those fields, and a
//! one-element array for a list whose every item has that shape. Fields the
//! pages do not read are not listed, so adding a field never fails.

use super::*;

/// Every way `value` does not have `shape`, each with where it is.
fn mismatches(at: &str, value: &Value, shape: &Value, out: &mut Vec<String>) {
    match shape {
        Value::String(kind) => {
            let (kind, optional) = match kind.strip_suffix('?') {
                Some(kind) => (kind, true),
                None => (kind.as_str(), false),
            };
            let fits = match kind {
                "string" => value.is_string(),
                "number" => value.is_number(),
                "bool" => value.is_boolean(),
                "array" => value.is_array(),
                "object" => value.is_object(),
                "any" => !value.is_null(),
                other => panic!("contract.rs: \"{other}\" is not a kind of value"),
            };
            if !(fits || (optional && value.is_null())) {
                out.push(format!("{at}: expected {kind}{}, got {value}", if optional { " or null" } else { "" }));
            }
        }
        Value::Object(fields) => match value.as_object() {
            Some(object) => {
                for (name, inner) in fields {
                    match object.get(name) {
                        Some(found) => mismatches(&format!("{at}.{name}"), found, inner, out),
                        None => out.push(format!("{at}.{name}: the field is missing")),
                    }
                }
            }
            None => out.push(format!("{at}: expected an object, got {value}")),
        },
        Value::Array(item) => match value.as_array() {
            Some(items) => {
                assert_eq!(item.len(), 1, "contract.rs: a list shape has exactly one item shape");
                if items.is_empty() {
                    out.push(format!("{at}: the list is empty, so its items could not be checked"));
                }
                for (i, found) in items.iter().enumerate() {
                    mismatches(&format!("{at}[{i}]"), found, &item[0], out);
                }
            }
            None => out.push(format!("{at}: expected a list, got {value}")),
        },
        other => panic!("contract.rs: {other} is not a shape"),
    }
}

/// Fails, listing every difference, unless `value` has `shape`.
fn has_shape(what: &str, value: &Value, shape: Value) {
    let mut out = Vec::new();
    mismatches(what, value, &shape, &mut out);
    assert!(out.is_empty(), "{what} is not what the front end reads:\n  {}\n\nthe answer was: {value}", out.join("\n  "));
}

fn usage_shape() -> Value {
    json!({
        "period": "string", "period_end": "string", "monthly_runs": "number", "used": "number", "remaining": "number", "max_active_runs": "number",
        "ai_requests": { "limit": "number", "per": "string", "used_by_world": "object", "used_this_month": "number" },
        "packs": { "strategies_left": "number", "runs_left": "number" },
    })
}

fn me_shape() -> Value {
    json!({
        "id": "number", "email": "string", "display_name": "string", "roles": ["string"], "plan": "string",
        "entitlements": { "analytics_access": "bool", "quick_test_access": "bool", "private_worlds": "bool" },
        "usage": usage_shape(),
        "features": { "compile": "bool", "billing": "bool" },
    })
}

fn world_shape() -> Value {
    json!({
        "id": "string",
        "metadata": { "name": "string", "description": "string", "guide": "string", "version": "string" },
        "decisions": { "allowed": ["string"] },
        "outcomes": { "allowed": "array" },
        "interaction": { "policy": "string" },
        "randomness": { "schedule_mode": "string" },
        "time": { "iterations": "number" },
        "determinism": { "rng_seed": "number" },
        "payoff": { "matrix": { "C": { "C": ["number"], "D": ["number"] }, "D": { "C": ["number"], "D": ["number"] } } },
        "scoring": { "aggregate": "string" },
        "kpis": { "definitions": [{ "id": "string", "label": "string" }] },
    })
}

fn issue_shape() -> Value {
    json!([{ "path": "string", "message": "string" }])
}

#[tokio::test]
async fn the_account_pages_get_the_user_the_usage_and_the_plans() {
    let app = TestApp::new().await;
    let anon = app.anonymous();

    assert_eq!(anon.get("/api/session").await.1, json!({ "user": null }));
    has_shape("GET /api/session", &app.get("/api/session").await.1, json!({ "user": me_shape() }));
    has_shape("GET /api/me", &app.get("/api/me").await.1, me_shape());
    has_shape("PATCH /api/me", &app.call(Method::PATCH, "/api/me", Some(json!({ "display_name": "Ada" }))).await.1, me_shape());
    has_shape("GET /api/usage", &app.get("/api/usage").await.1, usage_shape());
    has_shape("GET /api/me/entitlements", &app.get("/api/me/entitlements").await.1, json!({ "plan": "string", "analytics_access": "bool", "quick_test_access": "bool", "private_worlds": "bool" }));

    let plan = json!({ "ai_requests": { "limit": "number", "per": "string" }, "monthly_runs": "number", "max_active_runs": "number" });
    has_shape("GET /api/plans", &anon.get("/api/plans").await.1, json!({ "free": plan, "subscriber": plan }));
    // The pages tell the two kinds of allowance apart by this word.
    let (_, plans) = anon.get("/api/plans").await;
    assert_eq!((plans["free"]["ai_requests"]["per"].as_str(), plans["subscriber"]["ai_requests"]["per"].as_str()), (Some("world"), Some("month")));

    // Payment: the plans page reads the prices, the profile what was paid for.
    let (shop, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let price = json!({ "amount": "number", "currency": "string" });
    has_shape("GET /api/plans (billing)", &shop.anonymous().get("/api/plans").await.1["billing"], json!({
        "subscription": { "amount": "number", "currency": "string", "interval": "string" },
        "packs": [{ "id": "string", "strategies": "number", "runs": "number", "amount": price["amount"], "currency": price["currency"] }],
        "tax_added": "bool",
    }));
    has_shape("POST /api/billing/checkout", &shop.post("/api/billing/checkout", json!({ "item": "small", "agree": true })).await.1, json!({ "url": "string" }));
    shop.buy_pack(&stripe, "small", 1).await;
    shop.subscribe(&stripe, "sub_1", unix_now() - 60, unix_now() + 30 * DAY).await;
    has_shape("GET /api/billing", &shop.get("/api/billing").await.1, json!({
        "enabled": "bool", "plan": "string", "can_manage": "bool",
        "subscription": { "status": "string", "paid": "bool", "period_end": "string", "cancel_at_period_end": "bool" },
        "packs": { "strategies_left": "number", "runs_left": "number" },
        "purchases": [{ "pack": "string", "strategies": "number", "runs": "number", "amount": "number", "currency": "string", "created_at": "string", "refunded": "bool" }],
    }));
    has_shape("POST /api/billing/portal", &shop.post("/api/billing/portal", json!({})).await.1, json!({ "url": "string" }));
    has_shape("GET /api/me/export", &shop.get("/api/me/export").await.1, json!({ "account": { "email": "string" }, "strategies": "array", "runs": "array", "purchases": "array" }));
    shop.close().await;

    // Signing in: the page shows how long the link is good for, and takes the user from the confirmation.
    let (_, sent) = anon.post("/api/auth/email", json!({ "email": "grace@example.com" })).await;
    has_shape("POST /api/auth/email", &sent, json!({ "sent": "bool", "expires_in_minutes": "number" }));
    let (_, confirmed) = anon.post("/api/auth/email/confirm", json!({ "token": app.emailed_token() })).await;
    has_shape("POST /api/auth/email/confirm", &confirmed, me_shape());
    assert_eq!(anon.get("/api/auth/providers").await.1, json!({ "providers": [] }));
    let (with_providers, _, _) = app_with_fake_providers().await;
    has_shape("GET /api/auth/providers", &with_providers.anonymous().get("/api/auth/providers").await.1, json!({ "providers": [{ "id": "string", "name": "string" }] }));
    with_providers.close().await;
    app.close().await;
}

#[tokio::test]
async fn the_world_pages_get_the_world_its_standings_and_its_indicators() {
    let app = TestApp::new().await;
    has_shape("GET /api/worlds", &app.get("/api/worlds").await.1, json!({ "worlds": [world_shape()] }));
    has_shape("GET /api/worlds/{id}", &app.get("/api/worlds/prisoners-dilemma").await.1, world_shape());

    // Before any run: the pages test `run_id == null` and an empty list of snapshots.
    let world = "/api/worlds/prisoners-dilemma";
    has_shape("the leaderboard before any run", &app.get(&format!("{world}/leaderboard")).await.1, json!({ "run_id": "number?", "entries": "array" }));
    assert_eq!(app.get(&format!("{world}/leaderboard")).await.1["run_id"], Value::Null);
    assert_eq!(app.get(&format!("{world}/kpis?limit=1")).await.1["snapshots"], json!([]));

    app.run(pd(json!({ "iterations": 20 }))).await;
    app.run(pd(json!({ "iterations": 20, "seed": 7 }))).await;
    has_shape(
        "GET /api/worlds/{id}/leaderboard",
        &app.get(&format!("{world}/leaderboard")).await.1,
        json!({ "run_id": "number", "seed": "number", "iterations": "number", "entries": [{ "rank": "number", "actor_id": "string", "name": "string", "average": "number", "variance": "number", "entries": "number" }] }),
    );
    let (_, history) = app.get(&format!("{world}/kpis?limit=1")).await;
    has_shape("GET /api/worlds/{id}/kpis", &history, json!({ "definitions": [{ "id": "string", "label": "string" }], "snapshots": [{ "run_id": "number", "values": "object" }] }));
    // One value per indicator the world defines, under the indicator's id; the change is null only for a first run.
    for definition in history["definitions"].as_array().unwrap() {
        let value = &history["snapshots"][0]["values"][definition["id"].as_str().unwrap()];
        has_shape("an indicator's value", value, json!({ "label": "string", "value": "number", "delta": "number" }));
    }
    has_shape(
        "GET /api/worlds/{id}/pairwise",
        &app.get(&format!("{world}/pairwise")).await.1,
        json!({ "run_id": "number", "actors": [{ "id": "string", "name": "string" }], "pairs": [{ "a": "string", "b": "string", "average_payoff": ["number"], "outcome_counts": "object" }] }),
    );
    has_shape("POST /api/worlds/validate", &app.post("/api/worlds/validate", json!({})).await.1, json!({ "valid": "bool", "errors": issue_shape() }));
    app.close().await;
}

#[tokio::test]
async fn the_strategy_pages_get_each_strategy_the_preview_and_what_the_ai_wrote() {
    let (app, api) = app_with_fake_llm().await;
    let listed = json!({ "id": "string", "name": "string", "description": "string", "mode": "string", "builtin": "bool", "visibility": "string", "world_id": "string", "mine": "bool", "owner_name": "string?", "updated_at": "string" });
    let rules = json!({ "states": ["string"], "initial_state": "string", "action_map": "object", "transitions": "array" });

    let (_, mine) = app.post("/api/strategies", grudger(json!({ "prompt": "be nice until betrayed", "visibility": "public" }))).await;
    let (_, code) = app.post("/api/strategies", json!({ "name": "Coded", "mode": "code", "source": "function decide() { return 'C'; }" })).await;
    for (what, strategy) in [("POST /api/strategies (rules)", &mine), ("POST /api/strategies (code)", &code)] {
        has_shape(what, strategy, listed.clone());
    }
    has_shape("a rules strategy", &mine, rules.clone());
    has_shape("a rules strategy's move", &mine["transitions"][0], json!({ "from": "string", "to": "string", "guard": { "op": "string" } }));
    has_shape("the owner's copy", &mine, json!({ "prompt": "string" }));
    has_shape("a code strategy", &code, json!({ "source": "string", "language": "string" }));
    assert_eq!((mine["mode"].as_str(), code["mode"].as_str(), mine["mine"].as_bool()), (Some("rules"), Some("code"), Some(true)));

    let (_, list) = app.get("/api/strategies").await;
    has_shape("GET /api/strategies", &list, json!({ "strategies": [listed.clone()] }));
    let id = mine["id"].as_str().unwrap();
    has_shape("GET /api/strategies/{id}", &app.get(&format!("/api/strategies/{id}")).await.1, listed.clone());
    has_shape("PUT /api/strategies/{id}", &app.call(Method::PUT, &format!("/api/strategies/{id}"), Some(grudger(json!({ "visibility": "public" })))).await.1, listed.clone());
    // Seen by someone else: theirs is false and there is an author to show.
    let other = app.signed_in_as("bob@example.com").await;
    let (_, seen) = other.get(&format!("/api/strategies/{id}")).await;
    has_shape("someone else's strategy", &seen, json!({ "mine": "bool", "owner_name": "string" }));
    assert_eq!(seen["mine"], false);

    let (_, preview) = app.post("/api/strategies/preview", json!({ "world_id": "prisoners-dilemma", "strategy": grudger(json!({})) })).await;
    has_shape(
        "POST /api/strategies/preview",
        &preview,
        json!({ "valid": "bool", "errors": "array", "warnings": [{ "message": "string" }], "samples": [{ "opponent": { "name": "string" }, "rounds": "number", "own_moves": ["string"], "opponent_moves": ["string"], "totals": ["number"] }] }),
    );
    let (_, broken) = app.post("/api/strategies/preview", json!({ "strategy": grudger(json!({ "initial_state": "NOWHERE" })) })).await;
    has_shape("a preview with problems", &broken, json!({ "valid": "bool", "errors": issue_shape(), "warnings": "array", "samples": "array" }));

    api.say(&GRIM_JSON.replace("\"rationale\":", r#""flags":[{"kind":"assumption","asked":"be nice","why":"Vague.","instead":"Cooperates first.","code":true}],"rationale":"#));
    let (status, compiled) = app.post("/api/strategies/compile", json!({ "world_id": "prisoners-dilemma", "prompt": "Be nice until betrayed, then never again." })).await;
    assert_eq!(status, StatusCode::OK, "{compiled}");
    has_shape(
        "POST /api/strategies/compile",
        &compiled,
        json!({
            "strategy": { "name": "string", "description": "string", "states": ["string"], "initial_state": "string", "action_map": "object", "transitions": "array" },
            "rationale": "string", "valid": "bool", "errors": "array",
            "flags": [{ "kind": "string", "asked": "string", "why": "string", "instead": "string", "code": "bool" }],
        }),
    );
    assert_eq!(app.call(Method::DELETE, &format!("/api/strategies/{id}"), None).await.1, json!({ "deleted": id }));
    app.close().await;
}

#[tokio::test]
async fn the_run_pages_get_the_run_its_matches_its_race_and_its_files() {
    let app = TestApp::new().await;
    let head = json!({ "id": "number", "status": "string", "world_id": "string", "world_name": "string", "seed": "number", "iterations": "number", "actor_count": "number", "private": "bool", "created_at": "string" });

    // Just queued: the head, and nothing of a result yet.
    let (_, queued) = app.post("/api/runs", pd(json!({ "iterations": 20, "policy": "random", "schedule_mode": "sample_per_round" }))).await;
    has_shape("POST /api/runs", &queued, head.clone());
    let id = queued["id"].as_i64().unwrap();
    let run = app.run(pd(json!({ "iterations": 20, "policy": "random", "schedule_mode": "sample_per_round" }))).await;
    has_shape("a finished run", &run, head.clone());
    has_shape(
        "a finished run's result",
        &run,
        json!({
            "decisions": ["string"], "ranking": ["string"], "schedule_mode": "string", "pairwise": [["any?"]],
            "actors": [{ "id": "string", "name": "string", "strategy_id": "string", "total": "number", "average": "number" }],
            "kpis": [{ "label": "string", "value": "number" }],
        }),
    );
    // The pages look actors up by the ids in the ranking, and index `pairwise` by an actor's place in `actors`.
    let actors: Vec<&str> = run["actors"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect();
    for ranked in run["ranking"].as_array().unwrap() {
        assert!(actors.contains(&ranked.as_str().unwrap()), "{ranked} is ranked but is not an actor");
    }
    assert_eq!(run["pairwise"].as_array().unwrap().len(), actors.len());
    assert_eq!(run["pairwise"][0][0], Value::Null, "nobody plays themselves");

    // Wait for the first one too, then the list.
    for _ in 0..1000 {
        if app.get(&format!("/api/runs/{id}")).await.1["status"] == "completed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (_, list) = app.get("/api/runs").await;
    has_shape("GET /api/runs", &list, json!({ "runs": [{ "id": "number", "status": "string", "world_name": "string", "seed": "number", "iterations": "number", "actor_count": "number", "winner": { "id": "string", "name": "string", "average": "number" } }] }));

    let id = run["id"].as_i64().unwrap();
    has_shape(
        "GET /api/runs/{id}/matches/{a}/{b}",
        &app.get(&format!("/api/runs/{id}/matches/tit_for_tat/always_defect")).await.1,
        json!({ "iterations": "number", "decisions": ["string"], "moves_a": ["number"], "moves_b": ["number"], "a": { "id": "string", "name": "string" }, "b": { "id": "string", "name": "string" } }),
    );
    let (_, race) = app.get(&format!("/api/runs/{id}/race")).await;
    has_shape("GET /api/runs/{id}/race", &race, json!({ "iterations": "number", "rounds": ["number"], "matches": [{ "a": "string", "b": "string", "totals": [["number"]] }] }));
    // One total per sampled round, for each side, ending on the final round.
    assert_eq!(race["rounds"].as_array().unwrap().last(), Some(&json!(20)));
    for m in race["matches"].as_array().unwrap() {
        assert_eq!((m["totals"].as_array().unwrap().len(), m["totals"][0].as_array().unwrap().len()), (2, race["rounds"].as_array().unwrap().len()));
    }
    let (_, files) = app.get(&format!("/api/runs/{id}/artifacts")).await;
    has_shape("GET /api/runs/{id}/artifacts", &files, json!({ "artifacts": [{ "name": "string", "href": "string", "size_bytes": "number" }] }));
    // The page links straight to `href`.
    for file in files["artifacts"].as_array().unwrap() {
        assert_eq!(app.get(file["href"].as_str().unwrap()).await.0, StatusCode::OK, "{file}");
    }

    // A failed run: the head, and the reason in place of a result.
    let (_, breaks) = app.post("/api/strategies", json!({ "name": "Breaks", "mode": "code", "source": "function decide(i) { if (i.roundIndex === 25) throw new Error('boom'); return 'C'; }" })).await;
    let failed = app.run(pd(json!({ "actors": [breaks["id"], "always_defect"], "iterations": 50 }))).await;
    has_shape("a failed run", &failed, head);
    has_shape("a failed run's reason", &failed, json!({ "error": { "message": "string", "details": issue_shape() } }));
    assert_eq!(app.get(&format!("/api/runs/{}/artifacts", failed["id"])).await.1["artifacts"], json!([]));
    app.close().await;
}

#[tokio::test]
async fn every_refusal_gives_the_pages_a_code_a_message_and_a_list_they_can_loop_over() {
    let app = TestApp::new().await;
    let refused = json!({ "error": { "code": "string", "message": "string" } });
    for (what, (status, body)) in [
        ("not signed in", app.anonymous().get("/api/me").await),
        ("not found", app.get("/api/runs/999").await),
        ("not JSON", app.raw(Method::POST, "/api/runs", Some("application/json"), Some("{".into())).await.pipe(|(s, _, b)| (s, b))),
        ("no such route", app.get("/api/nothing").await),
    ] {
        assert!(status.is_client_error(), "{what}: {status}");
        has_shape(what, &body, refused.clone());
        assert!(body["error"].get("details").is_none(), "{what}: no details when there are none, and the front end then uses an empty list");
    }
    // The pages show these, one per line, as "path: message".
    let (_, invalid) = app.post("/api/runs", pd(json!({ "iterations": 0, "actors": ["nobody", "tit_for_tat"] }))).await;
    has_shape("a request that is not valid", &invalid, json!({ "error": { "code": "string", "message": "string", "details": issue_shape() } }));
    // The pages tell these apart by their code.
    let (_, forbidden) = app.call(Method::DELETE, "/api/strategies/tit_for_tat", None).await;
    assert_eq!((invalid["error"]["code"].as_str(), forbidden["error"]["code"].as_str()), (Some("validation_failed"), Some("forbidden")));
    app.close().await;
}

/// `value.pipe(f)` is `f(value)`: keeps a conversion at the end of the line it belongs to.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
