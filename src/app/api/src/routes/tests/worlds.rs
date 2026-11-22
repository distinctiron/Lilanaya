//! Worlds: the list, one world, and checking a world definition.

use super::*;

#[tokio::test]
async fn worlds_and_strategies_are_listed() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/api/worlds").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["worlds"][0]["id"], "prisoners-dilemma");

    let (status, world) = app.get("/api/worlds/prisoners-dilemma").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(world["payoff"]["matrix"]["D"]["C"], json!([5, 0]));
    assert_eq!(world["kpis"]["definitions"].as_array().unwrap().len(), 3);

    let (_, body) = app.get("/api/strategies").await;
    let ids: Vec<&str> = body["strategies"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["always_cooperate", "always_defect", "tit_for_tat", "win_stay_lose_shift"]);
    app.close().await;
}

#[tokio::test]
async fn validate_reports_problems_without_failing_the_request() {
    let app = TestApp::new().await;
    let (_, mut world) = app.get("/api/worlds/prisoners-dilemma").await;
    let (status, body) = app.post("/api/worlds/validate", world.clone()).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "valid": true, "errors": [] })));

    world["time"]["iterations"] = json!(0);
    world["payoff"]["matrix"]["C"].as_object_mut().unwrap().remove("D");
    world["kpis"]["definitions"][0]["query"]["filter"]["outcome"] = json!("C/Z");
    let (status, body) = app.post("/api/worlds/validate", world.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], false);
    let paths: Vec<&str> = body["errors"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["time.iterations", "payoff.matrix.C", "kpis.definitions[0].query.filter.outcome"]);

    world.as_object_mut().unwrap().remove("decisions");
    let (status, body) = app.post("/api/worlds/validate", world).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], false);
    assert!(body["errors"][0]["message"].as_str().unwrap().contains("decisions"));
    app.close().await;
}
