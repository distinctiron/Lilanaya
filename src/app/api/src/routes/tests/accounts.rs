//! Accounts: the emailed link, the session, the profile, usage and plans.

use super::*;

#[tokio::test]
async fn sign_in_by_emailed_link_then_sign_out() {
    let app = TestApp::new().await;
    let anon = app.anonymous();

    let (status, body) = anon.get("/api/me").await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("unauthenticated")));
    assert_eq!(anon.get("/api/session").await, (StatusCode::OK, json!({ "user": null })));
    assert_eq!(app.get("/api/session").await.1["user"]["email"], "ada@example.com");

    let (status, body) = anon.post("/api/auth/email", json!({ "email": "not-an-address" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["details"][0]["path"], "email");

    // Asking for a link sends one email with a one-time token.
    let (status, body) = anon.post("/api/auth/email", json!({ "email": " Grace@Example.com " })).await;
    assert_eq!((status, &body), (StatusCode::ACCEPTED, &json!({ "sent": true, "expires_in_minutes": 15 })));
    {
        let mail = app.mail.lock().unwrap();
        let sent = mail.last().unwrap();
        assert_eq!(sent.to, "grace@example.com");
        assert!(sent.body.contains("http://localhost:3000/log-in/confirm?token="), "{}", sent.body);
    }
    let token = app.emailed_token();
    assert_eq!(token.len(), 64);

    let (status, body) = anon.post("/api/auth/email/confirm", json!({ "token": "0".repeat(64) })).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_token")));

    let (status, headers, me) = anon.send(Method::POST, "/api/auth/email/confirm", Some(json!({ "token": token }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"], "grace@example.com");
    assert_eq!(me["display_name"], "grace");
    assert_eq!(me["roles"], json!(["user"]));
    assert_eq!(me["plan"], "free");
    let grace = TestApp { cookie: Some(headers["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_string()), ..app.clone() };

    // The link works once.
    let (status, _) = anon.post("/api/auth/email/confirm", json!({ "token": token })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, me) = grace.get("/api/me").await;
    assert_eq!((status, me["email"].as_str()), (StatusCode::OK, Some("grace@example.com")));

    let (status, me) = grace.call(Method::PATCH, "/api/me", Some(json!({ "display_name": "  Grace H.  " }))).await;
    assert_eq!((status, me["display_name"].as_str()), (StatusCode::OK, Some("Grace H.")));
    let (status, _) = grace.call(Method::PATCH, "/api/me", Some(json!({ "display_name": " " }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Signing in again later finds the same user, name kept.
    let again = app.signed_in_as("GRACE@example.com").await;
    let (_, me2) = again.get("/api/me").await;
    assert_eq!((me2["id"].clone(), me2["display_name"].as_str()), (me["id"].clone(), Some("Grace H.")));

    let (status, headers, _) = grace.send(Method::POST, "/api/auth/sign-out", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["set-cookie"].to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(grace.get("/api/me").await.0, StatusCode::UNAUTHORIZED, "the old cookie no longer works");
    assert_eq!(again.get("/api/me").await.0, StatusCode::OK, "other sessions are untouched");
    app.close().await;
}

#[tokio::test]
async fn usage_and_entitlements_follow_the_user() {
    let app = TestApp::new().await;
    assert_eq!(app.anonymous().post("/api/runs", pd(json!({}))).await.0, StatusCode::UNAUTHORIZED, "runs need a signed-in user");
    assert_eq!(app.anonymous().get("/api/usage").await.0, StatusCode::UNAUTHORIZED);

    let (_, usage) = app.get("/api/usage").await;
    assert_eq!((usage["monthly_runs"].clone(), usage["used"].clone(), usage["remaining"].clone()), (json!(100), json!(0), json!(100)));
    assert_eq!(usage["period"].as_str().unwrap().len(), 7);
    app.run(pd(json!({ "iterations": 5 }))).await;
    app.run(pd(json!({ "iterations": 5 }))).await;
    let (_, me) = app.get("/api/me").await;
    assert_eq!((me["usage"]["used"].clone(), me["usage"]["remaining"].clone()), (json!(2), json!(98)));

    // Another user's count is their own.
    let bob = app.signed_in_as("bob@example.com").await;
    assert_eq!(bob.get("/api/usage").await.1["used"], 0);

    let (_, flags) = app.get("/api/me/entitlements").await;
    assert_eq!(flags, json!({ "plan": "free", "analytics_access": false, "quick_test_access": false, "private_worlds": false }));
    app.store.set_plan(me["id"].as_i64().unwrap(), "subscriber").await.unwrap();
    let (_, flags) = app.get("/api/me/entitlements").await;
    assert_eq!(flags, json!({ "plan": "subscriber", "analytics_access": true, "quick_test_access": true, "private_worlds": true }));
    assert_eq!(app.get("/api/usage").await.1["monthly_runs"], 1000);
    app.close().await;
}

#[tokio::test]
async fn logging_in_never_makes_an_account_and_signing_up_does() {
    let app = TestApp::new().await;
    let anon = app.anonymous();
    let latest = || {
        let mail = app.mail.lock().unwrap();
        let sent = mail.last().unwrap();
        (sent.subject.clone(), sent.body.clone())
    };

    // Logging in with an address that has no account: the page is told the same, the email says there is no account.
    let (status, body) = anon.post("/api/auth/email", json!({ "email": "new@example.com", "intent": "log_in" })).await;
    assert_eq!((status, &body), (StatusCode::ACCEPTED, &json!({ "sent": true, "expires_in_minutes": 15 })));
    let (subject, text) = latest();
    assert_eq!(subject, "No Lilanaya account for this address yet");
    assert!(text.contains("http://localhost:3000/sign-up") && !text.contains("token="), "{text}");

    // Signing up sends the link, and using it makes the account.
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "new@example.com", "intent": "sign_up" })).await.0, StatusCode::ACCEPTED);
    assert_eq!(latest().0, "Finish signing up to Lilanaya");
    let (status, me) = anon.post("/api/auth/email/confirm", json!({ "token": app.emailed_token() })).await;
    assert_eq!((status, me["email"].as_str()), (StatusCode::OK, Some("new@example.com")));

    // From then on both ways send a log-in link.
    for intent in ["log_in", "sign_up"] {
        assert_eq!(anon.post("/api/auth/email", json!({ "email": "new@example.com", "intent": intent })).await.0, StatusCode::ACCEPTED);
        let (subject, text) = latest();
        assert_eq!(subject, "Your Lilanaya log-in link");
        assert!(text.contains("http://localhost:3000/log-in/confirm?token="), "{text}");
    }
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "new@example.com", "intent": "browse" })).await.0, StatusCode::BAD_REQUEST);
    app.close().await;
}

#[tokio::test]
async fn the_plans_are_public() {
    let app = TestApp::new().await;
    let (status, plans) = app.anonymous().get("/api/plans").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        plans,
        json!({
            "free": { "ai_requests": { "limit": 3, "per": "world" }, "monthly_runs": 100, "max_active_runs": 2 },
            "subscriber": { "ai_requests": { "limit": 20, "per": "month" }, "monthly_runs": 1000, "max_active_runs": 2 },
        })
    );
    app.close().await;
}
