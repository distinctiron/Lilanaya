//! Accounts: the emailed link, the password, the session, the profile, usage and plans.

use super::*;

#[tokio::test]
async fn sign_up_by_emailed_link_log_in_with_the_password_then_sign_out() {
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

    let (status, body) = anon.post("/api/auth/email/confirm", json!({ "token": "0".repeat(64), "password": "a password of my own" })).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_token")));

    let (status, headers, me) = anon.send(Method::POST, "/api/auth/email/confirm", Some(json!({ "token": token, "password": "a password of my own" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"], "grace@example.com");
    assert_eq!(me["has_password"], true);
    assert_eq!(me["display_name"], "grace");
    assert_eq!(me["roles"], json!(["user"]));
    assert_eq!(me["plan"], "free");
    let grace = TestApp { cookie: Some(headers["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_string()), ..app.clone() };

    // The link works once.
    let (status, _) = anon.post("/api/auth/email/confirm", json!({ "token": token, "password": "a password of my own" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, me) = grace.get("/api/me").await;
    assert_eq!((status, me["email"].as_str()), (StatusCode::OK, Some("grace@example.com")));

    let (status, me) = grace.call(Method::PATCH, "/api/me", Some(json!({ "display_name": "  Grace H.  " }))).await;
    assert_eq!((status, me["display_name"].as_str()), (StatusCode::OK, Some("Grace H.")));
    let (status, _) = grace.call(Method::PATCH, "/api/me", Some(json!({ "display_name": " " }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Logging in later takes the address and the password, no email, and finds the same user, name kept.
    let sent = app.mail.lock().unwrap().len();
    let (status, headers, me2) = anon.send(Method::POST, "/api/auth/log-in", Some(json!({ "email": "GRACE@example.com ", "password": "a password of my own" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(app.mail.lock().unwrap().len(), sent);
    let again = TestApp { cookie: cookie_set(&headers, "lilanaya_session"), ..app.clone() };
    assert_eq!(again.get("/api/me").await.1, me2);
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
    let (subject, text) = latest();
    assert_eq!(subject, "Finish signing up to Lilanaya");
    assert!(text.contains("Open this link to choose a password and finish signing up to Lilanaya:"), "{text}");
    let (status, me) = anon.post("/api/auth/email/confirm", json!({ "token": app.emailed_token(), "password": TEST_PASSWORD })).await;
    assert_eq!((status, me["email"].as_str()), (StatusCode::OK, Some("new@example.com")));

    // From then on both ways send a link to choose a new password.
    for intent in ["log_in", "sign_up"] {
        assert_eq!(anon.post("/api/auth/email", json!({ "email": "new@example.com", "intent": intent })).await.0, StatusCode::ACCEPTED);
        let (subject, text) = latest();
        assert_eq!(subject, "Choose a new Lilanaya password");
        assert!(text.contains("Open this link to choose a new password and log in to Lilanaya:"), "{text}");
        assert!(text.contains("http://localhost:3000/log-in/confirm?token="), "{text}");
    }
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "new@example.com", "intent": "browse" })).await.0, StatusCode::BAD_REQUEST);
    // A link that was asked for and never opened changes nothing: the password still works.
    assert_eq!(anon.post("/api/auth/log-in", json!({ "email": "new@example.com", "password": TEST_PASSWORD })).await.0, StatusCode::OK);
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
            // Payment is off here: nothing is for sale, so no prices.
            "billing": null,
        })
    );
    app.close().await;
}

#[tokio::test]
async fn an_account_made_through_another_site_has_no_password_until_one_is_chosen_by_email() {
    let (app, fake, _) = app_with_fake_providers().await;
    fake.approve_as(json!({ "sub": "g-1", "email": "eve@example.com", "email_verified": true, "name": "Eve" }));
    let eve = app.federated("google", "").await.1.expect("eve is in");
    let (_, me) = eve.get("/api/me").await;
    assert_eq!(me["has_password"], false);
    assert_eq!(app.get("/api/me").await.1["has_password"], true);

    // The link to her address lets her choose one; it is the same account.
    assert_eq!(eve.post("/api/auth/email", json!({ "email": "eve@example.com", "intent": "log_in" })).await.0, StatusCode::ACCEPTED);
    let (status, with_password) = eve.post("/api/auth/email/confirm", json!({ "token": app.emailed_token(), "password": "eve's own password" })).await;
    assert_eq!((status, with_password["id"].clone(), with_password["has_password"].clone()), (StatusCode::OK, me["id"].clone(), json!(true)));
    assert_eq!(app.anonymous().post("/api/auth/log-in", json!({ "email": "eve@example.com", "password": "eve's own password" })).await.1["id"], me["id"]);
    // And Google still lets her in.
    assert!(app.federated("google", "").await.1.is_some());
    app.close().await;
}
