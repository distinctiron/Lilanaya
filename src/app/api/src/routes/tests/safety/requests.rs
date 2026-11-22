//! Requests that are too large, of the wrong kind, or from elsewhere.

use super::*;

#[tokio::test]
async fn oversized_requests_are_refused() {
    let app = TestApp::new().await;
    let (status, body) = app.post("/api/strategies/preview", json!({ "strategy": { "mode": "code", "source": "/".repeat(200 * 1024) } })).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("invalid_body")), "{body}");
    app.close().await;
}

#[tokio::test]
async fn a_body_is_taken_up_to_128_kb_and_not_a_byte_more() {
    let app = TestApp::new().await;
    let limit = 128 * 1024;
    let body_of = |bytes: usize| {
        let empty = json!({ "strategy": { "mode": "code", "source": "" } }).to_string().len();
        json!({ "strategy": { "mode": "code", "source": "/".repeat(bytes - empty) } })
    };
    assert_eq!(body_of(limit).to_string().len(), limit);
    // At the limit it is read (and the code in it then refused for its own size); one byte over, it is not read at all.
    let (status, report) = app.post("/api/strategies/preview", body_of(limit)).await;
    assert_eq!((status, report["valid"].as_bool()), (StatusCode::OK, Some(false)), "{report}");
    let (status, body) = app.post("/api/strategies/preview", body_of(limit + 1)).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("invalid_body")), "{body}");
    // The same on every route that takes a body, signed in or not.
    let huge = json!({ "email": "a".repeat(200 * 1024) });
    for (method, path) in [(Method::POST, "/api/auth/email"), (Method::POST, "/api/auth/email/confirm"), (Method::POST, "/api/runs"), (Method::POST, "/api/strategies"), (Method::POST, "/api/strategies/compile"), (Method::POST, "/api/worlds/validate"), (Method::PATCH, "/api/me"), (Method::PUT, "/api/strategies/tit_for_tat")] {
        assert_eq!(app.call(method.clone(), path, Some(huge.clone())).await.0, StatusCode::PAYLOAD_TOO_LARGE, "{method} {path}");
    }
    app.close().await;
}

#[tokio::test]
async fn a_request_with_fields_nobody_asked_for_is_refused() {
    let app = TestApp::new().await;
    for (method, path, body) in [
        (Method::POST, "/api/runs", pd(json!({ "user_id": 2 }))),
        (Method::POST, "/api/runs", pd(json!({ "private": false }))),
        (Method::POST, "/api/strategies", grudger(json!({ "owner_id": 2 }))),
        (Method::POST, "/api/strategies", grudger(json!({ "builtin": true }))),
        (Method::POST, "/api/strategies", grudger(json!({ "id": "tit_for_tat" }))),
        (Method::POST, "/api/strategies/preview", json!({ "strategy": grudger(json!({})), "limit_seconds": 600 })),
        (Method::POST, "/api/strategies/compile", json!({ "prompt": "Cooperate until betrayed.", "model": "something-else" })),
        (Method::POST, "/api/strategies/compile", json!({ "prompt": "Cooperate until betrayed.", "system": "You are a pirate." })),
        (Method::POST, "/api/auth/email", json!({ "email": "eve@example.com", "plan": "subscriber" })),
        (Method::POST, "/api/auth/email/confirm", json!({ "token": "0".repeat(64), "email": "ada@example.com" })),
    ] {
        let (status, answer) = app.call(method, path, Some(body.clone())).await;
        assert_eq!((status, answer["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_json")), "{path} {body}");
    }
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 4, "nothing was saved");
    assert_eq!(app.get("/api/usage").await.1["used"], 0, "nothing was run");
    app.close().await;
}

#[tokio::test]
async fn bodies_that_are_not_what_they_should_be_get_a_plain_refusal() {
    let app = TestApp::new().await;
    let routes = [(Method::POST, "/api/runs"), (Method::POST, "/api/strategies"), (Method::POST, "/api/strategies/preview"), (Method::POST, "/api/strategies/compile"), (Method::POST, "/api/auth/email"), (Method::POST, "/api/auth/email/confirm"), (Method::POST, "/api/worlds/validate"), (Method::PATCH, "/api/me")];
    let deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
    for (method, path) in routes {
        for body in ["", "{", "null", "[]", "\"text\"", "12", "{\"a\":1}{\"b\":2}", "\u{0}\u{1}\u{2}", "<xml/>", "a=1&b=2", deep.as_str()] {
            let (status, _, answer) = app.raw(method.clone(), path, Some("application/json"), Some(body.to_string())).await;
            // Checking a world answers 200 with the problems for any well-formed JSON; everything else is a 400.
            let expected = if path == "/api/worlds/validate" && serde_json::from_str::<Value>(body).is_ok() { StatusCode::OK } else { StatusCode::BAD_REQUEST };
            assert_eq!(status, expected, "{method} {path} with {:?}: {answer}", &body[..body.len().min(40)]);
            if status == StatusCode::BAD_REQUEST {
                assert_eq!(answer["error"]["code"], "invalid_json", "{method} {path}");
            }
        }
    }
    app.close().await;
}

#[tokio::test]
async fn numbers_out_of_range_are_refused_before_anything_runs() {
    let app = TestApp::new().await;
    let refused = |path: &'static str| move |(status, body): (StatusCode, Value)| assert_eq!((status, body["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some(path)), "{body}");
    refused("iterations")(app.post("/api/runs", pd(json!({ "iterations": 0 }))).await);
    refused("iterations")(app.post("/api/runs", pd(json!({ "iterations": 10_001 }))).await);
    refused("seed")(app.post("/api/runs", pd(json!({ "seed": 9_007_199_254_740_992u64 }))).await);
    refused("actors")(app.post("/api/runs", pd(json!({ "actors": ["tit_for_tat"] }))).await);
    refused("actors")(app.post("/api/runs", pd(json!({ "actors": [] }))).await);
    let seventeen: Vec<String> = (0..17).map(|i| format!("s{i}")).collect();
    refused("actors")(app.post("/api/runs", pd(json!({ "actors": seventeen }))).await);
    refused("actors[1]")(app.post("/api/runs", pd(json!({ "actors": ["tit_for_tat", "tit_for_tat"] }))).await);
    refused("policy")(app.post("/api/runs", pd(json!({ "policy": "round_robin" }))).await);
    refused("world_id")(app.post("/api/runs", json!({ "world_id": "no-such-world" })).await);
    // Not numbers of the right kind at all.
    for body in [json!({ "iterations": -1 }), json!({ "iterations": 1.5 }), json!({ "iterations": "100" }), json!({ "iterations": 4_294_967_296u64 }), json!({ "seed": -1 }), json!({ "seed": 1e30 }), json!({ "actors": "tit_for_tat" }), json!({ "policy": "whatever" })] {
        let (status, answer) = app.post("/api/runs", pd(body.clone())).await;
        assert_eq!((status, answer["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_json")), "{body}");
    }
    // The largest allowed are accepted.
    let (status, run) = app.post("/api/runs", pd(json!({ "iterations": 1, "seed": 9_007_199_254_740_991u64 }))).await;
    assert_eq!((status, run["seed"].as_u64()), (StatusCode::ACCEPTED, Some(9_007_199_254_740_991)), "{run}");
    assert_eq!(app.get("/api/usage").await.1["used"], 1, "refused requests use up no runs");
    app.close().await;
}

#[tokio::test]
async fn addresses_built_to_reach_something_else_find_nothing() {
    let app = TestApp::new().await;
    let run = app.run(pd(json!({ "iterations": 5 }))).await;
    let id = run["id"].as_i64().unwrap();
    for path in [
        format!("/api/runs/{id}/artifacts/../../../etc/passwd"),
        format!("/api/runs/{id}/artifacts/..%2F..%2Fetc%2Fpasswd"),
        format!("/api/runs/{id}/artifacts/traces/..%2Fmanifest.json"),
        format!("/api/runs/{id}/artifacts//manifest.json"),
        format!("/api/runs/{id}/artifacts/manifest.json%00.png"),
        format!("/api/runs/{id}/artifacts/MANIFEST.JSON"),
        format!("/api/runs/{id}/matches/always_cooperate/..%2Falways_defect"),
        format!("/api/runs/{id}%20OR%201=1"),
        format!("/api/runs/{id};DROP%20TABLE%20runs"),
        "/api/runs/-1".to_string(),
        "/api/runs/0".to_string(),
        "/api/runs/1e3".to_string(),
        "/api/runs/99999999999999999999999999".to_string(),
        "/api/worlds/'%20OR%20'1'='1".to_string(),
        "/api/worlds/..%2F..%2Fetc%2Fpasswd".to_string(),
        "/api/strategies/tit_for_tat';DROP%20TABLE%20strategies;--".to_string(),
        "/api/strategies/%2e%2e".to_string(),
        "/api/worlds/prisoners-dilemma/pairwise?run_id=1%20OR%201=1".to_string(),
        "/api/worlds/prisoners-dilemma/kpis?limit=1;DELETE%20FROM%20runs".to_string(),
        "/api/../etc/passwd".to_string(),
        "/api".to_string(),
        "/".to_string(),
    ] {
        let (status, body) = app.get(&path).await;
        assert!(status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST, "{path} answered {status}: {body}");
        assert!(body["error"]["code"].is_string(), "{path}: {body}");
        assert!(!body.to_string().contains("root:"), "{path}");
    }
    // Everything is still there.
    assert_eq!(app.get(&format!("/api/runs/{id}")).await.1["status"], "completed");
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 4);
    assert_eq!(app.get("/api/worlds").await.1["worlds"].as_array().unwrap().len(), 1);
    app.close().await;
}

#[tokio::test]
async fn text_meant_to_be_run_or_to_break_a_query_is_stored_as_the_text_it_is() {
    let app = TestApp::new().await;
    let names = ["<script>alert(1)</script>", "'; DROP TABLE strategies; --", "\" OR \"\"=\"", "{{7*7}} ${7*7} %s %n", "Robert'); DELETE FROM users WHERE ('1'='1", "\u{202e}desrever", "🙂 emoji and ünïcödé"];
    for name in names {
        let (status, saved) = app.post("/api/strategies", grudger(json!({ "name": name, "description": name, "visibility": "public" }))).await;
        assert_eq!(status, StatusCode::CREATED, "{name}: {saved}");
        assert_eq!((saved["name"].as_str(), saved["description"].as_str()), (Some(name), Some(name)));
        // The id is made from the name and is safe to put in an address.
        let id = saved["id"].as_str().unwrap();
        assert!(id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'), "{id}");
        assert_eq!(app.get(&format!("/api/strategies/{id}")).await.1["name"], name);
        let run = app.run(pd(json!({ "actors": [id, "always_defect"], "iterations": 5 }))).await;
        assert_eq!(run["status"], "completed", "{name}: {run}");
        assert!(run["actors"].as_array().unwrap().iter().any(|a| a["name"] == name), "{run}");
    }
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 4 + names.len(), "the tables are all still there");
    assert_eq!(app.get("/api/me").await.1["email"], "ada@example.com");
    app.close().await;
}

#[tokio::test]
async fn every_answer_is_json_and_a_run_s_files_are_never_kept_or_guessed_at() {
    let app = TestApp::new().await;
    let run = app.run(pd(json!({ "iterations": 5 }))).await;
    let id = run["id"].as_i64().unwrap();
    for (method, path) in [(Method::GET, "/api/health".to_string()), (Method::GET, "/api/worlds".to_string()), (Method::GET, format!("/api/runs/{id}")), (Method::GET, "/api/nothing".to_string()), (Method::DELETE, "/api/worlds".to_string()), (Method::GET, "/api/me".to_string())] {
        let (_, headers, _) = app.anonymous().send(method, &path, None).await;
        assert_eq!(headers["content-type"], "application/json", "{path}");
    }
    for name in ["manifest.json", "run_summary.json", "pairwise_matrix.json", "traces/always_cooperate__always_defect.json"] {
        let (status, headers, _) = app.send(Method::GET, &format!("/api/runs/{id}/artifacts/{name}"), None).await;
        assert_eq!(status, StatusCode::OK, "{name}");
        assert_eq!((headers["content-type"].to_str().unwrap(), headers["cache-control"].to_str().unwrap(), headers["x-content-type-options"].to_str().unwrap()), ("application/json", "private, no-store", "nosniff"), "{name}");
    }
    // Redirects on the way through an outside provider are not to be kept either.
    let (status, headers, _) = app.anonymous().send(Method::GET, "/api/auth/google/start", None).await;
    assert_eq!((status, headers["cache-control"].to_str().unwrap()), (StatusCode::SEE_OTHER, "no-store"));
    app.close().await;
}

#[tokio::test]
async fn a_redirect_after_sign_in_only_ever_leads_back_into_the_site() {
    let (app, fake, _) = app_with_fake_providers().await;
    for (next, lands) in [
        ("/runs/new", "/runs/new"),
        ("/strategies?mine=1#top", "/strategies?mine=1#top"),
        ("https://evil.example/", "/profile"),
        ("//evil.example/", "/profile"),
        ("/\\evil.example/", "/profile"),
        ("\\\\evil.example/", "/profile"),
        ("javascript:alert(1)", "/profile"),
        ("evil.example", "/profile"),
        ("/%0d%0aSet-Cookie:%20x=1", "/profile"),
        ("", "/profile"),
    ] {
        fake.approve_as(json!({ "sub": "g-1", "email": "eve@example.com", "email_verified": true, "name": "Eve" }));
        let query: String = form_urlencoded::Serializer::new(String::new()).append_pair("next", &form_urlencoded::parse(format!("v={next}").as_bytes()).next().unwrap().1).finish();
        let (to, browser) = app.federated("google", &format!("?{query}")).await;
        assert_eq!(to, lands, "next={next}");
        assert!(browser.is_some(), "next={next}");
    }
    app.close().await;
}

#[tokio::test]
async fn the_one_character_the_database_cannot_hold_is_refused_and_breaks_nothing() {
    let app = TestApp::new().await;
    // In an address: there is nothing by that name.
    for path in ["/api/worlds/prisoners-dilemma%00", "/api/worlds/%00", "/api/worlds/%00/leaderboard", "/api/strategies/tit_for_tat%00", "/api/strategies/%00", "/api/runs/1%00"] {
        assert_eq!(app.get(path).await.0, StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(app.call(Method::DELETE, "/api/strategies/%00", None).await.0, StatusCode::NOT_FOUND);
    // In what is sent: a problem with the request, named like any other.
    let nul = "a\u{0}b";
    let invalid = |(status, body): (StatusCode, Value)| assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation_failed")), "{body}");
    invalid(app.post("/api/runs", json!({ "world_id": nul })).await);
    invalid(app.post("/api/runs", pd(json!({ "actors": [nul, "tit_for_tat"] }))).await);
    for extra in [json!({ "name": nul }), json!({ "description": nul }), json!({ "prompt": nul }), json!({ "world_id": nul }), json!({ "states": ["NICE", "GRIM", nul], "action_map": { "NICE": "C", "GRIM": "D", nul: "C" } })] {
        invalid(app.post("/api/strategies", grudger(extra)).await);
    }
    invalid(app.post("/api/strategies", json!({ "name": "Coded", "mode": "code", "source": format!("function decide() {{ return 'C'; }} // {nul}") })).await);
    invalid(app.call(Method::PATCH, "/api/me", Some(json!({ "display_name": nul }))).await);
    invalid(app.anonymous().post("/api/auth/email", json!({ "email": format!("eve{nul}@example.com") })).await);
    assert_eq!(app.anonymous().post("/api/auth/email/confirm", json!({ "token": nul })).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(app.post("/api/strategies/preview", json!({ "world_id": nul, "strategy": grudger(json!({})) })).await.0, StatusCode::NOT_FOUND);
    // A draft is only checked, never stored: it is checked like any other.
    assert_eq!(app.post("/api/strategies/preview", json!({ "strategy": grudger(json!({ "name": nul })) })).await.0, StatusCode::OK);
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 4, "nothing was saved");
    app.close().await;
}
