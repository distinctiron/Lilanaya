//! Golden run: a fixed world, the baseline actors and a fixed seed must
//! serialise to exactly the bytes stored in `tests/golden/`. Any drift in the
//! engine's behaviour or output format fails this test.
//!
//! If a change to the output is intended, regenerate the files with
//! `UPDATE_GOLDEN=1 cargo test -p lilanaya-engine --test golden`, review the
//! diff, and commit it.

use lilanaya_engine::{baseline_strategies, prisoners_dilemma, run, Actor, Policy, ScheduleMode, Strategy, World};
use std::path::PathBuf;

fn baselines() -> Vec<Actor> {
    baseline_strategies()
        .into_iter()
        .map(|s| Actor { id: s.id.clone(), name: s.name.clone(), strategy: s })
        .collect()
}

fn golden_run() -> (String, String) {
    let mut world = prisoners_dilemma();
    world.time.iterations = 200;
    written(&world, &baselines(), 42)
}

/// The summary and the traces as the golden files hold them.
fn written(world: &World, actors: &[Actor], seed: u64) -> (String, String) {
    let out = run(world, actors, seed).expect("golden inputs are valid");
    let summary = serde_json::to_string_pretty(&out.result).unwrap() + "\n";
    let traces = serde_json::to_string(&out.traces).unwrap() + "\n";
    (summary, traces)
}

fn check(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e} (run with UPDATE_GOLDEN=1 to create it)", path.display()));
    assert!(expected == actual, "{name} differs from the golden file; see the header of tests/golden.rs");
}

#[test]
fn baseline_run_is_byte_identical_to_the_golden_files() {
    let (summary, traces) = golden_run();
    check("pd_baselines_seed42.summary.json", &summary);
    check("pd_baselines_seed42.traces.json", &traces);
}

#[test]
fn repeated_runs_are_byte_identical() {
    let first = golden_run();
    for _ in 0..5 {
        assert!(golden_run() == first);
    }
}

/// A strategy as a player would write it, using every kind of condition:
/// cooperates, answers two defections in a row with defection until the other
/// side cooperates twice running, and defects through the last three rounds.
fn patient_grudger() -> Strategy {
    serde_json::from_value(serde_json::json!({
        "id": "patient-grudger",
        "name": "Patient Grudger",
        "description": "Forgives one defection, punishes two, and closes with three defections.",
        "states": ["FRIENDLY", "PUNISHING", "ENDGAME"],
        "initial_state": "FRIENDLY",
        "action_map": { "FRIENDLY": "C", "PUNISHING": "D", "ENDGAME": "D" },
        "transitions": [
            { "from": "FRIENDLY", "to": "ENDGAME", "guard": { "op": "rounds_left", "at_most": 3 } },
            { "from": "PUNISHING", "to": "ENDGAME", "guard": { "op": "rounds_left", "at_most": 3 } },
            { "from": "FRIENDLY", "to": "PUNISHING", "guard": { "op": "opponent_streak", "decision": "D", "at_least": 2 } },
            { "from": "PUNISHING", "to": "FRIENDLY", "guard": { "op": "and", "all": [
                { "op": "round_index", "at_least": 4 },
                { "op": "or", "any": [
                    { "op": "opponent_streak", "decision": "C", "at_least": 2 },
                    { "op": "outcome_streak", "outcome": "D/C", "at_least": 2 }
                ] },
                { "op": "not", "guard": { "op": "last_opponent_action", "is": "D" } },
                { "op": "last_own_action", "is": "D" }
            ] } }
        ]
    }))
    .unwrap()
}

#[test]
fn a_shuffled_schedule_is_byte_identical_to_the_golden_files() {
    let mut world = prisoners_dilemma();
    world.time.iterations = 50;
    world.interaction.policy = Policy::Random;
    world.randomness.schedule_mode = ScheduleMode::ShuffleAll;
    let (summary, traces) = written(&world, &baselines(), 7);
    check("pd_shuffle_all_seed7.summary.json", &summary);
    check("pd_shuffle_all_seed7.traces.json", &traces);
}

#[test]
fn a_schedule_drawn_per_round_is_byte_identical_to_the_golden_files() {
    let mut world = prisoners_dilemma();
    world.time.iterations = 50;
    world.interaction.policy = Policy::Random;
    world.randomness.schedule_mode = ScheduleMode::SamplePerRound;
    let (summary, traces) = written(&world, &baselines(), 7);
    check("pd_sample_per_round_seed7.summary.json", &summary);
    check("pd_sample_per_round_seed7.traces.json", &traces);
}

#[test]
fn a_run_with_a_player_s_own_rules_is_byte_identical_to_the_golden_files() {
    let mut world = prisoners_dilemma();
    world.time.iterations = 60;
    let mut actors = baselines();
    let own = patient_grudger();
    actors.push(Actor { id: own.id.clone(), name: own.name.clone(), strategy: own });
    let (summary, traces) = written(&world, &actors, 2026);
    check("pd_own_rules_seed2026.summary.json", &summary);
    check("pd_own_rules_seed2026.traces.json", &traces);
}
