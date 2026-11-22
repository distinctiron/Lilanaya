//! Rules that must hold for every world, every set of strategies and every seed,
//! checked over a few hundred generated cases.
//!
//! The cases come from the engine's own seeded generator, so every run of this
//! file checks exactly the same cases: a failure names the case, and running
//! the test again reproduces it.

use lilanaya_engine::rng::Rng;
use lilanaya_engine::{prisoners_dilemma, run, Actor, Guard, Policy, RunOutput, ScheduleMode, Strategy, Transition, World};
use std::collections::BTreeMap;

const CASES: u64 = 300;

fn pick<'a, T>(rng: &mut Rng, items: &'a [T]) -> &'a T {
    &items[rng.below(items.len() as u64) as usize]
}

/// A world with 2 to 4 decisions, a random payoff matrix and a short match.
fn world(rng: &mut Rng) -> World {
    let mut world = prisoners_dilemma();
    let names = ["C", "D", "WAIT", "LEAVE"];
    let count = 2 + rng.below(3) as usize;
    let decisions: Vec<String> = names[..count].iter().map(|d| d.to_string()).collect();
    world.payoff.matrix = decisions
        .iter()
        .map(|a| (a.clone(), decisions.iter().map(|b| (b.clone(), [rng.below(11) as i64 - 3, rng.below(11) as i64 - 3])).collect()))
        .collect();
    world.decisions.allowed = decisions;
    // Derived from the decisions.
    world.outcomes.allowed.clear();
    // The built-in indicators are about C and D only; they have their own tests.
    world.kpis.definitions.clear();
    world.time.iterations = 1 + rng.below(40) as u32;
    world.interaction.policy = *pick(rng, &[Policy::AllPairs, Policy::Random]);
    world.randomness.schedule_mode = *pick(rng, &[ScheduleMode::ShuffleAll, ScheduleMode::SamplePerRound]);
    world
}

fn guard(rng: &mut Rng, decisions: &[String], depth: u32) -> Guard {
    let kinds = if depth >= 2 { 6 } else { 9 };
    match rng.below(kinds) {
        0 => Guard::LastOpponentAction { is: pick(rng, decisions).clone() },
        1 => Guard::LastOwnAction { is: pick(rng, decisions).clone() },
        2 => Guard::OpponentStreak { decision: pick(rng, decisions).clone(), at_least: 1 + rng.below(4) as u32 },
        3 => Guard::OutcomeStreak { outcome: format!("{}/{}", pick(rng, decisions), pick(rng, decisions)), at_least: 1 + rng.below(4) as u32 },
        4 => Guard::RoundIndex { at_least: rng.below(30) as u32 },
        5 => Guard::RoundsLeft { at_most: 1 + rng.below(10) as u32 },
        6 => Guard::And { all: (0..1 + rng.below(3)).map(|_| guard(rng, decisions, depth + 1)).collect() },
        7 => Guard::Or { any: (0..1 + rng.below(3)).map(|_| guard(rng, decisions, depth + 1)).collect() },
        _ => Guard::Not { guard: Box::new(guard(rng, decisions, depth + 1)) },
    }
}

/// A valid state machine of 1 to 5 states for a world with these decisions.
fn strategy(rng: &mut Rng, id: &str, decisions: &[String]) -> Strategy {
    let states: Vec<String> = (0..1 + rng.below(5)).map(|i| format!("S{i}")).collect();
    let action_map: BTreeMap<String, String> = states.iter().map(|s| (s.clone(), pick(rng, decisions).clone())).collect();
    let transitions = (0..rng.below(8)).map(|_| Transition { from: pick(rng, &states).clone(), to: pick(rng, &states).clone(), guard: guard(rng, decisions, 0) }).collect();
    Strategy { id: id.into(), name: format!("Generated {id}"), description: String::new(), initial_state: pick(rng, &states).clone(), states, action_map, transitions }
}

fn actors(rng: &mut Rng, decisions: &[String]) -> Vec<Actor> {
    (0..2 + rng.below(5))
        .map(|i| {
            let id = format!("actor-{i}");
            Actor { name: format!("Actor {i}"), strategy: strategy(rng, &id, decisions), id }
        })
        .collect()
}

/// One generated case: a world, its actors and a seed.
fn case(n: u64) -> (World, Vec<Actor>, u64) {
    let mut rng = Rng::new(0xC0FF_EE00 + n);
    let world = world(&mut rng);
    let actors = actors(&mut rng, &world.decisions.allowed);
    let seed = rng.next_u64() >> 11;
    (world, actors, seed)
}

fn bytes(out: &RunOutput) -> String {
    serde_json::to_string(out).unwrap()
}

fn played(world: &World, actors: &[Actor], seed: u64, n: u64) -> RunOutput {
    run(world, actors, seed).unwrap_or_else(|issues| panic!("case {n}: a generated case must be valid, got {issues:?}"))
}

#[test]
fn the_same_world_actors_and_seed_always_give_the_same_bytes() {
    for n in 0..CASES {
        let (world, actors, seed) = case(n);
        let first = bytes(&played(&world, &actors, seed, n));
        assert!(first == bytes(&played(&world, &actors, seed, n)), "case {n}: two runs of the same input differ");
    }
}

#[test]
fn the_order_actors_are_entered_in_changes_nothing() {
    for n in 0..CASES {
        let (world, mut actors, seed) = case(n);
        let first = bytes(&played(&world, &actors, seed, n));
        actors.reverse();
        assert!(first == bytes(&played(&world, &actors, seed, n)), "case {n}: reversed");
        Rng::new(n).shuffle(&mut actors);
        assert!(first == bytes(&played(&world, &actors, seed, n)), "case {n}: shuffled");
    }
}

#[test]
fn the_summary_adds_up_to_what_the_traces_record() {
    for n in 0..CASES {
        let (world, actors, seed) = case(n);
        let out = played(&world, &actors, seed, n);
        let r = &out.result;
        let decisions = &world.decisions.allowed;
        let rounds = world.time.iterations as u64;
        let pairs = actors.len() * (actors.len() - 1) / 2;

        assert_eq!((r.matches.len(), out.traces.len()), (pairs, pairs), "case {n}: one match and one trace per pair");
        let mut totals: BTreeMap<&str, i64> = BTreeMap::new();
        let mut distribution: BTreeMap<String, u64> = BTreeMap::new();
        for (m, trace) in r.matches.iter().zip(&out.traces) {
            assert_eq!((&m.a, &m.b), (&trace.a, &trace.b), "case {n}: traces follow the matches");
            assert!(m.a < m.b, "case {n}: a pair is stored with the smaller id first");
            assert_eq!(trace.moves.len() as u64, rounds, "case {n}: every pair plays every round");

            let mut score = [0i64; 2];
            let mut counts: BTreeMap<String, u64> = BTreeMap::new();
            for moves in &trace.moves {
                let (a, b) = (&decisions[moves[0] as usize], &decisions[moves[1] as usize]);
                let won = world.payoff.matrix[a][b];
                score = [score[0] + won[0], score[1] + won[1]];
                *counts.entry(format!("{a}/{b}")).or_default() += 1;
            }
            assert_eq!(m.totals, score, "case {n}: {} v {}: totals are the payoffs of the rounds played", m.a, m.b);
            assert_eq!(m.averages, [score[0] as f64 / rounds as f64, score[1] as f64 / rounds as f64], "case {n}");
            for (outcome, count) in &m.outcome_counts {
                assert_eq!(*count, counts.get(outcome).copied().unwrap_or(0), "case {n}: {} v {}: rounds that ended {outcome}", m.a, m.b);
                assert!(m.outcome_runs[outcome] <= *count, "case {n}: no more streaks than rounds");
                assert_eq!(m.outcome_runs[outcome] == 0, *count == 0, "case {n}: an outcome that happened has a streak");
                *distribution.entry(outcome.clone()).or_default() += count;
            }
            assert_eq!(m.outcome_counts.values().sum::<u64>(), rounds, "case {n}");
            *totals.entry(&m.a).or_default() += score[0];
            *totals.entry(&m.b).or_default() += score[1];
        }
        assert_eq!(r.outcome_distribution, distribution, "case {n}: the run's outcome mix is the sum of its matches'");

        let ids: Vec<&str> = r.actors.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "case {n}: actors are listed by id");
        for (i, a) in r.actors.iter().enumerate() {
            assert_eq!(a.total, totals[a.id.as_str()], "case {n}: {}'s total is the sum of its matches", a.id);
            assert_eq!((a.matches as usize, a.rounds), (actors.len() - 1, (actors.len() as u64 - 1) * rounds), "case {n}");
            assert_eq!(a.average, a.total as f64 / a.rounds as f64, "case {n}");
            assert!(a.variance >= 0.0, "case {n}");
            assert_eq!(r.pairwise[i][i], None, "case {n}: nobody plays themselves");
            assert_eq!(r.pairwise[i].iter().flatten().count(), actors.len() - 1, "case {n}");
        }

        let mut expected: Vec<&str> = ids.clone();
        expected.sort_by(|x, y| totals[y].cmp(&totals[x]).then(x.cmp(y)));
        assert_eq!(r.ranking, expected, "case {n}: ranked by total, ties by id");
    }
}

/// What happened in one match: its numbers, written out, and its rounds.
type Played = (String, Vec<[u8; 2]>);

#[test]
fn the_seed_and_the_schedule_order_matches_but_never_decide_one() {
    for n in 0..CASES {
        let (mut world, actors, seed) = case(n);
        let by_pair = |out: &RunOutput| -> BTreeMap<(String, String), Played> {
            out.result
                .matches
                .iter()
                .zip(&out.traces)
                // The match's own seed differs with the run's seed, by design; everything else must not.
                .map(|(m, t)| ((m.a.clone(), m.b.clone()), (serde_json::to_string(&(&m.totals, &m.outcome_counts, &m.outcome_runs, &m.first_played)).unwrap(), t.moves.clone())))
                .collect()
        };
        world.interaction.policy = Policy::AllPairs;
        let plain = played(&world, &actors, seed, n);
        for (policy, mode) in [(Policy::Random, ScheduleMode::ShuffleAll), (Policy::Random, ScheduleMode::SamplePerRound), (Policy::AllPairs, ScheduleMode::ShuffleAll)] {
            world.interaction.policy = policy;
            world.randomness.schedule_mode = mode;
            for other_seed in [seed, seed ^ 1, 0, (1 << 53) - 1] {
                let out = played(&world, &actors, other_seed, n);
                assert!(by_pair(&plain) == by_pair(&out), "case {n}: {policy:?} {mode:?} seed {other_seed} changed a match");
                assert_eq!(plain.result.ranking, out.result.ranking, "case {n}: {policy:?} {mode:?} seed {other_seed} changed the ranking");
                assert_eq!(out.result.schedule_mode, (policy == Policy::Random).then_some(mode), "case {n}: the schedule is recorded only when random");
            }
        }
    }
}

#[test]
fn a_run_survives_being_written_out_and_read_back() {
    for n in 0..CASES {
        let (world, actors, seed) = case(n);
        let out = played(&world, &actors, seed, n);
        let back: RunOutput = serde_json::from_str(&bytes(&out)).unwrap();
        assert!(back == out, "case {n}: the run read back differs");
        let world_back: World = serde_json::from_str(&serde_json::to_string(&world).unwrap()).unwrap();
        assert!(world_back == world, "case {n}: the world read back differs");
        for a in &actors {
            let strategy_back: Strategy = serde_json::from_str(&serde_json::to_string(&a.strategy).unwrap()).unwrap();
            assert!(strategy_back == a.strategy, "case {n}: {} read back differs", a.id);
        }
    }
}

/// Breaks one thing about the case, the way a careless or hostile author might.
fn damage(rng: &mut Rng, world: &mut World, actors: &mut Vec<Actor>) {
    let which = rng.below(actors.len() as u64) as usize;
    match rng.below(14) {
        0 => world.time.iterations = 0,
        1 => world.decisions.allowed.truncate(1),
        2 => world.decisions.allowed.push(world.decisions.allowed[0].clone()),
        3 => world.decisions.allowed.push("A/B".into()),
        4 => {
            world.payoff.matrix.pop_first();
        }
        5 => world.interaction.policy = Policy::RoundRobin,
        6 => world.id = "Not A Slug".into(),
        7 => actors.truncate(1),
        8 => actors[which].id = actors[(which + 1) % actors.len()].id.clone(),
        9 => actors[which].strategy.initial_state = "NOWHERE".into(),
        10 => {
            actors[which].strategy.action_map.pop_first();
        }
        11 => actors[which].strategy.transitions.push(Transition { from: "NOWHERE".into(), to: "S0".into(), guard: Guard::RoundIndex { at_least: 0 } }),
        12 => actors[which].strategy.transitions.push(Transition { from: "S0".into(), to: "S0".into(), guard: Guard::LastOpponentAction { is: "NOT_A_DECISION".into() } }),
        _ => actors[which].strategy.states.clear(),
    }
}

#[test]
fn a_broken_world_or_strategy_is_refused_with_reasons_and_never_crashes() {
    for n in 0..CASES {
        let (mut world, mut actors, seed) = case(n);
        let mut rng = Rng::new(0xBAD_0000 + n);
        for _ in 0..1 + rng.below(3) {
            damage(&mut rng, &mut world, &mut actors);
        }
        let issues = match run(&world, &actors, seed) {
            Ok(_) => panic!("case {n}: a damaged case ran"),
            Err(issues) => issues,
        };
        assert!(!issues.is_empty(), "case {n}: refused without a reason");
        for issue in &issues {
            assert!(!issue.message.trim().is_empty(), "case {n}: an issue at \"{}\" says nothing", issue.path);
        }
        // The same problems, in the same order, every time.
        assert_eq!(run(&world, &actors, seed).err(), Some(issues), "case {n}");
    }
}

#[test]
fn what_the_validators_pass_runs_and_what_they_refuse_does_not() {
    for n in 0..CASES {
        let (mut world, mut actors, seed) = case(n);
        let mut rng = Rng::new(0xFEED_0000 + n);
        if rng.below(2) == 0 {
            damage(&mut rng, &mut world, &mut actors);
        }
        let ids: Vec<&String> = actors.iter().map(|a| &a.id).collect();
        let unique = ids.iter().enumerate().all(|(i, id)| !ids[..i].contains(id));
        let checks_pass = world.validate().is_empty() && actors.len() >= 2 && unique && actors.iter().all(|a| a.strategy.validate(&world.decisions.allowed).is_empty());
        assert_eq!(run(&world, &actors, seed).is_ok(), checks_pass, "case {n}: the validators and the engine disagree");
    }
}
