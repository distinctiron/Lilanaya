//! Sessions, sign-in links and the federated round trip.

use super::*;

#[tokio::test]
async fn an_unverified_address_does_not_sign_anyone_in() {
    let (app, fake, _) = app_with_fake_providers().await;
    for provider in ["google", "apple", "facebook", "github"] {
        // Someone claims ada's address at the provider without having proved it is theirs.
        fake.approve_as(json!({ "sub": "666", "email": "ada@example.com", "email_verified": false, "name": "Not Ada" }));
        let (to, browser) = app.federated(provider, "?next=/runs").await;
        assert_eq!(to, "/log-in?error=no_email&next=%2Fruns", "{provider}");
        assert!(browser.is_none(), "{provider}");
    }
    app.close().await;
}

#[tokio::test]
async fn the_answer_must_come_back_to_the_browser_that_asked() {
    let (app, fake, _) = app_with_fake_providers().await;
    fake.approve_as(json!({ "sub": "g-1", "email": "eve@example.com", "email_verified": true, "name": "Eve" }));
    let (browser, state, _) = app.start_federated("google", "?next=//evil.example").await;
    let callback = |provider: &str, query: String| format!("/api/auth/{provider}/callback?{query}");
    let ends_at = |answer: (StatusCode, axum::http::HeaderMap, Value)| {
        assert_eq!(answer.0, StatusCode::SEE_OTHER);
        assert!(cookie_set(&answer.1, "lilanaya_session").is_none());
        location(&answer.1)
    };

    // Another browser (no cookie), a made-up state, or the answer of another provider: refused before the provider is asked anything.
    let good = format!("code=good-code&state={state}");
    assert_eq!(ends_at(app.anonymous().send(Method::GET, &callback("google", good.clone()), None).await), "/log-in?error=state_mismatch");
    assert_eq!(ends_at(browser.send(Method::GET, &callback("google", format!("code=good-code&state={}", "0".repeat(64))), None).await), "/log-in?error=state_mismatch");
    assert_eq!(ends_at(browser.send(Method::GET, &callback("google", "code=good-code".into()), None).await), "/log-in?error=state_mismatch");
    assert_eq!(ends_at(browser.send(Method::GET, &callback("github", good.clone()), None).await), "/log-in?error=state_mismatch");
    assert!(fake.seen.lock().unwrap().is_empty());

    // Pressing Cancel at the provider, a provider error, and a code the provider refuses.
    assert_eq!(ends_at(browser.send(Method::GET, &callback("google", format!("error=access_denied&state={state}")), None).await), "/log-in?error=cancelled");
    assert_eq!(ends_at(browser.send(Method::GET, &callback("google", format!("error=server_error&state={state}")), None).await), "/log-in?error=provider_error");
    assert_eq!(ends_at(browser.send(Method::GET, &callback("google", format!("state={state}")), None).await), "/log-in?error=provider_error");
    assert_eq!(ends_at(browser.send(Method::GET, &callback("google", format!("code=stale&state={state}")), None).await), "/log-in?error=provider_error");
    let (github, github_state, _) = app.start_federated("github", "").await;
    assert_eq!(ends_at(github.send(Method::GET, &callback("github", format!("code=stale&state={github_state}")), None).await), "/log-in?error=provider_error");

    // The real answer works, and "next" pointing at another site was dropped.
    let (status, headers, _) = browser.send(Method::GET, &callback("google", good), None).await;
    assert_eq!((status, location(&headers).as_str()), (StatusCode::SEE_OTHER, "/profile"));
    assert!(cookie_set(&headers, "lilanaya_session").is_some());
    app.close().await;
}

#[tokio::test]
async fn the_session_cookie_is_out_of_reach_of_scripts_and_other_sites() {
    for (public_url, secure) in [("http://localhost:3000", false), ("https://lilanaya.example", true)] {
        let app = TestApp::with_config(Config { public_url: public_url.into(), ..Config::for_tests() }).await;
        let anon = app.anonymous();
        assert_eq!(anon.post("/api/auth/email", json!({ "email": "grace@example.com" })).await.0, StatusCode::ACCEPTED);
        let (_, headers, _) = anon.send(Method::POST, "/api/auth/email/confirm", Some(json!({ "token": app.emailed_token() }))).await;
        let cookie = headers["set-cookie"].to_str().unwrap();
        let parts: Vec<&str> = cookie.split("; ").collect();
        assert!(parts[0].starts_with("lilanaya_session=") && parts[0].len() == "lilanaya_session=".len() + 64, "{cookie}");
        for needed in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=2592000"] {
            assert!(parts.contains(&needed), "{public_url}: the cookie lacks {needed}: {cookie}");
        }
        assert_eq!(parts.contains(&"Secure"), secure, "{public_url}: {cookie}");
        assert!(!cookie.contains("Domain="), "the cookie must belong to this host alone: {cookie}");

        // Signing out clears it with the same attributes.
        let (_, headers, _) = anon.send(Method::POST, "/api/auth/sign-out", None).await;
        let cleared = headers["set-cookie"].to_str().unwrap();
        assert!(cleared.starts_with("lilanaya_session=;") && cleared.contains("Max-Age=0") && cleared.contains("HttpOnly") && cleared.contains("Secure") == secure, "{cleared}");
        app.close().await;
    }
}

#[tokio::test]
async fn a_cookie_nobody_was_given_signs_nobody_in() {
    let app = TestApp::new().await;
    let real = app.cookie.clone().unwrap();
    let token = real.split_once('=').unwrap().1;
    let flipped = format!("{}{}", &token[..63], if token.ends_with('0') { '1' } else { '0' });
    for forged in [
        "lilanaya_session=".to_string(),
        format!("lilanaya_session={}", "0".repeat(64)),
        format!("lilanaya_session={flipped}"),
        format!("lilanaya_session={}", token.to_uppercase()),
        format!("lilanaya_session={}", crate::auth::hash_token(token)),
        "lilanaya_session=' OR '1'='1".to_string(),
        format!("lilanaya_session={}", "a".repeat(5000)),
        format!("session={token}"),
        format!("lilanaya_session_x={token}"),
    ] {
        let browser = TestApp { cookie: Some(forged.clone()), ..app.clone() };
        assert_eq!(browser.get("/api/me").await.0, StatusCode::UNAUTHORIZED, "{forged}");
        assert_eq!(browser.get("/api/session").await, (StatusCode::OK, json!({ "user": null })), "{forged}");
        assert_eq!(browser.post("/api/runs", pd(json!({}))).await.0, StatusCode::UNAUTHORIZED, "{forged}");
    }
    // The real one, among other cookies, still works.
    let browser = TestApp { cookie: Some(format!("theme=dark; {real}; other=1")), ..app.clone() };
    assert_eq!(browser.get("/api/me").await.0, StatusCode::OK);
    app.close().await;
}

#[tokio::test]
async fn sign_in_links_and_sessions_stop_working_when_their_time_is_up() {
    let app = TestApp::new().await;
    let anon = app.anonymous();

    // A link is good for 15 minutes.
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "grace@example.com" })).await.0, StatusCode::ACCEPTED);
    let token = app.emailed_token();
    app.store.backdate("sign_in_tokens", "14 minutes").await;
    app.store.backdate("sign_in_tokens", "2 minutes").await;
    let (status, body) = anon.post("/api/auth/email/confirm", json!({ "token": token })).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_token")), "{body}");
    assert_eq!(app.get("/api/strategies").await.0, StatusCode::OK);

    // A session is good for 30 days.
    assert_eq!(app.get("/api/me").await.0, StatusCode::OK);
    app.store.backdate("sessions", "29 days").await;
    assert_eq!(app.get("/api/me").await.0, StatusCode::OK, "a day before the end");
    app.store.backdate("sessions", "2 days").await;
    assert_eq!(app.get("/api/me").await.0, StatusCode::UNAUTHORIZED, "a day after it");
    assert_eq!(app.get("/api/session").await.1, json!({ "user": null }));
    app.close().await;
}

#[tokio::test]
async fn a_sign_in_link_is_worth_one_session_for_one_address() {
    let app = TestApp::new().await;
    let anon = app.anonymous();
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "grace@example.com" })).await.0, StatusCode::ACCEPTED);
    let token = app.emailed_token();

    // Ten browsers present the same link at once: one gets in.
    let attempts: Vec<_> = (0..10)
        .map(|_| {
            let (anon, token) = (anon.clone(), token.clone());
            tokio::spawn(async move { anon.post("/api/auth/email/confirm", json!({ "token": token })).await.0 })
        })
        .collect();
    let mut answers = Vec::new();
    for attempt in attempts {
        answers.push(attempt.await.unwrap());
    }
    assert_eq!(answers.iter().filter(|s| **s == StatusCode::OK).count(), 1, "{answers:?}");
    assert_eq!(answers.iter().filter(|s| **s == StatusCode::BAD_REQUEST).count(), 9, "{answers:?}");

    // Near misses of a token are worth nothing, and say nothing about why.
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "grace@example.com" })).await.0, StatusCode::ACCEPTED);
    let fresh = app.emailed_token();
    let mut answers = Vec::new();
    for wrong in [fresh[..63].to_string(), format!("{fresh}0"), fresh.to_uppercase(), crate::auth::hash_token(&fresh), String::new(), "0".repeat(64)] {
        let (status, body) = anon.post("/api/auth/email/confirm", json!({ "token": wrong })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong}");
        answers.push(body);
    }
    assert!(answers.windows(2).all(|pair| pair[0] == pair[1]), "every wrong token gets the same answer");
    assert_eq!(anon.post("/api/auth/email/confirm", json!({ "token": format!("  {fresh}\n") })).await.0, StatusCode::OK, "the real one, as pasted from an email");
    app.close().await;
}

#[tokio::test]
async fn a_display_name_is_plain_text_of_a_sensible_length() {
    let app = TestApp::new().await;
    for bad in ["", "   ", "line\nbreak", "tab\there", "bell\u{7}", &"x".repeat(61)] {
        let (status, body) = app.call(Method::PATCH, "/api/me", Some(json!({ "display_name": bad }))).await;
        assert_eq!((status, body["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("display_name")), "{bad:?}");
    }
    // Markup is kept as the text it is; nothing in the API treats it as anything else.
    let name = "<img src=x onerror=alert(1)>";
    let (status, me) = app.call(Method::PATCH, "/api/me", Some(json!({ "display_name": name }))).await;
    assert_eq!((status, me["display_name"].as_str()), (StatusCode::OK, Some(name)));
    // Nobody can rename, or become, someone else through the profile.
    for extra in [json!({ "display_name": "x", "email": "eve@example.com" }), json!({ "display_name": "x", "plan": "subscriber" }), json!({ "display_name": "x", "roles": ["admin"] }), json!({ "display_name": "x", "id": 2 })] {
        assert_eq!(app.call(Method::PATCH, "/api/me", Some(extra.clone())).await.0, StatusCode::BAD_REQUEST, "{extra}");
    }
    let (_, me) = app.get("/api/me").await;
    assert_eq!((me["email"].as_str(), me["plan"].as_str(), me["roles"].clone()), (Some("ada@example.com"), Some("free"), json!(["user"])));
    app.close().await;
}
