//! A run's files, built on request from what the database holds:
//! `run_summary.json`, `pairwise_matrix.json`, `traces/{a}__{b}.json` (one per
//! pairing) and `manifest.json`, which lists the others with size and SHA-256.
//! Nothing is stored: the same run always gives the same bytes.

use lilanaya_engine::{RunOutput, RunResult};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const CONTENT_TYPE: &str = "application/json";

/// One file, as listed in the manifest.
#[derive(Clone, Debug, Serialize)]
pub struct ArtifactMeta {
    pub name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}

fn meta(name: &str, bytes: &[u8]) -> ArtifactMeta {
    ArtifactMeta {
        name: name.to_string(),
        content_type: CONTENT_TYPE.to_string(),
        size_bytes: bytes.len() as i64,
        sha256: Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect(),
    }
}

/// Every file of a finished run, the manifest last: `(what the manifest says about it, its contents)`.
pub fn files(run_id: i64, out: &RunOutput) -> Vec<(ArtifactMeta, Vec<u8>)> {
    let mut files: Vec<(ArtifactMeta, Vec<u8>)> = run_files(run_id, out).into_iter().map(|(name, bytes)| (meta(&name, &bytes), bytes)).collect();
    let manifest = pretty(&json!({
        "run_id": run_id,
        "world_id": out.result.world_id,
        "seed": out.result.seed,
        "iterations": out.result.iterations,
        "policy": out.result.policy,
        "schedule_mode": out.result.schedule_mode,
        "artifacts": files.iter().map(|(meta, _)| meta).collect::<Vec<_>>(),
    }));
    files.push((meta("manifest.json", &manifest), manifest));
    files
}

fn pretty(value: &impl Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialise artifact");
    bytes.push(b'\n');
    bytes
}

/// The files of a run, apart from the manifest: `(name, contents)`.
fn run_files(run_id: i64, out: &RunOutput) -> Vec<(String, Vec<u8>)> {
    let r = &out.result;
    let mut files = vec![
        ("run_summary.json".to_string(), pretty(&json!({ "run_id": run_id, "summary": r }))),
        (
            "pairwise_matrix.json".to_string(),
            pretty(&json!({
                "run_id": run_id,
                "world_id": r.world_id,
                "seed": r.seed,
                "iterations": r.iterations,
                "actors": r.actors.iter().map(|a| json!({ "id": a.id, "name": a.name })).collect::<Vec<_>>(),
                // average_payoff[row][col]: the row actor's average per round against the column actor.
                "average_payoff": r.pairwise,
                "pairs": pair_metrics(r),
            })),
        ),
    ];
    for trace in &out.traces {
        let body = json!({
            "run_id": run_id,
            "a": trace.a,
            "b": trace.b,
            "decisions": r.decisions,
            // Per round: [a's decision, b's decision] as indices into `decisions`.
            "moves": trace.moves,
        });
        // Compact: traces are the bulk of a run's bytes.
        let mut bytes = serde_json::to_vec(&body).expect("serialise trace");
        bytes.push(b'\n');
        files.push((format!("traces/{}__{}.json", trace.a, trace.b), bytes));
    }
    files
}

/// Per-pairing metrics: average payoffs, outcome mix, first defection round, punish duration.
pub fn pair_metrics(r: &RunResult) -> Vec<Value> {
    // "Defection" only means something in worlds that have a decision called D.
    let has_defect = r.decisions.iter().any(|d| d == "D");
    r.matches
        .iter()
        .map(|m| {
            let (first_defection, punish) = if has_defect {
                let first = m.first_played.get("D").and_then(|sides| sides.iter().flatten().min().copied());
                let rounds = m.outcome_counts.get("D/D").copied().unwrap_or(0);
                let streaks = m.outcome_runs.get("D/D").copied().unwrap_or(0);
                (json!(first), if streaks == 0 { Value::Null } else { json!(rounds as f64 / streaks as f64) })
            } else {
                (Value::Null, Value::Null)
            };
            json!({
                "a": m.a,
                "b": m.b,
                "average_payoff": m.averages,
                "outcome_counts": m.outcome_counts,
                // The first round (from 0) in which either actor defected; null if neither ever did.
                "first_defection_round": first_defection,
                // Average length of an unbroken stretch of mutual defection; null if there was none.
                "punish_duration_average": punish,
            })
        })
        .collect()
}
