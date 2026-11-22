//! World model (schema v1, the subset the engine executes today) and validation.

use crate::kpi::{self, Kpis};
use crate::Issue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_DECISIONS: usize = 16;
pub const MAX_ITERATIONS: u32 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct World {
    pub id: String,
    pub metadata: Metadata,
    pub decisions: Decisions,
    /// Leave empty to derive every `a/b` combination of the decisions.
    #[serde(default)]
    pub outcomes: Outcomes,
    pub interaction: Interaction,
    /// Only consulted when the interaction policy is `random`.
    #[serde(default)]
    pub randomness: Randomness,
    pub time: Time,
    pub determinism: Determinism,
    pub payoff: Payoff,
    pub scoring: Scoring,
    /// Saved indicators, evaluated after every run.
    #[serde(default)]
    pub kpis: Kpis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The longer explanation shown when the world is folded out: paragraphs separated by a blank line.
    #[serde(default)]
    pub guide: String,
    pub version: String,
    #[serde(default)]
    pub visibility: Visibility,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Public,
    Private,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decisions {
    pub allowed: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcomes {
    #[serde(default)]
    pub allowed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interaction {
    pub policy: Policy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// Every pair, in actor-id order.
    AllPairs,
    RoundRobin,
    /// Every pair, in an order drawn from the run seed (see `Randomness`).
    Random,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Randomness {
    #[serde(default)]
    pub schedule_mode: ScheduleMode,
}

/// How a `random` schedule uses the seed. Either way every pair plays every
/// round exactly once, and the same seed gives the same order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleMode {
    /// Shuffle the list of pairs once; each pair then plays all its rounds.
    #[default]
    ShuffleAll,
    /// Round by round: every pair plays one round, in a fresh shuffled order each round.
    SamplePerRound,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Time {
    /// Rounds played by each pairing.
    pub iterations: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Determinism {
    /// Default seed for runs of this world.
    pub rng_seed: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payoff {
    #[serde(rename = "type")]
    pub kind: PayoffType,
    /// `matrix[a][b] = [payoff to the first actor, payoff to the second]`
    /// when the first plays `a` and the second plays `b`. Whole numbers only,
    /// so totals are exact.
    pub matrix: BTreeMap<String, BTreeMap<String, [i64; 2]>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoffType {
    Matrix,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scoring {
    pub aggregate: Aggregate,
    #[serde(default)]
    pub ranking: Ranking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    /// Rank by total payoff.
    Sum,
    /// Rank by average payoff per round.
    Avg,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ranking {
    #[default]
    Descending,
}

impl World {
    /// Every `a/b` combination of the allowed decisions, in declaration order.
    pub fn derived_outcomes(&self) -> Vec<String> {
        let d = &self.decisions.allowed;
        d.iter().flat_map(|a| d.iter().map(move |b| format!("{a}/{b}"))).collect()
    }

    /// The outcome list in effect: the explicit one, or the derived one.
    pub fn outcomes(&self) -> Vec<String> {
        if self.outcomes.allowed.is_empty() {
            self.derived_outcomes()
        } else {
            self.outcomes.allowed.clone()
        }
    }

    /// Checks the world and returns every problem found (empty means valid).
    pub fn validate(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        let mut add = |path: &str, message: String| issues.push(Issue::new(path, message));

        if !is_slug(&self.id) {
            add("id", "must be 1–64 characters: lower-case letters, digits, '-' or '_'".into());
        }
        if self.metadata.name.trim().is_empty() {
            add("metadata.name", "must not be empty".into());
        }
        if self.metadata.version.trim().is_empty() {
            add("metadata.version", "must not be empty".into());
        }

        let decisions = &self.decisions.allowed;
        if decisions.len() < 2 {
            add("decisions.allowed", "needs at least two decisions".into());
        }
        if decisions.len() > MAX_DECISIONS {
            add("decisions.allowed", format!("has {} decisions; the limit is {MAX_DECISIONS}", decisions.len()));
        }
        for (i, d) in decisions.iter().enumerate() {
            let path = format!("decisions.allowed[{i}]");
            if d.trim().is_empty() {
                add(&path, "must not be empty".into());
            } else if d.contains('/') {
                add(&path, format!("\"{d}\" must not contain '/', which separates the two decisions in an outcome"));
            }
            if decisions[..i].contains(d) {
                add(&path, format!("\"{d}\" is listed more than once"));
            }
        }

        if !self.outcomes.allowed.is_empty() {
            let derived = self.derived_outcomes();
            for (i, o) in self.outcomes.allowed.iter().enumerate() {
                let path = format!("outcomes.allowed[{i}]");
                if !derived.contains(o) {
                    add(&path, format!("\"{o}\" is not a pair of allowed decisions written as \"a/b\""));
                }
                if self.outcomes.allowed[..i].contains(o) {
                    add(&path, format!("\"{o}\" is listed more than once"));
                }
            }
            for o in &derived {
                if !self.outcomes.allowed.contains(o) {
                    add("outcomes.allowed", format!("is missing \"{o}\", which the decisions can produce"));
                }
            }
        }

        if self.interaction.policy == Policy::RoundRobin {
            add("interaction.policy", "\"round_robin\" is not supported yet; use \"all_pairs\" or \"random\"".into());
        }

        if self.time.iterations == 0 {
            add("time.iterations", "must be a positive whole number".into());
        } else if self.time.iterations > MAX_ITERATIONS {
            add("time.iterations", format!("must be at most {MAX_ITERATIONS}"));
        }

        for a in decisions {
            match self.payoff.matrix.get(a) {
                None => add("payoff.matrix", format!("is missing the row for decision \"{a}\"")),
                Some(row) => {
                    for b in decisions {
                        if !row.contains_key(b) {
                            add(&format!("payoff.matrix.{a}"), format!("is missing the cell for \"{a}/{b}\""));
                        }
                    }
                }
            }
        }
        for (a, row) in &self.payoff.matrix {
            if !decisions.contains(a) {
                add(&format!("payoff.matrix.{a}"), format!("\"{a}\" is not an allowed decision"));
                continue;
            }
            for b in row.keys() {
                if !decisions.contains(b) {
                    add(&format!("payoff.matrix.{a}.{b}"), format!("\"{b}\" is not an allowed decision"));
                }
            }
        }

        issues.extend(kpi::validate(&self.kpis, decisions));
        issues
    }
}

/// Ids are used in URLs and as sort keys, so keep them plain.
pub fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Built-in template: the repeated Prisoner's Dilemma.
pub fn prisoners_dilemma() -> World {
    let row = |cells: [(&str, [i64; 2]); 2]| cells.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    World {
        id: "prisoners-dilemma".into(),
        metadata: Metadata {
            name: "Repeated Prisoner's Dilemma".into(),
            description: "Two actors each choose to cooperate (C) or defect (D), round after round. \
                          Mutual cooperation pays 3 each, mutual defection 1 each; defecting against \
                          a cooperator pays 5 and leaves the cooperator with 0."
                .into(),
            guide: "The story: two partners in crime are questioned in separate rooms. Each can keep quiet,                     which is cooperating with the other, or talk, which is defecting. Neither knows what the                     other will choose.\n\n\
                    The catch: in a single round, defecting always pays you more, whatever the other does.                     Against a cooperator you get 5 instead of 3, and against a defector 1 instead of 0. But                     if both think that way, both get 1, when both could have had 3.\n\n\
                    Repeating changes the game: here the same two meet round after round and remember what                     happened. A defection can be answered in the next round, so trust can be built, broken                     and repaired. How to do that is what a strategy decides.\n\n\
                    How a run is scored: every strategy plays one match against every other strategy. The                     points from all its matches add up, and the highest total wins. Winning single matches                     is not the goal; the total is."
                .into(),
            version: "1.0.0".into(),
            visibility: Visibility::Public,
        },
        decisions: Decisions { allowed: vec!["C".into(), "D".into()] },
        outcomes: Outcomes { allowed: vec!["C/C".into(), "C/D".into(), "D/C".into(), "D/D".into()] },
        interaction: Interaction { policy: Policy::AllPairs },
        randomness: Randomness::default(),
        time: Time { iterations: 1000 },
        determinism: Determinism { rng_seed: 42 },
        payoff: Payoff {
            kind: PayoffType::Matrix,
            matrix: [
                ("C".to_string(), row([("C", [3, 3]), ("D", [0, 5])])),
                ("D".to_string(), row([("C", [5, 0]), ("D", [1, 1])])),
            ]
            .into_iter()
            .collect(),
        },
        scoring: Scoring { aggregate: Aggregate::Sum, ranking: Ranking::Descending },
        kpis: kpi::prisoners_dilemma_kpis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(issues: &[Issue]) -> Vec<&str> {
        issues.iter().map(|i| i.path.as_str()).collect()
    }

    #[test]
    fn template_is_valid_and_round_trips_through_json() {
        let world = prisoners_dilemma();
        assert_eq!(world.validate(), vec![]);
        let json = serde_json::to_string(&world).unwrap();
        assert_eq!(serde_json::from_str::<World>(&json).unwrap(), world);
    }

    #[test]
    fn outcomes_are_derived_when_omitted() {
        let mut world = prisoners_dilemma();
        world.outcomes.allowed.clear();
        assert_eq!(world.validate(), vec![]);
        assert_eq!(world.outcomes(), ["C/C", "C/D", "D/C", "D/D"]);
    }

    #[test]
    fn reports_every_problem_with_a_path() {
        let mut world = prisoners_dilemma();
        world.id = "Not A Slug".into();
        world.metadata.name = " ".into();
        world.decisions.allowed.push("C".into());
        world.time.iterations = 0;
        world.interaction.policy = Policy::RoundRobin;
        let issues = world.validate();
        for expected in ["id", "metadata.name", "decisions.allowed[2]", "time.iterations", "interaction.policy"] {
            assert!(paths(&issues).contains(&expected), "missing {expected} in {issues:?}");
        }
    }

    #[test]
    fn payoff_matrix_must_be_complete_and_known() {
        let mut world = prisoners_dilemma();
        world.payoff.matrix.get_mut("C").unwrap().remove("D");
        world.payoff.matrix.insert("X".into(), BTreeMap::new());
        let issues = world.validate();
        assert!(issues.iter().any(|i| i.message.contains("missing the cell for \"C/D\"")), "{issues:?}");
        assert!(issues.iter().any(|i| i.path == "payoff.matrix.X"), "{issues:?}");
    }

    #[test]
    fn explicit_outcomes_must_match_the_decisions() {
        let mut world = prisoners_dilemma();
        world.outcomes.allowed = vec!["C/C".into(), "C/X".into()];
        let issues = world.validate();
        assert!(issues.iter().any(|i| i.path == "outcomes.allowed[1]"), "{issues:?}");
        assert!(issues.iter().any(|i| i.message.contains("missing \"D/D\"")), "{issues:?}");
    }

    #[test]
    fn unknown_enum_values_fail_to_parse_with_a_readable_message() {
        let mut json = serde_json::to_value(prisoners_dilemma()).unwrap();
        json["interaction"]["policy"] = "swiss".into();
        let err = serde_json::from_value::<World>(json).unwrap_err().to_string();
        assert!(err.contains("swiss") && err.contains("all_pairs"), "{err}");
    }
}
