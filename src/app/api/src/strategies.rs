//! Strategies as the API stores them: rules (the engine's state machine) or
//! code (JavaScript run in the sandbox). Also: turning stored actors into
//! engine entrants, the static preview, and plain-English compile.

use crate::llm::{Llm, LlmError};
use crate::sandbox::JsStrategy;
use crate::store::RunInput;
use lilanaya_engine::{baseline_strategies, run_entrants, Entrant, Issue, Logic, Policy, RunOutput, Strategy, World};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_DESCRIPTION_CHARS: usize = 500;
pub const MAX_PROMPT_CHARS: usize = 2000;
const PREVIEW_ROUNDS: u32 = 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CodeStrategy {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Always `javascript` today.
    pub language: String,
    pub source: String,
}

/// A stored strategy document. Told apart by shape: code has `source`, rules have `states`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StrategyDoc {
    Code(CodeStrategy),
    Rules(Strategy),
}

impl StrategyDoc {
    pub fn id(&self) -> &str {
        match self {
            StrategyDoc::Code(c) => &c.id,
            StrategyDoc::Rules(r) => &r.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            StrategyDoc::Code(c) => &c.name,
            StrategyDoc::Rules(r) => &r.name,
        }
    }

    pub fn mode(&self) -> &'static str {
        match self {
            StrategyDoc::Code(_) => "code",
            StrategyDoc::Rules(_) => "rules",
        }
    }

    /// Problems that stop the strategy from playing in a world with these
    /// decisions. For code this compiles it but does not run it; see [`preview`].
    pub fn validate(&self, decisions: &[String]) -> Vec<Issue> {
        match self {
            StrategyDoc::Rules(r) => r.validate(decisions),
            StrategyDoc::Code(c) => match JsStrategy::load(&c.source, decisions) {
                Ok(_) => Vec::new(),
                Err(message) => vec![Issue::new("source", message)],
            },
        }
    }
}

/// A participant as frozen in a run's input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunActor {
    pub id: String,
    pub name: String,
    pub strategy: StrategyDoc,
}

fn entrant(id: &str, name: &str, doc: &StrategyDoc, decisions: &[String]) -> Result<Entrant, String> {
    let logic = match doc {
        StrategyDoc::Rules(r) => Logic::Fsm(r.clone()),
        StrategyDoc::Code(c) => Logic::Custom(Box::new(JsStrategy::load(&c.source, decisions)?)),
    };
    Ok(Entrant { id: id.into(), name: name.into(), strategy_id: doc.id().into(), logic })
}

/// Runs a queued run. Blocking and CPU-bound: call it off the async threads.
pub fn execute(input: &RunInput) -> Result<RunOutput, Vec<Issue>> {
    let decisions = &input.world.decisions.allowed;
    let mut entrants = Vec::with_capacity(input.actors.len());
    let mut issues = Vec::new();
    for actor in &input.actors {
        match entrant(&actor.id, &actor.name, &actor.strategy, decisions) {
            Ok(e) => entrants.push(e),
            Err(message) => issues.push(Issue::new(format!("actors.{}", actor.id), format!("{} could not be loaded: {message}", actor.name))),
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    run_entrants(&input.world, entrants, input.seed)
}

/// A strategy that plays one decision for ever.
fn always(decision: &str, id: &str, name: &str) -> Strategy {
    Strategy {
        id: id.into(),
        name: name.into(),
        description: String::new(),
        states: vec!["ONLY".into()],
        initial_state: "ONLY".into(),
        action_map: BTreeMap::from([("ONLY".to_string(), decision.to_string())]),
        transitions: Vec::new(),
    }
}

/// The opponents of the sample matches: one that always plays the world's
/// first decision and one that always plays its last (in the Prisoner's
/// Dilemma: Always Cooperate and Always Defect).
fn sample_opponents(world: &World) -> Vec<Strategy> {
    let decisions = &world.decisions.allowed;
    if decisions.len() == 2 && decisions[0] == "C" && decisions[1] == "D" {
        return baseline_strategies().into_iter().filter(|s| s.id == "always_cooperate" || s.id == "always_defect").collect();
    }
    let mut picks = vec![&decisions[0]];
    if decisions.len() > 1 {
        picks.push(&decisions[decisions.len() - 1]);
    }
    picks.into_iter().enumerate().map(|(i, d)| always(d, &format!("always_{i}"), &format!("Always {d}"))).collect()
}

/// The static preview: problems, warnings and short sample matches. Costs no quota.
/// `{valid, errors, warnings, samples: [{opponent, rounds, own_moves, opponent_moves, totals}]}`
pub fn preview(doc: &StrategyDoc, world: &World) -> Value {
    let decisions = &world.decisions.allowed;
    let mut errors = doc.validate(decisions);
    let warnings = match doc {
        StrategyDoc::Rules(r) if errors.is_empty() => r.warnings(),
        _ => Vec::new(),
    };
    let mut samples = Vec::new();
    if errors.is_empty() {
        let mut short = world.clone();
        short.time.iterations = PREVIEW_ROUNDS;
        short.interaction.policy = Policy::AllPairs;
        short.kpis.definitions.clear();
        for opponent in sample_opponents(world) {
            // Ids chosen so the draft is always the first of the pair.
            let entrants = entrant("a_draft", doc.name(), doc, decisions).map(|draft| {
                vec![draft, Entrant { id: "b_opponent".into(), name: opponent.name.clone(), strategy_id: opponent.id.clone(), logic: Logic::Fsm(opponent.clone()) }]
            });
            match entrants.map_err(|m| vec![Issue::new("source", m)]).and_then(|e| run_entrants(&short, e, world.determinism.rng_seed)) {
                Ok(out) => {
                    let names = |side: usize| -> Vec<&str> { out.traces[0].moves.iter().map(|m| decisions[m[side] as usize].as_str()).collect() };
                    samples.push(json!({
                        "opponent": { "id": opponent.id, "name": opponent.name },
                        "rounds": PREVIEW_ROUNDS,
                        "own_moves": names(0),
                        "opponent_moves": names(1),
                        "totals": out.result.matches[0].totals,
                    }));
                }
                Err(issues) => {
                    // A code strategy that loads but fails while playing.
                    for issue in issues {
                        let message = format!("against {}: {}", opponent.name, issue.message);
                        if !errors.iter().any(|e| e.message == message) {
                            errors.push(Issue::new("source", message));
                        }
                    }
                }
            }
        }
        if !errors.is_empty() {
            samples.clear();
        }
    }
    json!({ "valid": errors.is_empty(), "errors": errors, "warnings": warnings, "samples": samples })
}

// --- plain English to rules ---

const COMPILE_SYSTEM: &str = r#"You turn a plain-English description of a strategy for a repeated two-player game into a small state machine for the Lilanaya simulator.

How the machine runs
- The strategy is always in exactly one state. Every state plays one fixed decision.
- Before each round, the transitions that leave the current state are tried in the order they are listed. The first one whose guard is true is taken; if none is true, the strategy stays where it is. It then plays the decision of the state it is in.
- In the first round there is no previous round, so every guard about the previous round is false and the strategy plays the decision of the initial state.
- There is no randomness, no memory other than the current state, and no arithmetic. Anything the strategy must remember has to be a state.

Guards (JSON objects; "own" is this strategy, "opponent" is the other player)
- {"op": "last_opponent_action", "is": D}: the opponent's previous decision was D.
- {"op": "last_own_action", "is": D}: this strategy's own previous decision was D.
- {"op": "opponent_streak", "decision": D, "at_least": N}: the opponent has played D in each of the last N rounds (N >= 1).
- {"op": "outcome_streak", "outcome": "X/Y", "at_least": N}: each of the last N rounds ended with own decision X and opponent decision Y.
- {"op": "round_index", "at_least": N}: at least N rounds have already been played.
- {"op": "and", "all": [guards]}, {"op": "or", "any": [guards]}, {"op": "not", "guard": guard}.

Answer with exactly one JSON object and nothing else: no code fence, no text before or after.
{
  "name": a short name for the strategy,
  "description": one sentence saying what it does, written for a player,
  "states": [state names: UPPER_CASE words, at most 64 states],
  "initial_state": one of the states,
  "action_map": {state: decision} with an entry for every state,
  "transitions": [{"from": state, "to": state, "guard": guard}],
  "rationale": two to four sentences on how the machine carries out the description, naming any part of the description you could not express and what you did instead
}
Use only decisions the world allows. Prefer the smallest machine that does what was asked.
If the text is not a description of a strategy for this game, answer {"error": one sentence saying what is missing} instead."#;

#[derive(Deserialize)]
struct CompiledAnswer {
    error: Option<String>,
    name: Option<String>,
    description: Option<String>,
    states: Option<Vec<String>>,
    initial_state: Option<String>,
    action_map: Option<BTreeMap<String, String>>,
    transitions: Option<Vec<lilanaya_engine::Transition>>,
    rationale: Option<String>,
}

pub enum CompileError {
    /// The description cannot be turned into a strategy; the message is for the user.
    NotAStrategy(String),
    Llm(LlmError),
}

/// What compile hands back: rules to load into the editor, why the model
/// built them that way, and any problems the validator still finds.
pub struct Compiled {
    pub strategy: Strategy,
    pub rationale: String,
    pub errors: Vec<Issue>,
}

fn world_brief(world: &World) -> String {
    let decisions = &world.decisions.allowed;
    let mut text = format!("World: {}\n{}\nAllowed decisions: {}\n", world.metadata.name, world.metadata.description, decisions.join(", "));
    text.push_str("Payoffs per round (own decision / opponent decision -> own points, opponent points):\n");
    for own in decisions {
        for opponent in decisions {
            if let Some(p) = world.payoff.matrix.get(own).and_then(|row| row.get(opponent)) {
                text.push_str(&format!("- {own}/{opponent} -> {}, {}\n", p[0], p[1]));
            }
        }
    }
    text
}

/// The JSON object in a model answer, tolerating a code fence or stray text around it.
fn extract_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

fn parse_answer(text: &str, decisions: &[String]) -> Result<Result<Compiled, String>, Vec<Issue>> {
    let object = extract_object(text).ok_or_else(|| vec![Issue::new("", "the answer contained no JSON object")])?;
    let answer: CompiledAnswer = serde_json::from_str(object).map_err(|e| vec![Issue::new("", format!("the answer is not the JSON asked for: {e}"))])?;
    if let Some(error) = answer.error {
        return Ok(Err(error));
    }
    let strategy = Strategy {
        id: "draft".into(),
        name: answer.name.unwrap_or_default().chars().take(MAX_NAME_CHARS).collect(),
        description: answer.description.unwrap_or_default().chars().take(MAX_DESCRIPTION_CHARS).collect(),
        states: answer.states.unwrap_or_default(),
        initial_state: answer.initial_state.unwrap_or_default(),
        action_map: answer.action_map.unwrap_or_default(),
        transitions: answer.transitions.unwrap_or_default(),
    };
    let errors = strategy.validate(decisions);
    Ok(Ok(Compiled { strategy, rationale: answer.rationale.unwrap_or_default(), errors }))
}

/// Asks the model for rules, checks them with the engine's validator and, if
/// they do not pass, shows the model the problems once and asks again.
pub async fn compile(llm: &Llm, world: &World, prompt: &str) -> Result<Compiled, CompileError> {
    let decisions = &world.decisions.allowed;
    let ask = format!("{}\nStrategy description:\n{}", world_brief(world), prompt.trim());
    let mut messages = vec![json!({ "role": "user", "content": ask })];
    let first = llm.complete(COMPILE_SYSTEM, &messages).await.map_err(CompileError::Llm)?;

    let problems = match parse_answer(&first, decisions) {
        Ok(Err(not_a_strategy)) => return Err(CompileError::NotAStrategy(not_a_strategy)),
        Ok(Ok(compiled)) if compiled.errors.is_empty() => return Ok(compiled),
        Ok(Ok(compiled)) => compiled.errors,
        Err(problems) => problems,
    };
    let list: String = problems.iter().map(|p| if p.path.is_empty() { format!("- {}\n", p.message) } else { format!("- {}: {}\n", p.path, p.message) }).collect();
    messages.push(json!({ "role": "assistant", "content": first }));
    messages.push(json!({ "role": "user", "content": format!("The simulator's validator rejected that answer:\n{list}Answer again with the complete corrected JSON object and nothing else.") }));
    let second = llm.complete(COMPILE_SYSTEM, &messages).await.map_err(CompileError::Llm)?;
    match parse_answer(&second, decisions) {
        Ok(Ok(compiled)) => Ok(compiled),
        Ok(Err(not_a_strategy)) => Err(CompileError::NotAStrategy(not_a_strategy)),
        Err(_) => Err(CompileError::Llm(LlmError::NoAnswer("The model did not produce usable rules; try describing the strategy differently".into()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lilanaya_engine::prisoners_dilemma;

    fn code(source: &str) -> StrategyDoc {
        StrategyDoc::Code(CodeStrategy { id: "mine".into(), name: "Mine".into(), description: String::new(), language: "javascript".into(), source: source.into() })
    }

    #[test]
    fn stored_documents_are_told_apart_by_shape() {
        let rules = serde_json::to_value(&baseline_strategies()[2]).unwrap();
        assert!(matches!(serde_json::from_value::<StrategyDoc>(rules).unwrap(), StrategyDoc::Rules(_)));
        let doc = code("function decide() { return 'C'; }");
        let round_trip: StrategyDoc = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
        assert_eq!(round_trip, doc);
        assert_eq!(doc.mode(), "code");
    }

    #[test]
    fn preview_plays_two_sample_matches() {
        let tft = StrategyDoc::Rules(baseline_strategies().into_iter().find(|s| s.id == "tit_for_tat").unwrap());
        let p = preview(&tft, &prisoners_dilemma());
        assert_eq!(p["valid"], true);
        assert_eq!(p["warnings"], json!([]));
        assert_eq!(p["samples"][0]["opponent"]["name"], "Always Cooperate");
        assert_eq!(p["samples"][0]["totals"], json!([60, 60]));
        assert_eq!(p["samples"][1]["opponent"]["name"], "Always Defect");
        assert_eq!(p["samples"][1]["own_moves"].as_array().unwrap()[..3], [json!("C"), json!("D"), json!("D")]);
        assert_eq!(p["samples"][1]["totals"], json!([19, 24]));

        // The same strategy written as code plays the same matches.
        let js = code("function decide(input) { return input.lastOpponentAction ?? 'C'; }");
        assert_eq!(preview(&js, &prisoners_dilemma())["samples"], p["samples"]);
    }

    #[test]
    fn preview_reports_code_that_fails_only_while_playing() {
        let p = preview(&code("function decide(input) { if (input.opponentCounts.D >= 2) throw new Error('lost my temper'); return 'C'; }"), &prisoners_dilemma());
        assert_eq!(p["valid"], false);
        assert_eq!(p["samples"], json!([]));
        assert_eq!(p["errors"], json!([{ "path": "source", "message": "against Always Defect: Mine failed in round 3: lost my temper" }]));

        let p = preview(&code("let x = ;"), &prisoners_dilemma());
        assert_eq!(p["valid"], false);
        assert_eq!(p["errors"][0]["path"], "source");
    }

    #[test]
    fn answers_are_read_leniently_and_checked_strictly() {
        let cd = vec!["C".to_string(), "D".to_string()];
        let fenced = "```json\n{\"name\":\"Grim\",\"description\":\"d\",\"states\":[\"NICE\",\"GRIM\"],\"initial_state\":\"NICE\",\"action_map\":{\"NICE\":\"C\",\"GRIM\":\"D\"},\"transitions\":[{\"from\":\"NICE\",\"to\":\"GRIM\",\"guard\":{\"op\":\"last_opponent_action\",\"is\":\"D\"}}],\"rationale\":\"r\"}\n```";
        let compiled = parse_answer(fenced, &cd).unwrap().ok().unwrap();
        assert_eq!(compiled.strategy.name, "Grim");
        assert!(compiled.errors.is_empty());

        let bad = fenced.replace("\"is\":\"D\"", "\"is\":\"X\"");
        let compiled = parse_answer(&bad, &cd).unwrap().ok().unwrap();
        assert_eq!(compiled.errors[0].path, "transitions[0].guard.is");

        assert_eq!(parse_answer("{\"error\": \"That is a recipe.\"}", &cd).unwrap().err().unwrap(), "That is a recipe.");
        assert!(parse_answer("I cannot.", &cd).is_err());
    }
}
