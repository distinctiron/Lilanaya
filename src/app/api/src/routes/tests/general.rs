//! What every route has in common.

use super::*;

#[tokio::test]
async fn health_and_hello() {
    let app = TestApp::new().await;
    assert_eq!(app.get("/api/health").await, (StatusCode::OK, json!({ "status": "ok" })));
    assert_eq!(app.get("/api/hello").await.1["message"], "Hello from Lilanaya!");
    app.close().await;
}

#[tokio::test]
async fn errors_share_one_shape() {
    let app = TestApp::new().await;
    for (method, path, status, code) in [
        (Method::GET, "/api/worlds/nope", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/runs/99", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/runs/abc", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/nothing/here", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/worlds/nope/leaderboard", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/worlds/prisoners-dilemma/pairwise?run_id=99", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/worlds/prisoners-dilemma/pairwise?run_id=x", StatusCode::BAD_REQUEST, "invalid_query"),
        (Method::GET, "/api/worlds/prisoners-dilemma/kpis?limit=0", StatusCode::BAD_REQUEST, "invalid_query"),
        (Method::DELETE, "/api/worlds", StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"),
    ] {
        let (got, body) = app.call(method, path, None).await;
        assert_eq!(got, status, "{path}");
        assert_eq!(body["error"]["code"], code, "{path}");
        assert!(body["error"]["message"].is_string(), "{path}");
    }

    let req = Request::builder().method(Method::POST).uri("/api/runs").header("cookie", app.cookie.as_deref().unwrap()).body(Body::from("{not json")).unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["error"]["code"], "invalid_json");
    app.close().await;
}
