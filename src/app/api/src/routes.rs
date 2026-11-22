//! HTTP routes. Everything lives under `/api` and speaks JSON.

use crate::artifacts::{self, pair_metrics};
use crate::auth::{User, Viewer};
use crate::error::ApiError;
use crate::store::{Queued, RunInput, RunRow};
use crate::strategies::{RunActor, StrategyDoc};
use crate::worker::AppState;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lilanaya_engine::{Issue, MatchResult, Policy, RunResult, ScheduleMode, World};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const MAX_RUN_ITERATIONS: u32 = 10_000;
pub const MAX_RUN_ACTORS: usize = 16;
/// Largest integer JavaScript represents exactly; seeds must survive a round trip through the browser.
pub const MAX_SEED: u64 = (1 << 53) - 1;
const RUN_LIST_LIMIT: i64 = 100;
/// The largest request body: room for the biggest strategy that passes validation, and little more.
const MAX_BODY_BYTES: usize = 128 * 1024;
const KPI_HISTORY_DEFAULT: i64 = 10;
const KPI_HISTORY_MAX: i64 = 100;

pub(crate) type Shared = State<Arc<AppState>>;
pub(crate) type ApiResult = Result<Json<Value>, ApiError>;

pub fn app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/hello", get(hello))
        .route("/api/worlds", get(list_worlds))
        .route("/api/worlds/validate", post(validate_world))
        .route("/api/worlds/{id}", get(get_world))
        .route("/api/worlds/{id}/leaderboard", get(leaderboard))
        .route("/api/worlds/{id}/pairwise", get(pairwise))
        .route("/api/worlds/{id}/kpis", get(kpis))
        .route("/api/auth/email", post(account::request_link))
        .route("/api/auth/email/confirm", post(account::confirm_link))
        .route("/api/auth/sign-out", post(account::sign_out))
        .route("/api/auth/providers", get(federated::providers))
        .route("/api/auth/{provider}/start", get(federated::start))
        .route("/api/auth/{provider}/callback", get(federated::callback).post(federated::callback_form))
        .route("/api/plans", get(account::plans))
        .route("/api/session", get(account::session))
        .route("/api/me", get(account::me).patch(account::update_me))
        .route("/api/me/entitlements", get(account::entitlements))
        .route("/api/usage", get(account::usage))
        .route("/api/strategies", get(authoring::list).post(authoring::create))
        .route("/api/strategies/preview", post(authoring::preview))
        .route("/api/strategies/compile", post(authoring::compile))
        .route("/api/strategies/{id}", put(authoring::update).get(authoring::get_one).delete(authoring::delete))
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/{id}", get(get_run))
        .route("/api/runs/{id}/matches/{a}/{b}", get(get_match))
        .route("/api/runs/{id}/race", get(get_race))
        .route("/api/runs/{id}/artifacts", get(list_artifacts))
        .route("/api/runs/{id}/artifacts/{*name}", get(get_artifact))
        .fallback(|| async { ApiError::not_found("This route") })
        .method_not_allowed_fallback(|| async {
            ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", "This route does not support that method")
        })
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn hello() -> Json<Value> {
    Json(json!({ "message": "Hello from Lilanaya!" }))
}

// --- worlds ---

async fn list_worlds(State(state): Shared) -> ApiResult {
    Ok(Json(json!({ "worlds": state.store.worlds().await? })))
}

async fn get_world(State(state): Shared, Path(id): Path<String>) -> ApiResult {
    Ok(Json(json!(find_world(&state, &id).await?)))
}

/// Always answers 200 for well-formed JSON: `{"valid": bool, "errors": [{path, message}]}`.
async fn validate_world(body: Result<Bytes, BytesRejection>) -> ApiResult {
    let value: Value = parse_json(body)?;
    let errors = match serde_json::from_value::<World>(value) {
        Ok(world) => world.validate(),
        // The document does not have the shape of a world (missing field, wrong type, unknown enum value).
        Err(e) => vec![Issue::new("", e.to_string())],
    };
    Ok(Json(json!({ "valid": errors.is_empty(), "errors": errors })))
}

/// The ranking of the world's latest completed run.
async fn leaderboard(State(state): Shared, Path(id): Path<String>) -> ApiResult {
    let world = find_world(&state, &id).await?;
    let Some(run) = state.store.completed_run(&world.id, None).await? else {
        return Ok(Json(json!({ "world_id": world.id, "run_id": null, "entries": [] })));
    };
    let r = result_of(&run)?;
    let entries: Vec<Value> = r
        .ranking
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let a = r.actors.iter().find(|a| &a.id == id)?;
            Some(json!({
                "rank": i + 1,
                "actor_id": a.id,
                "name": a.name,
                "average": a.average,
                "variance": a.variance,
                "total": a.total,
                "entries": a.matches,
            }))
        })
        .collect();
    Ok(Json(json!({
        "world_id": world.id,
        "run_id": run.id,
        "seed": r.seed,
        "iterations": r.iterations,
        "created_at": run.created_at,
        "entries": entries,
    })))
}

#[derive(Deserialize)]
struct RunQuery {
    run_id: Option<String>,
    limit: Option<String>,
}

fn parse_run_id(raw: &Option<String>) -> Result<Option<i64>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => s.parse::<i64>().map(Some).map_err(|_| ApiError::bad_request("invalid_query", "run_id must be a whole number")),
    }
}

/// Pairwise metrics of one completed run (`?run_id=`, default: the latest):
/// average payoffs, outcome mix, first defection round and punish duration.
async fn pairwise(State(state): Shared, Path(id): Path<String>, Query(q): Query<RunQuery>) -> ApiResult {
    let world = find_world(&state, &id).await?;
    let run_id = parse_run_id(&q.run_id)?;
    let Some(run) = state.store.completed_run(&world.id, run_id).await? else {
        return match run_id {
            Some(n) => Err(ApiError::not_found(format!("A completed run {n} of world \"{}\"", world.id))),
            None => Ok(Json(json!({ "world_id": world.id, "run_id": null, "actors": [], "pairs": [] }))),
        };
    };
    let r = result_of(&run)?;
    let pairs = pair_metrics(r);
    Ok(Json(json!({
        "world_id": world.id,
        "run_id": run.id,
        "seed": r.seed,
        "iterations": r.iterations,
        "actors": r.actors.iter().map(|a| json!({ "id": a.id, "name": a.name })).collect::<Vec<_>>(),
        "pairs": pairs,
    })))
}

/// KPI snapshots of the world's completed runs, oldest first, each value with
/// its change from the previous run. `?limit=` (default 10) picks how many
/// runs; `?run_id=` ends the list at that run.
async fn kpis(State(state): Shared, Path(id): Path<String>, Query(q): Query<RunQuery>) -> ApiResult {
    let world = find_world(&state, &id).await?;
    let up_to = parse_run_id(&q.run_id)?;
    let limit = match &q.limit {
        None => KPI_HISTORY_DEFAULT,
        Some(s) => match s.parse::<i64>() {
            Ok(n) if (1..=KPI_HISTORY_MAX).contains(&n) => n,
            _ => return Err(ApiError::bad_request("invalid_query", format!("limit must be between 1 and {KPI_HISTORY_MAX}"))),
        },
    };
    // One extra run, so the oldest snapshot shown still has something to compare with.
    let rows = state.store.kpi_history(&world.id, up_to, limit + 1).await?;

    let mut snapshots: Vec<Value> = Vec::new();
    let mut previous: BTreeMap<String, f64> = BTreeMap::new();
    let mut current: Option<(i64, Value, Map<String, Value>, BTreeMap<String, f64>)> = None;
    let mut flush = |entry: Option<(i64, Value, Map<String, Value>, BTreeMap<String, f64>)>, previous: &mut BTreeMap<String, f64>| {
        if let Some((_, mut head, values, seen)) = entry {
            head["values"] = Value::Object(values);
            snapshots.push(head);
            *previous = seen;
        }
    };
    for row in rows {
        if current.as_ref().map(|c| c.0) != Some(row.run_id) {
            flush(current.take(), &mut previous);
            let head = json!({ "run_id": row.run_id, "seed": row.seed, "iterations": row.iterations, "created_at": row.created_at });
            current = Some((row.run_id, head, Map::new(), BTreeMap::new()));
        }
        let (_, _, values, seen) = current.as_mut().expect("set above");
        let delta = previous.get(&row.kpi_id).map(|before| row.value - before);
        values.insert(row.kpi_id.clone(), json!({ "label": row.label, "value": row.value, "delta": delta }));
        seen.insert(row.kpi_id, row.value);
    }
    flush(current.take(), &mut previous);
    if snapshots.len() as i64 > limit {
        snapshots.remove(0);
    }

    Ok(Json(json!({
        "world_id": world.id,
        "definitions": world.kpis.definitions,
        "snapshots": snapshots,
    })))
}

// --- runs ---

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    world_id: String,
    /// Strategy ids to enter, one actor each: any the user may see. Default: every built-in strategy.
    actors: Option<Vec<String>>,
    /// Rounds per pairing. Default: the world's.
    iterations: Option<u32>,
    /// Default: the world's seed.
    seed: Option<u64>,
    /// Pairing order: `all_pairs` or `random`. Default: the world's.
    policy: Option<Policy>,
    /// For `random`: `shuffle_all` or `sample_per_round`. Default: the world's.
    schedule_mode: Option<ScheduleMode>,
}

/// Checks the request, queues the run and answers 202 with the queued run.
/// Poll `GET /api/runs/{id}` until `status` is `completed` or `failed`.
/// Needs a signed-in user. A run that enters a private strategy is itself private.
/// Answers 429 when the user has used up the month's runs (`run_limit_reached`)
/// or already has as many runs waiting or running as one user may (`too_many_runs`).
async fn create_run(State(state): Shared, user: User, body: Result<Bytes, BytesRejection>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let req: RunRequest = parse_json(body)?;
    let invalid = |issues| ApiError::invalid("The run request is not valid", issues);
    let mut issues = Vec::new();

    let Some(mut world) = state.store.world(&req.world_id).await? else {
        return Err(invalid(vec![Issue::new("world_id", format!("there is no world \"{}\"", req.world_id))]));
    };
    if let Some(iterations) = req.iterations {
        world.time.iterations = iterations;
    }
    if world.time.iterations == 0 || world.time.iterations > MAX_RUN_ITERATIONS {
        issues.push(Issue::new("iterations", format!("must be between 1 and {MAX_RUN_ITERATIONS}")));
    }
    if let Some(policy) = req.policy {
        world.interaction.policy = policy;
        if policy == Policy::RoundRobin {
            issues.push(Issue::new("policy", "\"round_robin\" is not supported yet; use \"all_pairs\" or \"random\""));
        }
    }
    if let Some(mode) = req.schedule_mode {
        world.randomness.schedule_mode = mode;
    }
    let seed = req.seed.unwrap_or(world.determinism.rng_seed);
    if seed > MAX_SEED {
        issues.push(Issue::new("seed", format!("must be at most {MAX_SEED}")));
    }

    let strategies = state.store.strategies(Some(user.id)).await?;
    let ids: Vec<String> = req.actors.unwrap_or_else(|| strategies.iter().filter(|s| s.builtin).map(|s| s.doc.id().to_string()).collect());
    let mut private = false;
    if ids.len() < 2 || ids.len() > MAX_RUN_ACTORS {
        issues.push(Issue::new("actors", format!("pick between 2 and {MAX_RUN_ACTORS} actors")));
    }
    let mut actors = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        if ids[..i].contains(id) {
            issues.push(Issue::new(format!("actors[{i}]"), format!("\"{id}\" is entered more than once")));
            continue;
        }
        match strategies.iter().find(|s| s.doc.id() == id) {
            Some(s) => {
                // Catch rules that do not fit this world now, not in the worker. Code was run when it
                // was saved and is not run again here: nothing a user wrote executes in a request.
                if let StrategyDoc::Rules(rules) = &s.doc {
                    issues.extend(rules.validate(&world.decisions.allowed).into_iter().map(|e| Issue::new(format!("actors[{i}].{}", e.path), e.message)));
                }
                private |= s.visibility == "private";
                actors.push(RunActor { id: id.clone(), name: s.doc.name().to_string(), strategy: s.doc.clone() });
            }
            None => issues.push(Issue::new(format!("actors[{i}]"), format!("there is no strategy \"{id}\""))),
        }
    }
    if !issues.is_empty() {
        return Err(invalid(issues));
    }

    let monthly = state.config.monthly_runs(&user.plan);
    let active = state.config.max_active_runs;
    let run = match state.store.enqueue(&RunInput { world, actors, seed }, user.id, private, monthly, active).await? {
        Queued::Run(run) => run,
        Queued::MonthlyLimit => {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "run_limit_reached",
                format!("You have used all {monthly} runs your plan gives you this month. The count starts again on the 1st."),
            ))
        }
        Queued::TooManyActive => {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_runs",
                format!("You already have {active} {} waiting or running. Start this one when one of them has finished.", if active == 1 { "run" } else { "runs" }),
            ))
        }
    };
    state.wake.notify_one();
    Ok((StatusCode::ACCEPTED, Json(run_json(&run))))
}

/// The latest runs, newest first: public ones and the viewer's own.
async fn list_runs(State(state): Shared, viewer: Viewer) -> ApiResult {
    let runs = state.store.recent_runs(RUN_LIST_LIMIT, viewer.id()).await?;
    let list: Vec<Value> = runs
        .iter()
        .map(|run| {
            let winner = run.result.as_ref().and_then(|r| {
                let id = r.ranking.first()?;
                let a = r.actors.iter().find(|a| &a.id == id)?;
                Some(json!({ "id": a.id, "name": a.name, "average": a.average }))
            });
            let mut value = run_head(run);
            value["winner"] = json!(winner);
            value
        })
        .collect();
    Ok(Json(json!({ "runs": list })))
}

async fn get_run(State(state): Shared, viewer: Viewer, Path(id): Path<String>) -> ApiResult {
    Ok(Json(run_json(&find_run(&state, &id, &viewer).await?)))
}

/// The round-by-round record of one pairing, oriented as asked: `a` is the
/// first actor in the URL, whichever way round the pair is stored.
async fn get_match(State(state): Shared, viewer: Viewer, Path((id, a, b)): Path<(String, String, String)>) -> ApiResult {
    let run = find_run(&state, &id, &viewer).await?;
    let missing = || ApiError::not_found(format!("A match between \"{a}\" and \"{b}\" in run {id}"));
    let r = run.result.as_ref().ok_or_else(missing)?;
    let (summary, flipped) = r
        .matches
        .iter()
        .find_map(|m| {
            if m.a == a && m.b == b {
                Some((m.clone(), false))
            } else if m.a == b && m.b == a {
                Some((flip(m), true))
            } else {
                None
            }
        })
        .ok_or_else(missing)?;

    let (stored_a, stored_b) = if flipped { (&b, &a) } else { (&a, &b) };
    let moves = state.store.trace(run.id, stored_a, stored_b).await?.ok_or_else(missing)?;
    let side = |n: usize| -> Vec<u8> { moves.iter().map(|m| m[n]).collect() };
    let (moves_a, moves_b) = if flipped { (side(1), side(0)) } else { (side(0), side(1)) };
    let actor = |id: &str| {
        let actor = r.actors.iter().find(|x| x.id == id);
        json!({ "id": id, "name": actor.map(|x| x.name.as_str()) })
    };
    Ok(Json(json!({
        "run_id": run.id,
        "world_id": r.world_id,
        "seed": r.seed,
        "iterations": r.iterations,
        "decisions": r.decisions,
        "a": actor(&a),
        "b": actor(&b),
        "summary": summary,
        // Per round, an index into `decisions`.
        "moves_a": moves_a,
        "moves_b": moves_b,
    })))
}

/// At most this many points per match in the race: enough for a smooth line, small enough to send for every match.
const RACE_POINTS: usize = 60;

/// What the race on the results page is drawn from: for every match, both
/// sides' points so far at a number of sampled rounds.
/// `rounds` are the round counts sampled (the last is the final round);
/// `matches[i].totals[side][k]` is that side's points in the match after `rounds[k]` rounds.
async fn get_race(State(state): Shared, viewer: Viewer, Path(id): Path<String>) -> ApiResult {
    let run = find_run(&state, &id, &viewer).await?;
    let r = run.result.as_ref().ok_or_else(|| ApiError::not_found(format!("Results of run {id}")))?;
    let world = state.store.run_world(run.id).await?.ok_or_else(|| ApiError::internal(format!("run {} has no input", run.id)))?;
    let traces = state.store.traces(run.id).await?;

    let iterations = r.iterations as usize;
    let points = iterations.min(RACE_POINTS);
    let rounds: Vec<usize> = (1..=points).map(|k| (k * iterations).div_ceil(points)).collect();
    // Payoffs by decision index, as the traces record them.
    let payoffs: Vec<Vec<[i64; 2]>> = r
        .decisions
        .iter()
        .map(|own| r.decisions.iter().map(|other| world.payoff.matrix.get(own).and_then(|row| row.get(other)).copied().unwrap_or([0, 0])).collect())
        .collect();
    let matches: Vec<Value> = traces
        .iter()
        .map(|(a, b, moves)| {
            let (mut sum, mut next) = ([0i64; 2], 0usize);
            let mut totals = [Vec::with_capacity(points), Vec::with_capacity(points)];
            for (round, m) in moves.iter().enumerate() {
                if let Some(won) = payoffs.get(m[0] as usize).and_then(|row| row.get(m[1] as usize)) {
                    sum = [sum[0] + won[0], sum[1] + won[1]];
                }
                if next < points && round + 1 == rounds[next] {
                    totals[0].push(sum[0]);
                    totals[1].push(sum[1]);
                    next += 1;
                }
            }
            json!({ "a": a, "b": b, "totals": totals })
        })
        .collect();
    Ok(Json(json!({ "run_id": run.id, "iterations": r.iterations, "rounds": rounds, "matches": matches })))
}

/// The files of a run, built from what the database holds. A run that has not completed has none.
async fn run_files(state: &AppState, run: &RunRow) -> Result<Vec<(artifacts::ArtifactMeta, Vec<u8>)>, ApiError> {
    let Some(result) = &run.result else {
        return Ok(Vec::new());
    };
    let out = state.store.run_output(run.id, result).await?;
    let id = run.id;
    // A large run is megabytes of JSON to write and hash: keep that off the async threads.
    tokio::task::spawn_blocking(move || artifacts::files(id, &out)).await.map_err(|e| ApiError::internal(format!("build run files: {e}")))
}

/// The run's manifest: its files, with size and checksum. `href` answers with the file.
async fn list_artifacts(State(state): Shared, viewer: Viewer, Path(id): Path<String>) -> ApiResult {
    let run = find_run(&state, &id, &viewer).await?;
    let artifacts: Vec<Value> = run_files(&state, &run)
        .await?
        .into_iter()
        .map(|(a, _)| {
            json!({
                "name": a.name,
                "content_type": a.content_type,
                "size_bytes": a.size_bytes,
                "sha256": a.sha256,
                "href": format!("/api/runs/{}/artifacts/{}", run.id, a.name),
            })
        })
        .collect();
    Ok(Json(json!({ "run_id": run.id, "status": run.status, "artifacts": artifacts })))
}

/// One of the run's files.
async fn get_artifact(State(state): Shared, viewer: Viewer, Path((id, name)): Path<(String, String)>) -> Result<Response, ApiError> {
    let run = find_run(&state, &id, &viewer).await?;
    let (_, bytes) = run_files(&state, &run).await?.into_iter().find(|(a, _)| a.name == name).ok_or_else(|| ApiError::not_found(format!("File \"{name}\" of run {id}")))?;
    let headers = [
        (header::CONTENT_TYPE, artifacts::CONTENT_TYPE),
        // A private run's files must not be kept by anything between us and the browser.
        (header::CACHE_CONTROL, "private, no-store"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ];
    Ok((headers, bytes).into_response())
}

/// The same match seen from the other side.
fn flip(m: &MatchResult) -> MatchResult {
    let swap_keys = |map: &BTreeMap<String, u64>| -> BTreeMap<String, u64> {
        map.iter()
            .map(|(k, v)| match k.split_once('/') {
                Some((x, y)) => (format!("{y}/{x}"), *v),
                None => (k.clone(), *v),
            })
            .collect()
    };
    MatchResult {
        a: m.b.clone(),
        b: m.a.clone(),
        seed: m.seed.clone(),
        totals: [m.totals[1], m.totals[0]],
        averages: [m.averages[1], m.averages[0]],
        outcome_counts: swap_keys(&m.outcome_counts),
        outcome_runs: swap_keys(&m.outcome_runs),
        first_played: m.first_played.iter().map(|(k, v)| (k.clone(), [v[1], v[0]])).collect(),
    }
}

/// What every run answer carries, whatever its status.
fn run_head(run: &RunRow) -> Value {
    json!({
        "id": run.id,
        "status": run.status,
        "world_id": run.world_id,
        "world_name": run.world_name,
        "seed": run.seed,
        "iterations": run.iterations,
        "actor_count": run.actor_count,
        "private": run.private,
        "created_at": run.created_at,
        "finished_at": run.finished_at,
    })
}

/// The head, plus the engine's summary once completed, or `error` if failed.
fn run_json(run: &RunRow) -> Value {
    let mut value = run_head(run);
    if let Some(result) = &run.result {
        if let (Some(target), Value::Object(fields)) = (value.as_object_mut(), json!(result)) {
            target.extend(fields);
        }
    }
    if let Some(error) = &run.error {
        value["error"] = error.clone();
    }
    value
}

// --- helpers ---

async fn find_world(state: &AppState, id: &str) -> Result<World, ApiError> {
    state.store.world(id).await?.ok_or_else(|| ApiError::not_found(format!("World \"{id}\"")))
}

/// The run, if the viewer may see it. Someone else's private run answers
/// "not found", so its existence is not given away.
async fn find_run(state: &AppState, id: &str, viewer: &Viewer) -> Result<RunRow, ApiError> {
    let missing = || ApiError::not_found(format!("Run \"{id}\""));
    let n = id.parse::<i64>().map_err(|_| missing())?;
    let run = state.store.run(n).await?.ok_or_else(missing)?;
    if run.private && (run.user_id.is_none() || run.user_id != viewer.id()) {
        return Err(missing());
    }
    Ok(run)
}

fn result_of(run: &RunRow) -> Result<&RunResult, ApiError> {
    run.result.as_ref().ok_or_else(|| ApiError::internal(format!("run {} is completed but has no result", run.id)))
}

fn parse_json<T: DeserializeOwned>(body: Result<Bytes, BytesRejection>) -> Result<T, ApiError> {
    let bytes = body.map_err(|e| ApiError::new(e.status(), "invalid_body", e.body_text()))?;
    serde_json::from_slice(&bytes).map_err(|e| ApiError::bad_request("invalid_json", format!("The request body is not valid: {e}")))
}

mod account;
mod authoring;
mod federated;

#[cfg(test)]
mod tests;
