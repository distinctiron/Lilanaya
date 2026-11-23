//! Nothing is given that was not paid for, and nobody but Stripe can say what was paid.

use super::*;

/// What the account has that costs money to give.
async fn holdings(app: &TestApp) -> Value {
    let (_, me) = app.get("/api/me").await;
    json!({ "plan": me["plan"], "packs": me["usage"]["packs"] })
}

fn nothing() -> Value {
    json!({ "plan": "free", "packs": { "strategies_left": 0, "runs_left": 0 } })
}

#[tokio::test]
async fn a_message_that_stripe_did_not_sign_just_now_is_refused_and_gives_nothing() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    // A real, paid session, so that the only thing wrong with each message is who signed it, or when.
    let session = app.start_checkout("large").await;
    stripe.pay(&session, None, Some("pi_1"), 2199);
    let body = json!({ "id": "evt_1", "type": "checkout.session.completed", "data": { "object": { "id": session, "customer": "cus_1" } } }).to_string();
    let now = unix_now();
    let good = signature(TEST_WEBHOOK_SECRET, &body, now);

    let forged = [
        ("no signature", None),
        ("an empty one", Some(String::new())),
        ("signed with another secret", Some(signature("whsec_guess", &body, now))),
        ("signed for another body", Some(signature(TEST_WEBHOOK_SECRET, &body.replace("evt_1", "evt_2"), now))),
        ("recorded six minutes ago", Some(signature(TEST_WEBHOOK_SECRET, &body, now - 360))),
        ("dated in the future", Some(signature(TEST_WEBHOOK_SECRET, &body, now + 360))),
        ("the time changed after signing", Some(good.replace(&format!("t={now}"), &format!("t={}", now - 1)))),
        ("only the time", Some(format!("t={now}"))),
    ];
    for (what, header) in forged {
        let (status, answer) = app.anonymous().webhook_raw(&body, header.as_deref()).await;
        assert_eq!((status, answer["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_signature")), "{what}");
        assert_eq!(holdings(&app).await, nothing(), "{what}");
    }
    // A signed-in user's cookie does not stand in for Stripe's signature.
    assert_eq!(app.webhook_raw(&body, None).await.0, StatusCode::BAD_REQUEST);
    // Signed, but not JSON, or not an event.
    for junk in ["not json", "{}", "[]"] {
        let (status, _) = app.webhook_raw(junk, Some(&signature(TEST_WEBHOOK_SECRET, junk, now))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{junk}");
    }
    assert_eq!(holdings(&app).await, nothing());

    // The same message with Stripe's signature is acted on, once.
    assert_eq!(app.webhook_raw(&body, Some(&good)).await.0, StatusCode::OK);
    assert_eq!(app.webhook_raw(&body, Some(&good)).await.1, json!({ "received": true, "duplicate": true }));
    assert_eq!(holdings(&app).await["packs"], json!({ "strategies_left": 50, "runs_left": 500 }));
    app.close().await;
}

#[tokio::test]
async fn what_a_message_claims_is_checked_with_stripe_itself() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let session = app.start_checkout("large").await;
    // A signed message says the session is paid; Stripe's own record says it is still open.
    let claim = json!({ "id": session, "object": "checkout.session", "customer": "cus_1", "payment_status": "paid", "status": "complete", "metadata": { "item": "large" } });
    assert_eq!(app.stripe_says("evt_1", "checkout.session.completed", claim).await.0, StatusCode::OK);
    assert_eq!(holdings(&app).await, nothing());
    // A session Stripe has never heard of.
    assert_eq!(app.stripe_says("evt_2", "checkout.session.completed", json!({ "id": "cs_test_made_up", "customer": "cus_1" })).await.0, StatusCode::OK);
    // A subscription that is said to be active but is not at Stripe, and one that is not there at all.
    stripe.put_subscription("sub_1", "cus_1", "incomplete", unix_now(), unix_now() + 30 * DAY, "open");
    for (n, sub) in ["sub_1", "sub_made_up"].iter().enumerate() {
        assert_eq!(app.stripe_says(&format!("evt_sub_{n}"), "customer.subscription.updated", json!({ "id": sub, "customer": "cus_1", "status": "active" })).await.0, StatusCode::OK);
    }
    assert_eq!(holdings(&app).await, nothing());

    // If Stripe cannot be asked, the message is not accepted, so Stripe sends it again later.
    stripe.pay(&session, None, Some("pi_1"), 2199);
    *stripe.broken.lock().unwrap() = Some(503);
    let object = json!({ "id": session, "customer": "cus_1" });
    assert_eq!(app.stripe_says("evt_3", "checkout.session.completed", object.clone()).await.0, StatusCode::BAD_GATEWAY);
    assert_eq!(holdings(&app).await, nothing());
    *stripe.broken.lock().unwrap() = None;
    assert_eq!(app.stripe_says("evt_3", "checkout.session.completed", object).await, (StatusCode::OK, json!({ "received": true })));
    assert_eq!(holdings(&app).await["packs"]["strategies_left"], 50);
    app.close().await;
}

#[tokio::test]
async fn nobody_gets_what_somebody_else_paid_for() {
    let (ada, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let bob = ada.signed_in_as("bob@example.com").await;
    let session = ada.start_checkout("large").await;
    stripe.pay(&session, None, Some("pi_1"), 2199);

    // Bob comes "back from Stripe" with Ada's session: to him there is no such payment, and it is not applied.
    let refused = bob.post("/api/billing/confirm", json!({ "session_id": session })).await;
    assert_eq!((refused.0, refused.1["error"]["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
    // Exactly what he is told about a session that does not exist.
    let made_up = bob.post("/api/billing/confirm", json!({ "session_id": "cs_test_99" })).await;
    assert_eq!(refused, made_up);
    assert_eq!((holdings(&ada).await, holdings(&bob).await), (nothing(), nothing()));
    // Ids that are not ids are not put in an address at Stripe.
    for id in ["../../v1/customers", "cs_test_1?expand[]=customer", "", "cs test"] {
        assert_eq!(bob.post("/api/billing/confirm", json!({ "session_id": id })).await.0, StatusCode::NOT_FOUND, "{id:?}");
    }

    // A paid session that names Bob but was paid by another customer record than the one made for him.
    let bobs = bob.start_checkout("small").await;
    stripe.pay(&bobs, None, Some("pi_2"), 599);
    stripe.sessions.lock().unwrap().get_mut(&bobs).unwrap()["customer"] = json!("cus_1");
    assert_eq!(bob.post("/api/billing/confirm", json!({ "session_id": bobs })).await.1["paid"], false);
    assert_eq!(holdings(&bob).await, nothing());

    // Ada's subscription does not make Bob a subscriber, whatever customer a message about it names.
    let now = unix_now();
    ada.subscribe(&stripe, "sub_1", now - 60, now + 30 * DAY).await;
    assert_eq!(bob.stripe_says("evt_x", "customer.subscription.updated", json!({ "id": "sub_1", "customer": "cus_2" })).await.0, StatusCode::OK);
    assert_eq!((holdings(&ada).await["plan"].as_str(), holdings(&bob).await["plan"].as_str()), (Some("subscriber"), Some("free")));
    // Bob cannot open Ada's portal, see what she bought, or take her data.
    assert_eq!(bob.get("/api/billing").await.1["subscription"], Value::Null);
    assert_eq!(bob.post("/api/billing/portal", json!({})).await.1, json!({ "url": "https://billing.stripe.test/cus_2" }));
    assert!(!bob.get("/api/me/export").await.1.to_string().contains("sub_1"));
    ada.close().await;
}

#[tokio::test]
async fn the_buyer_cannot_choose_what_a_payment_is_worth() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    // Only what is in the catalog, by its name; no amounts, prices or quantities from the browser.
    for body in [
        json!({ "item": "small", "agree": true, "strategies": 1000 }),
        json!({ "item": "small", "agree": true, "price": "price_free" }),
        json!({ "item": "price_small", "agree": true }),
        json!({ "item": "", "agree": true }),
        json!({ "item": ["small"], "agree": true }),
        json!({ "agree": true }),
    ] {
        let (status, answer) = app.post("/api/billing/checkout", body.clone()).await;
        assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY, "{body}: {status} {answer}");
    }
    assert!(stripe.posted("/v1/checkout/sessions").is_empty());
    // The price and the size sent to Stripe come from the server's own settings.
    app.start_checkout("small").await;
    let form = &stripe.posted("/v1/checkout/sessions")[0];
    assert_eq!((form["line_items[0][price]"].as_str(), form["line_items[0][quantity]"].as_str(), form["client_reference_id"].as_str()), ("price_small", "1", "1"));

    // A pack that has no price set is not for sale.
    let fake = FakeStripe::default();
    let only_subscription = Billing::new(&fake.start().await, "sk_test_key".into(), TEST_WEBHOOK_SECRET.into(), "price_sub".into(), vec![], true).unwrap();
    let bare = TestApp::build(None, OAuth::none(), Some(only_subscription), Config::for_tests()).await;
    let refused = bare.post("/api/billing/checkout", json!({ "item": "small", "agree": true })).await;
    assert_eq!((refused.0, refused.1["error"]["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_for_sale")));
    bare.close().await;
    app.close().await;
}

#[tokio::test]
async fn runs_arriving_together_cannot_spend_a_pack_twice() {
    let (app, _stripe) = app_with_fake_stripe(None, Config { free_monthly_runs: 0, max_active_runs: 50, ..Config::for_tests() }).await;
    // A pack with three runs in it.
    let me = app.my_id().await;
    let tiny = crate::billing::Pack { id: "small", strategies: 0, runs: 3 };
    let paid = crate::billing::Session { id: "cs_tiny".into(), user_id: Some(me), customer: None, item: Some("small".into()), paid: true, subscription: None, payment_intent: None, amount: 1, currency: "eur".into() };
    assert!(app.store.record_purchase(me, &tiny, &paid).await.unwrap());
    assert!(!app.store.record_purchase(me, &tiny, &paid).await.unwrap(), "the same payment is not noted twice");

    let mut asks = Vec::new();
    for _ in 0..12 {
        let app = app.clone();
        asks.push(tokio::spawn(async move { app.post("/api/runs", pd(json!({ "iterations": 5 }))).await.0 }));
    }
    let mut accepted = 0;
    for ask in asks {
        match ask.await.unwrap() {
            StatusCode::ACCEPTED => accepted += 1,
            StatusCode::TOO_MANY_REQUESTS => {}
            other => panic!("unexpected answer {other}"),
        }
    }
    assert_eq!(accepted, 3);
    assert_eq!(app.get("/api/usage").await.1["packs"]["runs_left"], 0);
    app.close().await;
}

#[tokio::test]
async fn strategies_asked_for_together_cannot_spend_a_pack_twice() {
    let api = FakeApi::default();
    let (app, _stripe) = app_with_fake_stripe(Some(api.llm().await), Config { compiles_per_hour: 50, free_ai_requests_per_world: 0, ..Config::for_tests() }).await;
    let me = app.my_id().await;
    let tiny = crate::billing::Pack { id: "small", strategies: 2, runs: 0 };
    let paid = crate::billing::Session { id: "cs_tiny".into(), user_id: Some(me), customer: None, item: Some("small".into()), paid: true, subscription: None, payment_intent: None, amount: 1, currency: "eur".into() };
    app.store.record_purchase(me, &tiny, &paid).await.unwrap();
    for _ in 0..8 {
        api.say(GRIM_JSON);
    }
    let mut asks = Vec::new();
    for _ in 0..8 {
        let app = app.clone();
        asks.push(tokio::spawn(async move { app.post("/api/strategies/compile", json!({ "prompt": "Cooperate until betrayed, then defect for ever." })).await.0 }));
    }
    let mut written = 0;
    for ask in asks {
        if ask.await.unwrap() == StatusCode::OK {
            written += 1;
        }
    }
    assert_eq!(written, 2);
    assert_eq!(api.seen.lock().unwrap().len(), 2, "the model was asked twice, not eight times");
    app.close().await;
}

#[tokio::test]
async fn a_late_message_about_an_old_subscription_does_not_end_the_new_one() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let now = unix_now();
    app.subscribe(&stripe, "sub_old", now - 40 * DAY, now - 10 * DAY).await;
    stripe.subscriptions.lock().unwrap().get_mut("sub_old").unwrap()["status"] = json!("canceled");
    let old = json!({ "id": "sub_old", "object": "subscription", "customer": "cus_1" });
    assert_eq!(app.stripe_says("evt_ended", "customer.subscription.deleted", old.clone()).await.0, StatusCode::OK);
    assert_eq!(holdings(&app).await["plan"], "free");
    app.subscribe(&stripe, "sub_new", now - 60, now + 30 * DAY).await;
    assert_eq!(holdings(&app).await["plan"], "subscriber");
    // Stripe delivers an old message again, under a new id.
    assert_eq!(app.stripe_says("evt_ended_again", "customer.subscription.updated", old).await.0, StatusCode::OK);
    assert_eq!(holdings(&app).await["plan"], "subscriber");
    assert_eq!(app.get("/api/billing").await.1["subscription"]["status"], "active");
    app.close().await;
}

#[tokio::test]
async fn subscribing_late_in_the_month_does_not_give_two_allowances_for_one_payment() {
    let api = FakeApi::default();
    let (app, stripe) = app_with_fake_stripe(Some(api.llm().await), Config { compiles_per_hour: 50, subscriber_ai_requests_per_month: 2, ..Config::for_tests() }).await;
    let ask = || app.post("/api/strategies/compile", json!({ "prompt": "Cooperate until betrayed, then defect for ever." }));
    let now = unix_now();
    // Paid ten days ago; the calendar month has turned since (the calls are made "last month").
    app.subscribe(&stripe, "sub_1", now - 10 * DAY, now + 20 * DAY).await;
    api.say(GRIM_JSON).say(GRIM_JSON);
    assert_eq!(ask().await.0, StatusCode::OK);
    assert_eq!(ask().await.0, StatusCode::OK);
    app.store.backdate("llm_calls", "9 days").await;
    assert_eq!(ask().await.1["error"]["code"], "ai_limit_reached", "the first of the month is not a new allowance: the paid period is");
    assert_eq!(api.seen.lock().unwrap().len(), 2);
    app.close().await;
}
