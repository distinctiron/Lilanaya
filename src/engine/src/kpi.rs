//! Key performance indicators: saved queries over a run's rounds.
//!
//! A definition picks a scope (whole world, per actor, per pair), a filter
//! (an outcome or a decision), a window of rounds and an aggregation. This
//! module supports the combinations listed on [`KpiAggregation`]; validation
//! rejects the rest with a message rather than guessing.

use crate::Issue;
use serde::{Deserialize, Serialize};

pub const MAX_KPIS: usize = 32;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Kpis {
    #[serde(default)]
    pub definitions: Vec<KpiDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KpiDefinition {
    pub id: String,
    pub label: String,
    pub scope: KpiScope,
    pub query: KpiQuery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KpiScope {
    /// One value over all pairings.
    World,
    /// As `World`, plus a value per actor.
    Actor,
    /// As `World`, plus a value per pairing.
    Pair,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KpiQuery {
    pub filter: KpiFilter,
    #[serde(default)]
    pub window: KpiWindow,
    pub aggregation: KpiAggregation,
}

/// Exactly one of `outcome` and `decision` must be set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KpiFilter {
    /// `"x/y"`: rounds in which one actor played `x` and the other `y`, whichever way round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Plays of this decision (each round has two plays, one per actor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// Threshold (0 to 1) for `percentage_of_actors`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio_gt: Option<f64>,
}

/// Which rounds of each pairing count. Rounds are numbered from 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KpiWindow {
    #[default]
    AllRounds,
    FirstNRounds { n: u32 },
    LastNRounds { n: u32 },
    /// From `start` up to but not including `end`.
    Range { start: u32, end: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KpiAggregation {
    /// Number of matching rounds (outcome filter) or plays (decision filter).
    Count,
    /// Matching rounds as a percentage of rounds, or matching plays as a percentage of plays.
    PercentageOfRounds,
    /// Actor scope with a decision filter and `ratio_gt`: the percentage of
    /// actors for whom that decision is more than `ratio_gt` of their plays.
    PercentageOfActors,
}

/// The value of one indicator for one run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KpiValue {
    pub id: String,
    pub label: String,
    pub value: f64,
    /// Per actor (`actors` has one id) or per pair (two ids), for those scopes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breakdown: Vec<KpiPart>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KpiPart {
    pub actors: Vec<String>,
    pub value: f64,
}

pub(crate) fn validate(kpis: &Kpis, decisions: &[String]) -> Vec<Issue> {
    let mut issues = Vec::new();
    let defs = &kpis.definitions;
    if defs.len() > MAX_KPIS {
        issues.push(Issue::new("kpis.definitions", format!("has {} indicators; the limit is {MAX_KPIS}", defs.len())));
    }
    for (i, def) in defs.iter().enumerate() {
        let at = |field: &str| format!("kpis.definitions[{i}].{field}");
        if !crate::world::is_slug(&def.id) {
            issues.push(Issue::new(at("id"), "must be 1–64 characters: lower-case letters, digits, '-' or '_'"));
        }
        if defs[..i].iter().any(|other| other.id == def.id) {
            issues.push(Issue::new(at("id"), format!("\"{}\" is used by more than one indicator", def.id)));
        }
        if def.label.trim().is_empty() {
            issues.push(Issue::new(at("label"), "must not be empty"));
        }

        let filter = &def.query.filter;
        let known = |d: &str| decisions.iter().any(|x| x == d);
        match (&filter.outcome, &filter.decision) {
            (Some(_), Some(_)) | (None, None) => {
                issues.push(Issue::new(at("query.filter"), "set exactly one of \"outcome\" and \"decision\""));
            }
            (Some(outcome), None) => match outcome.split_once('/') {
                Some((x, y)) if known(x) && known(y) => {}
                _ => issues.push(Issue::new(
                    at("query.filter.outcome"),
                    format!("\"{outcome}\" is not a pair of allowed decisions written as \"a/b\""),
                )),
            },
            (None, Some(decision)) => {
                if !known(decision) {
                    issues.push(Issue::new(at("query.filter.decision"), format!("\"{decision}\" is not an allowed decision")));
                }
            }
        }

        match def.query.aggregation {
            KpiAggregation::PercentageOfActors => {
                if def.scope != KpiScope::Actor || filter.decision.is_none() {
                    issues.push(Issue::new(
                        at("query.aggregation"),
                        "\"percentage_of_actors\" needs scope \"actor\" and a \"decision\" filter",
                    ));
                }
                match filter.ratio_gt {
                    Some(r) if (0.0..=1.0).contains(&r) => {}
                    Some(_) => issues.push(Issue::new(at("query.filter.ratio_gt"), "must be between 0 and 1")),
                    None => issues.push(Issue::new(at("query.filter.ratio_gt"), "is required for \"percentage_of_actors\"")),
                }
            }
            KpiAggregation::Count | KpiAggregation::PercentageOfRounds => {
                if filter.ratio_gt.is_some() {
                    issues.push(Issue::new(at("query.filter.ratio_gt"), "is only used with \"percentage_of_actors\""));
                }
                if def.scope == KpiScope::Actor && filter.decision.is_none() {
                    issues.push(Issue::new(at("query.filter"), "scope \"actor\" needs a \"decision\" filter"));
                }
            }
        }

        match def.query.window {
            KpiWindow::AllRounds => {}
            KpiWindow::FirstNRounds { n } | KpiWindow::LastNRounds { n } => {
                if n == 0 {
                    issues.push(Issue::new(at("query.window.n"), "must be 1 or more"));
                }
            }
            KpiWindow::Range { start, end } => {
                if start >= end {
                    issues.push(Issue::new(at("query.window"), "\"start\" must be less than \"end\""));
                }
            }
        }
    }
    issues
}

/// One pairing's rounds, as the engine recorded them.
pub(crate) struct Pairing<'a> {
    /// Indices into the actor list.
    pub a: usize,
    pub b: usize,
    pub moves: &'a [[u8; 2]],
}

/// Evaluates every (already validated) definition over a finished run.
pub(crate) fn evaluate(kpis: &Kpis, decisions: &[String], actor_ids: &[&str], pairings: &[Pairing]) -> Vec<KpiValue> {
    let d = decisions.len();
    let index = |name: &str| decisions.iter().position(|x| x == name).expect("validated decision");
    let percent = |part: u64, whole: u64| if whole == 0 { 0.0 } else { part as f64 * 100.0 / whole as f64 };

    kpis.definitions
        .iter()
        .map(|def| {
            // Per pairing, within the window: counts[x * d + y] = rounds where a played x and b played y.
            let windows: Vec<(Vec<u64>, u64)> = pairings
                .iter()
                .map(|p| {
                    let total = p.moves.len();
                    let (lo, hi) = match def.query.window {
                        KpiWindow::AllRounds => (0, total),
                        KpiWindow::FirstNRounds { n } => (0, total.min(n as usize)),
                        KpiWindow::LastNRounds { n } => (total - total.min(n as usize), total),
                        KpiWindow::Range { start, end } => (total.min(start as usize), total.min(end as usize)),
                    };
                    let mut counts = vec![0u64; d * d];
                    for m in &p.moves[lo..hi] {
                        counts[m[0] as usize * d + m[1] as usize] += 1;
                    }
                    (counts, (hi - lo) as u64)
                })
                .collect();

            let filter = &def.query.filter;
            // Per pairing: (matches, out of) for the pair, and (plays by a, plays by b) for a decision filter.
            let per_pair: Vec<(u64, u64, [u64; 2])> = windows
                .iter()
                .map(|(counts, rounds)| match (&filter.outcome, &filter.decision) {
                    (Some(outcome), _) => {
                        let (x, y) = outcome.split_once('/').expect("validated outcome");
                        let (x, y) = (index(x), index(y));
                        let hits = counts[x * d + y] + if x == y { 0 } else { counts[y * d + x] };
                        (hits, *rounds, [0, 0])
                    }
                    (None, Some(decision)) => {
                        let x = index(decision);
                        let by_a: u64 = (0..d).map(|y| counts[x * d + y]).sum();
                        let by_b: u64 = (0..d).map(|y| counts[y * d + x]).sum();
                        (by_a + by_b, rounds * 2, [by_a, by_b])
                    }
                    (None, None) => unreachable!("validated filter"),
                })
                .collect();

            let hits: u64 = per_pair.iter().map(|p| p.0).sum();
            let out_of: u64 = per_pair.iter().map(|p| p.1).sum();

            // Per actor: (plays of the decision, all plays), in actor order.
            let per_actor = || -> Vec<(u64, u64)> {
                let mut totals = vec![(0u64, 0u64); actor_ids.len()];
                for (p, (_, rounds, by)) in pairings.iter().zip(&per_pair) {
                    let window_rounds = rounds / 2;
                    totals[p.a].0 += by[0];
                    totals[p.a].1 += window_rounds;
                    totals[p.b].0 += by[1];
                    totals[p.b].1 += window_rounds;
                }
                totals
            };

            let aggregate = |part: u64, whole: u64| match def.query.aggregation {
                KpiAggregation::Count => part as f64,
                _ => percent(part, whole),
            };

            let (value, breakdown) = match (def.query.aggregation, def.scope) {
                (KpiAggregation::PercentageOfActors, _) => {
                    let ratio = filter.ratio_gt.expect("validated ratio");
                    let actors = per_actor();
                    let over = actors.iter().filter(|(part, whole)| *whole > 0 && *part as f64 / *whole as f64 > ratio).count();
                    (percent(over as u64, actors.len() as u64), Vec::new())
                }
                (_, KpiScope::World) => (aggregate(hits, out_of), Vec::new()),
                (_, KpiScope::Pair) => {
                    let parts = pairings
                        .iter()
                        .zip(&per_pair)
                        .map(|(p, (part, whole, _))| KpiPart {
                            actors: vec![actor_ids[p.a].to_string(), actor_ids[p.b].to_string()],
                            value: aggregate(*part, *whole),
                        })
                        .collect();
                    (aggregate(hits, out_of), parts)
                }
                (_, KpiScope::Actor) => {
                    let parts = per_actor()
                        .into_iter()
                        .zip(actor_ids)
                        .map(|((part, whole), id)| KpiPart { actors: vec![id.to_string()], value: aggregate(part, whole) })
                        .collect();
                    (aggregate(hits, out_of), parts)
                }
            };
            KpiValue { id: def.id.clone(), label: def.label.clone(), value, breakdown }
        })
        .collect()
}

/// The indicators saved on the built-in Prisoner's Dilemma world.
pub(crate) fn prisoners_dilemma_kpis() -> Kpis {
    let def = |id: &str, label: &str, scope, filter, window, aggregation| KpiDefinition {
        id: id.into(),
        label: label.into(),
        scope,
        query: KpiQuery { filter, window, aggregation },
    };
    let outcome = |o: &str| KpiFilter { outcome: Some(o.into()), ..KpiFilter::default() };
    Kpis {
        definitions: vec![
            def(
                "mutual_cooperation_rate",
                "Mutual cooperation (% of rounds)",
                KpiScope::World,
                outcome("C/C"),
                KpiWindow::AllRounds,
                KpiAggregation::PercentageOfRounds,
            ),
            def(
                "percent_defectors",
                "Actors defecting more than half the time (%)",
                KpiScope::Actor,
                KpiFilter { decision: Some("D".into()), ratio_gt: Some(0.5), ..KpiFilter::default() },
                KpiWindow::AllRounds,
                KpiAggregation::PercentageOfActors,
            ),
            def(
                "exploitation_count",
                "Exploitation: one cooperates, one defects (rounds, last 100)",
                KpiScope::Pair,
                outcome("C/D"),
                KpiWindow::LastNRounds { n: 100 },
                KpiAggregation::Count,
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cd() -> Vec<String> {
        vec!["C".into(), "D".into()]
    }

    fn eval(def: KpiDefinition, pairings: &[Pairing]) -> KpiValue {
        let kpis = Kpis { definitions: vec![def] };
        assert_eq!(validate(&kpis, &cd()), vec![]);
        evaluate(&kpis, &cd(), &["x", "y", "z"], pairings).remove(0)
    }

    fn def(scope: KpiScope, filter: KpiFilter, window: KpiWindow, aggregation: KpiAggregation) -> KpiDefinition {
        KpiDefinition { id: "k".into(), label: "K".into(), scope, query: KpiQuery { filter, window, aggregation } }
    }

    const C: u8 = 0;
    const D: u8 = 1;
    // x vs y: C/C, C/D, D/C, D/D.   x vs z: D/C ×4.
    const XY: [[u8; 2]; 4] = [[C, C], [C, D], [D, C], [D, D]];
    const XZ: [[u8; 2]; 4] = [[D, C], [D, C], [D, C], [D, C]];

    fn pairings() -> Vec<Pairing<'static>> {
        vec![Pairing { a: 0, b: 1, moves: &XY }, Pairing { a: 0, b: 2, moves: &XZ }]
    }

    fn outcome(o: &str) -> KpiFilter {
        KpiFilter { outcome: Some(o.into()), ..KpiFilter::default() }
    }

    fn decision(d: &str, ratio_gt: Option<f64>) -> KpiFilter {
        KpiFilter { decision: Some(d.into()), ratio_gt, ..KpiFilter::default() }
    }

    #[test]
    fn outcome_filters_count_both_orientations() {
        let k = eval(def(KpiScope::World, outcome("C/D"), KpiWindow::AllRounds, KpiAggregation::Count), &pairings());
        assert_eq!(k.value, 6.0); // C/D and D/C in x–y, four D/C in x–z
        let k = eval(def(KpiScope::World, outcome("C/C"), KpiWindow::AllRounds, KpiAggregation::PercentageOfRounds), &pairings());
        assert_eq!(k.value, 12.5); // 1 of 8 rounds
    }

    #[test]
    fn windows_select_rounds() {
        let count = |window| eval(def(KpiScope::World, outcome("D/D"), window, KpiAggregation::Count), &pairings()).value;
        assert_eq!(count(KpiWindow::AllRounds), 1.0);
        assert_eq!(count(KpiWindow::FirstNRounds { n: 3 }), 0.0);
        assert_eq!(count(KpiWindow::LastNRounds { n: 1 }), 1.0);
        assert_eq!(count(KpiWindow::LastNRounds { n: 100 }), 1.0);
        assert_eq!(count(KpiWindow::Range { start: 3, end: 9 }), 1.0);
        assert_eq!(count(KpiWindow::Range { start: 10, end: 20 }), 0.0);
    }

    #[test]
    fn pair_and_actor_scopes_add_a_breakdown() {
        let k = eval(def(KpiScope::Pair, outcome("C/D"), KpiWindow::AllRounds, KpiAggregation::Count), &pairings());
        assert_eq!(k.value, 6.0);
        assert_eq!(k.breakdown.iter().map(|p| (p.actors.join("+"), p.value)).collect::<Vec<_>>(), [("x+y".to_string(), 2.0), ("x+z".to_string(), 4.0)]);

        let k = eval(def(KpiScope::Actor, decision("D", None), KpiWindow::AllRounds, KpiAggregation::PercentageOfRounds), &pairings());
        assert_eq!(k.value, 50.0); // 8 of 16 plays
        // x: 6 of 8 plays are D; y: 2 of 4; z: 0 of 4.
        assert_eq!(k.breakdown.iter().map(|p| p.value).collect::<Vec<_>>(), [75.0, 50.0, 0.0]);
    }

    #[test]
    fn percentage_of_actors_over_a_ratio() {
        let k = eval(def(KpiScope::Actor, decision("D", Some(0.5)), KpiWindow::AllRounds, KpiAggregation::PercentageOfActors), &pairings());
        assert!((k.value - 100.0 / 3.0).abs() < 1e-9); // only x is above one half; y is exactly at it
    }

    #[test]
    fn unsupported_or_malformed_definitions_are_rejected() {
        let bad = Kpis {
            definitions: vec![
                def(KpiScope::World, KpiFilter::default(), KpiWindow::AllRounds, KpiAggregation::Count),
                def(KpiScope::World, outcome("C/X"), KpiWindow::FirstNRounds { n: 0 }, KpiAggregation::Count),
                def(KpiScope::World, decision("D", None), KpiWindow::Range { start: 5, end: 5 }, KpiAggregation::PercentageOfActors),
            ],
        };
        let issues = validate(&bad, &cd());
        let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
        for expected in [
            "kpis.definitions[0].query.filter",
            "kpis.definitions[1].id",
            "kpis.definitions[1].query.filter.outcome",
            "kpis.definitions[1].query.window.n",
            "kpis.definitions[2].query.aggregation",
            "kpis.definitions[2].query.filter.ratio_gt",
            "kpis.definitions[2].query.window",
        ] {
            assert!(paths.contains(&expected), "missing {expected} in {issues:?}");
        }
    }

    #[test]
    fn the_built_in_indicators_are_valid() {
        assert_eq!(validate(&prisoners_dilemma_kpis(), &cd()), vec![]);
    }
}
