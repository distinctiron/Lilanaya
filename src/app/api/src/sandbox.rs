//! JavaScript code strategies, run in an embedded QuickJS interpreter.
//!
//! A code strategy defines `function decide(input)` and returns one of the
//! world's decisions. Each actor gets its own interpreter with:
//! - a memory cap and a stack cap;
//! - a step budget per call and over the whole run (so an endless loop ends
//!   the run instead of hanging the worker), counted in interpreter steps, not
//!   wall-clock time, so the same run fails or passes the same way everywhere;
//! - no clock, no randomness, no modules, no I/O: nothing that could make two
//!   runs of the same inputs differ.
//!
//! Steps are not time: one step can be a slow built-in. The clock is kept by
//! `isolate.rs`, which runs all of this in a child process it can stop.

use lilanaya_engine::{Decider, View};
use rquickjs::{Array, Context, Function, Object, Persistent, Runtime, Value};
use std::cell::Cell;
use std::rc::Rc;

pub const MAX_SOURCE_BYTES: usize = 16 * 1024;
/// A strategy needs kilobytes. A small cap also keeps single steps cheap: a built-in can
/// only be as slow as the values it works on are large.
const MEMORY_LIMIT_BYTES: usize = 1024 * 1024;
const STACK_LIMIT_BYTES: usize = 256 * 1024;
/// The interpreter checks in about every 5,000 loop turns or calls (measured); each check is one tick.
const LOAD_TICKS: u64 = 200;
/// About a million loop turns in one call.
const CALL_TICKS: u64 = 200;
/// About a hundred million loop turns over a run. Calling a simple `decide`
/// 2.4 million times (the largest run allowed) uses about 500.
const RUN_TICKS: u64 = 20_000;

/// Removes everything that would make a strategy depend on anything but its input.
const PRELUDE: &str = r#"
for (const name of ['Date', 'performance', 'WeakRef', 'FinalizationRegistry', 'Atomics', 'SharedArrayBuffer', 'queueMicrotask']) {
    delete globalThis[name];
}
delete Math.random;
"#;

pub struct JsStrategy {
    decide: Persistent<Function<'static>>,
    context: Context,
    // Dropped after the context and the saved function (declaration order).
    _runtime: Runtime,
    decisions: Vec<String>,
    /// Ticks left for the current call; the interrupt handler counts it down.
    call_left: Rc<Cell<u64>>,
    run_left: u64,
}

impl JsStrategy {
    /// Compiles `source` and finds its `decide` function.
    pub fn load(source: &str, decisions: &[String]) -> Result<Self, String> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(format!("the code is {} bytes; the limit is {MAX_SOURCE_BYTES}", source.len()));
        }
        let runtime = Runtime::new().map_err(|e| format!("could not start the interpreter: {e}"))?;
        runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
        runtime.set_max_stack_size(STACK_LIMIT_BYTES);
        let call_left = Rc::new(Cell::new(LOAD_TICKS));
        let left = call_left.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || match left.get() {
            0 => true,
            n => {
                left.set(n - 1);
                false
            }
        })));
        let context = Context::full(&runtime).map_err(|e| format!("could not start the interpreter: {e}"))?;
        let decide = context.with(|ctx| -> Result<Persistent<Function<'static>>, String> {
            ctx.eval::<(), _>(PRELUDE).map_err(|e| explain(&ctx, e, false))?;
            ctx.eval::<(), _>(source).map_err(|e| explain(&ctx, e, call_left.get() == 0))?;
            let found: Value = ctx.globals().get("decide").map_err(|e| explain(&ctx, e, false))?;
            match found.into_function() {
                Some(f) => Ok(Persistent::save(&ctx, f)),
                None => Err("the code must define a function named decide, for example: function decide(input) { return \"C\"; }".into()),
            }
        })?;
        Ok(JsStrategy { decide, context, _runtime: runtime, decisions: decisions.to_vec(), call_left, run_left: RUN_TICKS })
    }
}

impl Decider for JsStrategy {
    fn decide(&mut self, view: &View) -> Result<u8, String> {
        if self.run_left == 0 {
            return Err("used up its computing allowance for the run".into());
        }
        let budget = CALL_TICKS.min(self.run_left);
        self.call_left.set(budget);
        let decisions = &self.decisions;
        let name = |i: u8| decisions[i as usize].as_str();
        let result = self.context.with(|ctx| -> Result<u8, String> {
            let fail = |e: rquickjs::Error| explain(&ctx, e, self.call_left.get() == 0);
            let counts = |values: &[u32]| -> rquickjs::Result<Object> {
                let object = Object::new(ctx.clone())?;
                for (decision, count) in decisions.iter().zip(values) {
                    object.set(decision.as_str(), *count)?;
                }
                Ok(object)
            };
            let build = || -> rquickjs::Result<Object> {
                let input = Object::new(ctx.clone())?;
                input.set("roundIndex", view.round)?;
                input.set("rounds", view.rounds)?;
                input.set("lastOwnAction", view.last.map(|(own, _)| name(own)))?;
                input.set("lastOpponentAction", view.last.map(|(_, opponent)| name(opponent)))?;
                input.set("opponentStreak", view.opponent_streak)?;
                input.set("outcomeStreak", view.outcome_streak)?;
                input.set("ownCounts", counts(view.own_counts)?)?;
                input.set("opponentCounts", counts(view.opponent_counts)?)?;
                input.set("ownScore", view.own_score as f64)?;
                input.set("opponentScore", view.opponent_score as f64)?;
                let allowed = Array::new(ctx.clone())?;
                for (i, decision) in decisions.iter().enumerate() {
                    allowed.set(i, decision.as_str())?;
                }
                let world = Object::new(ctx.clone())?;
                world.set("allowedDecisions", allowed)?;
                input.set("world", world)?;
                Ok(input)
            };
            let input = build().map_err(fail)?;
            let decide = self.decide.clone().restore(&ctx).map_err(fail)?;
            let answer: Value = decide.call((input,)).map_err(fail)?;
            let text: Option<String> = answer.as_string().and_then(|s| s.to_string().ok());
            match text.as_deref().and_then(|t| decisions.iter().position(|d| d == t)) {
                Some(index) => Ok(index as u8),
                None => {
                    let shown = match text {
                        Some(t) => format!("\"{}\"", t.chars().take(40).collect::<String>()),
                        None => format!("a value of type {}", answer.type_name()),
                    };
                    Err(format!("decide returned {shown}; it must return one of: {}", decisions.join(", ")))
                }
            }
        });
        self.run_left -= budget - self.call_left.get();
        result
    }
}

/// Turns an interpreter error into words for the strategy's author.
fn explain(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error, out_of_steps: bool) -> String {
    let thrown = if error.is_exception() { Some(ctx.catch()) } else { None };
    if out_of_steps {
        return "took too many steps (is there a loop that never ends?)".into();
    }
    let Some(thrown) = thrown else {
        return error.to_string();
    };
    let message = match thrown.as_exception() {
        Some(exception) => exception.message().unwrap_or_else(|| "an error was thrown".into()),
        None => thrown.as_string().and_then(|s| s.to_string().ok()).unwrap_or_else(|| "a value was thrown".into()),
    };
    let message: String = message.chars().take(200).collect();
    match message.as_str() {
        "out of memory" => "ran out of memory (the limit is 1 MB)".into(),
        "stack overflow" | "Maximum call stack size exceeded" => "nested calls too deeply (is a function calling itself without end?)".into(),
        _ => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cd() -> Vec<String> {
        vec!["C".into(), "D".into()]
    }

    fn first_round<'a>(zero: &'a [u32]) -> View<'a> {
        View { round: 0, rounds: 10, last: None, opponent_streak: 0, outcome_streak: 0, own_counts: zero, opponent_counts: zero, own_score: 0, opponent_score: 0 }
    }

    fn load(source: &str) -> Result<JsStrategy, String> {
        JsStrategy::load(source, &cd())
    }

    #[test]
    fn a_strategy_sees_its_input_and_answers_with_a_decision() {
        let mut s = load(
            r#"function decide(input) {
                if (input.roundIndex === 0) return input.world.allowedDecisions[0];
                return input.opponentCounts.D > input.opponentCounts.C ? "D" : input.lastOpponentAction;
            }"#,
        )
        .unwrap();
        let zero = [0, 0];
        assert_eq!(s.decide(&first_round(&zero)), Ok(0));
        let view = View { round: 3, rounds: 10, last: Some((0, 1)), opponent_streak: 1, outcome_streak: 1, own_counts: &[3, 0], opponent_counts: &[2, 1], own_score: 6, opponent_score: 11 };
        assert_eq!(s.decide(&view), Ok(1), "copies the opponent's last move");
        let view = View { opponent_counts: &[1, 2], last: Some((0, 0)), ..view };
        assert_eq!(s.decide(&view), Ok(1), "defects once the opponent has mostly defected");
    }

    #[test]
    fn problems_are_explained() {
        let zero = [0, 0];
        let message = |source: &str| match load(source) {
            Err(e) => e,
            Ok(mut s) => s.decide(&first_round(&zero)).unwrap_err(),
        };
        assert!(load("function decide( {").is_err(), "a syntax error is reported when loading");
        assert!(message("const x = 1;").contains("must define a function named decide"));
        assert_eq!(message("function decide() { return 'X'; }"), "decide returned \"X\"; it must return one of: C, D");
        assert_eq!(message("function decide() { return 1; }"), "decide returned a value of type int; it must return one of: C, D");
        assert_eq!(message("function decide() { throw new Error('boom'); }"), "boom");
        assert!(message("function decide(input) { return input.missing.field; }").contains("undefined"));
        assert!(message(&"//".repeat(MAX_SOURCE_BYTES)).contains("the limit is"));
    }

    #[test]
    fn endless_loops_deep_recursion_and_memory_hogs_are_stopped() {
        let zero = [0, 0];
        let message = |source: &str| match load(source) {
            Err(e) => e,
            Ok(mut s) => s.decide(&first_round(&zero)).unwrap_err(),
        };
        assert_eq!(message("function decide() { while (true) {} }"), "took too many steps (is there a loop that never ends?)");
        assert_eq!(message("while (true) {}"), "took too many steps (is there a loop that never ends?)", "also while loading");
        assert!(message("function decide() { return decide(); }").contains("nested calls too deeply"));
        assert!(message("function decide() { const a = []; while (true) a.push(new Array(1000).fill(7)); }").contains("memory"));
        assert!(message("const S = 'a'.repeat(4000000); function decide() { return 'C'; }").contains("memory"), "large values cannot be built");
    }

    #[test]
    fn the_run_allowance_is_shared_by_all_calls() {
        // Each call burns about 100 ticks: under the per-call limit of 200.
        let mut s = load("function decide() { let n = 0; for (let i = 0; i < 500000; i++) n += i; return 'C'; }").unwrap();
        s.run_left = 450;
        let zero = [0, 0];
        let results: Vec<Result<u8, String>> = (0..7).map(|_| s.decide(&first_round(&zero))).collect();
        assert_eq!(results[..4], [Ok(0), Ok(0), Ok(0), Ok(0)]);
        assert_eq!(results[4], Err("took too many steps (is there a loop that never ends?)".into()), "the fifth call gets only what is left");
        assert_eq!(results[6], Err("used up its computing allowance for the run".into()));
    }

    #[test]
    fn there_is_no_clock_randomness_or_module_loading() {
        let zero = [0, 0];
        let mut s = load(
            r#"function decide() {
                const gone = [typeof Date, typeof Math.random, typeof performance, typeof require, typeof fetch, typeof process, typeof std, typeof os];
                return gone.every((t) => t === 'undefined') ? 'C' : 'D';
            }"#,
        )
        .unwrap();
        assert_eq!(s.decide(&first_round(&zero)), Ok(0));
        assert!(load("import fs from 'fs'; function decide() { return 'C'; }").is_err());
    }
}
