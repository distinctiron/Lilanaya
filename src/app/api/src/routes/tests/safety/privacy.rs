//! What is private stays invisible, and nothing personal is handed out.
//! (Who may call which route is the table in `tests/access.rs`.)

use super::*;

#[tokio::test]
async fn nothing_private_shows_up_in_any_list_or_analysis_others_can_read() {
    let ada = TestApp::new().await;
    let bob = ada.signed_in_as("bob@example.com").await;
    let (_, secret) = ada.post("/api/strategies", grudger(json!({ "name": "Secret Weapon", "description": "ada's hidden plan", "prompt": "my private description of it" }))).await;
    let secret_id = secret["id"].as_str().unwrap();
    let private_run = ada.run(pd(json!({ "actors": [secret_id, "always_defect"], "iterations": 20 }))).await;
    assert_eq!(private_run["private"], true);
    let public_run = ada.run(pd(json!({ "iterations": 20 }))).await;
    let (private_id, public_id) = (private_run["id"].as_i64().unwrap(), public_run["id"].as_i64().unwrap());

    let world = "/api/worlds/prisoners-dilemma";
    for caller in [&bob, &ada.anonymous()] {
        for path in ["/api/strategies".to_string(), "/api/runs".to_string(), format!("{world}/leaderboard"), format!("{world}/pairwise"), format!("{world}/kpis?limit=100"), format!("/api/runs/{public_id}"), format!("/api/runs/{public_id}/artifacts")] {
            let (status, body) = caller.get(&path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            let text = body.to_string();
            for hidden in [secret_id, "Secret Weapon", "hidden plan", "private description"] {
                assert!(!text.contains(hidden), "{path} shows \"{hidden}\" to someone who is not its owner");
            }
            if path != format!("/api/runs/{public_id}") && path != format!("/api/runs/{public_id}/artifacts") {
                assert!(!text.contains(&format!("\"run_id\":{private_id}")) && !text.contains(&format!("\"id\":{private_id},")), "{path} lists the private run");
            }
        }
    }
    // The world's analytics were fed by the public run alone.
    assert_eq!(bob.get(&format!("{world}/leaderboard")).await.1["run_id"], public_id);
    assert_eq!(bob.get(&format!("{world}/kpis")).await.1["snapshots"].as_array().unwrap().len(), 1);
    ada.close().await;
}

#[tokio::test]
async fn nobody_s_email_address_or_description_is_handed_to_anyone_else() {
    let ada = TestApp::new().await;
    ada.call(Method::PATCH, "/api/me", Some(json!({ "display_name": "Ada L." }))).await;
    let bob = ada.signed_in_as("bob@example.com").await;
    let (_, shared) = ada.post("/api/strategies", grudger(json!({ "visibility": "public", "prompt": "what I typed for the AI" }))).await;
    let id = shared["id"].as_str().unwrap();
    let run = ada.run(pd(json!({ "actors": [id, "always_defect"], "iterations": 20 }))).await;
    let run_id = run["id"].as_i64().unwrap();
    assert_eq!(run["private"], false);

    let world = "/api/worlds/prisoners-dilemma";
    let paths = [
        "/api/strategies".to_string(),
        format!("/api/strategies/{id}"),
        "/api/runs".to_string(),
        format!("/api/runs/{run_id}"),
        format!("/api/runs/{run_id}/race"),
        format!("/api/runs/{run_id}/artifacts"),
        format!("/api/runs/{run_id}/artifacts/run_summary.json"),
        format!("/api/runs/{run_id}/matches/always_defect/{id}"),
        format!("{world}/leaderboard"),
        format!("{world}/pairwise"),
        format!("{world}/kpis"),
        "/api/session".to_string(),
        "/api/plans".to_string(),
    ];
    for caller in [&bob, &ada.anonymous()] {
        for path in &paths {
            let (status, body) = caller.get(path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            let text = body.to_string();
            assert!(!text.contains("ada@example.com"), "{path} shows the owner's email address");
            assert!(!text.contains("what I typed for the AI"), "{path} shows the owner's description for the AI");
        }
    }
    // What others do see of the author is the name they chose; the owner sees their own description.
    assert_eq!(bob.get(&format!("/api/strategies/{id}")).await.1["owner_name"], "Ada L.");
    assert_eq!(ada.get(&format!("/api/strategies/{id}")).await.1["prompt"], "what I typed for the AI");
    // Each account sees only itself.
    assert_eq!(bob.get("/api/me").await.1["email"], "bob@example.com");
    assert_eq!(bob.get("/api/usage").await.1["used"], 0, "another user's runs are not counted as mine");
    ada.close().await;
}

#[tokio::test]
async fn a_private_strategy_cannot_be_borrowed_copied_or_overwritten_by_someone_else() {
    let ada = TestApp::new().await;
    let bob = ada.signed_in_as("bob@example.com").await;
    let (_, secret) = ada.post("/api/strategies", grudger(json!({}))).await;
    let id = secret["id"].as_str().unwrap();
    let path = format!("/api/strategies/{id}");

    // Entering it in a run: refused as if there were no such strategy.
    let (status, body) = bob.post("/api/runs", pd(json!({ "actors": [id, "always_defect"] }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["details"][0]["message"], format!("there is no strategy \"{id}\""));
    let (_, missing) = bob.post("/api/runs", pd(json!({ "actors": ["no-such-strategy", "always_defect"] }))).await;
    assert_eq!(body["error"]["details"][0]["message"].as_str().unwrap().replace(id, "X"), missing["error"]["details"][0]["message"].as_str().unwrap().replace("no-such-strategy", "X"));

    // Changing or deleting it, also by making it "public" on the way.
    assert_eq!(bob.call(Method::PUT, &path, Some(grudger(json!({ "visibility": "public", "name": "Mine now" })))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(bob.call(Method::DELETE, &path, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ada.anonymous().call(Method::PUT, &path, Some(grudger(json!({})))).await.0, StatusCode::UNAUTHORIZED);
    let (_, still) = ada.get(&path).await;
    assert_eq!((still["name"].as_str(), still["visibility"].as_str()), (Some("Grudger"), Some("private")));

    // A strategy made public and private again disappears for others again; runs they made with it stay theirs.
    ada.call(Method::PUT, &path, Some(grudger(json!({ "visibility": "public" })))).await;
    let run = bob.run(pd(json!({ "actors": [id, "always_defect"], "iterations": 10 }))).await;
    assert_eq!(run["status"], "completed", "{run}");
    ada.call(Method::PUT, &path, Some(grudger(json!({ "visibility": "private" })))).await;
    assert_eq!(bob.get(&path).await.0, StatusCode::NOT_FOUND);
    assert_eq!(bob.post("/api/runs", pd(json!({ "actors": [id, "always_defect"] }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bob.get(&format!("/api/runs/{}", run["id"])).await.0, StatusCode::OK);
    ada.close().await;
}
