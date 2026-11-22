//! Handler tests. They need Postgres (`DATABASE_URL`, set by compose): each
//! test gets its own schema and its own worker, so tests do not see each other.

use super::*;
use crate::artifacts::{ArtifactStore, S3Config};
use crate::config::Config;
use crate::llm::tests::FakeApi;
use crate::llm::{Llm, Provider};
use crate::mail::{Mailer, Sent};
use crate::store::Store;
use crate::worker;
use axum::body::Body;
use axum::http::{Method, Request};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tower::ServiceExt;

/// The API with its own schema, bucket prefix and worker, seen through one
/// browser: `cookie` is that browser's session. `TestApp::new` starts signed in
/// as ada@example.com; `anonymous` and `signed_in_as` give other browsers.
#[derive(Clone)]
struct TestApp {
    router: Router,
    store: Store,
    artifacts: ArtifactStore,
    schema: String,
    mail: Arc<Mutex<Vec<Sent>>>,
    cookie: Option<String>,
}

impl TestApp {
    async fn new() -> Self {
        Self::with_llm(None).await
    }

    async fn with_llm(llm: Option<Llm>) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set: run the tests with `docker compose run --rm api cargo test`");
        let schema = format!("test_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst));
        let store = Store::connect(&url, Some(&schema)).await.expect("connect to the test database");
        // Sign for the address the tests themselves can reach, and keep each test's files apart.
        let mut s3 = S3Config::from_env().expect("S3 settings (set by compose)");
        s3.public_endpoint = s3.endpoint.clone();
        s3.prefix = format!("{schema}/");
        let artifacts = ArtifactStore::new(&s3).expect("object storage client");
        let mail = Arc::new(Mutex::new(Vec::new()));
        let state = AppState::new(store.clone(), artifacts.clone(), Mailer::Memory(mail.clone()), llm, Config::for_tests());
        worker::spawn(state.clone());
        let app = TestApp { router: app(state), store, artifacts, schema, mail, cookie: None };
        app.signed_in_as("ada@example.com").await
    }

    async fn close(self) {
        self.store.drop_schema(&self.schema).await;
    }

    /// The same API seen by a browser with no session.
    fn anonymous(&self) -> Self {
        TestApp { cookie: None, ..self.clone() }
    }

    /// The token in the latest sign-in email.
    fn emailed_token(&self) -> String {
        let mail = self.mail.lock().unwrap();
        let body = &mail.last().expect("an email was sent").body;
        let start = body.find("token=").expect("the email has a link") + "token=".len();
        body[start..].chars().take_while(|c| c.is_ascii_hexdigit()).collect()
    }

    /// Goes through the emailed-link sign-in and returns a browser holding the session.
    async fn signed_in_as(&self, email: &str) -> Self {
        let (status, body) = self.anonymous().post("/api/auth/email", json!({ "email": email })).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let (status, headers, body) = self.anonymous().send(Method::POST, "/api/auth/email/confirm", Some(json!({ "token": self.emailed_token() }))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let set_cookie = headers["set-cookie"].to_str().unwrap();
        assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Lax"), "{set_cookie}");
        TestApp { cookie: Some(set_cookie.split(';').next().unwrap().to_string()), ..self.clone() }
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, axum::http::HeaderMap, Value) {
        let body = body.map_or_else(Body::empty, |v| Body::from(v.to_string()));
        let mut req = Request::builder().method(method).uri(path);
        if let Some(cookie) = &self.cookie {
            req = req.header("cookie", cookie);
        }
        let res = self.router.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{path}: body is not JSON ({e}): {bytes:?}"));
        (status, headers, value)
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, _, value) = self.send(method, path, body).await;
        (status, value)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.call(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, path, Some(body)).await
    }

    /// Queues a run and waits for the worker to finish it; returns the finished run.
    async fn run(&self, body: Value) -> Value {
        let (status, queued) = self.post("/api/runs", body).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
        assert!(queued["status"] == "queued", "{queued}");
        let id = queued["id"].as_i64().unwrap();
        for _ in 0..1000 {
            let (_, run) = self.get(&format!("/api/runs/{id}")).await;
            match run["status"].as_str() {
                Some("completed") | Some("failed") => return run,
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        panic!("run {id} did not finish");
    }
}

fn pd(extra: Value) -> Value {
    let mut body = json!({ "world_id": "prisoners-dilemma" });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    body
}

#[tokio::test]
async fn health_and_hello() {
    let app = TestApp::new().await;
    assert_eq!(app.get("/api/health").await, (StatusCode::OK, json!({ "status": "ok" })));
    assert_eq!(app.get("/api/hello").await.1["message"], "Hello from Lilanaya!");
    app.close().await;
}

#[tokio::test]
async fn worlds_and_strategies_are_listed() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/api/worlds").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["worlds"][0]["id"], "prisoners-dilemma");

    let (status, world) = app.get("/api/worlds/prisoners-dilemma").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(world["payoff"]["matrix"]["D"]["C"], json!([5, 0]));
    assert_eq!(world["kpis"]["definitions"].as_array().unwrap().len(), 3);

    let (_, body) = app.get("/api/strategies").await;
    let ids: Vec<&str> = body["strategies"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["always_cooperate", "always_defect", "tit_for_tat", "win_stay_lose_shift"]);
    app.close().await;
}

#[tokio::test]
async fn errors_share_one_shape() {
    let app = TestApp::new().await;
    for (method, path, status, code) in [
        (Method::GET, "/api/worlds/nope", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/runs/99", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/runs/abc", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/nothing/here", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/worlds/nope/leaderboard", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/worlds/prisoners-dilemma/pairwise?run_id=99", StatusCode::NOT_FOUND, "not_found"),
        (Method::GET, "/api/worlds/prisoners-dilemma/pairwise?run_id=x", StatusCode::BAD_REQUEST, "invalid_query"),
        (Method::GET, "/api/worlds/prisoners-dilemma/kpis?limit=0", StatusCode::BAD_REQUEST, "invalid_query"),
        (Method::DELETE, "/api/worlds", StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"),
    ] {
        let (got, body) = app.call(method, path, None).await;
        assert_eq!(got, status, "{path}");
        assert_eq!(body["error"]["code"], code, "{path}");
        assert!(body["error"]["message"].is_string(), "{path}");
    }

    let req = Request::builder().method(Method::POST).uri("/api/runs").header("cookie", app.cookie.as_deref().unwrap()).body(Body::from("{not json")).unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["error"]["code"], "invalid_json");
    app.close().await;
}

#[tokio::test]
async fn validate_reports_problems_without_failing_the_request() {
    let app = TestApp::new().await;
    let (_, mut world) = app.get("/api/worlds/prisoners-dilemma").await;
    let (status, body) = app.post("/api/worlds/validate", world.clone()).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "valid": true, "errors": [] })));

    world["time"]["iterations"] = json!(0);
    world["payoff"]["matrix"]["C"].as_object_mut().unwrap().remove("D");
    world["kpis"]["definitions"][0]["query"]["filter"]["outcome"] = json!("C/Z");
    let (status, body) = app.post("/api/worlds/validate", world.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], false);
    let paths: Vec<&str> = body["errors"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["time.iterations", "payoff.matrix.C", "kpis.definitions[0].query.filter.outcome"]);

    world.as_object_mut().unwrap().remove("decisions");
    let (status, body) = app.post("/api/worlds/validate", world).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["valid"], false);
    assert!(body["errors"][0]["message"].as_str().unwrap().contains("decisions"));
    app.close().await;
}

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
async fn a_completed_run_writes_its_artifacts_to_object_storage() {
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

    // Every listed file exists in the bucket with the recorded size and checksum.
    for f in files {
        let key = format!("{}/runs/1/{}", app.schema, f["name"].as_str().unwrap());
        let bytes = app.artifacts.read(&key).await.unwrap_or_else(|e| panic!("{key}: {e}"));
        assert_eq!(bytes.len() as i64, f["size_bytes"].as_i64().unwrap(), "{key}");
        let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(sha, f["sha256"].as_str().unwrap(), "{key}");
    }

    // The stored summary is the run the API reports; the stored manifest lists the other files.
    let summary: Value = serde_json::from_slice(&app.artifacts.read(&format!("{}/runs/1/run_summary.json", app.schema)).await.unwrap()).unwrap();
    assert_eq!(summary["summary"]["ranking"], run["ranking"]);
    assert_eq!(summary["summary"]["seed"], 3);
    let stored: Value = serde_json::from_slice(&app.artifacts.read(&format!("{}/runs/1/manifest.json", app.schema)).await.unwrap()).unwrap();
    assert_eq!(stored["artifacts"].as_array().unwrap().len(), 8);
    let trace: Value =
        serde_json::from_slice(&app.artifacts.read(&format!("{}/runs/1/traces/always_defect__tit_for_tat.json", app.schema)).await.unwrap()).unwrap();
    assert_eq!(trace["moves"].as_array().unwrap().len(), 20);
    assert_eq!(trace["moves"][0], json!([1, 0]));

    // Asking for a file redirects to a signed, expiring URL.
    let req = Request::builder().uri("/api/runs/1/artifacts/traces/always_defect__tit_for_tat.json").body(Body::empty()).unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    let location = res.headers()["location"].to_str().unwrap().to_string();
    assert!(location.contains("X-Amz-Signature=") && location.contains("X-Amz-Expires=300"), "{location}");
    assert!(location.contains(&format!("{}/runs/1/traces/always_defect__tit_for_tat.json", app.schema)), "{location}");

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

// --- accounts ---

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
        assert!(sent.body.contains("http://localhost:3000/sign-in/confirm?token="), "{}", sent.body);
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

// --- strategy authoring ---

fn grudger(extra: Value) -> Value {
    let mut body = json!({
        "name": "Grudger", "description": "Cooperates until betrayed, then never forgives.", "mode": "rules",
        "states": ["NICE", "GRIM"], "initial_state": "NICE", "action_map": { "NICE": "C", "GRIM": "D" },
        "transitions": [{ "from": "NICE", "to": "GRIM", "guard": { "op": "last_opponent_action", "is": "D" } }],
    });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    body
}

fn ids(list: &Value) -> Vec<String> {
    list["strategies"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn own_strategies_are_created_edited_deleted_and_private_by_default() {
    let app = TestApp::new().await;
    let bob = app.signed_in_as("bob@example.com").await;
    let anon = app.anonymous();

    assert_eq!(anon.post("/api/strategies", grudger(json!({}))).await.0, StatusCode::UNAUTHORIZED);

    let (status, made) = app.post("/api/strategies", grudger(json!({ "prompt": "be nice until betrayed" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("grudger-") && id.len() == "grudger-".len() + 6, "{id}");
    assert_eq!(
        (made["mode"].as_str(), made["visibility"].as_str(), made["mine"].as_bool(), made["builtin"].as_bool(), made["owner_name"].as_str()),
        (Some("rules"), Some("private"), Some(true), Some(false), Some("ada"))
    );
    assert_eq!(made["prompt"], "be nice until betrayed");
    assert_eq!(made["states"], json!(["NICE", "GRIM"]));

    // Private: the owner sees it; nobody else does, and they cannot tell it exists.
    let path = format!("/api/strategies/{id}");
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 5);
    assert_eq!(ids(&app.get("/api/strategies").await.1)[..4], ["always_cooperate", "always_defect", "tit_for_tat", "win_stay_lose_shift"]);
    assert!(!ids(&bob.get("/api/strategies").await.1).contains(&id));
    assert!(!ids(&anon.get("/api/strategies").await.1).contains(&id));
    assert_eq!(app.get(&path).await.0, StatusCode::OK);
    assert_eq!(bob.get(&path).await.0, StatusCode::NOT_FOUND);
    assert_eq!(anon.get(&path).await.0, StatusCode::NOT_FOUND);
    assert_eq!(bob.call(Method::PUT, &path, Some(grudger(json!({})))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(bob.call(Method::DELETE, &path, None).await.0, StatusCode::NOT_FOUND);

    // Published: others can read it, without the prompt, but still not change it.
    let (status, updated) = app.call(Method::PUT, &path, Some(grudger(json!({ "name": "Grim Trigger", "visibility": "public" })))).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!((updated["id"].as_str(), updated["name"].as_str(), updated["visibility"].as_str()), (Some(id.as_str()), Some("Grim Trigger"), Some("public")));
    let (status, seen) = bob.get(&path).await;
    assert_eq!((status, seen["mine"].as_bool(), seen["owner_name"].as_str()), (StatusCode::OK, Some(false), Some("ada")));
    assert!(seen.get("prompt").is_none());
    assert_eq!(bob.call(Method::PUT, &path, Some(grudger(json!({})))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(bob.call(Method::DELETE, &path, None).await.0, StatusCode::FORBIDDEN);

    // The mode is fixed once saved; built-ins are nobody's to change.
    let (status, body) = app.call(Method::PUT, &path, Some(json!({ "name": "x", "mode": "code", "source": "function decide() { return 'C'; }" }))).await;
    assert_eq!((status, body["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("mode")));
    assert_eq!(app.call(Method::PUT, "/api/strategies/tit_for_tat", Some(grudger(json!({})))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.call(Method::DELETE, "/api/strategies/tit_for_tat", None).await.0, StatusCode::FORBIDDEN);

    assert_eq!(app.call(Method::DELETE, &path, None).await.0, StatusCode::OK);
    assert_eq!(app.get(&path).await.0, StatusCode::NOT_FOUND);
    app.close().await;
}

#[tokio::test]
async fn invalid_strategies_are_refused_with_every_problem() {
    let app = TestApp::new().await;
    let mut bad = grudger(json!({ "name": "", "visibility": "secret", "world_id": "nope" }));
    bad["action_map"]["GRIM"] = json!("X");
    let (status, body) = app.post("/api/strategies", bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let paths: Vec<&str> = body["error"]["details"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["visibility", "world_id", "name"], "the rules are only checked against a world that exists");

    let mut bad = grudger(json!({}));
    bad["action_map"]["GRIM"] = json!("X");
    bad["transitions"][0]["to"] = json!("NOWHERE");
    let (_, body) = app.post("/api/strategies", bad).await;
    let paths: Vec<&str> = body["error"]["details"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["action_map.GRIM", "transitions[0].to"]);

    for (body, path) in [
        (json!({ "name": "A", "mode": "code" }), "source"),
        (json!({ "name": "A", "mode": "code", "source": "function nope() {}" }), "source"),
        (json!({ "name": "A", "mode": "code", "source": "function decide() { return 'Z'; }" }), "source"),
        (json!({ "name": "A", "mode": "code", "source": "function decide() { while (true) {} }" }), "source"),
        (json!({ "name": "A", "mode": "spells" }), "mode"),
    ] {
        let (status, answer) = app.post("/api/strategies", body.clone()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(answer["error"]["details"][0]["path"], path, "{answer}");
    }
    assert_eq!(ids(&app.get("/api/strategies").await.1).len(), 4, "nothing was saved");
    app.close().await;
}

#[tokio::test]
async fn own_strategies_play_in_runs_and_private_ones_keep_the_run_private() {
    let app = TestApp::new().await;
    let bob = app.signed_in_as("bob@example.com").await;

    let (_, rules) = app.post("/api/strategies", grudger(json!({}))).await;
    let (status, code) = app
        .post(
            "/api/strategies",
            json!({ "name": "Copycat JS", "mode": "code", "visibility": "public", "source": "function decide(input) { return input.lastOpponentAction ?? 'C'; }" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{code}");
    assert_eq!((code["mode"].as_str(), code["language"].as_str()), (Some("code"), Some("javascript")));
    let (grudger_id, copycat_id) = (rules["id"].as_str().unwrap(), code["id"].as_str().unwrap());

    // Bob cannot enter Ada's private strategy; he can enter her public one.
    let (status, body) = bob.post("/api/runs", pd(json!({ "actors": [grudger_id, "always_defect"] }))).await;
    assert_eq!((status, body["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("actors[0]")));
    let public = bob.run(pd(json!({ "actors": [copycat_id, "always_defect", "tit_for_tat"], "iterations": 50 }))).await;
    assert_eq!((public["status"].as_str(), public["private"].as_bool()), (Some("completed"), Some(false)), "{public}");
    // The JavaScript copycat scores exactly like the built-in Tit for Tat against Always Defect.
    let average = |id: &str| public["matches"].as_array().unwrap().iter().find(|m| (m["a"] == id) != (m["b"] == id) && (m["a"] == "always_defect" || m["b"] == "always_defect")).unwrap()["totals"].clone();
    assert_eq!(average(copycat_id).as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).sum::<i64>(), 54 + 49);
    assert_eq!(average("tit_for_tat").as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).sum::<i64>(), 54 + 49);

    // Ada's run with her private strategy is hers alone.
    let private = app.run(pd(json!({ "actors": [grudger_id, copycat_id, "always_defect"], "iterations": 30 }))).await;
    assert_eq!((private["status"].as_str(), private["private"].as_bool()), (Some("completed"), Some(true)), "{private}");
    let id = private["id"].as_i64().unwrap();
    for path in [format!("/api/runs/{id}"), format!("/api/runs/{id}/artifacts"), format!("/api/runs/{id}/matches/always_defect/{grudger_id}")] {
        assert_eq!(app.get(&path).await.0, StatusCode::OK, "{path}");
        assert_eq!(bob.get(&path).await.0, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(app.anonymous().get(&path).await.0, StatusCode::NOT_FOUND, "{path}");
    }
    let listed = |list: Value| list["runs"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect::<Vec<_>>();
    assert_eq!(listed(app.get("/api/runs").await.1), [id, public["id"].as_i64().unwrap()]);
    assert_eq!(listed(bob.get("/api/runs").await.1), [public["id"].as_i64().unwrap()]);
    // World analytics only ever show public runs.
    assert_eq!(app.get("/api/worlds/prisoners-dilemma/leaderboard").await.1["run_id"], public["id"]);
    assert_eq!(app.get(&format!("/api/worlds/prisoners-dilemma/pairwise?run_id={id}")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/api/worlds/prisoners-dilemma/kpis").await.1["snapshots"].as_array().unwrap().len(), 1);

    // Deleting a strategy leaves finished runs readable: they carry their own copy.
    assert_eq!(app.call(Method::DELETE, &format!("/api/strategies/{grudger_id}"), None).await.0, StatusCode::OK);
    assert_eq!(app.get(&format!("/api/runs/{id}")).await.1["ranking"].as_array().unwrap().len(), 3);
    app.close().await;
}

#[tokio::test]
async fn preview_checks_a_draft_and_plays_sample_matches() {
    let app = TestApp::new().await;
    assert_eq!(app.anonymous().post("/api/strategies/preview", json!({ "strategy": grudger(json!({})) })).await.0, StatusCode::UNAUTHORIZED);

    let (status, p) = app.post("/api/strategies/preview", json!({ "strategy": grudger(json!({ "name": "" })) })).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!((p["valid"].as_bool(), p["errors"].clone()), (Some(true), json!([])));
    assert_eq!(p["warnings"], json!([{ "path": "states[1]", "message": "once in \"GRIM\", the strategy never leaves it" }]));
    assert_eq!(p["samples"][0]["opponent"]["name"], "Always Cooperate");
    assert_eq!(p["samples"][1]["own_moves"].as_array().unwrap()[..3], [json!("C"), json!("D"), json!("D")]);
    assert_eq!(p["samples"][1]["totals"], json!([19, 24]));

    let mut broken = grudger(json!({}));
    broken["initial_state"] = json!("START");
    let (status, p) = app.post("/api/strategies/preview", json!({ "strategy": broken })).await;
    assert_eq!((status, p["valid"].as_bool()), (StatusCode::OK, Some(false)));
    assert_eq!(p["errors"][0]["path"], "initial_state");
    assert_eq!(p["samples"], json!([]));

    let (_, p) = app.post("/api/strategies/preview", json!({ "strategy": { "mode": "code", "source": "function decide(i) { return i.roundIndex % 2 ? 'D' : 'C'; }" } })).await;
    assert_eq!(p["valid"], true, "{p}");
    assert_eq!(p["samples"][0]["own_moves"].as_array().unwrap()[..4], [json!("C"), json!("D"), json!("C"), json!("D")]);
    assert_eq!(app.get("/api/usage").await.1["used"], 0, "previews use no quota");
    app.close().await;
}

// --- plain English to rules ---

const GRIM_JSON: &str = r#"{"name":"Grim","description":"Cooperates until betrayed.","states":["NICE","GRIM"],"initial_state":"NICE","action_map":{"NICE":"C","GRIM":"D"},"transitions":[{"from":"NICE","to":"GRIM","guard":{"op":"last_opponent_action","is":"D"}}],"rationale":"Two states."}"#;

async fn app_with_fake_llm() -> (TestApp, FakeApi) {
    let api = FakeApi::default();
    let llm = Llm::new(Provider::Anthropic, &api.start().await, "sk-test".into(), "claude-opus-5-5".into(), Some("medium".into())).unwrap();
    (TestApp::with_llm(Some(llm)).await, api)
}

#[tokio::test]
async fn plain_english_is_compiled_to_rules_and_checked() {
    let (app, api) = app_with_fake_llm().await;
    assert_eq!(app.get("/api/me").await.1["features"]["compile"], true);
    assert_eq!(app.anonymous().post("/api/strategies/compile", json!({ "prompt": "cooperate until betrayed" })).await.0, StatusCode::UNAUTHORIZED);
    let (status, body) = app.post("/api/strategies/compile", json!({ "prompt": "short" })).await;
    assert_eq!((status, body["error"]["details"][0]["path"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("prompt")));
    assert_eq!(api.seen.lock().unwrap().len(), 0, "nothing is sent for a request that is refused");

    api.say(GRIM_JSON);
    let (status, body) = app.post("/api/strategies/compile", json!({ "prompt": "Cooperate until the other player betrays me, then never forgive." })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["valid"].as_bool(), body["rationale"].as_str()), (Some(true), Some("Two states.")));
    assert_eq!(body["strategy"]["states"], json!(["NICE", "GRIM"]));
    assert_eq!(body["flags"], json!([]));
    {
        let seen = api.seen.lock().unwrap();
        let sent = &seen[0].1;
        assert!(sent["system"].as_str().unwrap().contains("last_opponent_action"));
        assert!(sent["system"].as_str().unwrap().contains("Never assume silently"));
        let ask = sent["messages"][0]["content"].as_str().unwrap();
        assert!(ask.contains("Allowed decisions: C, D") && ask.contains("- D/C -> 5, 0") && ask.contains("never forgive"), "{ask}");
    }

    // Flags are handed on for the editor to show.
    api.say(&GRIM_JSON.replace("\"rationale\":\"Two states.\"", "\"rationale\":\"Two states.\",\"flags\":[{\"kind\":\"violation\",\"asked\":\"defect one time in ten\",\"why\":\"A strategy may not use chance.\",\"instead\":\"Left out.\",\"code\":false}]"));
    let (status, flagged) = app.post("/api/strategies/compile", json!({ "prompt": "Cooperate until betrayed, but defect one time in ten anyway." })).await;
    assert_eq!((status, flagged["valid"].as_bool()), (StatusCode::OK, Some(true)), "{flagged}");
    assert_eq!(flagged["flags"], json!([{ "kind": "violation", "asked": "defect one time in ten", "why": "A strategy may not use chance.", "instead": "Left out.", "code": false }]));

    // A description that rests on breaking the rules is refused under its own code.
    api.say(r#"{"error": "A strategy may not know who its opponent is.", "breaks_rules": true}"#);
    let (status, refused) = app.post("/api/strategies/compile", json!({ "prompt": "Read the description of my opponent and beat it." })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused["error"], json!({ "code": "rule_violation", "message": "A strategy may not know who its opponent is." }));

    // The compiled rules go straight into a save.
    let mut save = body["strategy"].clone();
    save["mode"] = json!("rules");
    assert_eq!(app.post("/api/strategies", save).await.0, StatusCode::CREATED);
    app.close().await;
}

#[tokio::test]
async fn rejected_rules_are_sent_back_to_the_model_once() {
    let (app, api) = app_with_fake_llm().await;
    api.say(&GRIM_JSON.replace("\"is\":\"D\"", "\"is\":\"betrayal\"")).say(&format!("```json\n{GRIM_JSON}\n```"));
    let (status, body) = app.post("/api/strategies/compile", json!({ "prompt": "Cooperate until betrayed, then defect for ever." })).await;
    assert_eq!((status, body["valid"].as_bool()), (StatusCode::OK, Some(true)), "{body}");
    {
        let seen = api.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let retry = &seen[1].1["messages"];
        assert_eq!(retry.as_array().unwrap().len(), 3);
        assert_eq!(retry[1]["role"], "assistant");
        assert!(retry[2]["content"].as_str().unwrap().contains("transitions[0].guard.is: \"betrayal\" is not a decision this world allows"), "{retry}");
    }

    // Still wrong after the retry: the rules come back with the problems, for the editor.
    api.say(&GRIM_JSON.replace("\"GRIM\":\"D\"", "\"GRIM\":\"X\"")).say(&GRIM_JSON.replace("\"GRIM\":\"D\"", "\"GRIM\":\"X\""));
    let (status, body) = app.post("/api/strategies/compile", json!({ "prompt": "Cooperate until betrayed, then defect for ever." })).await;
    assert_eq!((status, body["valid"].as_bool()), (StatusCode::OK, Some(false)));
    assert_eq!(body["errors"][0]["path"], "action_map.GRIM");
    app.close().await;
}

#[tokio::test]
async fn compile_failures_are_explained_and_limited() {
    let (app, api) = app_with_fake_llm().await;
    api.say(r#"{"error": "This describes a pancake recipe, not a strategy."}"#);
    let (status, body) = app.post("/api/strategies/compile", json!({ "prompt": "Mix flour, eggs and milk, then fry." })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], json!({ "code": "not_compiled", "message": "This describes a pancake recipe, not a strategy." }));

    api.reply(401, json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } }));
    let (status, body) = app.post("/api/strategies/compile", json!({ "prompt": "Always cooperate, whatever happens." })).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("compile_unavailable")));
    assert!(!body.to_string().contains("x-api-key"), "provider details stay in the log");

    api.reply(200, json!({ "stop_reason": "refusal", "content": [] }));
    let (status, _) = app.post("/api/strategies/compile", json!({ "prompt": "Always cooperate, whatever happens." })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // The test configuration allows three compiles an hour.
    let (status, body) = app.post("/api/strategies/compile", json!({ "prompt": "Always cooperate, whatever happens." })).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_many_requests")));
    app.close().await;

    // Without a model configured the feature says so.
    let plain = TestApp::new().await;
    assert_eq!(plain.get("/api/me").await.1["features"]["compile"], false);
    let (status, body) = plain.post("/api/strategies/compile", json!({ "prompt": "Always cooperate, whatever happens." })).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("compile_unavailable")));
    plain.close().await;
}
