//! Paying: the subscription, the packs, and leaving. Stripe is a stand-in (`billing::tests::FakeStripe`).

use super::*;

const PROMPT: &str = "Cooperate until betrayed, then defect for ever.";

fn code(answer: &(StatusCode, Value)) -> Option<&str> {
    answer.1["error"]["code"].as_str()
}

#[tokio::test]
async fn subscribing_makes_a_subscriber_for_the_period_that_was_paid() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let (_, me) = app.get("/api/me").await;
    assert_eq!((me["plan"].as_str(), me["features"]["billing"].as_bool()), (Some("free"), Some(true)));
    assert_eq!(app.get("/api/billing").await.1, json!({ "enabled": true, "plan": "free", "subscription": null, "packs": { "strategies_left": 0, "runs_left": 0 }, "purchases": [], "can_manage": false }));

    // Nothing is started without the tick in the box.
    let refused = app.post("/api/billing/checkout", json!({ "item": "subscription" })).await;
    assert_eq!((refused.0, refused.1["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("agree")), "{}", refused.1);
    assert!(stripe.posted("/v1/checkout/sessions").is_empty() && stripe.posted("/v1/customers").is_empty());

    let session = app.start_checkout("subscription").await;
    assert_eq!(stripe.posted("/v1/customers"), vec![BTreeMap::from([("email".to_string(), "ada@example.com".to_string()), ("metadata[user_id]".to_string(), "1".to_string())])]);
    let form = &stripe.posted("/v1/checkout/sessions")[0];
    assert_eq!((form["mode"].as_str(), form["customer"].as_str(), form["success_url"].as_str()), ("subscription", "cus_1", "http://localhost:3000/plans?paid={CHECKOUT_SESSION_ID}"));
    // Starting the page gives nothing: nothing is paid.
    assert_eq!(app.get("/api/me").await.1["plan"], "free");
    assert_eq!(app.post("/api/billing/confirm", json!({ "session_id": session })).await.1, json!({ "paid": false, "item": "subscription" }));
    assert_eq!(app.get("/api/me").await.1["plan"], "free");

    // The buyer pays; Stripe says so.
    let (start, end) = (unix_now() - 60, unix_now() + 30 * DAY);
    stripe.pay(&session, Some("sub_1"), None, 899);
    stripe.put_subscription("sub_1", "cus_1", "active", start, end, "paid");
    let said = app.stripe_says("evt_1", "checkout.session.completed", json!({ "id": session, "object": "checkout.session", "customer": "cus_1" })).await;
    assert_eq!(said, (StatusCode::OK, json!({ "received": true })));

    let (_, me) = app.get("/api/me").await;
    assert_eq!((me["plan"].as_str(), me["usage"]["monthly_runs"].as_i64(), me["usage"]["ai_requests"]["per"].as_str()), (Some("subscriber"), Some(1000), Some("month")));
    let (_, billing) = app.get("/api/billing").await;
    assert_eq!((billing["plan"].as_str(), billing["can_manage"].as_bool()), (Some("subscriber"), Some(true)));
    assert_eq!((billing["subscription"]["status"].as_str(), billing["subscription"]["paid"].as_bool(), billing["subscription"]["cancel_at_period_end"].as_bool()), (Some("active"), Some(true), Some(false)));
    // The allowance is counted over the paid period, and comes back when it ends.
    assert_eq!(me["usage"]["period_end"], billing["subscription"]["period_end"]);
    assert_eq!(me["usage"]["period_start"], billing["subscription"]["period_start"]);

    // One subscription at a time; the portal is where it is changed or cancelled.
    let again = app.post("/api/billing/checkout", json!({ "item": "subscription", "agree": true })).await;
    assert_eq!((again.0, code(&again)), (StatusCode::CONFLICT, Some("already_subscribed")));
    assert_eq!(app.post("/api/billing/portal", json!({})).await.1, json!({ "url": "https://billing.stripe.test/cus_1" }));
    // Someone who never bought anything has nothing to manage, and stays free.
    let bob = app.signed_in_as("bob@example.com").await;
    let nothing = bob.post("/api/billing/portal", json!({})).await;
    assert_eq!((nothing.0, code(&nothing)), (StatusCode::CONFLICT, Some("nothing_to_manage")));
    assert_eq!(bob.get("/api/me").await.1["plan"], "free");
    app.close().await;
}

#[tokio::test]
async fn coming_back_from_stripe_is_enough_when_stripe_s_message_is_late() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let session = app.start_checkout("subscription").await;
    stripe.pay(&session, Some("sub_1"), None, 899);
    stripe.put_subscription("sub_1", "cus_1", "active", unix_now() - 5, unix_now() + 30 * DAY, "paid");
    // No webhook: the browser returns to /plans?paid=<session> and the page asks.
    assert_eq!(app.post("/api/billing/confirm", json!({ "session_id": session })).await.1, json!({ "paid": true, "item": "subscription" }));
    assert_eq!(app.get("/api/me").await.1["plan"], "subscriber");
    // The message arriving afterwards, and the page asking again, change nothing.
    assert_eq!(app.stripe_says("evt_late", "checkout.session.completed", json!({ "id": session, "customer": "cus_1" })).await.0, StatusCode::OK);
    assert_eq!(app.post("/api/billing/confirm", json!({ "session_id": session })).await.0, StatusCode::OK);
    assert_eq!(app.get("/api/me").await.1["plan"], "subscriber");

    // A pack the same way, and only once.
    let session = app.start_checkout("medium").await;
    stripe.pay(&session, None, Some("pi_1"), 999);
    for _ in 0..3 {
        assert_eq!(app.post("/api/billing/confirm", json!({ "session_id": session })).await.1, json!({ "paid": true, "item": "medium" }));
    }
    assert_eq!(app.get("/api/billing").await.1["packs"], json!({ "strategies_left": 20, "runs_left": 200 }));
    app.close().await;
}

#[tokio::test]
async fn a_pack_adds_strategies_and_runs_once_however_often_stripe_says_so() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let (_, plans) = app.anonymous().get("/api/plans").await;
    assert_eq!(plans["billing"]["subscription"], json!({ "amount": 899, "currency": "eur", "interval": "month" }));
    assert_eq!(plans["billing"]["packs"][2], json!({ "id": "large", "strategies": 50, "runs": 500, "amount": 2199, "currency": "eur" }));
    assert_eq!(plans["billing"]["tax_added"], true);

    let session = app.start_checkout("small").await;
    let form = &stripe.posted("/v1/checkout/sessions")[0];
    assert_eq!((form["mode"].as_str(), form["line_items[0][price]"].as_str(), form["metadata[item]"].as_str()), ("payment", "price_small", "small"));
    stripe.pay(&session, None, Some("pi_1"), 749);
    let object = json!({ "id": session, "object": "checkout.session", "customer": "cus_1" });
    assert_eq!(app.stripe_says("evt_1", "checkout.session.completed", object.clone()).await, (StatusCode::OK, json!({ "received": true })));
    // The same message again, and the same payment in another message.
    assert_eq!(app.stripe_says("evt_1", "checkout.session.completed", object.clone()).await, (StatusCode::OK, json!({ "received": true, "duplicate": true })));
    assert_eq!(app.stripe_says("evt_2", "checkout.session.async_payment_succeeded", object).await.0, StatusCode::OK);

    let (_, billing) = app.get("/api/billing").await;
    assert_eq!(billing["packs"], json!({ "strategies_left": 10, "runs_left": 100 }));
    let bought = billing["purchases"].as_array().unwrap();
    assert_eq!(bought.len(), 1);
    // What was paid is what Stripe took, tax and all.
    assert_eq!((bought[0]["pack"].as_str(), bought[0]["amount"].as_i64(), bought[0]["currency"].as_str(), bought[0]["refunded"].as_bool()), (Some("small"), Some(749), Some("eur"), Some(false)));
    // A pack does not make a subscriber.
    assert_eq!(app.get("/api/me").await.1["plan"], "free");

    // Two more packs add up.
    app.buy_pack(&stripe, "large", 2).await;
    app.buy_pack(&stripe, "small", 3).await;
    assert_eq!(app.get("/api/usage").await.1["packs"], json!({ "strategies_left": 70, "runs_left": 700 }));
    assert_eq!(stripe.posted("/v1/customers").len(), 1, "one customer record per user");

    let nonsense = app.post("/api/billing/checkout", json!({ "item": "huge", "agree": true })).await;
    assert_eq!(nonsense.0, StatusCode::UNPROCESSABLE_ENTITY);
    app.close().await;
}

#[tokio::test]
async fn a_pack_is_used_only_when_the_plan_s_own_runs_are_used_up() {
    let (app, stripe) = app_with_fake_stripe(None, Config { free_monthly_runs: 1, ..Config::for_tests() }).await;
    app.buy_pack(&stripe, "small", 1).await;
    assert_eq!(app.run(pd(json!({ "iterations": 5 }))).await["status"], "completed");
    let (_, usage) = app.get("/api/usage").await;
    assert_eq!((usage["used"].as_i64(), usage["remaining"].as_i64(), usage["packs"]["runs_left"].as_i64()), (Some(1), Some(0), Some(100)), "the first run is the plan's");
    assert_eq!(app.run(pd(json!({ "iterations": 5 }))).await["status"], "completed");
    assert_eq!(app.run(pd(json!({ "iterations": 5 }))).await["status"], "completed");
    let (_, usage) = app.get("/api/usage").await;
    assert_eq!((usage["used"].as_i64(), usage["packs"]["runs_left"].as_i64()), (Some(1), Some(98)), "the next two are the pack's, and are not counted as the plan's");

    // Without a pack the limit stands, and says where more can be had.
    let bob = app.signed_in_as("bob@example.com").await;
    assert_eq!(bob.run(pd(json!({ "iterations": 5 }))).await["status"], "completed");
    let refused = bob.post("/api/runs", pd(json!({ "iterations": 5 }))).await;
    assert_eq!((refused.0, code(&refused)), (StatusCode::TOO_MANY_REQUESTS, Some("run_limit_reached")));
    let message = refused.1["error"]["message"].as_str().unwrap();
    assert!(message.contains("starts again on 1 ") && message.contains("Plans page"), "{message}");
    app.close().await;
}

#[tokio::test]
async fn a_pack_pays_for_strategies_the_ai_writes_once_the_plan_s_are_used_up() {
    let api = FakeApi::default();
    let (app, stripe) = app_with_fake_stripe(Some(api.llm().await), Config { compiles_per_hour: 50, free_ai_requests_per_world: 1, ..Config::for_tests() }).await;
    let ask = || app.post("/api/strategies/compile", json!({ "prompt": PROMPT }));
    api.say(GRIM_JSON);
    assert_eq!(ask().await.0, StatusCode::OK);
    let refused = ask().await;
    assert_eq!(code(&refused), Some("ai_limit_reached"));
    let message = refused.1["error"]["message"].as_str().unwrap();
    assert!(message.contains("the AI writes 1 strategies for you per world") && message.contains("Plans page") && !message.contains("AI request"), "{message}");

    app.buy_pack(&stripe, "small", 1).await;
    api.say(GRIM_JSON).say(GRIM_JSON);
    assert_eq!(ask().await.0, StatusCode::OK);
    assert_eq!(ask().await.0, StatusCode::OK);
    let (_, usage) = app.get("/api/usage").await;
    assert_eq!(usage["packs"]["strategies_left"], 8);
    // What the pack paid for is not counted against the free plan's own.
    assert_eq!(usage["ai_requests"]["used_by_world"], json!({ "prisoners-dilemma": 1 }));

    // The model cannot be reached: nothing was paid for, so nothing is taken from the pack.
    api.reply(401, json!({ "error": "bad key" }));
    assert_eq!(code(&ask().await), Some("compile_unavailable"));
    assert_eq!(app.get("/api/usage").await.1["packs"]["strategies_left"], 8);
    // The model answers with something unusable: it was asked, so it is paid for.
    api.say("no idea").say("still none");
    assert_eq!(code(&ask().await), Some("not_compiled"));
    assert_eq!(app.get("/api/usage").await.1["packs"]["strategies_left"], 7);
    // Deleting a strategy the AI wrote gives nothing back.
    let (status, saved) = app.post("/api/strategies", grudger(json!({ "prompt": PROMPT }))).await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(app.call(Method::DELETE, &format!("/api/strategies/{}", saved["id"].as_str().unwrap()), None).await.0, StatusCode::OK);
    assert_eq!(app.get("/api/usage").await.1["packs"]["strategies_left"], 7);
    app.close().await;
}

#[tokio::test]
async fn a_renewal_opens_a_new_allowance_only_when_it_has_been_paid() {
    let api = FakeApi::default();
    let (app, stripe) = app_with_fake_stripe(Some(api.llm().await), Config { compiles_per_hour: 50, subscriber_ai_requests_per_month: 2, ..Config::for_tests() }).await;
    let ask = || app.post("/api/strategies/compile", json!({ "prompt": PROMPT }));
    let now = unix_now();
    app.subscribe(&stripe, "sub_1", now - 29 * DAY, now + DAY).await;
    api.say(GRIM_JSON).say(GRIM_JSON);
    assert_eq!(ask().await.0, StatusCode::OK);
    assert_eq!(ask().await.0, StatusCode::OK);
    let refused = ask().await;
    assert_eq!(code(&refused), Some("ai_limit_reached"));
    assert!(refused.1["error"]["message"].as_str().unwrap().contains("They come back on "), "{}", refused.1);
    // The calls were made some time before the period turns.
    app.store.backdate("llm_calls", "2 hours").await;

    // Stripe starts the next period and makes its invoice, which is not paid yet.
    stripe.put_subscription("sub_1", "cus_1", "active", now - 60, now + 30 * DAY, "open");
    let sub = json!({ "id": "sub_1", "object": "subscription", "customer": "cus_1" });
    assert_eq!(app.stripe_says("evt_renew", "customer.subscription.updated", sub.clone()).await.0, StatusCode::OK);
    assert_eq!(code(&ask().await), Some("ai_limit_reached"), "an unpaid renewal opens nothing");
    assert_eq!(app.get("/api/me").await.1["plan"], "subscriber", "the period already paid for still runs");

    // The card is refused: still nothing, and Stripe's word for it is kept.
    stripe.put_subscription("sub_1", "cus_1", "past_due", now - 60, now + 30 * DAY, "open");
    assert_eq!(app.stripe_says("evt_failed", "invoice.payment_failed", json!({ "id": "in_2", "object": "invoice", "customer": "cus_1" })).await.0, StatusCode::OK);
    assert_eq!(code(&ask().await), Some("ai_limit_reached"));
    assert_eq!(app.get("/api/billing").await.1["subscription"]["status"], "past_due");

    // The invoice is paid: the new period counts from its start.
    stripe.put_subscription("sub_1", "cus_1", "active", now - 60, now + 30 * DAY, "paid");
    assert_eq!(app.stripe_says("evt_paid", "invoice.paid", json!({ "id": "in_2", "object": "invoice", "customer": "cus_1" })).await.0, StatusCode::OK);
    api.say(GRIM_JSON).say(GRIM_JSON);
    assert_eq!(ask().await.0, StatusCode::OK);
    assert_eq!(ask().await.0, StatusCode::OK);
    assert_eq!(code(&ask().await), Some("ai_limit_reached"), "and a period is the allowance once, not more");
    assert_eq!(api.seen.lock().unwrap().len(), 4, "the model was asked only for what was paid");
    app.close().await;
}

#[tokio::test]
async fn a_subscription_that_is_not_paid_on_ends_with_the_period_that_was() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let now = unix_now();
    app.subscribe(&stripe, "sub_1", now - 10 * DAY, now + 20 * DAY).await;
    let me = app.my_id().await;
    assert_eq!(app.get("/api/me").await.1["plan"], "subscriber");

    // The period ran out an hour ago and Stripe is still collecting: a subscriber still, on the old period's allowance.
    app.store.age_subscription(me, "20 days 1 hour").await;
    let (_, user) = app.get("/api/me").await;
    assert_eq!(user["plan"], "subscriber");
    assert_eq!(user["usage"]["period_start"], app.get("/api/billing").await.1["subscription"]["period_start"], "no new period has started");
    // A day after the end with no payment: free, whatever was last heard from Stripe.
    app.store.age_subscription(me, "1 day").await;
    let (_, user) = app.get("/api/me").await;
    assert_eq!((user["plan"].as_str(), user["usage"]["monthly_runs"].as_i64(), user["usage"]["ai_requests"]["per"].as_str()), (Some("free"), Some(100), Some("world")));
    assert_eq!(app.get("/api/billing").await.1["subscription"]["paid"], false);
    // Paying up brings it back.
    stripe.put_subscription("sub_1", "cus_1", "active", now - 60, now + 30 * DAY, "paid");
    assert_eq!(app.stripe_says("evt_paid", "invoice.paid", json!({ "id": "in_3", "customer": "cus_1" })).await.0, StatusCode::OK);
    assert_eq!(app.get("/api/me").await.1["plan"], "subscriber");
    app.close().await;
}

#[tokio::test]
async fn cancelling_keeps_what_was_paid_for_and_an_ended_subscription_ends_at_once() {
    let (app, stripe) = app_with_fake_stripe(None, Config::for_tests()).await;
    let now = unix_now();
    app.subscribe(&stripe, "sub_1", now - DAY, now + 29 * DAY).await;
    let sub = json!({ "id": "sub_1", "object": "subscription", "customer": "cus_1" });

    // Cancelled in the portal: it runs to the end of the period.
    stripe.subscriptions.lock().unwrap().get_mut("sub_1").unwrap()["cancel_at_period_end"] = json!(true);
    assert_eq!(app.stripe_says("evt_cancel", "customer.subscription.updated", sub.clone()).await.0, StatusCode::OK);
    let (_, billing) = app.get("/api/billing").await;
    assert_eq!((billing["plan"].as_str(), billing["subscription"]["cancel_at_period_end"].as_bool(), billing["subscription"]["paid"].as_bool()), (Some("subscriber"), Some(true), Some(true)));

    // Stripe ends it (the period is over, or it was ended by hand in Stripe): free from that moment.
    stripe.subscriptions.lock().unwrap().get_mut("sub_1").unwrap()["status"] = json!("canceled");
    assert_eq!(app.stripe_says("evt_end", "customer.subscription.deleted", sub).await.0, StatusCode::OK);
    let (_, billing) = app.get("/api/billing").await;
    assert_eq!((billing["plan"].as_str(), billing["subscription"]["status"].as_str(), billing["subscription"]["paid"].as_bool()), (Some("free"), Some("canceled"), Some(false)));

    // They may subscribe again, and the new subscription takes the old one's place.
    app.subscribe(&stripe, "sub_2", now, now + 30 * DAY).await;
    assert_eq!(app.get("/api/me").await.1["plan"], "subscriber");
    app.close().await;
}

#[tokio::test]
async fn a_pack_paid_back_no_longer_counts() {
    let (app, stripe) = app_with_fake_stripe(None, Config { free_monthly_runs: 0, ..Config::for_tests() }).await;
    app.buy_pack(&stripe, "small", 1).await;
    app.buy_pack(&stripe, "small", 2).await;
    assert_eq!(app.run(pd(json!({ "iterations": 5 }))).await["status"], "completed");
    assert_eq!(app.get("/api/usage").await.1["packs"], json!({ "strategies_left": 20, "runs_left": 199 }));

    // A message about a refund that names no amount takes nothing.
    assert_eq!(app.stripe_says("evt_none", "charge.refunded", json!({ "id": "ch_1", "object": "charge", "customer": "cus_1", "payment_intent": "pi_1", "refunded": false })).await.0, StatusCode::OK);
    assert_eq!(app.get("/api/usage").await.1["packs"], json!({ "strategies_left": 20, "runs_left": 199 }));
    // A quarter of the money back takes a quarter of the pack back, once however often it is said.
    for n in 0..2 {
        let part = json!({ "id": "ch_1", "object": "charge", "customer": "cus_1", "payment_intent": "pi_1", "refunded": false, "amount": 600, "amount_refunded": 150 });
        assert_eq!(app.stripe_says(&format!("evt_part_{n}"), "charge.refunded", part).await.0, StatusCode::OK);
    }
    let (_, billing) = app.get("/api/billing").await;
    assert_eq!(billing["packs"], json!({ "strategies_left": 17, "runs_left": 174 }), "3 of 10 strategies (rounded up) and 25 of 100 runs");
    assert_eq!(billing["purchases"][1]["refunded"], false, "the pack is still partly theirs");
    // All of it back takes the pack away.
    assert_eq!(app.stripe_says("evt_refund", "charge.refunded", json!({ "id": "ch_1", "object": "charge", "customer": "cus_1", "payment_intent": "pi_1", "refunded": true, "amount": 600, "amount_refunded": 600 })).await.0, StatusCode::OK);
    let (_, billing) = app.get("/api/billing").await;
    assert_eq!(billing["packs"], json!({ "strategies_left": 10, "runs_left": 99 }), "what was used stays used");
    let refunded: Vec<bool> = billing["purchases"].as_array().unwrap().iter().map(|p| p["refunded"].as_bool().unwrap()).collect();
    assert_eq!(refunded, [false, true], "newest first");

    // The bank takes the other payment back: nothing is left, and nothing goes below nothing.
    assert_eq!(app.stripe_says("evt_dispute", "charge.dispute.created", json!({ "id": "dp_1", "object": "dispute", "payment_intent": "pi_2" })).await.0, StatusCode::OK);
    assert_eq!(app.get("/api/usage").await.1["packs"], json!({ "strategies_left": 0, "runs_left": 0 }));
    let refused = app.post("/api/runs", pd(json!({ "iterations": 5 }))).await;
    assert_eq!(code(&refused), Some("run_limit_reached"));
    // A refund of something that is not ours is nothing to us.
    assert_eq!(app.stripe_says("evt_other", "charge.refunded", json!({ "id": "ch_9", "payment_intent": "pi_99", "refunded": true })).await.0, StatusCode::OK);
    app.close().await;
}

#[tokio::test]
async fn a_user_can_take_their_data_and_delete_their_account() {
    let api = FakeApi::default();
    let (app, stripe) = app_with_fake_stripe(Some(api.llm().await), Config { compiles_per_hour: 50, free_ai_requests_per_world: 2, ..Config::for_tests() }).await;
    let bob = app.signed_in_as("bob@example.com").await;
    api.say(GRIM_JSON).say(GRIM_JSON);
    for _ in 0..2 {
        assert_eq!(app.post("/api/strategies/compile", json!({ "prompt": PROMPT })).await.0, StatusCode::OK);
    }
    let (_, private) = app.post("/api/strategies", grudger(json!({ "prompt": PROMPT }))).await;
    let (_, public) = app.post("/api/strategies", grudger(json!({ "visibility": "public" }))).await;
    let private_run = app.run(pd(json!({ "actors": [private["id"], "always_defect"], "iterations": 5 }))).await;
    let public_run = app.run(pd(json!({ "iterations": 5 }))).await;
    app.buy_pack(&stripe, "small", 1).await;
    let now = unix_now();
    app.subscribe(&stripe, "sub_1", now - 60, now + 30 * DAY).await;

    // The file has what is theirs, and nobody else's.
    let (status, headers, data) = app.send(Method::GET, "/api/me/export", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-disposition"].to_str().unwrap().starts_with("attachment"));
    assert_eq!((data["account"]["email"].as_str(), Some(data["strategies"].as_array().unwrap().len()), Some(data["runs"].as_array().unwrap().len())), (Some("ada@example.com"), Some(2), Some(2)));
    assert_eq!((data["purchases"][0]["pack"].as_str(), data["subscription"]["status"].as_str(), Some(data["ai_written_strategies"].as_array().unwrap().len())), (Some("small"), Some("active"), Some(2)));
    assert_eq!(data["strategies"][0]["prompt"], PROMPT);
    let (_, bobs) = bob.get("/api/me/export").await;
    assert_eq!((bobs["account"]["email"].as_str(), Some(bobs["strategies"].as_array().unwrap().len()), bobs["subscription"].is_null()), (Some("bob@example.com"), Some(0), true));
    assert!(!bobs.to_string().contains("ada@example.com"));
    assert_eq!(app.anonymous().get("/api/me/export").await.0, StatusCode::UNAUTHORIZED);

    // Deleting takes the account's own address, typed out.
    for wrong in [json!({ "email": "bob@example.com" }), json!({ "email": "" }), json!({})] {
        let refused = app.call(Method::DELETE, "/api/me", Some(wrong)).await;
        assert!(refused.0 == StatusCode::UNPROCESSABLE_ENTITY || refused.0 == StatusCode::BAD_REQUEST, "{}", refused.1);
    }
    // If the subscription cannot be ended at Stripe, nothing is deleted: nobody is charged for an account that is gone.
    *stripe.broken.lock().unwrap() = Some(503);
    let refused = app.call(Method::DELETE, "/api/me", Some(json!({ "email": "ada@example.com" }))).await;
    assert_eq!((refused.0, code(&refused)), (StatusCode::BAD_GATEWAY, Some("payment_unavailable")));
    assert_eq!(app.get("/api/me").await.0, StatusCode::OK);
    *stripe.broken.lock().unwrap() = None;

    let (status, headers, body) = app.send(Method::DELETE, "/api/me", Some(json!({ "email": " Ada@Example.com " }))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "deleted": true })));
    assert!(headers["set-cookie"].to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(stripe.subscriptions.lock().unwrap()["sub_1"]["status"], "canceled", "the subscription was ended at Stripe");
    assert_eq!(app.get("/api/me").await.0, StatusCode::UNAUTHORIZED, "the session is gone with the account");
    // Their strategies and private runs are gone; a public run stays, as part of the world's history.
    for gone in [format!("/api/strategies/{}", private["id"].as_str().unwrap()), format!("/api/strategies/{}", public["id"].as_str().unwrap()), format!("/api/runs/{}", private_run["id"])] {
        assert_eq!(bob.get(&gone).await.0, StatusCode::NOT_FOUND, "{gone}");
    }
    assert_eq!(bob.get(&format!("/api/runs/{}", public_run["id"])).await.0, StatusCode::OK);
    assert_eq!(bob.get("/api/me").await.0, StatusCode::OK, "nobody else is touched");

    // Signing up again with the same address is a new, free account without what was bought,
    // and the free strategies the old one used are still used.
    let back = app.signed_in_as("ada@example.com").await;
    let (_, me) = back.get("/api/me").await;
    assert_eq!((me["plan"].as_str(), me["usage"]["packs"]["strategies_left"].as_i64(), me["usage"]["used"].as_i64()), (Some("free"), Some(0), Some(0)));
    assert_eq!(me["usage"]["ai_requests"]["used_by_world"], json!({ "prisoners-dilemma": 2 }));
    let refused = back.post("/api/strategies/compile", json!({ "prompt": PROMPT })).await;
    assert_eq!(code(&refused), Some("ai_limit_reached"), "deleting the account does not bring the free ones back");
    assert_eq!(api.seen.lock().unwrap().len(), 2);
    app.close().await;
}

#[tokio::test]
async fn with_payment_off_nothing_is_for_sale_and_nothing_else_changes() {
    let app = TestApp::new().await;
    assert_eq!(app.get("/api/me").await.1["features"]["billing"], false);
    assert_eq!(app.anonymous().get("/api/plans").await.1["billing"], Value::Null);
    assert_eq!(app.get("/api/billing").await.1["enabled"], false);
    for (path, body) in [("/api/billing/checkout", json!({ "item": "small", "agree": true })), ("/api/billing/portal", json!({})), ("/api/billing/confirm", json!({ "session_id": "cs_1" }))] {
        let refused = app.post(path, body).await;
        assert_eq!((refused.0, code(&refused)), (StatusCode::SERVICE_UNAVAILABLE, Some("payment_unavailable")), "{path}");
    }
    // An account without a subscription is deleted without Stripe.
    assert_eq!(app.call(Method::DELETE, "/api/me", Some(json!({ "email": "ada@example.com" }))).await.0, StatusCode::OK);
    app.close().await;
}

#[test]
fn a_day_is_written_as_people_write_it() {
    assert_eq!(day("2026-12-01T00:00:00Z"), "1 December 2026");
    assert_eq!(day("2027-01-23T18:30:00Z"), "23 January 2027");
    assert_eq!(day("not a date"), "not a date");
}
