//! Strategy authoring: list, read, create, edit, delete, preview and
//! plain-English compile. Everything that writes or computes needs a signed-in user.

use super::{find_world, parse_json, ApiResult, Shared};
use crate::auth::{new_token, User, Viewer};
use crate::error::ApiError;
use crate::llm::LlmError;
use crate::store::StrategyRow;
use crate::strategies::{self, CodeStrategy, CompileError, StrategyDoc, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, MAX_PROMPT_CHARS};
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use lilanaya_engine::{Issue, Strategy, Transition};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const DEFAULT_WORLD: &str = "prisoners-dilemma";
const MAX_OWN_STRATEGIES: usize = 100;

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
    if let (Some(doc), Some(world)) = (&doc, &world) {
        // The full check, sample matches included, so code that fails while playing is not saved.
        let (doc, world) = (doc.clone(), world.clone());
        let report = tokio::task::spawn_blocking(move || strategies::preview(&doc, &world)).await.map_err(|e| ApiError::internal(format!("preview: {e}")))?;
        issues.extend(serde_json::from_value::<Vec<Issue>>(report["errors"].clone()).unwrap_or_default());
    }
    match doc {
        Some(doc) if issues.is_empty() => Ok(Checked { doc, visibility, world_id, prompt }),
        _ => Err(ApiError::invalid("The strategy is not valid", issues)),
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
    let own = state.store.strategies(Some(user.id)).await?.iter().filter(|s| s.owner_id == Some(user.id)).count();
    if own >= MAX_OWN_STRATEGIES {
        return Err(ApiError::new(StatusCode::CONFLICT, "limit_reached", format!("You already have {MAX_OWN_STRATEGIES} strategies; delete one first")));
    }
    let id = new_id(&body.name);
    let checked = check(&state, body, &id).await?;
    state.store.insert_strategy(&checked.doc, user.id, &checked.visibility, &checked.world_id, checked.prompt.as_deref()).await?;
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
/// Always 200 for a well-formed draft: `{valid, errors, warnings, samples}`. Uses no quota.
pub async fn preview(State(state): Shared, _user: User, body: Result<Bytes, BytesRejection>) -> ApiResult {
    let req: PreviewBody = parse_json(body)?;
    let world = find_world(&state, req.world_id.as_deref().unwrap_or(DEFAULT_WORLD)).await?;
    let mut issues = Vec::new();
    let Some(doc) = req.strategy.into_doc("draft", &mut issues).filter(|_| issues.is_empty()) else {
        return Ok(Json(json!({ "valid": false, "errors": issues, "warnings": [], "samples": [] })));
    };
    let report = tokio::task::spawn_blocking(move || strategies::preview(&doc, &world)).await.map_err(|e| ApiError::internal(format!("preview: {e}")))?;
    Ok(Json(report))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompileBody {
    world_id: Option<String>,
    prompt: String,
}

/// Turns a plain-English description into rules with the language model.
/// The rules come back for the editor (not saved), with the model's reasoning
/// and any problems the validator still finds.
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
    let world = find_world(&state, req.world_id.as_deref().unwrap_or(DEFAULT_WORLD)).await?;
    if !state.store.record_llm_call(user.id, state.config.compiles_per_hour).await? {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            format!("You can compile {} descriptions an hour. Try again later, or edit the rules by hand.", state.config.compiles_per_hour),
        ));
    }
    match strategies::compile(llm, &world, prompt).await {
        Ok(compiled) => {
            let s = &compiled.strategy;
            Ok(Json(json!({
                "strategy": {
                    "name": s.name, "description": s.description, "states": s.states,
                    "initial_state": s.initial_state, "action_map": s.action_map, "transitions": s.transitions,
                },
                "rationale": compiled.rationale,
                "valid": compiled.errors.is_empty(),
                "errors": compiled.errors,
            })))
        }
        Err(CompileError::NotAStrategy(message)) | Err(CompileError::Llm(LlmError::NoAnswer(message))) => {
            Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "not_compiled", message))
        }
        Err(CompileError::Llm(LlmError::Unavailable(detail))) => {
            eprintln!("language model unavailable: {detail}");
            Err(ApiError::new(StatusCode::BAD_GATEWAY, "compile_unavailable", "The language model could not be reached. Try again in a moment, or build the rules by hand."))
        }
    }
}
