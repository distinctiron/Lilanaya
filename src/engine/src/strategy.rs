//! Strategies as finite state machines. The whole machine is data (states,
//! an action per state, guarded transitions), so it can be stored, shown and
//! audited; nothing here is a closure or user code.
//!
//! Each round, for each actor: the transitions out of the current state are
//! tried in order and the first whose guard holds is taken (otherwise the
//! state is kept); the actor then plays the action of the state it is in.

use crate::Issue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_STATES: usize = 64;
pub const MAX_TRANSITIONS: usize = 256;
const MAX_GUARD_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Strategy {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub states: Vec<String>,
    pub initial_state: String,
    /// State → the decision played while in that state.
    pub action_map: BTreeMap<String, String>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub from: String,
    pub to: String,
    pub guard: Guard,
}

/// A condition over what the actor has observed in the current pairing.
/// Guards about the previous round are false in the first round.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Guard {
    /// The opponent's previous decision was `is`.
    LastOpponentAction { is: String },
    /// This actor's own previous decision was `is`.
    LastOwnAction { is: String },
    /// The opponent has played `decision` in each of the last `at_least` rounds.
    OpponentStreak { decision: String, at_least: u32 },
    /// The last `at_least` rounds all ended in `outcome`, written `own/opponent`.
    OutcomeStreak { outcome: String, at_least: u32 },
    /// At least `at_least` rounds have been played (the first round is index 0).
    RoundIndex { at_least: u32 },
    /// At most `at_most` rounds are left in the match, counting the one about to be played.
    RoundsLeft { at_most: u32 },
    And { all: Vec<Guard> },
    Or { any: Vec<Guard> },
    Not { guard: Box<Guard> },
}

impl Strategy {
    /// Checks the strategy against a world's decisions; empty means valid.
    pub fn validate(&self, decisions: &[String]) -> Vec<Issue> {
        match self.compile(decisions) {
            Ok(_) => Vec::new(),
            Err(issues) => issues,
        }
    }

    /// Things that are allowed but probably not meant: states that can never
    /// be reached, and states that can never be left (in a machine with more
    /// than one state). Only meaningful for a strategy that validates.
    pub fn warnings(&self) -> Vec<Issue> {
        let mut warnings = Vec::new();
        let mut reached = vec![self.initial_state.as_str()];
        let mut next = 0;
        while next < reached.len() {
            let from = reached[next];
            next += 1;
            for t in self.transitions.iter().filter(|t| t.from == from) {
                if !reached.contains(&t.to.as_str()) {
                    reached.push(&t.to);
                }
            }
        }
        for (i, s) in self.states.iter().enumerate() {
            if !reached.contains(&s.as_str()) {
                warnings.push(Issue::new(format!("states[{i}]"), format!("\"{s}\" can never be reached from the start state")));
            } else if self.states.len() > 1 && !self.transitions.iter().any(|t| t.from == *s && t.to != *s) {
                warnings.push(Issue::new(format!("states[{i}]"), format!("once in \"{s}\", the strategy never leaves it")));
            }
        }
        warnings
    }

    /// Validates and lowers the strategy to index form for the inner loop.
    pub(crate) fn compile(&self, decisions: &[String]) -> Result<Compiled, Vec<Issue>> {
        let mut issues = Vec::new();

        if !crate::world::is_slug(&self.id) {
            issues.push(Issue::new("id", "must be 1–64 characters: lower-case letters, digits, '-' or '_'"));
        }
        if self.states.is_empty() {
            issues.push(Issue::new("states", "needs at least one state"));
        }
        if self.states.len() > MAX_STATES {
            issues.push(Issue::new("states", format!("has {} states; the limit is {MAX_STATES}", self.states.len())));
        }
        if self.transitions.len() > MAX_TRANSITIONS {
            issues.push(Issue::new(
                "transitions",
                format!("has {} transitions; the limit is {MAX_TRANSITIONS}", self.transitions.len()),
            ));
        }
        for (i, s) in self.states.iter().enumerate() {
            if self.states[..i].contains(s) {
                issues.push(Issue::new(format!("states[{i}]"), format!("\"{s}\" is listed more than once")));
            }
        }
        let state_index = |name: &str| self.states.iter().position(|s| s == name);
        let decision_index = |name: &str| decisions.iter().position(|d| d == name);

        let initial = state_index(&self.initial_state);
        if initial.is_none() {
            issues.push(Issue::new("initial_state", format!("\"{}\" is not one of the states", self.initial_state)));
        }

        let mut actions = Vec::with_capacity(self.states.len());
        for s in &self.states {
            match self.action_map.get(s) {
                None => issues.push(Issue::new("action_map", format!("has no action for state \"{s}\""))),
                Some(d) => match decision_index(d) {
                    Some(i) => actions.push(i as u8),
                    None => issues.push(Issue::new(
                        format!("action_map.{s}"),
                        format!("\"{d}\" is not a decision this world allows ({})", decisions.join(", ")),
                    )),
                },
            }
        }
        for s in self.action_map.keys() {
            if state_index(s).is_none() {
                issues.push(Issue::new(format!("action_map.{s}"), format!("\"{s}\" is not one of the states")));
            }
        }

        let mut transitions: Vec<Vec<(CGuard, usize)>> = vec![Vec::new(); self.states.len()];
        for (i, t) in self.transitions.iter().enumerate() {
            let from = state_index(&t.from);
            let to = state_index(&t.to);
            if from.is_none() {
                issues.push(Issue::new(format!("transitions[{i}].from"), format!("\"{}\" is not one of the states", t.from)));
            }
            if to.is_none() {
                issues.push(Issue::new(format!("transitions[{i}].to"), format!("\"{}\" is not one of the states", t.to)));
            }
            let guard = compile_guard(&t.guard, decisions, &format!("transitions[{i}].guard"), 0, &mut issues);
            if let (Some(from), Some(to), Some(guard)) = (from, to, guard) {
                transitions[from].push((guard, to));
            }
        }

        match initial {
            Some(initial) if issues.is_empty() => Ok(Compiled { initial, actions, transitions }),
            _ => Err(issues),
        }
    }
}

fn compile_guard(guard: &Guard, decisions: &[String], path: &str, depth: usize, issues: &mut Vec<Issue>) -> Option<CGuard> {
    if depth > MAX_GUARD_DEPTH {
        issues.push(Issue::new(path, format!("is nested more than {MAX_GUARD_DEPTH} levels deep")));
        return None;
    }
    let decision = |name: &str, field: &str, issues: &mut Vec<Issue>| {
        let found = decisions.iter().position(|d| d == name);
        if found.is_none() {
            issues.push(Issue::new(
                format!("{path}.{field}"),
                format!("\"{name}\" is not a decision this world allows ({})", decisions.join(", ")),
            ));
        }
        found.map(|i| i as u8)
    };
    let count = |n: u32, issues: &mut Vec<Issue>| {
        if n == 0 {
            issues.push(Issue::new(format!("{path}.at_least"), "must be 1 or more"));
            None
        } else {
            Some(n)
        }
    };
    match guard {
        Guard::LastOpponentAction { is } => decision(is, "is", issues).map(CGuard::LastOpponent),
        Guard::LastOwnAction { is } => decision(is, "is", issues).map(CGuard::LastOwn),
        Guard::OpponentStreak { decision: d, at_least } => {
            let d = decision(d, "decision", issues);
            let n = count(*at_least, issues);
            Some(CGuard::OpponentStreak(d?, n?))
        }
        Guard::OutcomeStreak { outcome, at_least } => {
            let n = count(*at_least, issues);
            let pair = match outcome.split_once('/') {
                Some((own, opponent)) => {
                    let own = decision(own, "outcome", issues);
                    let opponent = decision(opponent, "outcome", issues);
                    own.zip(opponent)
                }
                None => {
                    issues.push(Issue::new(
                        format!("{path}.outcome"),
                        format!("\"{outcome}\" must be written \"own/opponent\", for example \"{0}/{0}\"", decisions.first().map_or("C", |d| d.as_str())),
                    ));
                    None
                }
            };
            let (own, opponent) = pair?;
            Some(CGuard::OutcomeStreak(own, opponent, n?))
        }
        Guard::RoundIndex { at_least } => Some(CGuard::Round(*at_least)),
        Guard::RoundsLeft { at_most } => {
            if *at_most == 0 {
                issues.push(Issue::new(format!("{path}.at_most"), "must be 1 or more"));
                None
            } else {
                Some(CGuard::RoundsLeft(*at_most))
            }
        }
        Guard::And { all } => compile_list(all, decisions, &format!("{path}.all"), depth, issues).map(CGuard::And),
        Guard::Or { any } => compile_list(any, decisions, &format!("{path}.any"), depth, issues).map(CGuard::Or),
        Guard::Not { guard } => {
            compile_guard(guard, decisions, &format!("{path}.guard"), depth + 1, issues).map(|g| CGuard::Not(Box::new(g)))
        }
    }
}

fn compile_list(guards: &[Guard], decisions: &[String], path: &str, depth: usize, issues: &mut Vec<Issue>) -> Option<Vec<CGuard>> {
    if guards.is_empty() {
        issues.push(Issue::new(path, "needs at least one condition"));
        return None;
    }
    // Compile every item (no short-circuit) so all problems are reported at once.
    let compiled: Vec<Option<CGuard>> = guards
        .iter()
        .enumerate()
        .map(|(i, g)| compile_guard(g, decisions, &format!("{path}[{i}]"), depth + 1, issues))
        .collect();
    compiled.into_iter().collect()
}

/// Index form of a strategy: decisions and states are positions, not names.
#[derive(Clone, Debug)]
pub(crate) struct Compiled {
    pub initial: usize,
    /// State → decision index.
    pub actions: Vec<u8>,
    /// State → ordered `(guard, target state)` list.
    pub transitions: Vec<Vec<(CGuard, usize)>>,
}

#[derive(Clone, Debug)]
pub(crate) enum CGuard {
    LastOpponent(u8),
    LastOwn(u8),
    OpponentStreak(u8, u32),
    OutcomeStreak(u8, u8, u32),
    Round(u32),
    RoundsLeft(u32),
    And(Vec<CGuard>),
    Or(Vec<CGuard>),
    Not(Box<CGuard>),
}

/// What one actor knows going into a round of a pairing.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Observation {
    /// Index of the round about to be played (0 for the first).
    pub round: u32,
    /// How many rounds the pairing plays in all.
    pub rounds: u32,
    /// `(own, opponent)` decisions of the previous round.
    pub last: Option<(u8, u8)>,
    /// How many rounds in a row the opponent has played its last decision.
    pub opponent_streak: u32,
    /// How many rounds in a row have ended in the last outcome.
    pub outcome_streak: u32,
}

impl Observation {
    /// Before the first round of a pairing that plays `rounds` rounds.
    pub fn new(rounds: u32) -> Self {
        Observation { rounds, ..Observation::default() }
    }

    /// Folds a finished round into the observation.
    pub fn record(&mut self, own: u8, opponent: u8) {
        match self.last {
            Some((prev_own, prev_opponent)) => {
                self.opponent_streak = if prev_opponent == opponent { self.opponent_streak + 1 } else { 1 };
                self.outcome_streak =
                    if (prev_own, prev_opponent) == (own, opponent) { self.outcome_streak + 1 } else { 1 };
            }
            None => {
                self.opponent_streak = 1;
                self.outcome_streak = 1;
            }
        }
        self.last = Some((own, opponent));
        self.round += 1;
    }
}

impl CGuard {
    fn holds(&self, obs: &Observation) -> bool {
        match self {
            CGuard::LastOpponent(d) => obs.last.is_some_and(|(_, opponent)| opponent == *d),
            CGuard::LastOwn(d) => obs.last.is_some_and(|(own, _)| own == *d),
            CGuard::OpponentStreak(d, n) => {
                obs.last.is_some_and(|(_, opponent)| opponent == *d) && obs.opponent_streak >= *n
            }
            CGuard::OutcomeStreak(own, opponent, n) => obs.last == Some((*own, *opponent)) && obs.outcome_streak >= *n,
            CGuard::Round(n) => obs.round >= *n,
            CGuard::RoundsLeft(n) => obs.rounds.saturating_sub(obs.round) <= *n,
            CGuard::And(all) => all.iter().all(|g| g.holds(obs)),
            CGuard::Or(any) => any.iter().any(|g| g.holds(obs)),
            CGuard::Not(g) => !g.holds(obs),
        }
    }
}

impl Compiled {
    /// Advances from `state` given the observation; returns the new state and
    /// the decision to play.
    pub fn step(&self, state: usize, obs: &Observation) -> (usize, u8) {
        let next = self.transitions[state]
            .iter()
            .find(|(guard, _)| guard.holds(obs))
            .map_or(state, |&(_, to)| to);
        (next, self.actions[next])
    }
}

/// The built-in baseline strategies for two-decision worlds whose decisions
/// are named `C` (cooperate) and `D` (defect).
pub fn baseline_strategies() -> Vec<Strategy> {
    let actions = |pairs: &[(&str, &str)]| pairs.iter().map(|(s, d)| (s.to_string(), d.to_string())).collect();
    let on_opponent = |from: &str, to: &str, is: &str| Transition {
        from: from.into(),
        to: to.into(),
        guard: Guard::LastOpponentAction { is: is.into() },
    };
    vec![
        Strategy {
            id: "always_cooperate".into(),
            name: "Always Cooperate".into(),
            description: "Cooperates every round, whatever the opponent does.".into(),
            states: vec!["COOPERATE".into()],
            initial_state: "COOPERATE".into(),
            action_map: actions(&[("COOPERATE", "C")]),
            transitions: vec![],
        },
        Strategy {
            id: "always_defect".into(),
            name: "Always Defect".into(),
            description: "Defects every round, whatever the opponent does.".into(),
            states: vec!["DEFECT".into()],
            initial_state: "DEFECT".into(),
            action_map: actions(&[("DEFECT", "D")]),
            transitions: vec![],
        },
        Strategy {
            id: "tit_for_tat".into(),
            name: "Tit for Tat".into(),
            description: "Starts by cooperating, then copies the opponent's previous move.".into(),
            states: vec!["COOPERATE".into(), "DEFECT".into()],
            initial_state: "COOPERATE".into(),
            action_map: actions(&[("COOPERATE", "C"), ("DEFECT", "D")]),
            transitions: vec![on_opponent("COOPERATE", "DEFECT", "D"), on_opponent("DEFECT", "COOPERATE", "C")],
        },
        Strategy {
            id: "win_stay_lose_shift".into(),
            name: "Win-Stay Lose-Shift".into(),
            description: "Starts by cooperating. Repeats its move after a good result (the opponent \
                          cooperated) and switches after a bad one (the opponent defected)."
                .into(),
            states: vec!["COOPERATE".into(), "DEFECT".into()],
            initial_state: "COOPERATE".into(),
            action_map: actions(&[("COOPERATE", "C"), ("DEFECT", "D")]),
            transitions: vec![on_opponent("COOPERATE", "DEFECT", "D"), on_opponent("DEFECT", "COOPERATE", "D")],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cd() -> Vec<String> {
        vec!["C".into(), "D".into()]
    }

    /// Plays `strategy` against a fixed list of opponent moves; returns its moves as a string.
    fn play(strategy: &Strategy, opponent: &str) -> String {
        let decisions = cd();
        let compiled = strategy.compile(&decisions).unwrap();
        let mut state = compiled.initial;
        let mut obs = Observation::new(opponent.len() as u32);
        let mut out = String::new();
        for ch in opponent.chars() {
            let (next, own) = compiled.step(state, &obs);
            state = next;
            out.push_str(&decisions[own as usize]);
            obs.record(own, if ch == 'C' { 0 } else { 1 });
        }
        out
    }

    fn baseline(id: &str) -> Strategy {
        baseline_strategies().into_iter().find(|s| s.id == id).unwrap()
    }

    #[test]
    fn baselines_are_valid_and_serialisable() {
        for s in baseline_strategies() {
            assert_eq!(s.validate(&cd()), vec![], "{}", s.id);
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<Strategy>(&json).unwrap(), s);
        }
    }

    #[test]
    fn baselines_behave_as_named() {
        let opponent = "CDDCCDC";
        assert_eq!(play(&baseline("always_cooperate"), opponent), "CCCCCCC");
        assert_eq!(play(&baseline("always_defect"), opponent), "DDDDDDD");
        assert_eq!(play(&baseline("tit_for_tat"), opponent), "CCDDCCD");
        // WSLS: C; opp C → stay C; opp D → shift D; opp D → shift C; opp C → stay C; opp C → stay C; opp D → shift D.
        assert_eq!(play(&baseline("win_stay_lose_shift"), opponent), "CCDCCCD");
    }

    #[test]
    fn guard_json_shape_is_tagged_data() {
        let guard: Guard = serde_json::from_str(
            r#"{"op":"and","all":[{"op":"opponent_streak","decision":"C","at_least":3},{"op":"not","guard":{"op":"round_index","at_least":10}}]}"#,
        )
        .unwrap();
        assert!(matches!(guard, Guard::And { ref all } if all.len() == 2));
    }

    #[test]
    fn rounds_left_marks_the_end_of_the_match() {
        // Tit for Tat, except that the last 3 rounds are always D.
        let mut s = baseline("tit_for_tat");
        let (first, other) = (s.states[0].clone(), s.states[1].clone());
        s.states.push("END".into());
        s.action_map.insert("END".into(), "D".into());
        let end = |from: &String| Transition { from: from.clone(), to: "END".into(), guard: Guard::RoundsLeft { at_most: 3 } };
        s.transitions.splice(0..0, [end(&first), end(&other)]);
        assert_eq!(play(&s, "CCCCCCCC"), "CCCCCDDD");
        assert_eq!(play(&s, "CC"), "DD", "a match shorter than the end game is all end game");

        s.transitions[0].guard = Guard::RoundsLeft { at_most: 0 };
        assert_eq!(s.validate(&cd())[0].path, "transitions[0].guard.at_most");
        let json = serde_json::to_value(Guard::RoundsLeft { at_most: 3 }).unwrap();
        assert_eq!(json, serde_json::json!({ "op": "rounds_left", "at_most": 3 }));
    }

    #[test]
    fn forgiving_strategy_from_the_spec() {
        // Start C; after any D, defect until the opponent cooperates 3 times in a row.
        let s = Strategy {
            id: "forgive_after_3".into(),
            name: "Forgive after 3".into(),
            description: String::new(),
            states: vec!["COOP".into(), "PUNISH".into()],
            initial_state: "COOP".into(),
            action_map: [("COOP", "C"), ("PUNISH", "D")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            transitions: vec![
                Transition { from: "COOP".into(), to: "PUNISH".into(), guard: Guard::LastOpponentAction { is: "D".into() } },
                Transition {
                    from: "PUNISH".into(),
                    to: "COOP".into(),
                    guard: Guard::OpponentStreak { decision: "C".into(), at_least: 3 },
                },
            ],
        };
        assert_eq!(play(&s, "CDCCCCDC"), "CCDDDCCD");
    }

    #[test]
    fn outcome_streak_and_round_guards() {
        let mut s = baseline("always_cooperate");
        s.states.push("DEFECT".into());
        s.action_map.insert("DEFECT".into(), "D".into());
        s.transitions.push(Transition {
            from: "COOPERATE".into(),
            to: "DEFECT".into(),
            guard: Guard::Or {
                any: vec![
                    Guard::OutcomeStreak { outcome: "C/D".into(), at_least: 2 },
                    Guard::RoundIndex { at_least: 5 },
                ],
            },
        });
        assert_eq!(play(&s, "DCDDCC"), "CCCCDD");
        assert_eq!(play(&s, "CCCCCCC"), "CCCCCDD");
    }

    #[test]
    fn warnings_flag_unreachable_and_absorbing_states() {
        assert_eq!(baseline("tit_for_tat").warnings(), vec![]);
        assert_eq!(baseline("always_defect").warnings(), vec![]);
        let mut s = baseline("tit_for_tat");
        s.states.push("LOST".into());
        s.action_map.insert("LOST".into(), "D".into());
        s.transitions.pop(); // DEFECT no longer returns to COOPERATE
        let paths: Vec<String> = s.warnings().into_iter().map(|w| format!("{}: {}", w.path, w.message)).collect();
        assert_eq!(
            paths,
            ["states[1]: once in \"DEFECT\", the strategy never leaves it", "states[2]: \"LOST\" can never be reached from the start state"]
        );
    }

    #[test]
    fn validation_points_at_the_offending_rule() {
        let mut s = baseline("tit_for_tat");
        s.initial_state = "START".into();
        s.action_map.insert("DEFECT".into(), "X".into());
        s.transitions[0].to = "NOWHERE".into();
        s.transitions[1].guard = Guard::OpponentStreak { decision: "Z".into(), at_least: 0 };
        let issues = s.validate(&cd());
        let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
        for expected in [
            "initial_state",
            "action_map.DEFECT",
            "transitions[0].to",
            "transitions[1].guard.decision",
            "transitions[1].guard.at_least",
        ] {
            assert!(paths.contains(&expected), "missing {expected} in {issues:?}");
        }
    }
}
