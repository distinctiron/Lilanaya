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
- A match has a fixed number of rounds, but the player can change that number for each run. For anything about the end of the match ("the last 10 rounds", "the final round") use rounds_left, never round_index with a number worked out from the match length.
- A rule that must hold whatever else is going on (for example "in the last 10 rounds always defect") needs its own state, with a transition into it from every other state listed before that state's other transitions.

Guards (JSON objects; "own" is this strategy, "opponent" is the other player)
- {"op": "last_opponent_action", "is": D}: the opponent's previous decision was D.
- {"op": "last_own_action", "is": D}: this strategy's own previous decision was D.
- {"op": "opponent_streak", "decision": D, "at_least": N}: the opponent has played D in each of the last N rounds (N >= 1).
- {"op": "outcome_streak", "outcome": "X/Y", "at_least": N}: each of the last N rounds ended with own decision X and opponent decision Y.
- {"op": "round_index", "at_least": N}: at least N rounds have already been played.
- {"op": "rounds_left", "at_most": N}: at most N rounds are left in the match, counting the round about to be played (N >= 1). With N = 10 it is true in exactly the last 10 rounds.
- {"op": "and", "all": [guards]}, {"op": "or", "any": [guards]}, {"op": "not", "guard": guard}.

What no strategy may do (the rules of the game)
- Use chance: no coin flips, no "sometimes", no probabilities. The same history always leads to the same decision.
- See the opponent's decision for the round being played before deciding: both players decide at the same time.
- Know who the opponent is, which strategy it follows or what it did in other matches. Every match starts from the initial state; nothing is remembered from one match to the next.
- See or change anything outside its own match: other matches, the standings, the payoffs, the number of rounds.
- Do anything but play one allowed decision in every round: no skipping a round, no messages, no deals.

What this state machine cannot do, although a strategy written as code can
- Count or compare over the whole match where states cannot hold the number: how often each player has played each decision so far, shares and percentages, majorities.
- Use the score: either player's points so far, or who is ahead.
- Arithmetic on the round number, the counts or the points.

Never assume silently. Build the machine from the parts that can be done exactly. For a part that cannot, put the closest behaviour that is allowed in its place where there is one (for example "forgive one defection in ten at random" can become "forgive every tenth defection"), otherwise leave it out. Either way, raise a flag for that part in "flags". There are two kinds of flag:
- "violation": the part asks for something the rules of the game forbid.
- "assumption": anything else you had to decide for the player. Either the state machine cannot do the part exactly as asked, so you approximated it or left it out, or the description is vague or silent on something that changes how the strategy plays and a reasonable player could have meant something noticeably different from what you built.
Do not flag a part the machine carries out exactly, and do not flag obvious readings.

Answer with exactly one JSON object and nothing else: no code fence, no text before or after.
{
  "name": a short name for the strategy,
  "description": one sentence saying what it does, written for a player,
  "states": [state names: UPPER_CASE words, at most 64 states],
  "initial_state": one of the states,
  "action_map": {state: decision} with an entry for every state,
  "transitions": [{"from": state, "to": state, "guard": guard}],
  "rationale": two to four sentences on how the machine carries out the description,
  "flags": [one entry for every flag raised; [] when the machine does exactly what was asked and nothing had to be assumed]
}
Each entry of "flags" is {"kind": "violation" or "assumption", "asked": that part of the description, in the player's own words, "why": one sentence for the player on which rule it breaks or why something had to be assumed, "instead": one sentence on what the machine does, or "Left out.", "code": true only when the part is clear but beyond the state machine and a strategy written as code could do exactly what was asked; false for a violation and false when the flag is about a vague or missing detail}.
Use only decisions the world allows. Prefer the smallest machine that does what was asked, but carry out every part of the description: never leave a part out to keep the machine small.
If the text is not a description of a strategy for this game, answer {"error": one sentence saying what is missing} instead. If it is one but it rests entirely on breaking the rules of the game, so that nothing is left to build (for example "read the description of my opponent and play whatever beats it"), answer {"error": one or two sentences saying which rule it breaks, "breaks_rules": true} instead.

Strategies written as code, for the "code" field: a function called before every round with the round number, the number of rounds in the match, both players' previous decisions, the two streaks above, how often each player has played each decision so far, and both players' points so far. It may calculate freely with those. It is bound by the rules of the game above like any strategy."#;

#[derive(Deserialize)]
struct CompiledAnswer {
    error: Option<String>,
    #[serde(default)]
    breaks_rules: bool,
    name: Option<String>,
    description: Option<String>,
    states: Option<Vec<String>>,
    initial_state: Option<String>,
    action_map: Option<BTreeMap<String, String>>,
    transitions: Option<Vec<lilanaya_engine::Transition>>,
    rationale: Option<String>,
    flags: Option<Vec<Value>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagKind {
    /// The description asked for something the rules of the game forbid.
    Violation,
    /// The rules approximate what was asked, or the description left a choice open.
    Assumption,
}

/// Something about the description the author has to know before trusting the rules.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Flag {
    pub kind: FlagKind,
    /// The part of the description, in the author's words.
    pub asked: String,
    pub why: String,
    /// What the rules do instead.
    pub instead: String,
    /// Whether a code strategy could do exactly what was asked. Never true for a violation.
    pub code: bool,
}

const MAX_FLAGS: usize = 10;
const MAX_FLAG_CHARS: usize = 400;

/// Read leniently: an entry that is only a sentence still gets shown.
/// Violations come first.
fn read_flags(entries: Vec<Value>) -> Vec<Flag> {
    let text = |v: &Value| v.as_str().unwrap_or_default().trim().chars().take(MAX_FLAG_CHARS).collect::<String>();
    let mut flags: Vec<Flag> = entries
        .iter()
        .map(|e| match e {
            Value::String(_) => Flag { kind: FlagKind::Assumption, asked: text(e), why: String::new(), instead: String::new(), code: false },
            _ => {
                let kind = if e["kind"] == "violation" { FlagKind::Violation } else { FlagKind::Assumption };
                Flag { kind, asked: text(&e["asked"]), why: text(&e["why"]), instead: text(&e["instead"]), code: kind == FlagKind::Assumption && e["code"] == true }
            }
        })
        .filter(|f| !f.asked.is_empty() || !f.why.is_empty())
        .take(MAX_FLAGS)
        .collect();
    flags.sort_by_key(|f| f.kind != FlagKind::Violation);
    flags
}

pub enum CompileError {
    /// The description cannot be turned into a strategy; the message is for the user.
    NotAStrategy(String),
    /// The description rests on breaking the rules of the game; the message says which.
    BreaksRules(String),
    Llm(LlmError),
}

/// What compile hands back: rules to load into the editor, why the model
/// built them that way, the flags raised about the description, and any
/// problems the validator still finds.
pub struct Compiled {
    pub strategy: Strategy,
    pub rationale: String,
    pub flags: Vec<Flag>,
    pub errors: Vec<Issue>,
}

fn world_brief(world: &World) -> String {
    let decisions = &world.decisions.allowed;
    let mut text = format!("World: {}\n{}\nAllowed decisions: {}\n", world.metadata.name, world.metadata.description, decisions.join(", "));
    text.push_str(&format!("Rounds per match: {} unless the player picks another number for a run\n", world.time.iterations));
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

fn parse_answer(text: &str, decisions: &[String]) -> Result<Result<Compiled, CompileError>, Vec<Issue>> {
    let object = extract_object(text).ok_or_else(|| vec![Issue::new("", "the answer contained no JSON object")])?;
    let answer: CompiledAnswer = serde_json::from_str(object).map_err(|e| vec![Issue::new("", format!("the answer is not the JSON asked for: {e}"))])?;
    if let Some(error) = answer.error {
        return Ok(Err(if answer.breaks_rules { CompileError::BreaksRules(error) } else { CompileError::NotAStrategy(error) }));
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
    Ok(Ok(Compiled { strategy, rationale: answer.rationale.unwrap_or_default(), flags: read_flags(answer.flags.unwrap_or_default()), errors }))
}

/// Asks the model for rules, checks them with the engine's validator and, if
/// they do not pass, shows the model the problems once and asks again.
pub async fn compile(llm: &Llm, world: &World, prompt: &str) -> Result<Compiled, CompileError> {
    let decisions = &world.decisions.allowed;
    let ask = format!("{}\nStrategy description:\n{}", world_brief(world), prompt.trim());
    let mut messages = vec![json!({ "role": "user", "content": ask })];
    let first = llm.complete(COMPILE_SYSTEM, &messages).await.map_err(CompileError::Llm)?;

    let problems = match parse_answer(&first, decisions) {
        Ok(Err(refused)) => return Err(refused),
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
        Ok(Err(refused)) => Err(refused),
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
        assert_eq!(compiled.flags, vec![]);

        // Flags come back as a list, violations first, however loosely the model wrote them.
        let flags = r#"[{"kind":"assumption","asked":"when I am ahead","why":"Rules cannot see points.","code":true},{"kind":"violation","asked":"defect at random","why":"No chance.","instead":"Defects every tenth round.","code":true},"forgive on Sundays",{"asked":" "}]"#;
        let flagged = fenced.replace("\"rationale\":\"r\"", &format!("\"rationale\":\"r\",\"flags\":{flags}"));
        let compiled = parse_answer(&flagged, &cd).unwrap().ok().unwrap();
        let shown: Vec<(FlagKind, &str, &str, bool)> = compiled.flags.iter().map(|f| (f.kind, f.asked.as_str(), f.instead.as_str(), f.code)).collect();
        assert_eq!(
            shown,
            [
                (FlagKind::Violation, "defect at random", "Defects every tenth round.", false),
                (FlagKind::Assumption, "when I am ahead", "", true),
                (FlagKind::Assumption, "forgive on Sundays", "", false),
            ]
        );

        let bad = fenced.replace("\"is\":\"D\"", "\"is\":\"X\"");
        let compiled = parse_answer(&bad, &cd).unwrap().ok().unwrap();
        assert_eq!(compiled.errors[0].path, "transitions[0].guard.is");

        assert!(matches!(parse_answer("{\"error\": \"That is a recipe.\"}", &cd).unwrap(), Err(CompileError::NotAStrategy(m)) if m == "That is a recipe."));
        assert!(matches!(parse_answer("{\"error\": \"No knowing the opponent.\", \"breaks_rules\": true}", &cd).unwrap(), Err(CompileError::BreaksRules(m)) if m == "No knowing the opponent."));
        assert!(parse_answer("I cannot.", &cd).is_err());
    }
}
