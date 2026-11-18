//! Lilanaya simulation engine: pure, deterministic, no I/O.
//!
//! A run is fully determined by its world, its actors and its seed. Nothing in
//! this crate reads the clock, uses unseeded randomness or depends on hash-map
//! iteration order (maps are `BTreeMap`, schedules are sorted by actor id).

pub mod kpi;
pub mod rng;
pub mod schedule;
pub mod sim;
pub mod strategy;
pub mod world;

pub use sim::{run, run_entrants, Actor, ActorResult, Decider, Entrant, Logic, MatchResult, MatchTrace, RunOutput, RunResult, View};
pub use strategy::{baseline_strategies, Guard, Strategy, Transition};
pub use kpi::{KpiDefinition, KpiValue};
pub use world::{prisoners_dilemma, Policy, ScheduleMode, World};

use serde::{Deserialize, Serialize};

/// One validation problem: where it is (a dotted path into the document) and
/// what is wrong, in words a world or strategy author can act on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl Issue {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Issue { path: path.into(), message: message.into() }
    }
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}
