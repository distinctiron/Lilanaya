//! Sessions, sign-in links, passwords and the federated round trip.

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
        let (_, headers, _) = anon.send(Method::POST, "/api/auth/email/confirm", Some(json!({ "token": app.emailed_token(), "password": TEST_PASSWORD }))).await;
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
    let (status, body) = anon.post("/api/auth/email/confirm", json!({ "token": token, "password": TEST_PASSWORD })).await;
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
            tokio::spawn(async move { anon.post("/api/auth/email/confirm", json!({ "token": token, "password": TEST_PASSWORD })).await.0 })
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
        let (status, body) = anon.post("/api/auth/email/confirm", json!({ "token": wrong, "password": TEST_PASSWORD })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong}");
        answers.push(body);
    }
    assert!(answers.windows(2).all(|pair| pair[0] == pair[1]), "every wrong token gets the same answer");
    assert_eq!(anon.post("/api/auth/email/confirm", json!({ "token": format!("  {fresh}\n"), "password": TEST_PASSWORD })).await.0, StatusCode::OK, "the real one, as pasted from an email");
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

#[tokio::test]
async fn a_password_must_be_long_enough_and_a_poor_one_does_not_use_up_the_link() {
    let app = TestApp::new().await;
    let anon = app.anonymous();
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "grace@example.com" })).await.0, StatusCode::ACCEPTED);
    let token = app.emailed_token();
    for bad in [String::new(), "short".into(), "1234567".into(), "ååååååå".into(), "x".repeat(201)] {
        let (status, body) = anon.post("/api/auth/email/confirm", json!({ "token": token, "password": bad })).await;
        assert_eq!((status, body["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("password")), "{bad:?}");
    }
    // Without a password there is no getting in through the link at all.
    assert_eq!(anon.post("/api/auth/email/confirm", json!({ "token": token })).await.0, StatusCode::BAD_REQUEST);
    assert!(!app.store.user_exists("grace@example.com").await.unwrap(), "nothing was made of the failed tries");
    // The link is still good for a password that will do: eight characters, of any kind.
    assert_eq!(anon.post("/api/auth/email/confirm", json!({ "token": token, "password": "åäö 5678" })).await.0, StatusCode::OK);
    assert_eq!(anon.post("/api/auth/log-in", json!({ "email": "grace@example.com", "password": "åäö 5678" })).await.0, StatusCode::OK);
    assert_eq!(anon.post("/api/auth/log-in", json!({ "email": "grace@example.com", "password": "x".repeat(200) })).await.0, StatusCode::UNAUTHORIZED);
    app.close().await;
}

#[tokio::test]
async fn a_wrong_password_and_an_address_without_an_account_get_the_same_answer() {
    let (app, fake, _) = app_with_fake_providers().await;
    // Eve has an account, made through Google, and never chose a password.
    fake.approve_as(json!({ "sub": "g-1", "email": "eve@example.com", "email_verified": true, "name": "Eve" }));
    assert!(app.federated("google", "").await.1.is_some());
    let anon = app.anonymous();
    let mut answers = Vec::new();
    for (email, password) in [
        ("ada@example.com", "not the password"),
        ("ada@example.com", ""),
        ("ada@example.com", "CORRECT HORSE BATTERY"),
        ("ada@example.com", "' OR '1'='1"),
        ("nobody@example.com", TEST_PASSWORD),
        ("eve@example.com", TEST_PASSWORD),
        ("eve@example.com", ""),
        ("not-an-address", TEST_PASSWORD),
    ] {
        let (status, headers, body) = anon.send(Method::POST, "/api/auth/log-in", Some(json!({ "email": email, "password": password }))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{email} {password:?}");
        assert!(headers.get("set-cookie").is_none(), "{email} {password:?}");
        answers.push(body);
    }
    assert_eq!(answers[0]["error"], json!({ "code": "invalid_credentials", "message": "Wrong email address or password" }));
    assert!(answers.windows(2).all(|pair| pair[0] == pair[1]), "every failure gets the same answer");
    // Nothing but an address and a password is taken.
    assert_eq!(anon.post("/api/auth/log-in", json!({ "email": "ada@example.com", "password": TEST_PASSWORD, "user_id": 1 })).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(anon.post("/api/auth/log-in", json!({ "email": "ada@example.com" })).await.0, StatusCode::BAD_REQUEST);
    // The right one still works after all that.
    assert_eq!(anon.post("/api/auth/log-in", json!({ "email": " ADA@example.com ", "password": TEST_PASSWORD })).await.0, StatusCode::OK);
    app.close().await;
}

#[tokio::test]
async fn guessing_passwords_for_an_address_is_stopped_after_ten_wrong_ones() {
    let app = TestApp::new().await;
    let anon = app.anonymous();
    let try_with = |password: &'static str| {
        let anon = anon.clone();
        async move { anon.post("/api/auth/log-in", json!({ "email": "ada@example.com", "password": password })).await }
    };
    // Right ones in between are not counted.
    for _ in 0..3 {
        assert_eq!(try_with(TEST_PASSWORD).await.0, StatusCode::OK);
    }
    // Twenty guesses at once: ten are looked at, the rest are refused unseen.
    let guesses: Vec<_> = (0..20).map(|_| tokio::spawn(try_with("a wrong guess"))).collect();
    let mut answers = Vec::new();
    for guess in guesses {
        answers.push(guess.await.unwrap().0);
    }
    assert_eq!(answers.iter().filter(|s| **s == StatusCode::UNAUTHORIZED).count(), 10, "{answers:?}");
    assert_eq!(answers.iter().filter(|s| **s == StatusCode::TOO_MANY_REQUESTS).count(), 10, "{answers:?}");
    // From then on even the right password is refused, and the answer says what to do.
    let (status, body) = try_with(TEST_PASSWORD).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_many_requests")));
    assert!(body["error"]["message"].as_str().unwrap().contains("choose a new password by email"), "{body}");
    // Nobody else is held up, and the session ada already had goes on.
    assert_eq!(app.signed_in_as("bob@example.com").await.get("/api/me").await.0, StatusCode::OK);
    assert_eq!(app.get("/api/me").await.0, StatusCode::OK);
    // After 15 minutes the address can be tried again.
    app.store.backdate("log_in_attempts", "14 minutes").await;
    assert_eq!(try_with(TEST_PASSWORD).await.0, StatusCode::TOO_MANY_REQUESTS);
    app.store.backdate("log_in_attempts", "2 minutes").await;
    assert_eq!(try_with(TEST_PASSWORD).await.0, StatusCode::OK);

    // Choosing a new password by email lets the owner in at once, however many guesses there were.
    for _ in 0..10 {
        assert_eq!(try_with("a wrong guess").await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(try_with(TEST_PASSWORD).await.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "ada@example.com", "intent": "log_in" })).await.0, StatusCode::ACCEPTED);
    assert_eq!(anon.post("/api/auth/email/confirm", json!({ "token": app.emailed_token(), "password": "a new password" })).await.0, StatusCode::OK);
    assert_eq!(try_with("a new password").await.0, StatusCode::OK);
    app.close().await;
}

#[tokio::test]
async fn a_new_password_ends_the_old_one_and_every_session_that_came_with_it() {
    let app = TestApp::new().await;
    let anon = app.anonymous();
    let elsewhere = app.signed_in_as("ada@example.com").await;
    let bob = app.signed_in_as("bob@example.com").await;
    assert_eq!((app.get("/api/me").await.0, elsewhere.get("/api/me").await.0), (StatusCode::OK, StatusCode::OK));

    assert_eq!(anon.post("/api/auth/email", json!({ "email": "ada@example.com", "intent": "log_in" })).await.0, StatusCode::ACCEPTED);
    let (status, headers, me) = anon.send(Method::POST, "/api/auth/email/confirm", Some(json!({ "token": app.emailed_token(), "password": "a new password" }))).await;
    assert_eq!((status, me["email"].as_str(), me["has_password"].as_bool()), (StatusCode::OK, Some("ada@example.com"), Some(true)));
    let fresh = TestApp { cookie: cookie_set(&headers, "lilanaya_session"), ..app.clone() };

    assert_eq!(fresh.get("/api/me").await.0, StatusCode::OK, "the browser that chose the password is in");
    assert_eq!(app.get("/api/me").await.0, StatusCode::UNAUTHORIZED, "a session from before is over");
    assert_eq!(elsewhere.get("/api/me").await.0, StatusCode::UNAUTHORIZED, "on every device");
    assert_eq!(bob.get("/api/me").await.0, StatusCode::OK, "nobody else is logged out");
    assert_eq!(anon.post("/api/auth/log-in", json!({ "email": "ada@example.com", "password": TEST_PASSWORD })).await.0, StatusCode::UNAUTHORIZED, "the old password");
    assert_eq!(anon.post("/api/auth/log-in", json!({ "email": "ada@example.com", "password": "a new password" })).await.0, StatusCode::OK);
    app.close().await;
}

#[tokio::test]
async fn the_password_itself_is_kept_nowhere_and_its_hash_is_shown_to_nobody() {
    let app = TestApp::new().await;
    let kept: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE email = 'ada@example.com'").fetch_one(app.store.pool()).await.unwrap();
    assert!(kept.starts_with("$argon2id$") && !kept.contains(TEST_PASSWORD), "{kept}");
    let salt = kept.split('$').nth(4).unwrap();
    for path in ["/api/me", "/api/session", "/api/me/export", "/api/usage"] {
        let text = app.get(path).await.1.to_string();
        assert!(!text.contains(TEST_PASSWORD) && !text.contains("argon2") && !text.contains(salt) && !text.contains("password_hash"), "{path}: {text}");
    }
    // Two people with the same password do not have the same hash.
    app.signed_in_as("bob@example.com").await;
    let bobs: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE email = 'bob@example.com'").fetch_one(app.store.pool()).await.unwrap();
    assert_ne!(kept, bobs);
    // Deleting the account takes the wrong guesses made at it away too.
    assert_eq!(app.anonymous().post("/api/auth/log-in", json!({ "email": "ada@example.com", "password": "a wrong guess" })).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(app.call(Method::DELETE, "/api/me", Some(json!({ "email": "ada@example.com" }))).await.0, StatusCode::OK);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM log_in_attempts WHERE email = 'ada@example.com'").fetch_one(app.store.pool()).await.unwrap();
    assert_eq!(left, 0);
    app.close().await;
}
