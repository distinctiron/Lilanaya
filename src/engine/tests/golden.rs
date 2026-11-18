//! Golden run: a fixed world, the baseline actors and a fixed seed must
//! serialise to exactly the bytes stored in `tests/golden/`. Any drift in the
//! engine's behaviour or output format fails this test.
//!
//! If a change to the output is intended, regenerate the files with
//! `UPDATE_GOLDEN=1 cargo test -p lilanaya-engine --test golden`, review the
//! diff, and commit it.

use lilanaya_engine::{baseline_strategies, prisoners_dilemma, run, Actor};
use std::path::PathBuf;

fn golden_run() -> (String, String) {
    let mut world = prisoners_dilemma();
    world.time.iterations = 200;
    let actors: Vec<Actor> = baseline_strategies()
        .into_iter()
        .map(|s| Actor { id: s.id.clone(), name: s.name.clone(), strategy: s })
        .collect();
    let out = run(&world, &actors, 42).expect("golden inputs are valid");
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
