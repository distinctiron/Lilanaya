//! Runs: queueing, results, the match view, the race, analytics and files.

use super::*;

#[tokio::test]
async fn a_run_is_queued_executed_stored_and_feeds_the_leaderboard() {
    let app = TestApp::new().await;

    let (status, empty) = app.get("/api/worlds/prisoners-dilemma/leaderboard").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty, json!({ "world_id": "prisoners-dilemma", "run_id": null, "entries": [] }));

    let run = app.run(pd(json!({ "iterations": 200, "seed": 7 }))).await;
    assert_eq!(run["id"], 1);
    assert_eq!(run["status"], "completed");
    assert_eq!(run["seed"], 7);
    assert_eq!(run["iterations"], 200);
    assert_eq!(run["actor_count"], 4);
    assert_eq!(run["ranking"][0], "always_defect");
    assert_eq!(run["matches"].as_array().unwrap().len(), 6);
    assert_eq!(run["kpis"][0]["id"], "mutual_cooperation_rate");
    assert!(run["created_at"].as_str().unwrap().ends_with('Z'));
    assert!(run["finished_at"].is_string());

    let (_, list) = app.get("/api/runs").await;
    assert_eq!(list["runs"][0]["id"], 1);
    assert_eq!(list["runs"][0]["status"], "completed");
    assert_eq!(list["runs"][0]["winner"]["id"], "always_defect");

    // A second, smaller run becomes the latest.
    let second = app.run(pd(json!({ "actors": ["tit_for_tat", "always_cooperate"], "iterations": 10 }))).await;
    assert_eq!(second["id"], 2);
    assert_eq!(second["seed"], 42, "defaults to the world's seed");

    let (_, board) = app.get("/api/worlds/prisoners-dilemma/leaderboard").await;
    assert_eq!(board["run_id"], 2);
    let entries = board["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["rank"], 1);
    assert_eq!(entries[0]["average"], 3.0);
    assert_eq!(entries[0]["entries"], 1);
    assert_eq!(app.get("/api/runs").await.1["runs"][0]["id"], 2);
    app.close().await;
}

#[tokio::test]
async fn the_same_request_gives_the_same_result() {
    let app = TestApp::new().await;
    let strip = |mut run: Value| {
        for key in ["id", "created_at", "finished_at"] {
            run.as_object_mut().unwrap().remove(key);
        }
        run
    };
    for extra in [json!({}), json!({ "policy": "random" }), json!({ "policy": "random", "schedule_mode": "sample_per_round" })] {
        let mut req = pd(json!({ "iterations": 50, "seed": 123 }));
        req.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let a = strip(app.run(req.clone()).await);
        let b = strip(app.run(req).await);
        assert_eq!(a, b);
        assert_eq!(a["status"], "completed");
    }
    app.close().await;
}

#[tokio::test]
async fn random_schedules_are_recorded_on_the_run() {
    let app = TestApp::new().await;
    let plain = app.run(pd(json!({ "iterations": 20 }))).await;
    assert_eq!(plain["policy"], "all_pairs");
    assert!(plain.get("schedule_mode").is_none());

    let shuffled = app.run(pd(json!({ "iterations": 20, "policy": "random" }))).await;
    assert_eq!(shuffled["policy"], "random");
    assert_eq!(shuffled["schedule_mode"], "shuffle_all");
    assert_eq!(shuffled["ranking"], plain["ranking"]);

    let sampled = app.run(pd(json!({ "iterations": 20, "policy": "random", "schedule_mode": "sample_per_round" }))).await;
    assert_eq!(sampled["schedule_mode"], "sample_per_round");

    // Traces are stored under the sorted pair whatever the execution order.
    let (status, m) = app.get("/api/runs/2/matches/tit_for_tat/always_defect").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(m["moves_a"].as_array().unwrap().len(), 20);
    app.close().await;
}

#[tokio::test]
async fn invalid_run_requests_list_every_problem() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/api/runs",
            pd(json!({ "actors": ["tit_for_tat", "tit_for_tat", "nope"], "iterations": 0, "seed": MAX_SEED + 1, "policy": "round_robin" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "validation_failed");
    let paths: Vec<&str> = body["error"]["details"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["iterations", "policy", "seed", "actors[1]", "actors[2]"]);

    let (status, body) = app.post("/api/runs", json!({ "world_id": "nope" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["details"][0]["path"], "world_id");

    let (status, body) = app.post("/api/runs", pd(json!({ "surprise": 1 }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_json");

    let (status, _) = app.post("/api/runs", pd(json!({ "actors": ["tit_for_tat"] }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Nothing was queued by any of the above.
    assert_eq!(app.get("/api/runs").await.1["runs"], json!([]));
    app.close().await;
}

#[tokio::test]
async fn match_view_returns_moves_in_the_order_asked() {
    let app = TestApp::new().await;
    app.run(pd(json!({ "iterations": 6 }))).await;

    let (status, m) = app.get("/api/runs/1/matches/always_defect/tit_for_tat").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(m["a"], json!({ "id": "always_defect", "name": "Always Defect" }));
    assert_eq!(m["decisions"], json!(["C", "D"]));
    assert_eq!(m["moves_a"], json!([1, 1, 1, 1, 1, 1]));
    assert_eq!(m["moves_b"], json!([0, 1, 1, 1, 1, 1]));
    assert_eq!(m["summary"]["totals"], json!([10, 5]));
    assert_eq!(m["summary"]["outcome_counts"]["D/C"], 1);

    let (_, f) = app.get("/api/runs/1/matches/tit_for_tat/always_defect").await;
    assert_eq!(f["a"]["id"], "tit_for_tat");
    assert_eq!(f["moves_a"], m["moves_b"]);
    assert_eq!(f["moves_b"], m["moves_a"]);
    assert_eq!(f["summary"]["totals"], json!([5, 10]));
    assert_eq!(f["summary"]["outcome_counts"]["C/D"], 1);
    assert_eq!(f["summary"]["outcome_runs"]["C/D"], 1);
    assert_eq!(f["summary"]["first_played"]["D"], json!([1, 0]));

    let (status, body) = app.get("/api/runs/1/matches/tit_for_tat/tit_for_tat").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    app.close().await;
}

#[tokio::test]
async fn the_race_gives_every_match_round_by_round() {
    let app = TestApp::new().await;
    let run = app.run(pd(json!({ "iterations": 6 }))).await;
    let (status, race) = app.get("/api/runs/1/race").await;
    assert_eq!(status, StatusCode::OK, "{race}");
    assert_eq!(race["rounds"], json!([1, 2, 3, 4, 5, 6]));
    let matches = race["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 6, "four actors play six matches");
    // Always Defect against Tit for Tat: 5 to 0 in the first round, then 1 each.
    let m = matches.iter().find(|m| m["a"] == "always_defect" && m["b"] == "tit_for_tat").unwrap();
    assert_eq!(m["totals"], json!([[5, 6, 7, 8, 9, 10], [0, 1, 2, 3, 4, 5]]));
    // The last points of an actor's matches add up to its total in the results.
    for actor in run["actors"].as_array().unwrap() {
        let total: i64 = matches
            .iter()
            .filter_map(|m| if m["a"] == actor["id"] { Some(0) } else if m["b"] == actor["id"] { Some(1) } else { None }.map(|side| m["totals"][side][5].as_i64().unwrap()))
            .sum();
        assert_eq!(json!(total), actor["total"], "{}", actor["id"]);
    }

    // Long matches are sampled; the last sample is the final round.
    app.run(pd(json!({ "iterations": 1000 }))).await;
    let (_, long) = app.get("/api/runs/2/race").await;
    let rounds = long["rounds"].as_array().unwrap();
    assert_eq!((rounds.len(), rounds[59].as_u64()), (60, Some(1000)));
    assert_eq!(app.get("/api/runs/99/race").await.0, StatusCode::NOT_FOUND);
    app.close().await;
}

#[tokio::test]
async fn pairwise_metrics_come_from_a_completed_run() {
    let app = TestApp::new().await;
    let (status, empty) = app.get("/api/worlds/prisoners-dilemma/pairwise").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["run_id"], Value::Null);

    app.run(pd(json!({ "iterations": 200 }))).await;
    app.run(pd(json!({ "iterations": 10, "actors": ["always_cooperate", "tit_for_tat"] }))).await;

    let (status, body) = app.get("/api/worlds/prisoners-dilemma/pairwise?run_id=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["run_id"], 1);
    assert_eq!(body["actors"].as_array().unwrap().len(), 4);
    let pair = |a: &str, b: &str| body["pairs"].as_array().unwrap().iter().find(|p| p["a"] == a && p["b"] == b).unwrap().clone();

    let p = pair("always_defect", "tit_for_tat");
    assert_eq!(p["average_payoff"], json!([1.02, 0.995]));
    assert_eq!(p["outcome_counts"], json!({ "C/C": 0, "C/D": 0, "D/C": 1, "D/D": 199 }));
    assert_eq!(p["first_defection_round"], 0);
    assert_eq!(p["punish_duration_average"], 199.0);

    let p = pair("always_defect", "win_stay_lose_shift");
    assert_eq!(p["punish_duration_average"], 1.0);

    let p = pair("always_cooperate", "tit_for_tat");
    assert_eq!(p["first_defection_round"], Value::Null);
    assert_eq!(p["punish_duration_average"], Value::Null);

    // Default: the latest run.
    let (_, latest) = app.get("/api/worlds/prisoners-dilemma/pairwise").await;
    assert_eq!(latest["run_id"], 2);
    assert_eq!(latest["pairs"].as_array().unwrap().len(), 1);
    app.close().await;
}

#[tokio::test]
async fn kpi_snapshots_are_listed_oldest_first_with_deltas() {
    let app = TestApp::new().await;
    let (_, empty) = app.get("/api/worlds/prisoners-dilemma/kpis").await;
    assert_eq!(empty["snapshots"], json!([]));
    assert_eq!(empty["definitions"].as_array().unwrap().len(), 3);

    app.run(pd(json!({ "iterations": 200 }))).await; // mutual cooperation 50%
    app.run(pd(json!({ "iterations": 10, "actors": ["always_cooperate", "tit_for_tat"] }))).await; // 100%
    app.run(pd(json!({ "iterations": 10, "actors": ["always_cooperate", "always_defect"] }))).await; // 0%

    let (status, body) = app.get("/api/worlds/prisoners-dilemma/kpis").await;
    assert_eq!(status, StatusCode::OK);
    let snaps = body["snapshots"].as_array().unwrap();
    assert_eq!(snaps.iter().map(|s| s["run_id"].as_i64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    let mc = |i: usize| snaps[i]["values"]["mutual_cooperation_rate"].clone();
    assert_eq!(mc(0), json!({ "label": "Mutual cooperation (% of rounds)", "value": 50.0, "delta": null }));
    assert_eq!((mc(1)["value"].clone(), mc(1)["delta"].clone()), (json!(100.0), json!(50.0)));
    assert_eq!((mc(2)["value"].clone(), mc(2)["delta"].clone()), (json!(0.0), json!(-100.0)));
    assert_eq!(snaps[2]["values"]["percent_defectors"]["value"], 50.0);

    // limit keeps the newest runs, and deltas still compare with the run before.
    let (_, body) = app.get("/api/worlds/prisoners-dilemma/kpis?limit=2").await;
    let snaps = body["snapshots"].as_array().unwrap();
    assert_eq!(snaps.iter().map(|s| s["run_id"].as_i64().unwrap()).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(snaps[0]["values"]["mutual_cooperation_rate"]["delta"], 50.0);

    // run_id ends the list at that run.
    let (_, body) = app.get("/api/worlds/prisoners-dilemma/kpis?run_id=2&limit=1").await;
    let snaps = body["snapshots"].as_array().unwrap();
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0]["run_id"], 2);
    app.close().await;
}

#[tokio::test]
async fn a_completed_run_s_files_are_built_on_request() {
    use sha2::{Digest, Sha256};
    let app = TestApp::new().await;
    let run = app.run(pd(json!({ "iterations": 20, "seed": 3 }))).await;
    assert_eq!(run["status"], "completed", "{run}");

    let (status, manifest) = app.get("/api/runs/1/artifacts").await;
    assert_eq!(status, StatusCode::OK);
    let files = manifest["artifacts"].as_array().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 3 + 6, "summary, matrix, manifest and one trace per pairing: {names:?}");
    for expected in ["manifest.json", "pairwise_matrix.json", "run_summary.json", "traces/always_defect__tit_for_tat.json"] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }

    // Every listed file is served as JSON with the size and checksum the manifest gives.
    let fetch = |href: String| {
        let router = app.router.clone();
        async move {
            let res = router.oneshot(Request::builder().uri(&href).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{href}");
            assert_eq!(res.headers()["content-type"], "application/json", "{href}");
            assert_eq!(res.headers()["cache-control"], "private, no-store", "{href}");
            res.into_body().collect().await.unwrap().to_bytes()
        }
    };
    for f in files {
        let bytes = fetch(f["href"].as_str().unwrap().to_string()).await;
        assert_eq!(bytes.len() as i64, f["size_bytes"].as_i64().unwrap(), "{f}");
        let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(sha, f["sha256"].as_str().unwrap(), "{f}");
    }

    // The summary is the run the API reports; the manifest lists the other files.
    let (_, summary) = app.get("/api/runs/1/artifacts/run_summary.json").await;
    assert_eq!(summary["summary"]["ranking"], run["ranking"]);
    assert_eq!(summary["summary"]["seed"], 3);
    let (_, stored) = app.get("/api/runs/1/artifacts/manifest.json").await;
    assert_eq!(stored["artifacts"].as_array().unwrap().len(), 8);
    let (_, trace) = app.get("/api/runs/1/artifacts/traces/always_defect__tit_for_tat.json").await;
    assert_eq!(trace["moves"].as_array().unwrap().len(), 20);
    assert_eq!(trace["moves"][0], json!([1, 0]));

    let (status, body) = app.get("/api/runs/1/artifacts/nope.json").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    let (status, _) = app.get("/api/runs/99/artifacts").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    app.close().await;
}

#[tokio::test]
async fn interrupted_runs_go_back_in_the_queue() {
    let app = TestApp::new().await;
    app.run(pd(json!({ "iterations": 5 }))).await;
    // Nothing is mid-flight, so nothing is re-queued and the finished run is untouched.
    assert_eq!(app.store.requeue_interrupted().await.unwrap(), 0);
    assert_eq!(app.get("/api/runs/1").await.1["status"], "completed");
    app.close().await;
}
