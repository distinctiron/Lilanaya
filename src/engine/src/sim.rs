//! Running a simulation: every pairing plays the world's number of rounds.

use crate::kpi::{self, KpiValue, Pairing};
use crate::rng::{derive_seed, Rng};
use crate::schedule::all_pairs;
use crate::strategy::{Compiled, Observation, Strategy};
use crate::world::{is_slug, Aggregate, Policy, ScheduleMode, World};
use crate::Issue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_ACTORS: usize = 64;

/// A participant: an id, a display name and the strategy it plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub id: String,
    pub name: String,
    pub strategy: Strategy,
}

/// What a custom decision-maker (for example a sandboxed code strategy) is
/// told before each round of a pairing. Decisions are indices into the world's
/// `decisions.allowed`.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    /// Index of the round about to be played (0 for the first).
    pub round: u32,
    /// How many rounds the pairing plays in all.
    pub rounds: u32,
    /// `(own, opponent)` decisions of the previous round; `None` in the first.
    pub last: Option<(u8, u8)>,
    /// How many rounds in a row the opponent has played its last decision.
    pub opponent_streak: u32,
    /// How many rounds in a row have ended in the last outcome.
    pub outcome_streak: u32,
    /// Per decision: how often this side and the opponent have played it so far.
    pub own_counts: &'a [u32],
    pub opponent_counts: &'a [u32],
    pub own_score: i64,
    pub opponent_score: i64,
}

/// A decision-maker that is not a state machine. It must be deterministic:
/// the same sequence of views must give the same decisions. An `Err` stops
/// the run and is reported against the actor.
pub trait Decider {
    fn decide(&mut self, view: &View) -> Result<u8, String>;
}

pub enum Logic {
    Fsm(Strategy),
    Custom(Box<dyn Decider>),
}

/// A participant whose logic may be a state machine or a custom decider.
pub struct Entrant {
    pub id: String,
    pub name: String,
    pub strategy_id: String,
    pub logic: Logic,
}

enum Brain {
    Fsm(Compiled),
    Custom(Box<dyn Decider>),
}

/// The summary of a run. Serialising it gives the same bytes for the same
/// world, actors and seed, every time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub world_id: String,
    pub seed: u64,
    /// Rounds per pairing.
    pub iterations: u32,
    pub policy: Policy,
    /// How the seed ordered the pairings; present only when the policy is `random`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_mode: Option<ScheduleMode>,
    pub decisions: Vec<String>,
    /// Sorted by actor id; `pairwise` rows and columns follow this order.
    pub actors: Vec<ActorResult>,
    /// Actor ids, best first (by the world's scoring aggregate; ties by id).
    pub ranking: Vec<String>,
    /// One entry per pairing, in the order the schedule executed them.
    pub matches: Vec<MatchResult>,
    /// `pairwise[row][col]`: the row actor's average payoff per round against
    /// the column actor. `null` on the diagonal (actors do not play themselves).
    pub pairwise: Vec<Vec<Option<f64>>>,
    /// Outcome → number of rounds, over all pairings, written `a/b` with `a`
    /// the actor whose id sorts first.
    pub outcome_distribution: BTreeMap<String, u64>,
    /// The world's saved indicators, evaluated over this run.
    #[serde(default)]
    pub kpis: Vec<KpiValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActorResult {
    pub id: String,
    pub name: String,
    pub strategy_id: String,
    /// Pairings played.
    pub matches: u32,
    /// Rounds played over all pairings.
    pub rounds: u64,
    pub total: i64,
    /// Average payoff per round.
    pub average: f64,
    /// Variance of the per-pairing averages (population variance).
    pub variance: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    /// Actor ids; `a` sorts before `b`.
    pub a: String,
    pub b: String,
    /// Sub-seed derived from the run seed and the two ids, as 16 hex digits.
    /// Reserved for seeded noise; today's strategies are deterministic.
    pub seed: String,
    pub totals: [i64; 2],
    pub averages: [f64; 2],
    /// Outcome (`a`'s decision / `b`'s decision) → number of rounds.
    pub outcome_counts: BTreeMap<String, u64>,
    /// Outcome → number of unbroken streaks of it. `outcome_counts / outcome_runs`
    /// is the average streak length (for D/D: how long mutual punishment lasts).
    pub outcome_runs: BTreeMap<String, u64>,
    /// Decision → the first round (from 0) in which `a` and `b` played it, or `null`.
    pub first_played: BTreeMap<String, [Option<u32>; 2]>,
}

/// The round-by-round record of one pairing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchTrace {
    pub a: String,
    pub b: String,
    /// Per round: `[a's decision, b's decision]` as indices into the world's decisions.
    pub moves: Vec<[u8; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunOutput {
    pub result: RunResult,
    /// Same order as `result.matches`.
    pub traces: Vec<MatchTrace>,
}

/// Runs `actors` against each other in `world` using `seed`.
/// Returns every validation problem if the inputs are not runnable.
pub fn run(world: &World, actors: &[Actor], seed: u64) -> Result<RunOutput, Vec<Issue>> {
    let entrants = actors
        .iter()
        .map(|a| Entrant { id: a.id.clone(), name: a.name.clone(), strategy_id: a.strategy.id.clone(), logic: Logic::Fsm(a.strategy.clone()) })
        .collect();
    run_entrants(world, entrants, seed)
}

/// Like [`run`], for participants that may bring a custom [`Decider`].
/// A decider that fails mid-run ends the run with an issue at `actors.{id}`.
pub fn run_entrants(world: &World, actors: Vec<Entrant>, seed: u64) -> Result<RunOutput, Vec<Issue>> {
    let mut issues: Vec<Issue> = world.validate().into_iter().map(|i| prefix("world", i)).collect();
    let decisions = &world.decisions.allowed;

    let mut compiled: Vec<Option<Brain>> = Vec::with_capacity(actors.len());
    let mut meta: Vec<(String, String, String)> = Vec::with_capacity(actors.len());
    for (i, actor) in actors.into_iter().enumerate() {
        if !is_slug(&actor.id) {
            issues.push(Issue::new(
                format!("actors[{i}].id"),
                "must be 1–64 characters: lower-case letters, digits, '-' or '_'",
            ));
        }
        if meta.iter().any(|(id, _, _)| *id == actor.id) {
            issues.push(Issue::new(format!("actors[{i}].id"), format!("\"{}\" is used by more than one actor", actor.id)));
        }
        match actor.logic {
            Logic::Fsm(strategy) => match strategy.compile(decisions) {
                Ok(c) => compiled.push(Some(Brain::Fsm(c))),
                Err(errs) => {
                    issues.extend(errs.into_iter().map(|e| prefix(&format!("actors[{i}].strategy"), e)));
                    compiled.push(None);
                }
            },
            Logic::Custom(decider) => compiled.push(Some(Brain::Custom(decider))),
        }
        meta.push((actor.id, actor.name, actor.strategy_id));
    }
    if meta.len() < 2 {
        issues.push(Issue::new("actors", "a run needs at least two actors"));
    }
    if meta.len() > MAX_ACTORS {
        issues.push(Issue::new("actors", format!("has {} actors; the limit is {MAX_ACTORS}", meta.len())));
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    // Stable order: by actor id, regardless of the order actors were given in.
    let mut order: Vec<usize> = (0..meta.len()).collect();
    order.sort_by(|&x, &y| meta[x].0.cmp(&meta[y].0));
    struct Who {
        id: String,
        name: String,
        strategy_id: String,
    }
    let mut brains: Vec<Brain> = order.iter().map(|&i| compiled[i].take().expect("compiled")).collect();
    let actors: Vec<Who> = order
        .iter()
        .map(|&i| {
            let (id, name, strategy_id) = meta[i].clone();
            Who { id, name, strategy_id }
        })
        .collect();

    let n = actors.len();
    let d = decisions.len();
    let rounds = world.time.iterations;
    // payoff[a * d + b] = [to first, to second]
    let payoff: Vec<[i64; 2]> = decisions
        .iter()
        .flat_map(|a| decisions.iter().map(move |b| world.payoff.matrix[a][b]))
        .collect();
    let outcome_names: Vec<String> = world.derived_outcomes();

    // One state per pairing, in actor-id order.
    let pairs = all_pairs(n);
    let mut states: Vec<PairState> = pairs
        .iter()
        .map(|&(ia, ib)| PairState::new(brains[ia].initial(), brains[ib].initial(), d, rounds))
        .collect();
    let mut step = |index: usize, round: u32, states: &mut Vec<PairState>| -> Result<(), Vec<Issue>> {
        let (ia, ib) = pairs[index];
        states[index].step(&mut brains, [ia, ib], &payoff, d, round).map_err(|(side, message)| {
            let who = &actors[[ia, ib][side]];
            vec![Issue::new(format!("actors.{}", who.id), format!("{} failed in round {}: {message}", who.name, round + 1))]
        })
    };

    // The order pairings are executed in. It is part of the run's record
    // (`matches` follows it), and it is all the seed influences today.
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    let schedule_mode = match world.interaction.policy {
        Policy::AllPairs => None,
        Policy::Random => Some(world.randomness.schedule_mode),
        Policy::RoundRobin => unreachable!("rejected by validation"),
    };
    let mut schedule_rng = Rng::new(derive_seed(seed, &["schedule"]));
    match schedule_mode {
        None => {
            for &index in &order {
                for round in 0..rounds {
                    step(index, round, &mut states)?;
                }
            }
        }
        Some(ScheduleMode::ShuffleAll) => {
            schedule_rng.shuffle(&mut order);
            for &index in &order {
                for round in 0..rounds {
                    step(index, round, &mut states)?;
                }
            }
        }
        Some(ScheduleMode::SamplePerRound) => {
            // Each round draws its own order; `matches` keeps the first round's.
            let mut round_order = order.clone();
            for round in 0..rounds {
                schedule_rng.shuffle(&mut round_order);
                if round == 0 {
                    order.copy_from_slice(&round_order);
                }
                for &index in &round_order {
                    step(index, round, &mut states)?;
                }
            }
        }
    }

    let mut totals = vec![0i64; n];
    let mut pairwise: Vec<Vec<Option<f64>>> = vec![vec![None; n]; n];
    let mut all_counts = vec![0u64; d * d];
    let mut matches = Vec::with_capacity(pairs.len());
    let mut traces = Vec::with_capacity(pairs.len());
    let mut slots: Vec<Option<PairState>> = states.into_iter().map(Some).collect();
    for &index in &order {
        let (ia, ib) = pairs[index];
        let state = slots[index].take().expect("each pairing appears once in the order");
        let score = state.score;
        totals[ia] += score[0];
        totals[ib] += score[1];
        let averages = [score[0] as f64 / rounds as f64, score[1] as f64 / rounds as f64];
        pairwise[ia][ib] = Some(averages[0]);
        pairwise[ib][ia] = Some(averages[1]);
        for (all, c) in all_counts.iter_mut().zip(&state.counts) {
            *all += c;
        }

        let (a, b) = (actors[ia].id.clone(), actors[ib].id.clone());
        matches.push(MatchResult {
            seed: format!("{:016x}", derive_seed(seed, &[&a, &b])),
            totals: score,
            averages,
            outcome_counts: outcome_names.iter().cloned().zip(state.counts).collect(),
            outcome_runs: outcome_names.iter().cloned().zip(state.runs).collect(),
            first_played: decisions.iter().cloned().zip(state.first).collect(),
            a: a.clone(),
            b: b.clone(),
        });
        traces.push(MatchTrace { a, b, moves: state.moves });
    }

    let actor_ids: Vec<&str> = actors.iter().map(|a| a.id.as_str()).collect();
    let pairings: Vec<Pairing> = order
        .iter()
        .zip(&traces)
        .map(|(&index, trace)| Pairing { a: pairs[index].0, b: pairs[index].1, moves: &trace.moves })
        .collect();
    let kpis = kpi::evaluate(&world.kpis, decisions, &actor_ids, &pairings);

    let actor_results: Vec<ActorResult> = (0..n)
        .map(|i| {
            let played = (n - 1) as u32;
            let total_rounds = played as u64 * rounds as u64;
            let average = totals[i] as f64 / total_rounds as f64;
            // Fixed iteration order (by opponent id) keeps the floating-point sum reproducible.
            let variance = pairwise[i].iter().flatten().map(|v| (v - average) * (v - average)).sum::<f64>() / played as f64;
            ActorResult {
                id: actors[i].id.clone(),
                name: actors[i].name.clone(),
                strategy_id: actors[i].strategy_id.clone(),
                matches: played,
                rounds: total_rounds,
                total: totals[i],
                average,
                variance,
            }
        })
        .collect();

    // Every actor plays the same number of rounds, so sum and average agree on order;
    // totals are exact integers, which makes the comparison (and ties) unambiguous.
    let mut ranking: Vec<usize> = (0..n).collect();
    match world.scoring.aggregate {
        Aggregate::Sum | Aggregate::Avg => ranking.sort_by(|&x, &y| totals[y].cmp(&totals[x]).then(actors[x].id.cmp(&actors[y].id))),
    }

    Ok(RunOutput {
        result: RunResult {
            world_id: world.id.clone(),
            seed,
            iterations: rounds,
            policy: world.interaction.policy,
            schedule_mode,
            decisions: decisions.clone(),
            ranking: ranking.into_iter().map(|i| actors[i].id.clone()).collect(),
            actors: actor_results,
            matches,
            pairwise,
            outcome_distribution: outcome_names.into_iter().zip(all_counts).collect(),
            kpis,
        },
        traces,
    })
}

impl Brain {
    fn initial(&self) -> usize {
        match self {
            Brain::Fsm(c) => c.initial,
            Brain::Custom(_) => 0,
        }
    }
}

/// Everything one pairing accumulates while it plays.
struct PairState {
    state: [usize; 2],
    obs: [Observation; 2],
    score: [i64; 2],
    /// Per side, per decision: how often it has been played.
    played: [Vec<u32>; 2],
    /// Rounds per outcome (`a's decision * d + b's decision`).
    counts: Vec<u64>,
    /// Streaks per outcome.
    runs: Vec<u64>,
    last_outcome: Option<usize>,
    /// Per decision: the first round each side played it.
    first: Vec<[Option<u32>; 2]>,
    moves: Vec<[u8; 2]>,
}

impl PairState {
    fn new(initial_a: usize, initial_b: usize, d: usize, rounds: u32) -> Self {
        PairState {
            state: [initial_a, initial_b],
            obs: [Observation::new(rounds); 2],
            score: [0; 2],
            played: [vec![0; d], vec![0; d]],
            counts: vec![0; d * d],
            runs: vec![0; d * d],
            last_outcome: None,
            first: vec![[None; 2]; d],
            moves: Vec::with_capacity(rounds as usize),
        }
    }

    /// One side's decision for the coming round. A custom decider's answer is
    /// checked against the number of decisions.
    fn decide(&mut self, brain: &mut Brain, side: usize, d: usize) -> Result<u8, String> {
        match brain {
            Brain::Fsm(machine) => {
                let (next, decision) = machine.step(self.state[side], &self.obs[side]);
                self.state[side] = next;
                Ok(decision)
            }
            Brain::Custom(decider) => {
                let obs = &self.obs[side];
                let view = View {
                    round: obs.round,
                    rounds: obs.rounds,
                    last: obs.last,
                    opponent_streak: obs.opponent_streak,
                    outcome_streak: obs.outcome_streak,
                    own_counts: &self.played[side],
                    opponent_counts: &self.played[1 - side],
                    own_score: self.score[side],
                    opponent_score: self.score[1 - side],
                };
                let decision = decider.decide(&view)?;
                if decision as usize >= d {
                    return Err(format!("returned decision number {decision}, but the world has only {d}"));
                }
                Ok(decision)
            }
        }
    }

    /// Plays one round. Both decide from what they saw before it: A then B,
    /// neither sees the other's move. On failure: which side (0 or 1) and why.
    fn step(&mut self, brains: &mut [Brain], who: [usize; 2], payoff: &[[i64; 2]], d: usize, round: u32) -> Result<(), (usize, String)> {
        let move_a = self.decide(&mut brains[who[0]], 0, d).map_err(|e| (0, e))?;
        let move_b = self.decide(&mut brains[who[1]], 1, d).map_err(|e| (1, e))?;

        let outcome = move_a as usize * d + move_b as usize;
        self.score[0] += payoff[outcome][0];
        self.score[1] += payoff[outcome][1];
        self.played[0][move_a as usize] += 1;
        self.played[1][move_b as usize] += 1;
        self.counts[outcome] += 1;
        if self.last_outcome != Some(outcome) {
            self.runs[outcome] += 1;
            self.last_outcome = Some(outcome);
        }
        self.first[move_a as usize][0].get_or_insert(round);
        self.first[move_b as usize][1].get_or_insert(round);
        self.obs[0].record(move_a, move_b);
        self.obs[1].record(move_b, move_a);
        self.moves.push([move_a, move_b]);
        Ok(())
    }
}

fn prefix(path: &str, issue: Issue) -> Issue {
    Issue::new(format!("{path}.{}", issue.path), issue.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{baseline_strategies, prisoners_dilemma};

    fn baseline_actors() -> Vec<Actor> {
        baseline_strategies()
            .into_iter()
            .map(|s| Actor { id: s.id.clone(), name: s.name.clone(), strategy: s })
            .collect()
    }

    fn world(iterations: u32) -> World {
        let mut w = prisoners_dilemma();
        w.time.iterations = iterations;
        w
    }

    fn find<'a>(r: &'a RunResult, a: &str, b: &str) -> &'a MatchResult {
        r.matches.iter().find(|m| m.a == a && m.b == b).unwrap()
    }

    #[test]
    fn baseline_tournament_scores_match_hand_calculation() {
        let n = 200;
        let out = run(&world(n as u32), &baseline_actors(), 42).unwrap();
        let r = &out.result;

        assert_eq!(
            r.actors.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["always_cooperate", "always_defect", "tit_for_tat", "win_stay_lose_shift"]
        );
        assert_eq!(r.matches.len(), 6);
        assert_eq!(find(r, "always_cooperate", "always_defect").totals, [0, 5 * n]);
        assert_eq!(find(r, "always_cooperate", "tit_for_tat").totals, [3 * n, 3 * n]);
        assert_eq!(find(r, "always_cooperate", "win_stay_lose_shift").totals, [3 * n, 3 * n]);
        // Tit for Tat is exploited once, then both defect.
        assert_eq!(find(r, "always_defect", "tit_for_tat").totals, [5 + (n - 1), n - 1]);
        // WSLS alternates C, D against a defector: the defector earns 5, 1, 5, 1, …
        assert_eq!(find(r, "always_defect", "win_stay_lose_shift").totals, [3 * n, n / 2]);
        assert_eq!(find(r, "tit_for_tat", "win_stay_lose_shift").totals, [3 * n, 3 * n]);

        let total = |id: &str| r.actors.iter().find(|a| a.id == id).unwrap().total;
        assert_eq!(total("always_cooperate"), 6 * n);
        assert_eq!(total("always_defect"), 5 * n + 5 + (n - 1) + 3 * n);
        assert_eq!(total("tit_for_tat"), 3 * n + (n - 1) + 3 * n);
        assert_eq!(total("win_stay_lose_shift"), 3 * n + n / 2 + 3 * n);
        assert_eq!(r.ranking, ["always_defect", "tit_for_tat", "win_stay_lose_shift", "always_cooperate"]);

        let m = find(r, "always_defect", "tit_for_tat");
        assert_eq!(m.outcome_counts["D/C"], 1);
        assert_eq!(m.outcome_counts["D/D"], n as u64 - 1);
        assert_eq!(m.first_played["D"], [Some(0), Some(1)]);
        assert_eq!(m.first_played["C"], [None, Some(0)]);
        assert_eq!(r.outcome_distribution.values().sum::<u64>(), 6 * n as u64);
        assert_eq!(r.pairwise[0][1], Some(0.0));
        assert_eq!(r.pairwise[1][0], Some(5.0));
        assert_eq!(r.pairwise[2][2], None);

        assert_eq!(out.traces.len(), 6);
        assert_eq!(out.traces[4].moves[..4], [[1, 0], [1, 1], [1, 0], [1, 1]]);
        assert_eq!(r.schedule_mode, None);
    }

    #[test]
    fn streaks_and_indicators_are_reported() {
        let r = run(&world(200), &baseline_actors(), 42).unwrap().result;
        // Always Defect v WSLS alternates D/C, D/D: 100 streaks of each, one round long.
        let m = find(&r, "always_defect", "win_stay_lose_shift");
        assert_eq!((m.outcome_counts["D/D"], m.outcome_runs["D/D"]), (100, 100));
        // Always Defect v Tit for Tat: one long punishment.
        let m = find(&r, "always_defect", "tit_for_tat");
        assert_eq!((m.outcome_counts["D/D"], m.outcome_runs["D/D"]), (199, 1));

        let kpi = |id: &str| r.kpis.iter().find(|k| k.id == id).unwrap();
        // C/C all the way in three of six pairings.
        assert_eq!(kpi("mutual_cooperation_rate").value, 50.0);
        // Only Always Defect defects more than half the time (WSLS: 100 of 600).
        assert_eq!(kpi("percent_defectors").value, 25.0);
        // Last 100 rounds: 100 (AC v AD) + 0 (AD v TFT) + 50 (AD v WSLS).
        assert_eq!(kpi("exploitation_count").value, 150.0);
        assert_eq!(kpi("exploitation_count").breakdown.len(), 6);
    }

    #[test]
    fn random_schedules_reorder_matches_but_not_scores() {
        use crate::world::{Policy, ScheduleMode};
        let sorted = |mut r: RunResult| {
            r.matches.sort_by(|x, y| (&x.a, &x.b).cmp(&(&y.a, &y.b)));
            r.policy = Policy::AllPairs;
            r.schedule_mode = None;
            // Per-pair breakdowns follow match order; compare the headline values only.
            for k in &mut r.kpis {
                k.breakdown.clear();
            }
            r
        };
        let baseline = sorted(run(&world(60), &baseline_actors(), 5).unwrap().result);
        let order = |r: &RunResult| r.matches.iter().map(|m| format!("{}+{}", m.a, m.b)).collect::<Vec<_>>();

        for mode in [ScheduleMode::ShuffleAll, ScheduleMode::SamplePerRound] {
            let mut w = world(60);
            w.interaction.policy = Policy::Random;
            w.randomness.schedule_mode = mode;
            let first = run(&w, &baseline_actors(), 5).unwrap();
            assert_eq!(first, run(&w, &baseline_actors(), 5).unwrap(), "{mode:?}: same seed, same output");
            assert_eq!(first.result.schedule_mode, Some(mode));
            assert_eq!(first.result.matches.len(), 6, "{mode:?}: no pairing missing or repeated");

            // Some seed in a handful must give a different order from seed 5.
            let reordered = (6..16).any(|seed| order(&run(&w, &baseline_actors(), seed).unwrap().result) != order(&first.result));
            assert!(reordered, "{mode:?}: the seed should drive the order");
            for (m, t) in first.result.matches.iter().zip(&first.traces) {
                assert_eq!((&m.a, &m.b), (&t.a, &t.b));
            }
            assert_eq!(sorted(first.result), baseline, "{mode:?}: scores do not depend on the order");
        }
    }

    #[test]
    fn actor_order_does_not_change_the_result() {
        let mut reversed = baseline_actors();
        reversed.reverse();
        let a = run(&world(50), &baseline_actors(), 7).unwrap();
        let b = run(&world(50), &reversed, 7).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn the_seed_is_recorded_and_feeds_the_pair_seeds() {
        let a = run(&world(10), &baseline_actors(), 1).unwrap().result;
        let b = run(&world(10), &baseline_actors(), 2).unwrap().result;
        assert_eq!((a.seed, b.seed), (1, 2));
        assert_ne!(a.matches[0].seed, b.matches[0].seed);
        assert_ne!(a.matches[0].seed, a.matches[1].seed);
        // Deterministic strategies: the scores themselves do not depend on the seed.
        assert_eq!(a.ranking, b.ranking);
    }

    /// Copies the opponent's last move, like Tit for Tat, but as custom logic.
    struct Copycat;
    impl Decider for Copycat {
        fn decide(&mut self, view: &View) -> Result<u8, String> {
            Ok(view.last.map_or(0, |(_, opponent)| opponent))
        }
    }

    /// Fails once the opponent has defected three times.
    struct Fragile;
    impl Decider for Fragile {
        fn decide(&mut self, view: &View) -> Result<u8, String> {
            if view.opponent_counts[1] >= 3 {
                return Err("gave up".into());
            }
            Ok(0)
        }
    }

    fn entrants(custom_id: &str, decider: Box<dyn Decider>) -> Vec<Entrant> {
        let mut list: Vec<Entrant> = baseline_actors()
            .into_iter()
            .filter(|a| a.id != "tit_for_tat")
            .map(|a| Entrant { id: a.id, name: a.name, strategy_id: a.strategy.id.clone(), logic: Logic::Fsm(a.strategy) })
            .collect();
        list.push(Entrant { id: custom_id.into(), name: "Custom".into(), strategy_id: custom_id.into(), logic: Logic::Custom(decider) });
        list
    }

    #[test]
    fn a_custom_decider_plays_like_the_machine_it_imitates() {
        // Same id as the machine it replaces, so order, seeds and scores must all agree.
        let custom = run_entrants(&world(50), entrants("tit_for_tat", Box::new(Copycat)), 9).unwrap();
        let mut machines = run(&world(50), &baseline_actors(), 9).unwrap();
        for a in &mut machines.result.actors {
            if a.id == "tit_for_tat" {
                a.name = "Custom".into();
            }
        }
        assert_eq!(custom, machines);
    }

    #[test]
    fn a_failing_decider_stops_the_run_and_is_named() {
        let issues = run_entrants(&world(50), entrants("fragile", Box::new(Fragile)), 9).unwrap_err();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "actors.fragile");
        // Against Always Defect the third defection has been seen when round 4 is decided.
        assert_eq!(issues[0].message, "Custom failed in round 4: gave up");
    }

    #[test]
    fn invalid_inputs_are_reported_together() {
        let mut actors = baseline_actors();
        actors[1].id = actors[0].id.clone();
        actors[2].strategy.initial_state = "MISSING".into();
        let mut w = world(10);
        w.time.iterations = 0;
        let issues = run(&w, &actors, 1).unwrap_err();
        let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["world.time.iterations", "actors[1].id", "actors[2].strategy.initial_state"]);

        let issues = run(&world(10), &actors[..1], 1).unwrap_err();
        assert_eq!(issues[0].path, "actors");
    }
}
