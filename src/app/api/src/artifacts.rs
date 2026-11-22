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

#[cfg(test)]
mod tests {
    use super::*;
    use lilanaya_engine::{baseline_strategies, prisoners_dilemma, run, Actor, World};

    fn played(world: &World, seed: u64) -> RunOutput {
        let actors: Vec<Actor> = baseline_strategies().into_iter().map(|s| Actor { id: s.id.clone(), name: s.name.clone(), strategy: s }).collect();
        run(world, &actors, seed).unwrap()
    }

    fn short_world() -> World {
        let mut world = prisoners_dilemma();
        world.time.iterations = 30;
        world
    }

    fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn a_run_has_a_summary_a_matrix_a_trace_per_pairing_and_a_manifest_last() {
        let out = played(&short_world(), 42);
        let files = files(7, &out);
        let names: Vec<&str> = files.iter().map(|(meta, _)| meta.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "run_summary.json",
                "pairwise_matrix.json",
                "traces/always_cooperate__always_defect.json",
                "traces/always_cooperate__tit_for_tat.json",
                "traces/always_cooperate__win_stay_lose_shift.json",
                "traces/always_defect__tit_for_tat.json",
                "traces/always_defect__win_stay_lose_shift.json",
                "traces/tit_for_tat__win_stay_lose_shift.json",
                "manifest.json",
            ]
        );
        for (meta, bytes) in &files {
            assert_eq!(meta.content_type, "application/json", "{}", meta.name);
            assert!(bytes.ends_with(b"\n"), "{} ends with a new line", meta.name);
            serde_json::from_slice::<Value>(bytes).unwrap_or_else(|e| panic!("{} is not JSON: {e}", meta.name));
        }
    }

    #[test]
    fn the_manifest_says_of_every_other_file_how_large_it_is_and_what_its_checksum_is() {
        let out = played(&short_world(), 42);
        let files = files(7, &out);
        let (manifest_meta, manifest_bytes) = files.last().unwrap();
        // What is said about each file is true of its bytes, the manifest included.
        for (meta, bytes) in &files {
            assert_eq!((meta.size_bytes, meta.sha256.as_str()), (bytes.len() as i64, sha256(bytes).as_str()), "{}", meta.name);
        }
        let manifest: Value = serde_json::from_slice(manifest_bytes).unwrap();
        assert_eq!((manifest["run_id"].as_i64(), manifest["world_id"].as_str(), manifest["seed"].as_u64(), manifest["iterations"].as_u64()), (Some(7), Some("prisoners-dilemma"), Some(42), Some(30)));
        assert_eq!(manifest["policy"], "all_pairs");
        let listed = manifest["artifacts"].as_array().unwrap();
        assert_eq!(listed.len(), files.len() - 1, "every file but the manifest itself");
        for (entry, (meta, _)) in listed.iter().zip(&files) {
            assert_eq!(entry, &json!({ "name": meta.name, "content_type": meta.content_type, "size_bytes": meta.size_bytes, "sha256": meta.sha256 }));
        }
        assert_eq!(manifest_meta.name, "manifest.json");
    }

    #[test]
    fn the_same_run_always_gives_the_same_bytes_and_another_run_does_not() {
        let out = played(&short_world(), 42);
        let bytes = |run_id: i64, out: &RunOutput| files(run_id, out).into_iter().map(|(_, bytes)| bytes).collect::<Vec<_>>();
        assert!(bytes(7, &out) == bytes(7, &out));
        assert!(bytes(7, &out) == bytes(7, &played(&short_world(), 42)), "played again from the same inputs");
        assert!(bytes(7, &out) != bytes(8, &out), "the files carry the run's number");
        assert!(bytes(7, &out) != bytes(7, &played(&short_world(), 43)), "and its seed");
    }

    #[test]
    fn a_trace_is_the_rounds_of_one_pairing_and_the_files_agree_with_the_summary() {
        let out = played(&short_world(), 42);
        let files = files(7, &out);
        let read = |name: &str| -> Value { serde_json::from_slice(&files.iter().find(|(meta, _)| meta.name == name).unwrap().1).unwrap() };

        let trace = read("traces/always_defect__tit_for_tat.json");
        assert_eq!((trace["run_id"].as_i64(), trace["a"].as_str(), trace["b"].as_str()), (Some(7), Some("always_defect"), Some("tit_for_tat")));
        assert_eq!(trace["decisions"], json!(["C", "D"]));
        let moves = trace["moves"].as_array().unwrap();
        assert_eq!(moves.len(), 30);
        // Always Defect defects; Tit for Tat cooperates once and then copies.
        assert_eq!((&moves[0], &moves[1], &moves[29]), (&json!([1, 0]), &json!([1, 1]), &json!([1, 1])));

        let summary = read("run_summary.json");
        assert_eq!(summary["run_id"], 7);
        assert_eq!(summary["summary"], json!(out.result), "the summary is the engine's result, unchanged");

        let matrix = read("pairwise_matrix.json");
        assert_eq!(matrix["average_payoff"], json!(out.result.pairwise));
        assert_eq!(matrix["actors"].as_array().unwrap().len(), 4);
        assert_eq!(matrix["pairs"], json!(pair_metrics(&out.result)));
    }

    #[test]
    fn pair_metrics_say_when_the_first_defection_came_and_how_long_punishment_lasts() {
        let out = played(&short_world(), 42);
        let pairs = pair_metrics(&out.result);
        let pair = |a: &str, b: &str| pairs.iter().find(|p| p["a"] == a && p["b"] == b).unwrap_or_else(|| panic!("{a} v {b}"));

        // Nobody defects: no first defection, no punishment.
        let friends = pair("always_cooperate", "tit_for_tat");
        assert_eq!((&friends["first_defection_round"], &friends["punish_duration_average"]), (&Value::Null, &Value::Null));
        assert_eq!(friends["average_payoff"], json!([3.0, 3.0]));
        assert_eq!(friends["outcome_counts"], json!({ "C/C": 30, "C/D": 0, "D/C": 0, "D/D": 0 }));

        // Defection from the first round; from the second, one unbroken stretch of mutual defection.
        let feud = pair("always_defect", "tit_for_tat");
        assert_eq!(feud["first_defection_round"], 0);
        assert_eq!(feud["punish_duration_average"], 29.0);

        // Against Win-Stay Lose-Shift, mutual defection comes every other round, one round at a time.
        let flip = pair("always_defect", "win_stay_lose_shift");
        assert_eq!(flip["outcome_counts"]["D/D"], 15);
        assert_eq!(flip["punish_duration_average"], 1.0);

        // A world without a decision called D has no "defection" to speak of.
        let mut other = out.result.clone();
        other.decisions = vec!["STAY".into(), "GO".into()];
        for p in pair_metrics(&other) {
            assert_eq!((&p["first_defection_round"], &p["punish_duration_average"]), (&Value::Null, &Value::Null));
        }
    }
}
