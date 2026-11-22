//! Everything that executes a user's strategy runs in a child process: the
//! same program started with [`CHILD_ARG`], which reads one job from its
//! input, does it and prints the result.
//!
//! The reason is the clock. The sandbox counts interpreter steps, and a step
//! can be slow (a built-in working on a large value), so a strategy can stay
//! inside its step budget for minutes. A thread cannot be stopped from
//! outside; a process can. Each job gets a time limit and the child is killed
//! when it runs out. Being stopped is reported as a failure of the job, never
//! as a decision, so it cannot change the result of a match.
//!
//! A second gain: a fault in the interpreter takes down the child, not the API.

use crate::store::RunInput;
use crate::strategies::{self, StrategyDoc};
use lilanaya_engine::{Issue, RunOutput, World};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// The first argument that makes the program act as a child.
pub const CHILD_ARG: &str = "isolate-child";
/// The child's result is the line that starts with this; anything else it prints is ignored.
const MARKER: &str = "@@lilanaya-result@@ ";

/// How long a preview (two sample matches of 20 rounds) may take.
pub const PREVIEW_LIMIT: Duration = Duration::from_secs(5);
/// A run may take this long, plus [`RUN_PER_DECISION`] for every decision it asks for.
/// Measured in an unoptimised build: the largest run allowed (16 code strategies, 10,000
/// rounds, 2.4 million decisions) takes 28 seconds, about 12 microseconds a decision.
const RUN_BASE: Duration = Duration::from_secs(10);
const RUN_PER_DECISION: Duration = Duration::from_micros(40);

#[derive(Serialize, Deserialize)]
enum Job {
    Run(RunInput),
    Preview { doc: StrategyDoc, world: World },
}

#[derive(Serialize, Deserialize)]
enum Done {
    Run(Result<RunOutput, Vec<Issue>>),
    Preview(Value),
}

#[derive(Debug, PartialEq)]
pub enum Failure {
    /// The job was still going when its time ran out, and was stopped.
    TimedOut(Duration),
    /// The child could not be started, or ended without a result. The detail is for the log.
    Broken(String),
}

/// The time a run of this size may take before it is stopped.
pub fn run_limit(input: &RunInput) -> Duration {
    let actors = input.actors.len() as u32;
    // Every actor meets every other, and both sides decide in every round.
    let decisions = actors.saturating_mul(actors.saturating_sub(1)).saturating_mul(input.world.time.iterations);
    RUN_BASE + RUN_PER_DECISION.saturating_mul(decisions)
}

/// Runs a queued run in a child process.
pub async fn execute(input: RunInput, limit: Duration) -> Result<Result<RunOutput, Vec<Issue>>, Failure> {
    match run(&Job::Run(input), limit).await? {
        Done::Run(outcome) => Ok(outcome),
        Done::Preview(_) => Err(Failure::Broken("the child answered another job".into())),
    }
}

/// Checks a draft and plays its sample matches in a child process.
pub async fn preview(doc: StrategyDoc, world: World, limit: Duration) -> Result<Value, Failure> {
    match run(&Job::Preview { doc, world }, limit).await? {
        Done::Preview(report) => Ok(report),
        Done::Run(_) => Err(Failure::Broken("the child answered another job".into())),
    }
}

fn command() -> Result<Command, Failure> {
    let exe = std::env::current_exe().map_err(|e| Failure::Broken(format!("cannot find this program: {e}")))?;
    let mut command = Command::new(exe);
    #[cfg(not(test))]
    command.arg(CHILD_ARG);
    // Under `cargo test` this program is the test binary: start it on the one test that acts as the child.
    #[cfg(test)]
    command.args(["isolate::tests::child_process", "--exact", "--nocapture"]).env(tests::CHILD_ENV, "1");
    Ok(command)
}

async fn run(job: &Job, limit: Duration) -> Result<Done, Failure> {
    let request = serde_json::to_vec(job).map_err(|e| Failure::Broken(format!("write the job: {e}")))?;
    let mut child = command()?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        // Dropping the child (which the time limit below does) kills it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Failure::Broken(format!("start the child: {e}")))?;
    let mut stdin = child.stdin.take().ok_or_else(|| Failure::Broken("the child has no input".into()))?;
    let work = async move {
        stdin.write_all(&request).await.map_err(|e| Failure::Broken(format!("send the job: {e}")))?;
        drop(stdin);
        child.wait_with_output().await.map_err(|e| Failure::Broken(format!("wait for the child: {e}")))
    };
    let output = tokio::time::timeout(limit, work).await.map_err(|_| Failure::TimedOut(limit))??;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().rev().find_map(|l| l.strip_prefix(MARKER)).ok_or_else(|| Failure::Broken(format!("the child ended ({}) without a result", output.status)))?;
    serde_json::from_str(line).map_err(|e| Failure::Broken(format!("read the result: {e}")))
}

/// What the program does when started as a child: one job from input, its result to output.
pub fn child_main() -> ! {
    use std::io::{Read, Write};
    let mut request = Vec::new();
    let done = match std::io::stdin().read_to_end(&mut request).map_err(|e| e.to_string()).and_then(|_| serde_json::from_slice::<Job>(&request).map_err(|e| e.to_string())) {
        Ok(Job::Run(input)) => Done::Run(strategies::execute(&input)),
        Ok(Job::Preview { doc, world }) => Done::Preview(strategies::preview(&doc, &world)),
        Err(e) => {
            eprintln!("isolate child: cannot read the job: {e}");
            std::process::exit(2);
        }
    };
    let mut out = std::io::stdout().lock();
    let written = serde_json::to_string(&done).map_err(|e| e.to_string()).and_then(|json| writeln!(out, "\n{MARKER}{json}").and_then(|_| out.flush()).map_err(|e| e.to_string()));
    match written {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("isolate child: cannot write the result: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::strategies::{CodeStrategy, RunActor};
    use lilanaya_engine::prisoners_dilemma;
    use std::time::Instant;

    pub const CHILD_ENV: &str = "LILANAYA_ISOLATE_CHILD";

    /// Not a test of its own: this is where a child started by the other tests comes in.
    #[test]
    fn child_process() {
        if std::env::var_os(CHILD_ENV).is_some() {
            child_main();
        }
    }

    fn code(source: &str) -> StrategyDoc {
        StrategyDoc::Code(CodeStrategy { id: "mine".into(), name: "Mine".into(), description: String::new(), language: "javascript".into(), source: source.into() })
    }

    #[tokio::test]
    async fn a_preview_in_a_child_gives_the_same_report_as_one_in_process() {
        let doc = code("function decide(input) { return input.lastOpponentAction ?? 'C'; }");
        let world = prisoners_dilemma();
        let report = preview(doc.clone(), world.clone(), PREVIEW_LIMIT).await.unwrap();
        assert_eq!(report, strategies::preview(&doc, &world));
        assert_eq!(report["valid"], true);
    }

    #[tokio::test]
    async fn a_run_in_a_child_gives_the_engine_s_result() {
        let world = prisoners_dilemma();
        let actors: Vec<RunActor> = lilanaya_engine::baseline_strategies().into_iter().take(3).map(|s| RunActor { id: s.id.clone(), name: s.name.clone(), strategy: StrategyDoc::Rules(s) }).collect();
        let input = RunInput { world, actors, seed: 7 };
        let out = execute(input.clone(), run_limit(&input)).await.unwrap().unwrap();
        assert_eq!(out, strategies::execute(&input).unwrap());
        assert_eq!(out.traces.len(), 3);
    }

    /// The hole this module closes: every step of this strategy is a slow built-in, so it
    /// stays far inside its step budget (a handful of ticks a call) and runs for minutes.
    #[tokio::test]
    async fn a_strategy_that_is_slow_inside_its_step_budget_is_stopped_by_the_clock() {
        let slow = code(r#"const S = "a".repeat(400000); function decide() { let n = 0; for (let i = 0; i < 40000; i++) n += S.indexOf("b"); return "C"; }"#);
        let limit = Duration::from_secs(1);
        let started = Instant::now();
        assert_eq!(preview(slow, prisoners_dilemma(), limit).await, Err(Failure::TimedOut(limit)));
        assert!(started.elapsed() < Duration::from_secs(3), "stopped at the limit, not when the strategy is done: {:?}", started.elapsed());
    }

    #[test]
    fn bigger_runs_get_more_time() {
        let actor = |id: &str| RunActor { id: id.into(), name: id.into(), strategy: code("") };
        let mut input = RunInput { world: prisoners_dilemma(), actors: vec![actor("a"), actor("b")], seed: 1 };
        input.world.time.iterations = 100;
        assert_eq!(run_limit(&input), RUN_BASE + Duration::from_millis(8));
        input.actors = (0..16).map(|i| actor(&format!("s{i}"))).collect();
        input.world.time.iterations = 10_000;
        assert_eq!(run_limit(&input), RUN_BASE + Duration::from_secs(96));
    }
}
