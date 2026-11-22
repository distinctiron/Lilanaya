//! Code a user wrote: what it cannot reach, and how it is stopped.

use super::*;

#[tokio::test]
async fn a_strategy_that_takes_too_long_is_stopped_and_cannot_be_saved() {
    let app = TestApp::new().await;
    // Inside its step budget, but every step is a slow built-in: only the clock stops it.
    let slow = json!({ "name": "Slow", "mode": "code", "source": "const S = 'a'.repeat(400000); function decide() { let n = 0; for (let i = 0; i < 40000; i++) n += S.indexOf('b'); return 'C'; }" });
    let started = std::time::Instant::now();
    let (status, body) = app.post("/api/strategies", slow).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body["error"]["details"][0]["message"].as_str().unwrap().contains("was stopped"), "{body}");
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 4, "nothing was saved");
    app.close().await;
}

fn code(name: &str, source: &str) -> Value {
    json!({ "name": name, "mode": "code", "visibility": "public", "source": source })
}

#[tokio::test]
async fn code_reaches_no_clock_no_chance_no_network_and_nothing_of_the_machine() {
    let app = TestApp::new().await;
    // Throws the names of whatever it can reach, so a failure says what leaked.
    let probe = r#"
        function decide(input) {
            const everything = (new Function('return this'))();
            const names = ['Date', 'performance', 'require', 'process', 'fetch', 'XMLHttpRequest', 'WebSocket', 'setTimeout', 'setInterval',
                'queueMicrotask', 'std', 'os', 'console', 'print', 'scriptArgs', 'navigator', 'crypto', 'Deno', 'Bun', 'module', 'WeakRef',
                'FinalizationRegistry', 'Atomics', 'SharedArrayBuffer'];
            const found = names.filter((name) => typeof everything[name] !== 'undefined' || typeof globalThis[name] !== 'undefined');
            if (typeof Math.random !== 'undefined') found.push('Math.random');
            if (Object.keys(input).sort().join() !== 'lastOpponentAction,lastOwnAction,opponentCounts,opponentScore,opponentStreak,outcomeStreak,ownCounts,ownScore,roundIndex,rounds,world') found.push('input: ' + Object.keys(input).sort().join());
            if (found.length) throw new Error('reachable: ' + found.join(', '));
            return 'C';
        }"#;
    let (status, report) = app.post("/api/strategies/preview", json!({ "strategy": code("Probe", probe) })).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["valid"], true, "{report}");

    // Loading other code is not possible, in either spelling.
    for source in ["import fs from 'fs'; function decide() { return 'C'; }", "function decide() { import('fs'); return 'C'; } export default 1;"] {
        let (_, report) = app.post("/api/strategies/preview", json!({ "strategy": code("Importer", source) })).await;
        assert_eq!(report["valid"], false, "{source}: {report}");
    }
    app.close().await;
}

#[tokio::test]
async fn a_strategy_that_breaks_in_the_middle_of_a_run_fails_the_run_and_decides_nothing() {
    let app = TestApp::new().await;
    // Each passes the check on saving (20 rounds) and breaks later, in a run.
    for (name, source, says) in [
        ("Thrower", "function decide(i) { if (i.roundIndex === 25) throw new Error('boom'); return 'C'; }", "boom"),
        ("Looper", "function decide(i) { if (i.roundIndex === 25) while (true) {} return 'C'; }", "took too many steps"),
        ("Hog", "function decide(i) { if (i.roundIndex === 25) { const a = []; while (true) a.push(new Array(1000).fill(7)); } return 'C'; }", ""),
        ("Cheat", "function decide(i) { return i.roundIndex === 25 ? 'WIN' : 'C'; }", "must return one of: C, D"),
        ("Recurser", "function decide(i) { return i.roundIndex === 25 ? decide(i) : 'C'; }", "nested calls too deeply"),
    ] {
        let (status, saved) = app.post("/api/strategies", code(name, source)).await;
        assert_eq!(status, StatusCode::CREATED, "{name}: {saved}");
        let run = app.run(pd(json!({ "actors": [saved["id"], "always_defect", "tit_for_tat"], "iterations": 50 }))).await;
        assert_eq!(run["status"], "failed", "{name}: {run}");
        let detail = run["error"]["details"][0]["message"].as_str().unwrap_or_default();
        assert!(detail.contains(says) && detail.contains("round 26"), "{name}: {run}");
        // No ranking, no matches, no files: a broken strategy never loses a match, it voids the run.
        assert!(run.get("ranking").is_none() && run.get("matches").is_none(), "{name}: {run}");
        let id = run["id"].as_i64().unwrap();
        assert_eq!(app.get(&format!("/api/runs/{id}/artifacts")).await.1["artifacts"], json!([]), "{name}");
        assert_eq!(app.get(&format!("/api/runs/{id}/race")).await.0, StatusCode::NOT_FOUND, "{name}");
    }
    // A failed run feeds no public analytics.
    assert_eq!(app.get("/api/worlds/prisoners-dilemma/leaderboard").await.1["run_id"], Value::Null);
    assert_eq!(app.get("/api/worlds/prisoners-dilemma/kpis").await.1["snapshots"], json!([]));
    // The worker is still there for the next run.
    assert_eq!(app.run(pd(json!({ "iterations": 5 }))).await["status"], "completed");
    app.close().await;
}

#[tokio::test]
async fn the_same_code_gives_the_same_run_every_time() {
    let app = TestApp::new().await;
    // Uses everything a code strategy is given, so anything that varied between runs would show.
    let source = r#"
        const memory = [];
        function decide(i) {
            memory.push(i.lastOpponentAction);
            const mostlyDefects = i.opponentCounts.D * 2 > i.roundIndex;
            if (i.rounds - i.roundIndex <= 2) return 'D';
            if (i.ownScore < i.opponentScore && memory.length % 7 === 0) return 'D';
            return mostlyDefects || i.opponentStreak >= 3 && i.lastOpponentAction === 'D' ? 'D' : 'C';
        }"#;
    let (status, saved) = app.post("/api/strategies", code("Bookkeeper", source)).await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let request = pd(json!({ "actors": [saved["id"], "always_defect", "tit_for_tat", "win_stay_lose_shift"], "iterations": 80, "seed": 7, "policy": "random", "schedule_mode": "sample_per_round" }));
    let first = app.run(request.clone()).await;
    assert_eq!(first["status"], "completed", "{first}");
    for _ in 0..2 {
        let again = app.run(request.clone()).await;
        for part in ["ranking", "actors", "matches", "pairwise", "outcome_distribution", "kpis"] {
            assert_eq!(again[part], first[part], "{part} differs between two runs of the same request");
        }
        // Round for round the same (the files carry the run's number, so compare what was played).
        let traces: Vec<String> = first["matches"].as_array().unwrap().iter().map(|m| format!("traces/{}__{}.json", m["a"].as_str().unwrap(), m["b"].as_str().unwrap())).collect();
        assert_eq!(traces.len(), 6);
        for name in traces {
            let moves = |run: &Value| format!("/api/runs/{}/artifacts/{name}", run["id"]);
            let (played_first, played_again) = (app.get(&moves(&first)).await.1["moves"].clone(), app.get(&moves(&again)).await.1["moves"].clone());
            assert_eq!(played_first.as_array().map(Vec::len), Some(80), "{name}");
            assert_eq!(played_again, played_first, "{name}: the rounds played differ");
        }
    }
    app.close().await;
}

#[tokio::test]
async fn the_api_keeps_answering_while_slow_code_is_being_stopped() {
    let app = TestApp::new().await;
    let slow = json!({ "strategy": code("Slow", "const S = 'a'.repeat(400000); function decide() { let n = 0; for (let i = 0; i < 40000; i++) n += S.indexOf('b'); return 'C'; }") });
    // More slow checks than may run at once: the rest wait their turn, and none of it holds up other requests.
    let checks: Vec<_> = (0..6)
        .map(|_| {
            let (app, slow) = (app.clone(), slow.clone());
            tokio::spawn(async move { app.post("/api/strategies/preview", slow).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(500)).await;
    for path in ["/api/health", "/api/worlds", "/api/strategies", "/api/me"] {
        let started = std::time::Instant::now();
        assert_eq!(app.get(path).await.0, StatusCode::OK, "{path}");
        assert!(started.elapsed() < Duration::from_secs(1), "{path} took {:?} while slow code was running", started.elapsed());
    }
    for check in checks {
        let (status, report) = check.await.unwrap();
        assert_eq!((status, report["valid"].as_bool()), (StatusCode::OK, Some(false)), "{report}");
        assert!(report["errors"][0]["message"].as_str().unwrap().contains("was stopped"), "{report}");
    }
    app.close().await;
}

#[tokio::test]
async fn code_that_is_too_large_or_is_not_a_strategy_is_never_saved() {
    let app = TestApp::new().await;
    for (source, says) in [
        (format!("function decide() {{ return 'C'; }} //{}", "x".repeat(17 * 1024)), "the limit is"),
        ("const x = 1;".to_string(), "must define a function named decide"),
        ("function decide( {".to_string(), ""),
        ("while (true) {}".to_string(), "took too many steps"),
        ("function decide() { return 'X'; }".to_string(), "must return one of"),
    ] {
        let (status, body) = app.post("/api/strategies", code("Bad", &source)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(body["error"]["details"][0]["message"].as_str().unwrap().contains(says), "{body}");
    }
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 4, "only the built-in strategies are there");
    app.close().await;
}
