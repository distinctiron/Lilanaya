//! Strategy authoring: create, edit, delete, preview, and entering own strategies in runs.

use super::*;

#[tokio::test]
async fn own_strategies_are_created_edited_deleted_and_private_by_default() {
    let app = TestApp::new().await;
    let bob = app.signed_in_as("bob@example.com").await;
    let anon = app.anonymous();

    assert_eq!(anon.post("/api/strategies", grudger(json!({}))).await.0, StatusCode::UNAUTHORIZED);

    let (status, made) = app.post("/api/strategies", grudger(json!({ "prompt": "be nice until betrayed" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("grudger-") && id.len() == "grudger-".len() + 6, "{id}");
    assert_eq!(
        (made["mode"].as_str(), made["visibility"].as_str(), made["mine"].as_bool(), made["builtin"].as_bool(), made["owner_name"].as_str()),
        (Some("rules"), Some("private"), Some(true), Some(false), Some("ada"))
    );
    assert_eq!(made["prompt"], "be nice until betrayed");
    assert_eq!(made["states"], json!(["NICE", "GRIM"]));

    // Private: the owner sees it; nobody else does, and they cannot tell it exists.
    let path = format!("/api/strategies/{id}");
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 5);
    assert_eq!(ids(&app.get("/api/strategies").await.1)[..4], ["always_cooperate", "always_defect", "tit_for_tat", "win_stay_lose_shift"]);
    assert!(!ids(&bob.get("/api/strategies").await.1).contains(&id));
    assert!(!ids(&anon.get("/api/strategies").await.1).contains(&id));
    assert_eq!(app.get(&path).await.0, StatusCode::OK);
    assert_eq!(bob.get(&path).await.0, StatusCode::NOT_FOUND);
    assert_eq!(anon.get(&path).await.0, StatusCode::NOT_FOUND);
    assert_eq!(bob.call(Method::PUT, &path, Some(grudger(json!({})))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(bob.call(Method::DELETE, &path, None).await.0, StatusCode::NOT_FOUND);

    // Published: others can read it, without the prompt, but still not change it.
    let (status, updated) = app.call(Method::PUT, &path, Some(grudger(json!({ "name": "Grim Trigger", "visibility": "public" })))).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!((updated["id"].as_str(), updated["name"].as_str(), updated["visibility"].as_str()), (Some(id.as_str()), Some("Grim Trigger"), Some("public")));
    let (status, seen) = bob.get(&path).await;
    assert_eq!((status, seen["mine"].as_bool(), seen["owner_name"].as_str()), (StatusCode::OK, Some(false), Some("ada")));
    assert!(seen.get("prompt").is_none());
    assert_eq!(bob.call(Method::PUT, &path, Some(grudger(json!({})))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(bob.call(Method::DELETE, &path, None).await.0, StatusCode::FORBIDDEN);

    // The mode is fixed once saved; built-ins are nobody's to change.
    let (status, body) = app.call(Method::PUT, &path, Some(json!({ "name": "x", "mode": "code", "source": "function decide() { return 'C'; }" }))).await;
    assert_eq!((status, body["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("mode")));
    assert_eq!(app.call(Method::PUT, "/api/strategies/tit_for_tat", Some(grudger(json!({})))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.call(Method::DELETE, "/api/strategies/tit_for_tat", None).await.0, StatusCode::FORBIDDEN);

    assert_eq!(app.call(Method::DELETE, &path, None).await.0, StatusCode::OK);
    assert_eq!(app.get(&path).await.0, StatusCode::NOT_FOUND);
    app.close().await;
}

#[tokio::test]
async fn invalid_strategies_are_refused_with_every_problem() {
    let app = TestApp::new().await;
    let mut bad = grudger(json!({ "name": "", "visibility": "secret", "world_id": "nope" }));
    bad["action_map"]["GRIM"] = json!("X");
    let (status, body) = app.post("/api/strategies", bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let paths: Vec<&str> = body["error"]["details"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["visibility", "world_id", "name"], "the rules are only checked against a world that exists");

    let mut bad = grudger(json!({}));
    bad["action_map"]["GRIM"] = json!("X");
    bad["transitions"][0]["to"] = json!("NOWHERE");
    let (_, body) = app.post("/api/strategies", bad).await;
    let paths: Vec<&str> = body["error"]["details"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["action_map.GRIM", "transitions[0].to"]);

    for (body, path) in [
        (json!({ "name": "A", "mode": "code" }), "source"),
        (json!({ "name": "A", "mode": "code", "source": "function nope() {}" }), "source"),
        (json!({ "name": "A", "mode": "code", "source": "function decide() { return 'Z'; }" }), "source"),
        (json!({ "name": "A", "mode": "code", "source": "function decide() { while (true) {} }" }), "source"),
        (json!({ "name": "A", "mode": "spells" }), "mode"),
    ] {
        let (status, answer) = app.post("/api/strategies", body.clone()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(answer["error"]["details"][0]["path"], path, "{answer}");
    }
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 4, "nothing was saved");
    app.close().await;
}

#[tokio::test]
async fn own_strategies_play_in_runs_and_private_ones_keep_the_run_private() {
    let app = TestApp::new().await;
    let bob = app.signed_in_as("bob@example.com").await;

    let (_, rules) = app.post("/api/strategies", grudger(json!({}))).await;
    let (status, code) = app
        .post(
            "/api/strategies",
            json!({ "name": "Copycat JS", "mode": "code", "visibility": "public", "source": "function decide(input) { return input.lastOpponentAction ?? 'C'; }" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{code}");
    assert_eq!((code["mode"].as_str(), code["language"].as_str()), (Some("code"), Some("javascript")));
    let (grudger_id, copycat_id) = (rules["id"].as_str().unwrap(), code["id"].as_str().unwrap());

    // Bob cannot enter Ada's private strategy; he can enter her public one.
    let (status, body) = bob.post("/api/runs", pd(json!({ "actors": [grudger_id, "always_defect"] }))).await;
    assert_eq!((status, body["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("actors[0]")));
    let public = bob.run(pd(json!({ "actors": [copycat_id, "always_defect", "tit_for_tat"], "iterations": 50 }))).await;
    assert_eq!((public["status"].as_str(), public["private"].as_bool()), (Some("completed"), Some(false)), "{public}");
    // The JavaScript copycat scores exactly like the built-in Tit for Tat against Always Defect.
    let average = |id: &str| public["matches"].as_array().unwrap().iter().find(|m| (m["a"] == id) != (m["b"] == id) && (m["a"] == "always_defect" || m["b"] == "always_defect")).unwrap()["totals"].clone();
    assert_eq!(average(copycat_id).as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).sum::<i64>(), 54 + 49);
    assert_eq!(average("tit_for_tat").as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).sum::<i64>(), 54 + 49);

    // Ada's run with her private strategy is hers alone.
    let private = app.run(pd(json!({ "actors": [grudger_id, copycat_id, "always_defect"], "iterations": 30 }))).await;
    assert_eq!((private["status"].as_str(), private["private"].as_bool()), (Some("completed"), Some(true)), "{private}");
    let id = private["id"].as_i64().unwrap();
    for path in [format!("/api/runs/{id}"), format!("/api/runs/{id}/artifacts"), format!("/api/runs/{id}/matches/always_defect/{grudger_id}")] {
        assert_eq!(app.get(&path).await.0, StatusCode::OK, "{path}");
        assert_eq!(bob.get(&path).await.0, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(app.anonymous().get(&path).await.0, StatusCode::NOT_FOUND, "{path}");
    }
    let listed = |list: Value| list["runs"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect::<Vec<_>>();
    assert_eq!(listed(app.get("/api/runs").await.1), [id, public["id"].as_i64().unwrap()]);
    assert_eq!(listed(bob.get("/api/runs").await.1), [public["id"].as_i64().unwrap()]);
    // World analytics only ever show public runs.
    assert_eq!(app.get("/api/worlds/prisoners-dilemma/leaderboard").await.1["run_id"], public["id"]);
    assert_eq!(app.get(&format!("/api/worlds/prisoners-dilemma/pairwise?run_id={id}")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/api/worlds/prisoners-dilemma/kpis").await.1["snapshots"].as_array().unwrap().len(), 1);

    // Deleting a strategy leaves finished runs readable: they carry their own copy.
    assert_eq!(app.call(Method::DELETE, &format!("/api/strategies/{grudger_id}"), None).await.0, StatusCode::OK);
    assert_eq!(app.get(&format!("/api/runs/{id}")).await.1["ranking"].as_array().unwrap().len(), 3);
    app.close().await;
}

#[tokio::test]
async fn preview_checks_a_draft_and_plays_sample_matches() {
    let app = TestApp::new().await;
    assert_eq!(app.anonymous().post("/api/strategies/preview", json!({ "strategy": grudger(json!({})) })).await.0, StatusCode::UNAUTHORIZED);

    let (status, p) = app.post("/api/strategies/preview", json!({ "strategy": grudger(json!({ "name": "" })) })).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!((p["valid"].as_bool(), p["errors"].clone()), (Some(true), json!([])));
    assert_eq!(p["warnings"], json!([{ "path": "states[1]", "message": "once in \"GRIM\", the strategy never leaves it" }]));
    assert_eq!(p["samples"][0]["opponent"]["name"], "Always Cooperate");
    assert_eq!(p["samples"][1]["own_moves"].as_array().unwrap()[..3], [json!("C"), json!("D"), json!("D")]);
    assert_eq!(p["samples"][1]["totals"], json!([19, 24]));

    let mut broken = grudger(json!({}));
    broken["initial_state"] = json!("START");
    let (status, p) = app.post("/api/strategies/preview", json!({ "strategy": broken })).await;
    assert_eq!((status, p["valid"].as_bool()), (StatusCode::OK, Some(false)));
    assert_eq!(p["errors"][0]["path"], "initial_state");
    assert_eq!(p["samples"], json!([]));

    let (_, p) = app.post("/api/strategies/preview", json!({ "strategy": { "mode": "code", "source": "function decide(i) { return i.roundIndex % 2 ? 'D' : 'C'; }" } })).await;
    assert_eq!(p["valid"], true, "{p}");
    assert_eq!(p["samples"][0]["own_moves"].as_array().unwrap()[..4], [json!("C"), json!("D"), json!("C"), json!("D")]);
    assert_eq!(app.get("/api/usage").await.1["used"], 0, "previews use no quota");
    app.close().await;
}
