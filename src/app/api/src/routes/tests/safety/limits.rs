//! Every limit holds: one request at a time, and with requests arriving together.

use super::*;

#[tokio::test]
async fn runs_are_limited_per_month_and_in_how_many_wait_at_once() {
    // Two runs this month, one at a time.
    let app = TestApp::with_config(Config { free_monthly_runs: 2, max_active_runs: 1, ..Config::for_tests() }).await;
    let (_, usage) = app.get("/api/usage").await;
    assert_eq!((usage["enforced"].clone(), usage["max_active_runs"].clone()), (json!(true), json!(1)));

    // A strategy that thinks for a moment before every decision (well inside its step budget)
    // keeps the worker busy for seconds: long enough to ask for a second run.
    let thinker = json!({ "name": "Thinker", "mode": "code", "visibility": "public", "source": "function decide() { let n = 0; for (let i = 0; i < 30000; i++) n += i; return 'C'; }" });
    let (status, saved) = app.post("/api/strategies", thinker).await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let (status, first) = app.post("/api/runs", pd(json!({ "actors": [saved["id"], "always_defect"], "iterations": 2000 }))).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{first}");
    let (status, body) = app.post("/api/runs", pd(json!({ "iterations": 5 }))).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_many_runs")), "{body}");
    // Another user is not held up by it.
    let bob = app.signed_in_as("bob@example.com").await;
    assert_eq!(bob.run(pd(json!({ "iterations": 5 }))).await["status"], "completed");

    for _ in 0..6000 {
        if app.get("/api/runs/1").await.1["status"] == "completed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(app.get("/api/runs/1").await.1["status"], "completed");
    assert_eq!(app.run(pd(json!({ "iterations": 5 }))).await["status"], "completed", "a second run, once the first is done");
    let (status, body) = app.post("/api/runs", pd(json!({ "iterations": 5 }))).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("run_limit_reached")), "{body}");
    assert!(body["error"]["message"].as_str().unwrap().contains("all 2 runs"), "{body}");
    assert_eq!(app.get("/api/usage").await.1["remaining"], 0);
    app.close().await;
}

#[tokio::test]
async fn requests_arriving_together_cannot_all_pass_a_limit() {
    let app = TestApp::with_config(Config { free_monthly_runs: 3, max_active_runs: 100, ..Config::for_tests() }).await;
    let attempts: Vec<_> = (0..12).map(|_| {
        let app = app.clone();
        tokio::spawn(async move { app.post("/api/runs", pd(json!({ "iterations": 5 }))).await.0 })
    }).collect();
    let mut answers = Vec::new();
    for attempt in attempts {
        answers.push(attempt.await.unwrap());
    }
    let count = |status: StatusCode| answers.iter().filter(|a| **a == status).count();
    assert_eq!((count(StatusCode::ACCEPTED), count(StatusCode::TOO_MANY_REQUESTS)), (3, 9), "{answers:?}");
    app.close().await;
}

#[tokio::test]
async fn sign_in_links_are_rate_limited_per_address() {
    let app = TestApp::new().await;
    let anon = app.anonymous();
    for _ in 0..5 {
        assert_eq!(anon.post("/api/auth/email", json!({ "email": "eve@example.com" })).await.0, StatusCode::ACCEPTED);
    }
    let (status, body) = anon.post("/api/auth/email", json!({ "email": "eve@example.com" })).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_many_requests")));
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "other@example.com" })).await.0, StatusCode::ACCEPTED);
    app.close().await;
}

#[tokio::test]
async fn strategy_checks_are_limited_per_hour() {
    let app = TestApp::with_config(Config { previews_per_hour: 2, ..Config::for_tests() }).await;
    let draft = json!({ "strategy": grudger(json!({})) });
    assert_eq!(app.post("/api/strategies/preview", draft.clone()).await.0, StatusCode::OK);
    // A save is a check too.
    assert_eq!(app.post("/api/strategies", grudger(json!({}))).await.0, StatusCode::CREATED);
    let (status, body) = app.post("/api/strategies/preview", draft.clone()).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_many_requests")), "{body}");
    assert_eq!(app.post("/api/strategies", grudger(json!({}))).await.0, StatusCode::TOO_MANY_REQUESTS);
    // Another user has their own allowance.
    assert_eq!(app.signed_in_as("bob@example.com").await.post("/api/strategies/preview", draft).await.0, StatusCode::OK);
    app.close().await;
}

#[tokio::test]
async fn a_free_plan_has_three_ai_requests_per_world_for_good() {
    let api = FakeApi::default();
    let app = TestApp::build(Some(api.llm().await), OAuth::none(), Config { compiles_per_hour: 50, ..Config::for_tests() }).await;
    let prompt = json!({ "prompt": "Cooperate until betrayed, then defect for ever." });
    let ai = |usage: Value| usage["ai_requests"].clone();
    assert_eq!(ai(app.get("/api/usage").await.1), json!({ "limit": 3, "per": "world", "used_by_world": {}, "used_this_month": 0 }));

    // Not used up: a description the screen turns down, and a provider that does not answer.
    api.judge(200, verdict("instructions_to_the_ai", 0.01));
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.1["error"]["code"], "prompt_blocked");
    api.reply(500, json!({ "error": "down" })).reply(500, json!({ "error": "down" })).reply(500, json!({ "error": "down" }));
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.1["error"]["code"], "compile_unavailable");
    assert_eq!(ai(app.get("/api/usage").await.1)["used_by_world"], json!({}));

    // Used up: every time the model is asked, also when it says the description is no strategy.
    api.say(GRIM_JSON).say(r#"{"error": "This describes a pancake recipe, not a strategy."}"#).say(GRIM_JSON);
    let (status, first) = app.post("/api/strategies/compile", prompt.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.1["error"]["code"], "not_compiled");
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    assert_eq!(ai(app.get("/api/usage").await.1)["used_by_world"], json!({ "prisoners-dilemma": 3 }));
    let asked = api.seen.lock().unwrap().len();

    // The fourth is refused before anything is sent, and saving then deleting a strategy gives nothing back.
    let refused = |(status, body): (StatusCode, Value)| {
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("ai_limit_reached")), "{body}");
        assert!(body["error"]["message"].as_str().unwrap().contains("do not come back"), "{body}");
    };
    refused(app.post("/api/strategies/compile", prompt.clone()).await);
    let (status, saved) = app.post("/api/strategies", grudger(json!({ "prompt": "Cooperate until betrayed, then defect for ever." }))).await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(app.call(Method::DELETE, &format!("/api/strategies/{}", saved["id"].as_str().unwrap()), None).await.0, StatusCode::OK);
    refused(app.post("/api/strategies/compile", prompt.clone()).await);
    assert_eq!(api.seen.lock().unwrap().len(), asked, "a refused request is not sent");

    // Rules by hand and code are not AI requests: they still save.
    assert_eq!(app.post("/api/strategies", grudger(json!({}))).await.0, StatusCode::CREATED);
    assert_eq!(app.post("/api/strategies", json!({ "name": "Coded", "mode": "code", "source": "function decide() { return 'C'; }" })).await.0, StatusCode::CREATED);

    // The three belong to one user and one world.
    let bob = app.signed_in_as("bob@example.com").await;
    api.say(GRIM_JSON);
    assert_eq!(bob.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    let me = app.get("/api/me").await.1["id"].as_i64().unwrap();
    let limits = |allowance| CompileLimits { per_hour: 50, blocked_per_day: 5, per_day: 500, tokens_per_day: 2_000_000, allowance };
    assert_eq!(app.store.begin_llm_call(me, "prisoners-dilemma", limits(Allowance::PerWorld(3))).await.unwrap(), CompileGate::AllowanceUsed);
    assert!(matches!(app.store.begin_llm_call(me, "another-world", limits(Allowance::PerWorld(3))).await.unwrap(), CompileGate::Started(_)));
    app.close().await;
}

#[tokio::test]
async fn a_subscriber_has_twenty_ai_requests_a_month_across_worlds() {
    let api = FakeApi::default();
    let app = TestApp::build(Some(api.llm().await), OAuth::none(), Config { compiles_per_hour: 50, subscriber_ai_requests_per_month: 4, ..Config::for_tests() }).await;
    let prompt = json!({ "prompt": "Cooperate until betrayed, then defect for ever." });
    let me = app.get("/api/me").await.1["id"].as_i64().unwrap();
    app.store.set_plan(me, "subscriber").await.unwrap();
    for _ in 0..4 {
        api.say(GRIM_JSON);
        assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    }
    assert_eq!(app.get("/api/usage").await.1["ai_requests"], json!({ "limit": 4, "per": "month", "used_by_world": { "prisoners-dilemma": 4 }, "used_this_month": 4 }));
    let (status, body) = app.post("/api/strategies/compile", prompt.clone()).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("ai_limit_reached")), "{body}");
    assert!(body["error"]["message"].as_str().unwrap().contains("on the 1st"), "{body}");
    // The month's number covers every world.
    let limits = CompileLimits { per_hour: 50, blocked_per_day: 5, per_day: 500, tokens_per_day: 2_000_000, allowance: Allowance::PerMonth(4) };
    assert_eq!(app.store.begin_llm_call(me, "another-world", limits).await.unwrap(), CompileGate::AllowanceUsed);
    assert_eq!(Config::for_tests().subscriber_ai_requests_per_month, 20);
    app.close().await;
}

#[tokio::test]
async fn ai_requests_arriving_together_do_not_get_past_the_allowance() {
    let api = FakeApi::default();
    let app = TestApp::build(Some(api.llm().await), OAuth::none(), Config { compiles_per_hour: 50, ..Config::for_tests() }).await;
    for _ in 0..8 {
        api.say(GRIM_JSON);
    }
    let attempts: Vec<_> = (0..8).map(|_| {
        let app = app.clone();
        tokio::spawn(async move { app.post("/api/strategies/compile", json!({ "prompt": "Cooperate until betrayed, then defect for ever." })).await.0 })
    }).collect();
    let mut answers = Vec::new();
    for attempt in attempts {
        answers.push(attempt.await.unwrap());
    }
    let count = |status: StatusCode| answers.iter().filter(|a| **a == status).count();
    assert_eq!((count(StatusCode::OK), count(StatusCode::TOO_MANY_REQUESTS)), (3, 5), "{answers:?}");
    app.close().await;
}

#[tokio::test]
async fn compile_closes_for_everyone_when_the_day_s_ceiling_is_reached() {
    let prompt = json!({ "prompt": "Cooperate until betrayed, then defect for ever." });
    let closed = |(status, body): (StatusCode, Value)| {
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("compile_unavailable")), "{body}");
        assert!(body["error"]["message"].as_str().unwrap().contains("opens again tomorrow"), "{body}");
    };

    // By number of compiles: two a day, whoever asks.
    let api = FakeApi::default();
    let app = TestApp::build(Some(api.llm().await), OAuth::none(), Config { compiles_per_day: 2, ..Config::for_tests() }).await;
    let bob = app.signed_in_as("bob@example.com").await;
    api.say(GRIM_JSON).say(GRIM_JSON);
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    assert_eq!(bob.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    closed(bob.post("/api/strategies/compile", prompt.clone()).await);
    closed(app.post("/api/strategies/compile", prompt.clone()).await);
    assert_eq!(api.seen.lock().unwrap().len(), 2);
    app.close().await;

    // By tokens: the stand-in's screen uses 320 and its model 1,000 a call.
    let api = FakeApi::default();
    let app = TestApp::build(Some(api.llm().await), OAuth::none(), Config { compile_tokens_per_day: 1_000, ..Config::for_tests() }).await;
    api.say(GRIM_JSON);
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    closed(app.post("/api/strategies/compile", prompt.clone()).await);
    app.close().await;
}

#[tokio::test]
async fn the_month_s_runs_come_back_with_the_new_month_and_not_before() {
    let app = TestApp::with_config(Config { free_monthly_runs: 2, ..Config::for_tests() }).await;
    app.run(pd(json!({ "iterations": 5 }))).await;
    app.run(pd(json!({ "iterations": 5 }))).await;
    assert_eq!(app.post("/api/runs", pd(json!({ "iterations": 5 }))).await.1["error"]["code"], "run_limit_reached");
    let (_, usage) = app.get("/api/usage").await;
    assert_eq!((usage["used"].clone(), usage["remaining"].clone()), (json!(2), json!(0)));

    // The same runs seen from next month: 32 days on is always another month.
    app.store.backdate("runs", "32 days").await;
    let (_, usage) = app.get("/api/usage").await;
    assert_eq!((usage["used"].clone(), usage["remaining"].clone()), (json!(0), json!(2)));
    assert_eq!(app.run(pd(json!({ "iterations": 5 }))).await["status"], "completed");
    assert_eq!(app.get("/api/usage").await.1["used"], 1);
    // The old runs are still there to read.
    assert_eq!(app.get("/api/runs").await.1["runs"].as_array().unwrap().len(), 3);
    app.close().await;
}

#[tokio::test]
async fn the_hourly_and_quarter_hourly_limits_lift_when_their_time_has_passed() {
    let app = TestApp::with_config(Config { previews_per_hour: 2, ..Config::for_tests() }).await;
    let draft = json!({ "strategy": grudger(json!({})) });
    let check = || async { app.post("/api/strategies/preview", draft.clone()).await.0 };
    assert_eq!((check().await, check().await, check().await), (StatusCode::OK, StatusCode::OK, StatusCode::TOO_MANY_REQUESTS));
    app.store.backdate("preview_calls", "59 minutes").await;
    assert_eq!(check().await, StatusCode::TOO_MANY_REQUESTS, "a minute before the hour is up");
    app.store.backdate("preview_calls", "2 minutes").await;
    assert_eq!((check().await, check().await, check().await), (StatusCode::OK, StatusCode::OK, StatusCode::TOO_MANY_REQUESTS), "a minute after");

    // Sign-in links: 5 per address in 15 minutes.
    let anon = app.anonymous();
    let ask = || async { anon.post("/api/auth/email", json!({ "email": "eve@example.com" })).await.0 };
    for _ in 0..5 {
        assert_eq!(ask().await, StatusCode::ACCEPTED);
    }
    assert_eq!(ask().await, StatusCode::TOO_MANY_REQUESTS);
    // Written differently, it is still the same address.
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "  EVE@Example.COM " })).await.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(anon.post("/api/auth/email", json!({ "email": "eve@example.com", "intent": "log_in" })).await.0, StatusCode::TOO_MANY_REQUESTS, "whichever email would go out");
    app.store.backdate("sign_in_tokens", "14 minutes").await;
    assert_eq!(ask().await, StatusCode::TOO_MANY_REQUESTS, "a minute before the quarter is up");
    app.store.backdate("sign_in_tokens", "2 minutes").await;
    assert_eq!(ask().await, StatusCode::ACCEPTED, "a minute after");
    // A refused request sends no email.
    let sent = app.mail.lock().unwrap().iter().filter(|m| m.to == "eve@example.com").count();
    assert_eq!(sent, 6);
    app.close().await;
}

#[tokio::test]
async fn free_ai_requests_never_come_back_and_a_subscriber_s_come_back_each_month() {
    let api = FakeApi::default();
    let app = TestApp::build(Some(api.llm().await), OAuth::none(), Config { compiles_per_hour: 50, free_ai_requests_per_world: 2, subscriber_ai_requests_per_month: 2, ..Config::for_tests() }).await;
    let prompt = json!({ "prompt": "Cooperate until betrayed, then defect for ever." });
    let code = |(_, body): (StatusCode, Value)| body["error"]["code"].as_str().map(str::to_string);

    api.say(GRIM_JSON).say(GRIM_JSON);
    assert_eq!(code(app.post("/api/strategies/compile", prompt.clone()).await), None);
    assert_eq!(code(app.post("/api/strategies/compile", prompt.clone()).await), None);
    assert_eq!(code(app.post("/api/strategies/compile", prompt.clone()).await).as_deref(), Some("ai_limit_reached"));
    // Free plan: a year on, they are still used up.
    app.store.backdate("llm_calls", "400 days").await;
    assert_eq!(code(app.post("/api/strategies/compile", prompt.clone()).await).as_deref(), Some("ai_limit_reached"));
    assert_eq!(app.get("/api/usage").await.1["ai_requests"], json!({ "limit": 2, "per": "world", "used_by_world": { "prisoners-dilemma": 2 }, "used_this_month": 0 }));

    // The same person as a subscriber: this month's count is what matters, and last year's requests are not in it.
    let me = app.get("/api/me").await.1["id"].as_i64().unwrap();
    app.store.set_plan(me, "subscriber").await.unwrap();
    api.say(GRIM_JSON).say(GRIM_JSON);
    assert_eq!(code(app.post("/api/strategies/compile", prompt.clone()).await), None);
    assert_eq!(code(app.post("/api/strategies/compile", prompt.clone()).await), None);
    let (status, body) = app.post("/api/strategies/compile", prompt.clone()).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("ai_limit_reached")), "{body}");
    assert!(body["error"]["message"].as_str().unwrap().contains("come back on the 1st"), "{body}");
    app.store.backdate("llm_calls", "32 days").await;
    api.say(GRIM_JSON);
    assert_eq!(code(app.post("/api/strategies/compile", prompt.clone()).await), None, "a new month");
    assert_eq!(api.seen.lock().unwrap().len(), 5, "the model was asked only for the requests that were let through");
    app.close().await;
}

#[tokio::test]
async fn the_day_s_ceiling_and_a_suspension_lift_the_next_day() {
    let api = FakeApi::default();
    let app = TestApp::build(Some(api.llm().await), OAuth::none(), Config { compiles_per_hour: 50, compiles_per_day: 2, free_ai_requests_per_world: 50, ..Config::for_tests() }).await;
    let prompt = json!({ "prompt": "Cooperate until betrayed, then defect for ever." });
    let bob = app.signed_in_as("bob@example.com").await;

    api.say(GRIM_JSON).say(GRIM_JSON);
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    assert_eq!(bob.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    // Two for everyone together: the third is closed to whoever asks.
    for caller in [&app, &bob] {
        let (status, body) = caller.post("/api/strategies/compile", prompt.clone()).await;
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("compile_unavailable")), "{body}");
    }
    app.store.backdate("llm_calls", "25 hours").await;
    api.say(GRIM_JSON);
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK, "the next day");

    app.close().await;

    // Five descriptions turned down close compile to that user for 24 hours.
    let api = FakeApi::default();
    let app = TestApp::build(Some(api.llm().await), OAuth::none(), Config { compiles_per_hour: 50, ..Config::for_tests() }).await;
    let bob = app.signed_in_as("bob@example.com").await;
    for _ in 0..5 {
        api.judge(200, verdict("abusive_or_harmful", 0.01));
        assert_eq!(bob.post("/api/strategies/compile", prompt.clone()).await.1["error"]["code"], "prompt_blocked");
    }
    assert_eq!(bob.post("/api/strategies/compile", prompt.clone()).await.1["error"]["code"], "compile_suspended");
    app.store.backdate("llm_calls", "23 hours").await;
    assert_eq!(bob.post("/api/strategies/compile", prompt.clone()).await.1["error"]["code"], "compile_suspended", "an hour before the day is up");
    app.store.backdate("llm_calls", "2 hours").await;
    api.say(GRIM_JSON);
    assert_eq!(bob.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK, "an hour after");
    // It was closed to that user alone.
    api.say(GRIM_JSON);
    assert_eq!(app.post("/api/strategies/compile", prompt.clone()).await.0, StatusCode::OK);
    app.close().await;
}

#[tokio::test]
async fn every_limit_is_read_from_the_settings_and_shown_to_the_user() {
    let config = Config { free_monthly_runs: 7, subscriber_monthly_runs: 70, free_ai_requests_per_world: 4, subscriber_ai_requests_per_month: 40, max_active_runs: 3, ..Config::for_tests() };
    let app = TestApp::with_config(config).await;
    assert_eq!(
        app.anonymous().get("/api/plans").await.1,
        json!({
            "free": { "ai_requests": { "limit": 4, "per": "world" }, "monthly_runs": 7, "max_active_runs": 3 },
            "subscriber": { "ai_requests": { "limit": 40, "per": "month" }, "monthly_runs": 70, "max_active_runs": 3 },
        })
    );
    let (_, usage) = app.get("/api/usage").await;
    assert_eq!((usage["monthly_runs"].clone(), usage["remaining"].clone(), usage["max_active_runs"].clone(), usage["ai_requests"]["limit"].clone(), usage["enforced"].clone()), (json!(7), json!(7), json!(3), json!(4), json!(true)));
    let me = app.get("/api/me").await.1["id"].as_i64().unwrap();
    app.store.set_plan(me, "subscriber").await.unwrap();
    let (_, usage) = app.get("/api/usage").await;
    assert_eq!((usage["monthly_runs"].clone(), usage["ai_requests"]["limit"].clone(), usage["ai_requests"]["per"].clone()), (json!(70), json!(40), json!("month")));
    app.close().await;
}
