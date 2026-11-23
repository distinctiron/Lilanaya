//! Strategy authoring: list, read, create, edit, delete, preview and
//! plain-English compile. Everything that writes or computes needs a signed-in user.

use super::{find_world, parse_json, ApiResult, Shared};
use crate::auth::{hash_token, new_token, User, Viewer};
use crate::config::{Allowance, BLOCKED_DESCRIPTIONS_PER_DAY};
use crate::error::ApiError;
use crate::isolate::{self, Failure};
use crate::llm::{LlmError, Spent};
use crate::store::{CompileGate, CompileLimits, StrategyRow};
use crate::strategies::{self, CodeStrategy, CompileError, StrategyDoc, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, MAX_PROMPT_CHARS};
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use lilanaya_engine::{Issue, Strategy, Transition, World};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const DEFAULT_WORLD: &str = "prisoners-dilemma";
/// A stop against filling the database, far above what a person writes. It is not a plan limit:
/// those are on AI requests (`Config::ai_allowance`).
const MAX_OWN_STRATEGIES: i64 = 1000;

/// The stored document plus who owns it and whether the viewer does.
fn strategy_json(row: &StrategyRow, viewer: Option<i64>) -> Value {
    let mine = row.owner_id.is_some() && row.owner_id == viewer;
    let mut value = json!(row.doc);
    value["mode"] = json!(row.doc.mode());
    value["builtin"] = json!(row.builtin);
    value["visibility"] = json!(row.visibility);
    value["world_id"] = json!(row.world_id);
    value["owner_name"] = json!(row.owner_name);
    value["mine"] = json!(mine);
    value["updated_at"] = json!(row.updated_at);
    if mine {
        value["prompt"] = json!(row.prompt);
    }
    value
}

/// Public strategies and the viewer's own, built-ins first.
pub async fn list(State(state): Shared, viewer: Viewer) -> ApiResult {
    let rows = state.store.strategies(viewer.id()).await?;
    Ok(Json(json!({ "strategies": rows.iter().map(|r| strategy_json(r, viewer.id())).collect::<Vec<_>>() })))
}

/// Someone else's private strategy answers "not found".
async fn visible(state: &crate::worker::AppState, id: &str, viewer: Option<i64>) -> Result<StrategyRow, ApiError> {
    let missing = || ApiError::not_found(format!("Strategy \"{id}\""));
    let row = state.store.strategy(id).await?.ok_or_else(missing)?;
    if row.visibility == "private" && (row.owner_id.is_none() || row.owner_id != viewer) {
        return Err(missing());
    }
    Ok(row)
}

pub async fn get_one(State(state): Shared, viewer: Viewer, Path(id): Path<String>) -> ApiResult {
    Ok(Json(strategy_json(&visible(&state, &id, viewer.id()).await?, viewer.id())))
}

/// What an author sends. `mode` picks which fields count: `rules` reads
/// `states`, `initial_state`, `action_map`, `transitions`; `code` reads `source`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    mode: String,
    visibility: Option<String>,
    world_id: Option<String>,
    prompt: Option<String>,
    states: Option<Vec<String>>,
    initial_state: Option<String>,
    action_map: Option<BTreeMap<String, String>>,
    transitions: Option<Vec<Transition>>,
    source: Option<String>,
}

impl StrategyBody {
    /// The document this body describes, with every shape problem found.
    fn into_doc(self, id: &str, issues: &mut Vec<Issue>) -> Option<StrategyDoc> {
        let name = self.name.trim().to_string();
        let description = self.description.trim().to_string();
        match self.mode.as_str() {
            "rules" => {
                if self.source.is_some() {
                    issues.push(Issue::new("source", "a rules strategy has no code"));
                }
                Some(StrategyDoc::Rules(Strategy {
                    id: id.into(),
                    name,
                    description,
                    states: self.states.unwrap_or_default(),
                    initial_state: self.initial_state.unwrap_or_default(),
                    action_map: self.action_map.unwrap_or_default(),
                    transitions: self.transitions.unwrap_or_default(),
                }))
            }
            "code" => {
                if self.states.is_some() || self.transitions.is_some() || self.action_map.is_some() || self.initial_state.is_some() {
                    issues.push(Issue::new("states", "a code strategy has no states or transitions"));
                }
                match self.source {
                    Some(source) => Some(StrategyDoc::Code(CodeStrategy { id: id.into(), name, description, language: "javascript".into(), source })),
                    None => {
                        issues.push(Issue::new("source", "is required for a code strategy"));
                        None
                    }
                }
            }
            other => {
                issues.push(Issue::new("mode", format!("must be \"rules\" or \"code\", got \"{other}\"")));
                None
            }
        }
    }
}

/// Everything a save needs, checked: the document, where it is visible, its world and its prompt.
struct Checked {
    doc: StrategyDoc,
    visibility: String,
    world_id: String,
    prompt: Option<String>,
}

/// Counts one strategy check against the user's hourly allowance; 429 when it is used up.
async fn count_check(state: &crate::worker::AppState, user: &User) -> Result<(), ApiError> {
    let limit = state.config.previews_per_hour;
    if state.store.record_preview(user.id, limit).await? {
        return Ok(());
    }
    Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", format!("You can check or save a strategy {limit} times an hour. Try again in a little while.")))
}

/// The preview of a draft: problems, warnings and sample matches. The draft is run in a
/// child process that is stopped when it takes too long, and only a few run at a time.
async fn run_preview(state: &crate::worker::AppState, doc: StrategyDoc, world: World) -> Result<Value, ApiError> {
    let _slot = state.sandbox.acquire().await.map_err(|e| ApiError::internal(format!("preview: {e}")))?;
    match isolate::preview(doc, world, isolate::PREVIEW_LIMIT).await {
        Ok(report) => Ok(report),
        Err(Failure::TimedOut(limit)) => {
            let message = format!("took longer than {} seconds to play its sample matches and was stopped; a strategy must decide in a moment", limit.as_secs());
            Ok(json!({ "valid": false, "errors": [Issue::new("source", message)], "warnings": [], "samples": [] }))
        }
        Err(Failure::Broken(detail)) => Err(ApiError::internal(format!("preview: {detail}"))),
    }
}

async fn check(state: &crate::worker::AppState, body: StrategyBody, id: &str) -> Result<Checked, ApiError> {
    let mut issues = Vec::new();
    let visibility = body.visibility.clone().unwrap_or_else(|| "private".into());
    if visibility != "public" && visibility != "private" {
        issues.push(Issue::new("visibility", "must be \"public\" or \"private\""));
    }
    let world_id = body.world_id.clone().unwrap_or_else(|| DEFAULT_WORLD.into());
    let world = state.store.world(&world_id).await?;
    if world.is_none() {
        issues.push(Issue::new("world_id", format!("there is no world \"{world_id}\"")));
    }
    let prompt = body.prompt.clone().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if prompt.as_ref().is_some_and(|p| p.chars().count() > MAX_PROMPT_CHARS) {
        issues.push(Issue::new("prompt", format!("must be at most {MAX_PROMPT_CHARS} characters")));
    }
    let name_chars = body.name.trim().chars().count();
    if name_chars == 0 || name_chars > MAX_NAME_CHARS {
        issues.push(Issue::new("name", format!("must be 1 to {MAX_NAME_CHARS} characters")));
    }
    if body.description.trim().chars().count() > MAX_DESCRIPTION_CHARS {
        issues.push(Issue::new("description", format!("must be at most {MAX_DESCRIPTION_CHARS} characters")));
    }
    let doc = body.into_doc(id, &mut issues);
    if prompt.as_deref().is_some_and(|p| p.contains('\0')) || doc.as_ref().is_some_and(|doc| has_nul(&json!(doc))) {
        issues.push(Issue::new("", "must not contain the NUL character, which cannot be stored"));
    }
    if let (Some(doc), Some(world)) = (&doc, &world) {
        // The full check, sample matches included, so code that fails while playing is not saved.
        let report = run_preview(state, doc.clone(), world.clone()).await?;
        issues.extend(serde_json::from_value::<Vec<Issue>>(report["errors"].clone()).unwrap_or_default());
    }
    match doc {
        Some(doc) if issues.is_empty() => Ok(Checked { doc, visibility, world_id, prompt }),
        _ => Err(ApiError::invalid("The strategy is not valid", issues)),
    }
}

/// Whether any text in `value` (a name or a value, at any depth) holds the NUL character.
/// Postgres cannot store it, so a document with one in it cannot be saved.
fn has_nul(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(has_nul),
        Value::Object(fields) => fields.iter().any(|(name, value)| name.contains('\0') || has_nul(value)),
        _ => false,
    }
}

/// A readable id that will not clash: the name as a slug plus six random hex digits.
fn new_id(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('_') {
            slug.push('_');
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_end_matches('_');
    format!("{}-{}", if slug.is_empty() { "strategy" } else { slug }, &new_token()[..6])
}

pub async fn create(State(state): Shared, user: User, body: Result<Bytes, BytesRejection>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let body: StrategyBody = parse_json(body)?;
    count_check(&state, &user).await?;
    let id = new_id(&body.name);
    let checked = check(&state, body, &id).await?;
    if !state.store.insert_strategy(&checked.doc, user.id, &checked.visibility, &checked.world_id, checked.prompt.as_deref(), MAX_OWN_STRATEGIES).await? {
        return Err(ApiError::new(StatusCode::CONFLICT, "limit_reached", format!("You already have {MAX_OWN_STRATEGIES} strategies; delete one first")));
    }
    let row = state.store.strategy(&id).await?.ok_or_else(|| ApiError::internal("the strategy just saved is missing"))?;
    Ok((StatusCode::CREATED, Json(strategy_json(&row, Some(user.id)))))
}

/// Replaces one of the user's own strategies. Its mode cannot change.
pub async fn update(State(state): Shared, user: User, Path(id): Path<String>, body: Result<Bytes, BytesRejection>) -> ApiResult {
    let body: StrategyBody = parse_json(body)?;
    let existing = visible(&state, &id, Some(user.id)).await?;
    if existing.owner_id != Some(user.id) || existing.builtin {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "forbidden", "Only its owner can change a strategy"));
    }
    if body.mode != existing.doc.mode() {
        return Err(ApiError::invalid(
            "The strategy is not valid",
            vec![Issue::new("mode", format!("this strategy was created as \"{}\" and that cannot be changed", existing.doc.mode()))],
        ));
    }
    count_check(&state, &user).await?;
    let checked = check(&state, body, &id).await?;
    if !state.store.update_strategy(&checked.doc, user.id, &checked.visibility, &checked.world_id, checked.prompt.as_deref()).await? {
        return Err(ApiError::not_found(format!("Strategy \"{id}\"")));
    }
    let row = state.store.strategy(&id).await?.ok_or_else(|| ApiError::not_found(format!("Strategy \"{id}\"")))?;
    Ok(Json(strategy_json(&row, Some(user.id))))
}

pub async fn delete(State(state): Shared, user: User, Path(id): Path<String>) -> ApiResult {
    let existing = visible(&state, &id, Some(user.id)).await?;
    if existing.owner_id != Some(user.id) || existing.builtin {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "forbidden", "Only its owner can delete a strategy"));
    }
    state.store.delete_strategy(&id, user.id).await?;
    Ok(Json(json!({ "deleted": id })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewBody {
    world_id: Option<String>,
    strategy: StrategyBody,
}

/// Checks a draft without saving it and plays two short sample matches.
/// Always 200 for a well-formed draft: `{valid, errors, warnings, samples}`. Uses no run
/// quota, but checks are limited per hour (429).
pub async fn preview(State(state): Shared, user: User, body: Result<Bytes, BytesRejection>) -> ApiResult {
    let req: PreviewBody = parse_json(body)?;
    let world = find_world(&state, req.world_id.as_deref().unwrap_or(DEFAULT_WORLD)).await?;
    let mut issues = Vec::new();
    let Some(doc) = req.strategy.into_doc("draft", &mut issues).filter(|_| issues.is_empty()) else {
        return Ok(Json(json!({ "valid": false, "errors": issues, "warnings": [], "samples": [] })));
    };
    count_check(&state, &user).await?;
    Ok(Json(run_preview(&state, doc, world).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompileBody {
    world_id: Option<String>,
    prompt: String,
}

/// Turns a plain-English description into rules with the language model.
/// The rules come back for the editor (not saved), with the model's reasoning,
/// the flags raised about the description (`flags`: rule violations and assumptions)
/// and any problems the validator still finds.
///
/// The description is screened before every call to the model (`guard.rs`); one that
/// is turned down answers 422 `prompt_blocked` and never reaches the model.
pub async fn compile(State(state): Shared, user: User, body: Result<Bytes, BytesRejection>) -> ApiResult {
    let req: CompileBody = parse_json(body)?;
    let Some(llm) = &state.llm else {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "compile_unavailable", "Plain-English compile is not set up on this server. Build the rules by hand instead."));
    };
    let prompt = req.prompt.trim();
    let length = prompt.chars().count();
    if length < 10 || length > MAX_PROMPT_CHARS {
        return Err(ApiError::invalid("The description is not valid", vec![Issue::new("prompt", format!("must be 10 to {MAX_PROMPT_CHARS} characters"))]));
    }
    if prompt.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(ApiError::invalid("The description is not valid", vec![Issue::new("prompt", "must be plain text")]));
    }
    let world = find_world(&state, req.world_id.as_deref().unwrap_or(DEFAULT_WORLD)).await?;
    let limits = CompileLimits {
        per_hour: state.config.compiles_per_hour,
        blocked_per_day: BLOCKED_DESCRIPTIONS_PER_DAY,
        per_day: state.config.compiles_per_day,
        tokens_per_day: state.config.compile_tokens_per_day,
        allowance: state.config.ai_allowance(&user.plan),
    };
    let by_hand = "Build the rules by hand instead.";
    let call = match state.store.begin_llm_call(user.id, &world.id, limits).await? {
        CompileGate::Started(call) => call,
        CompileGate::AllowanceUsed => {
            let message = match limits.allowance {
                Allowance::PerWorld(n) => format!("On the free plan the AI writes {n} strategies for you per world, and you have used them all in this one. They do not come back. You can still build rules by hand or write the strategy in code, as often as you like. The Plans page has more."),
                Allowance::PerMonth(n) => format!("Your plan has the AI write {n} strategies a month for you, and you have used them all. They come back on {}. You can still build rules by hand or write the strategy in code, as often as you like. A pack on the Plans page gives you more straight away.", super::day(&state.store.usage(user.id).await?.period_end)),
            };
            return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "ai_limit_reached", message));
        }
        CompileGate::HourlyLimit => {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                format!("You can compile {} descriptions an hour. Try again later, or edit the rules by hand.", limits.per_hour),
            ))
        }
        CompileGate::Suspended => {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "compile_suspended",
                format!("Too many of your descriptions were not accepted today, so plain-English compile is closed to you for 24 hours. {by_hand}"),
            ))
        }
        CompileGate::DailyCeiling => {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "compile_unavailable",
                format!("Plain-English compile has reached its limit for today and opens again tomorrow. {by_hand}"),
            ))
        }
    };
    // Names the asker to the provider (so abuse can be traced to one of our users) without saying who they are.
    let asker = &hash_token(&format!("lilanaya-user-{}", user.id))[..32];
    let mut spent = Spent::default();
    let outcome = strategies::compile(llm, &world, prompt, asker, &mut spent).await;
    // The request is used up once the language model has been asked, whatever it answered: that is what is paid for.
    // It is given back only when the model was never reached.
    let counted = !matches!(outcome, Err(CompileError::Llm(LlmError::Blocked(_) | LlmError::Unavailable(_))));
    if let Err(e) = state.store.finish_llm_call(call, &spent, counted).await {
        eprintln!("compile: could not record call {call}: {e}");
    }
    match outcome {
        Ok(compiled) => {
            let s = &compiled.strategy;
            Ok(Json(json!({
                "strategy": {
                    "name": s.name, "description": s.description, "states": s.states,
                    "initial_state": s.initial_state, "action_map": s.action_map, "transitions": s.transitions,
                },
                "rationale": compiled.rationale,
                "flags": compiled.flags,
                "valid": compiled.errors.is_empty(),
                "errors": compiled.errors,
            })))
        }
        Err(CompileError::BreaksRules(message)) => Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "rule_violation", message)),
        Err(CompileError::NotAStrategy(message)) | Err(CompileError::Llm(LlmError::NoAnswer(message))) => {
            Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "not_compiled", message))
        }
        Err(CompileError::Llm(LlmError::Blocked(verdict))) => {
            eprintln!("compile: user {} blocked by the screen as {} ({:.2})", user.id, verdict.label, verdict.confidence);
            Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "prompt_blocked", verdict.message()))
        }
        Err(CompileError::Llm(LlmError::Unavailable(detail))) => {
            eprintln!("language model unavailable: {detail}");
            Err(ApiError::new(StatusCode::BAD_GATEWAY, "compile_unavailable", format!("Plain-English compile could not be reached. Try again in a moment, or build the rules by hand.")))
        }
    }
}
