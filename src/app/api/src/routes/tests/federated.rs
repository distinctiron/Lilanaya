//! Sign-in through Google, Apple, Facebook and GitHub, against stand-in providers.

use super::*;

#[tokio::test]
async fn only_providers_with_credentials_are_offered() {
    let app = TestApp::new().await;
    assert_eq!(app.anonymous().get("/api/auth/providers").await, (StatusCode::OK, json!({ "providers": [] })));
    let (status, headers, _) = app.anonymous().send(Method::GET, "/api/auth/google/start?next=/runs", None).await;
    assert_eq!((status, location(&headers).as_str()), (StatusCode::SEE_OTHER, "/log-in?error=provider_unavailable&next=%2Fruns"));
    app.close().await;

    let (app, _, _) = app_with_fake_providers().await;
    let (_, body) = app.anonymous().get("/api/auth/providers").await;
    assert_eq!(body, json!({ "providers": [{ "id": "google", "name": "Google" }, { "id": "apple", "name": "Apple" }, { "id": "facebook", "name": "Facebook" }, { "id": "github", "name": "GitHub" }] }));
    let (status, headers, _) = app.anonymous().send(Method::GET, "/api/auth/myspace/start", None).await;
    assert_eq!((status, location(&headers).as_str()), (StatusCode::SEE_OTHER, "/log-in?error=provider_unavailable"));
    app.close().await;
}

#[tokio::test]
async fn each_provider_signs_a_new_person_in_and_recognises_them_next_time() {
    let (app, fake, base) = app_with_fake_providers().await;
    for (n, provider) in ["google", "apple", "facebook", "github"].into_iter().enumerate() {
        let email = format!("{provider}.person@example.com");
        fake.approve_as(json!({ "sub": format!("{}", 1000 + n), "email": format!("{provider}.Person@Example.com"), "email_verified": true, "name": format!("Person at {provider}") }));

        // The browser is sent to the provider with our client id, the state and the address to come back to.
        let (_, state, to) = app.start_federated(provider, "").await;
        assert!(to.starts_with(&format!("{base}/{provider}/authorize?")), "{to}");
        let sent = query_of(&to);
        assert_eq!((sent["client_id"].as_str(), sent["response_type"].as_str(), state.len()), (TEST_CLIENT_ID, "code", 64));
        assert_eq!(sent["redirect_uri"], format!("http://localhost:3000/api/auth/{provider}/callback"));

        let (to, browser) = app.federated(provider, "").await;
        assert_eq!(to, "/profile", "{provider}");
        let browser = browser.expect("signed in");
        let (status, me) = browser.get("/api/me").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!((me["email"].as_str(), me["display_name"].as_str()), (Some(email.as_str()), Some(format!("Person at {provider}").as_str())), "{provider}");

        // The code was traded in with the secret (Apple: a signed token naming the team and the client).
        let (seen_provider, form) = fake.seen.lock().unwrap().last().unwrap().clone();
        assert_eq!(seen_provider, provider);
        assert_eq!((form["grant_type"].as_str(), form["code"].as_str(), form["client_id"].as_str()), ("authorization_code", "good-code", TEST_CLIENT_ID));
        assert_eq!(form["redirect_uri"], format!("http://localhost:3000/api/auth/{provider}/callback"));
        if provider == "apple" {
            assert_eq!(form["client_secret"].split('.').count(), 3, "a JWT");
        } else {
            assert_eq!(form["client_secret"], TEST_SECRET);
        }

        // Next time, with a new address at the provider and another name: still the same user, name kept.
        fake.approve_as(json!({ "sub": format!("{}", 1000 + n), "email": "moved@example.com", "email_verified": true, "name": "Someone Else" }));
        let (to, again) = app.federated(provider, "?next=%2Fruns%3Fmine%3D1").await;
        assert_eq!(to, "/runs?mine=1", "back to the page they came from");
        let (_, me2) = again.unwrap().get("/api/me").await;
        assert_eq!((&me2["id"], &me2["email"], &me2["display_name"]), (&me["id"], &me["email"], &me["display_name"]), "{provider}");
    }
    app.close().await;
}

#[tokio::test]
async fn a_verified_address_joins_the_account_that_already_has_it() {
    let (app, fake, _) = app_with_fake_providers().await;
    let (_, ada) = app.get("/api/me").await; // signed in by emailed link as ada@example.com
    for (provider, sub) in [("google", "g-ada"), ("github", "77")] {
        fake.approve_as(json!({ "sub": sub, "email": "Ada@example.com", "email_verified": true, "name": "Ada Lovelace" }));
        let (to, browser) = app.federated(provider, "").await;
        assert_eq!(to, "/profile");
        let (_, me) = browser.unwrap().get("/api/me").await;
        assert_eq!((&me["id"], &me["display_name"]), (&ada["id"], &ada["display_name"]), "{provider}: the same user, name untouched");
    }
    // A name longer than a display name may be is cut; no name at all falls back to the address.
    fake.approve_as(json!({ "sub": "g-long", "email": "long@example.com", "email_verified": true, "name": "x".repeat(200) }));
    let (_, me) = app.federated("google", "").await.1.unwrap().get("/api/me").await;
    assert_eq!(me["display_name"].as_str().unwrap().len(), 60);
    fake.approve_as(json!({ "sub": "g-quiet", "email": "quiet@example.com", "email_verified": true }));
    let (_, me) = app.federated("google", "").await.1.unwrap().get("/api/me").await;
    assert_eq!(me["display_name"], "quiet");
    app.close().await;
}

#[tokio::test]
async fn apple_posts_its_answer_and_is_passed_on_to_the_ordinary_callback() {
    let (app, fake, _) = app_with_fake_providers().await;
    // Apple's ID token has no name; the name comes once, in the posted form.
    fake.approve_as(json!({ "sub": "001234.abc", "email": "x7@privaterelay.appleid.com", "email_verified": "true" }));
    let (browser, state, to) = app.start_federated("apple", "").await;
    assert_eq!(query_of(&to)["response_mode"], "form_post");

    // The cross-site POST arrives without cookies.
    let form = format!("code=good-code&state={state}&user=%7B%22name%22%3A%7B%22firstName%22%3A%22Tim%22%2C%22lastName%22%3A%22Apple%22%7D%7D");
    let (status, headers, _) = app.anonymous().raw(Method::POST, "/api/auth/apple/callback", Some("application/x-www-form-urlencoded"), Some(form)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let onward = location(&headers);
    assert_eq!(onward, format!("/api/auth/apple/callback?code=good-code&state={state}&name=Tim+Apple"));

    let (status, headers, _) = browser.send(Method::GET, &onward, None).await;
    assert_eq!((status, location(&headers).as_str()), (StatusCode::SEE_OTHER, "/profile"));
    let signed_in = TestApp { cookie: cookie_set(&headers, "lilanaya_session"), ..app.clone() };
    let (_, me) = signed_in.get("/api/me").await;
    assert_eq!((me["email"].as_str(), me["display_name"].as_str()), (Some("x7@privaterelay.appleid.com"), Some("Tim Apple")));

    // Cancelling at Apple is posted too.
    let (browser, state, _) = app.start_federated("apple", "").await;
    let (_, headers, _) = app.anonymous().raw(Method::POST, "/api/auth/apple/callback", Some("application/x-www-form-urlencoded"), Some(format!("error=user_cancelled_authorize&state={state}"))).await;
    let (_, headers, _) = browser.send(Method::GET, &location(&headers), None).await;
    assert_eq!(location(&headers), "/log-in?error=cancelled");
    app.close().await;
}
