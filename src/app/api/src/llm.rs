//! The language model behind plain-English strategy compile, reached through
//! Vercel AI Gateway (its OpenAI-compatible Chat Completions API). One key
//! covers the model and the screening step in front of it (`guard.rs`).
//!
//! Nothing is sent to the model that the screen has not let through:
//! [`Llm::complete`] is the only way to the model, and it asks the screen first,
//! on every call. If the screen cannot be reached, the call is refused.
//!
//! Settings:
//! - `AI_GATEWAY_API_KEY`: an AI Gateway API key. Unset switches compile off.
//! - `LLM_MODEL`: the gateway's id of the model. Default `anthropic/claude-opus-5.5`.
//! - `LLM_EFFORT`: `low` | `medium` | `high` | …, or `none` to leave it out. Default `medium`.
//! - `JEV_MODEL`: the screening model. Default `typesafe-ai/jev`.
//! - `JEV_MIN_CONFIDENCE`: how sure the screen must be that a text is a strategy
//!   description, 0 to 1. Default 0.5.
//! - `AI_ZERO_DATA_RETENTION`: `on` (default) asks the gateway to use only providers
//!   that keep nothing; `off` leaves that to the gateway's own settings.
//! - `AI_GATEWAY_URL`: default `https://ai-gateway.vercel.sh`.

use crate::guard::{self, Verdict};
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_URL: &str = "https://ai-gateway.vercel.sh";
const DEFAULT_MODEL: &str = "anthropic/claude-opus-5.5";
const DEFAULT_JEV_MODEL: &str = "typesafe-ai/jev";
const DEFAULT_MIN_CONFIDENCE: f64 = 0.5;
/// Room for the model's thinking and its answer. A state machine of a dozen states is
/// a few hundred tokens; this is not enough to be worth abusing as a writing service.
const MAX_TOKENS: u32 = 6_000;
const ATTEMPTS: u32 = 3;

const DECLINED: &str = "The model declined to answer this description";
const TOO_LONG: &str = "The answer was too long; try a shorter description";

#[derive(Debug, PartialEq)]
pub enum LlmError {
    /// The model declined, or ran out of room: nothing usable came back.
    NoAnswer(String),
    /// The screen turned the text down: it never reached the model.
    Blocked(Verdict),
    /// A service could not be used (credentials, limits, outage). Logged; the user sees a general message.
    Unavailable(String),
}

/// What one compile has used and what the screen said, for the record in `llm_calls`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spent {
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// The screen's latest verdict; `None` if it was never reached.
    pub verdict: Option<Verdict>,
}

/// An answer the gateway did not give: the status (if it answered at all) and what it said.
pub(crate) struct Refused {
    pub status: Option<u16>,
    pub detail: String,
}

#[derive(Clone)]
pub struct Llm {
    http: reqwest::Client,
    /// The gateway's address, without a trailing slash.
    base: String,
    key: String,
    model: String,
    effort: Option<String>,
    pub(crate) jev_model: String,
    pub(crate) min_confidence: f64,
    zero_retention: bool,
}

impl Llm {
    /// `Ok(None)` when no key is set: the feature is switched off.
    pub fn from_env() -> Result<Option<Self>, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let Some(key) = var("AI_GATEWAY_API_KEY") else {
            return Ok(None);
        };
        let effort = match var("LLM_EFFORT") {
            None => Some("medium".to_string()),
            Some(e) if e == "none" => None,
            Some(e) => Some(e),
        };
        let min_confidence = match var("JEV_MIN_CONFIDENCE") {
            None => DEFAULT_MIN_CONFIDENCE,
            Some(raw) => raw.parse::<f64>().ok().filter(|p| (0.0..=1.0).contains(p)).ok_or_else(|| format!("JEV_MIN_CONFIDENCE must be a number from 0 to 1, got \"{raw}\""))?,
        };
        let zero_retention = match var("AI_ZERO_DATA_RETENTION").map(|v| v.trim().to_lowercase()).as_deref() {
            None | Some("on") | Some("true") | Some("1") => true,
            Some("off") | Some("false") | Some("0") => false,
            Some(other) => return Err(format!("AI_ZERO_DATA_RETENTION must be on or off, got \"{other}\"")),
        };
        let mut llm = Llm::new(&var("AI_GATEWAY_URL").unwrap_or_else(|| DEFAULT_URL.into()), key, var("LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into()), effort)?;
        llm.jev_model = var("JEV_MODEL").unwrap_or_else(|| DEFAULT_JEV_MODEL.into());
        llm.min_confidence = min_confidence;
        llm.zero_retention = zero_retention;
        Ok(Some(llm))
    }

    pub fn new(base: &str, key: String, model: String, effort: Option<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| format!("could not set up the HTTP client for the language model: {e}"))?;
        Ok(Llm {
            http,
            base: base.trim_end_matches('/').to_string(),
            key,
            model,
            effort,
            jev_model: DEFAULT_JEV_MODEL.into(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            zero_retention: true,
        })
    }

    pub fn describe(&self) -> String {
        format!("{} through Vercel AI Gateway, screened by {}", self.model, self.jev_model)
    }

    /// What every request to the gateway carries besides its own fields: who it is
    /// for (an id that names nobody, so the provider can tell our users apart) and
    /// that nothing of it is to be kept.
    pub(crate) fn common(&self, body: &mut Value, user: &str) {
        body["safety_identifier"] = json!(user);
        if self.zero_retention {
            body["providerOptions"] = json!({ "gateway": { "zeroDataRetention": true } });
        }
    }

    /// Posts `body` to `path` of the gateway and returns its JSON answer. Too many
    /// requests, overload and server errors are tried again; anything else is ours to fix.
    pub(crate) async fn post(&self, path: &str, body: &Value) -> Result<Value, Refused> {
        let mut wait = Duration::from_millis(500);
        let mut last = Refused { status: None, detail: String::new() };
        for attempt in 1..=ATTEMPTS {
            let response = match self.http.post(format!("{}{path}", self.base)).bearer_auth(&self.key).json(body).send().await {
                Ok(response) => response,
                Err(e) => {
                    last = Refused { status: None, detail: format!("request failed: {e}") };
                    if attempt < ATTEMPTS {
                        tokio::time::sleep(wait).await;
                        wait *= 4;
                    }
                    continue;
                }
            };
            let status = response.status();
            let request_id = ["x-vercel-id", "x-request-id", "request-id"]
                .iter()
                .find_map(|name| response.headers().get(*name).and_then(|v| v.to_str().ok()))
                .unwrap_or("-")
                .to_string();
            let text = response.text().await.unwrap_or_default();
            if status.is_success() {
                return serde_json::from_str(&text).map_err(|e| Refused { status: Some(status.as_u16()), detail: format!("answer is not JSON ({e}), request {request_id}") });
            }
            let detail: String = text.chars().take(500).collect();
            last = Refused { status: Some(status.as_u16()), detail: format!("HTTP {status}, request {request_id}: {detail}") };
            if !(status.as_u16() == 429 || status.is_server_error()) {
                break;
            }
            if attempt < ATTEMPTS {
                tokio::time::sleep(wait).await;
                wait *= 4;
            }
        }
        Err(last)
    }

    /// Sends one conversation and returns the text of the answer. `messages` are
    /// `{"role": "user" | "assistant", "content": "text"}`.
    ///
    /// `untrusted` is the text in the conversation that a user wrote. It is screened
    /// first, on every call; a text the screen turns down, or a screen that cannot be
    /// reached, ends the call before anything is sent to the model.
    pub async fn complete(&self, system: &str, messages: &[Value], untrusted: &str, user: &str, spent: &mut Spent) -> Result<String, LlmError> {
        let verdict = guard::screen(self, untrusted, user, spent).await.map_err(|e| LlmError::Unavailable(format!("screening: {e}")))?;
        spent.verdict = Some(verdict.clone());
        if !verdict.allowed {
            return Err(LlmError::Blocked(verdict));
        }

        let mut all = vec![json!({ "role": "system", "content": system })];
        all.extend_from_slice(messages);
        let mut body = json!({ "model": self.model, "max_tokens": MAX_TOKENS, "messages": all, "stream": false });
        if let Some(effort) = &self.effort {
            body["reasoning"] = json!({ "effort": effort });
        }
        self.common(&mut body, user);
        match self.post("/v1/chat/completions", &body).await {
            Ok(answer) => {
                spent.input_tokens += answer["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
                spent.output_tokens += answer["usage"]["completion_tokens"].as_i64().unwrap_or(0);
                read_completion(&answer)
            }
            // A provider's content filter turns a description down with a 400 before the model sees it.
            Err(Refused { status: Some(400), detail }) if detail.contains("content_filter") || detail.contains("content_policy") => Err(LlmError::NoAnswer(DECLINED.into())),
            Err(refused) => Err(LlmError::Unavailable(refused.detail)),
        }
    }
}

/// The text of a Chat Completions answer, or why there is none.
fn read_completion(answer: &Value) -> Result<String, LlmError> {
    let choice = &answer["choices"][0];
    match choice["finish_reason"].as_str() {
        Some("content_filter") => return Err(LlmError::NoAnswer(DECLINED.into())),
        Some("length") => return Err(LlmError::NoAnswer(TOO_LONG.into())),
        _ => {}
    }
    if choice["message"]["refusal"].as_str().is_some_and(|r| !r.trim().is_empty()) {
        return Err(LlmError::NoAnswer(DECLINED.into()));
    }
    let text: String = match &choice["message"]["content"] {
        Value::String(text) => text.clone(),
        // Some providers answer in parts.
        Value::Array(parts) => parts.iter().filter(|p| p["type"] == "text").filter_map(|p| p["text"].as_str()).collect(),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        return Err(LlmError::NoAnswer("The model gave an empty answer".into()));
    }
    Ok(text)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::{Json, Router};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    type Queue = Arc<Mutex<VecDeque<(u16, Value)>>>;

    /// A stand-in for the gateway. The model answers with the queued replies in
    /// order; the screen lets everything through unless a verdict is queued. Both
    /// record what they were sent.
    #[derive(Clone, Default)]
    pub struct FakeApi {
        replies: Queue,
        verdicts: Queue,
        /// Requests to the model: headers and body.
        pub seen: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
        /// Requests to the screen.
        pub screened: Arc<Mutex<Vec<Value>>>,
    }

    /// A normal answer from the model whose text is `text`.
    pub fn completion(text: &str) -> Value {
        json!({
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 900, "completion_tokens": 100, "total_tokens": 1000 },
        })
    }

    /// The screen's answer: it picked `label`, and gives a strategy description the probability `strategy`.
    pub fn verdict(label: &str, strategy: f64) -> Value {
        let rest = (1.0 - strategy) / 3.0;
        let probabilities: Vec<Value> = guard::LABELS.iter().map(|l| json!({ "value": l, "probability": if *l == guard::STRATEGY { strategy } else { rest } })).collect();
        json!({
            "answers": [{ "type": "choice", "name": "kind", "choice": label, "confidence": 0.9, "probabilities": probabilities }],
            "model": "typesafe-ai/jev",
            "usage": { "input_tokens": 300, "output_tokens": 20, "total_tokens": 320 },
        })
    }

    impl FakeApi {
        pub fn reply(&self, status: u16, body: Value) -> &Self {
            self.replies.lock().unwrap().push_back((status, body));
            self
        }

        /// Queues a normal answer whose text is `text`.
        pub fn say(&self, text: &str) -> &Self {
            self.reply(200, completion(text))
        }

        /// Queues what the screen says about the next text it is shown.
        pub fn judge(&self, status: u16, body: Value) -> &Self {
            self.verdicts.lock().unwrap().push_back((status, body));
            self
        }

        /// Starts listening on a free local port; returns the base URL.
        pub async fn start(&self) -> String {
            async fn chat(State(api): State<FakeApi>, headers: HeaderMap, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
                api.seen.lock().unwrap().push((headers, body));
                let (status, reply) = api.replies.lock().unwrap().pop_front().unwrap_or((500, json!({ "error": "no reply queued" })));
                (StatusCode::from_u16(status).unwrap(), Json(reply))
            }
            async fn decide(State(api): State<FakeApi>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
                api.screened.lock().unwrap().push(body);
                let (status, reply) = api.verdicts.lock().unwrap().pop_front().unwrap_or((200, verdict(guard::STRATEGY, 0.97)));
                (StatusCode::from_u16(status).unwrap(), Json(reply))
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new().route("/v1/chat/completions", post(chat)).route("/v1/decisions", post(decide)).with_state(self.clone());
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            format!("http://{address}")
        }

        /// A client of this stand-in.
        pub async fn llm(&self) -> Llm {
            Llm::new(&self.start().await, "gw-test".into(), DEFAULT_MODEL.into(), Some("medium".into())).unwrap()
        }
    }

    fn user(text: &str) -> Vec<Value> {
        vec![json!({ "role": "user", "content": text })]
    }

    async fn ask(llm: &Llm, spent: &mut Spent) -> Result<String, LlmError> {
        llm.complete("be brief", &user("hi"), "hi", "u-1", spent).await
    }

    #[tokio::test]
    async fn the_gateway_gets_the_key_the_conversation_and_the_limits() {
        let api = FakeApi::default();
        api.say("hello");
        let llm = api.llm().await;
        let mut spent = Spent::default();
        assert_eq!(ask(&llm, &mut spent).await, Ok("hello".into()));

        let seen = api.seen.lock().unwrap();
        let (headers, body) = &seen[0];
        assert_eq!(headers["authorization"], "Bearer gw-test");
        assert_eq!(
            body,
            &json!({
                "model": "anthropic/claude-opus-5.5", "max_tokens": 6000, "stream": false,
                "messages": [{ "role": "system", "content": "be brief" }, { "role": "user", "content": "hi" }],
                "reasoning": { "effort": "medium" },
                "safety_identifier": "u-1",
                "providerOptions": { "gateway": { "zeroDataRetention": true } },
            })
        );
        // The screen's tokens and the model's are both counted.
        assert_eq!((spent.input_tokens, spent.output_tokens), (300 + 900, 20 + 100));
        assert!(spent.verdict.as_ref().unwrap().allowed);
    }

    #[tokio::test]
    async fn every_call_is_screened_and_a_text_turned_down_never_reaches_the_model() {
        let api = FakeApi::default();
        let llm = api.llm().await;
        let mut spent = Spent::default();

        api.say("one").say("two");
        assert_eq!(ask(&llm, &mut spent).await, Ok("one".into()));
        assert_eq!(ask(&llm, &mut spent).await, Ok("two".into()));
        assert_eq!(api.screened.lock().unwrap().len(), 2, "one screening per call");

        api.judge(200, verdict("instructions_to_the_ai", 0.02)).say("never sent");
        let error = ask(&llm, &mut spent).await.unwrap_err();
        assert!(matches!(&error, LlmError::Blocked(v) if v.label == "instructions_to_the_ai" && !v.allowed), "{error:?}");
        assert_eq!(api.seen.lock().unwrap().len(), 2, "the model was not asked");
        assert!(!spent.verdict.as_ref().unwrap().allowed);
    }

    #[tokio::test]
    async fn a_screen_that_cannot_be_reached_ends_the_call() {
        let api = FakeApi::default();
        let llm = api.llm().await;
        api.judge(401, json!({ "error": { "message": "bad key", "type": "authentication_error" } })).say("never sent");
        let error = ask(&llm, &mut Spent::default()).await.unwrap_err();
        assert!(matches!(&error, LlmError::Unavailable(m) if m.contains("screening") && m.contains("401")), "{error:?}");
        assert_eq!(api.seen.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn busy_servers_are_retried_and_bad_requests_are_not() {
        let api = FakeApi::default();
        let llm = api.llm().await;
        api.reply(529, json!({ "error": { "type": "overloaded_error" } })).say("after a retry");
        assert_eq!(ask(&llm, &mut Spent::default()).await, Ok("after a retry".into()));
        assert_eq!(api.seen.lock().unwrap().len(), 2);

        api.reply(401, json!({ "error": { "type": "authentication_error", "message": "invalid api key" } })).say("never reached");
        let error = ask(&llm, &mut Spent::default()).await.unwrap_err();
        assert!(matches!(&error, LlmError::Unavailable(m) if m.contains("401") && m.contains("invalid api key")), "{error:?}");
        assert_eq!(api.seen.lock().unwrap().len(), 3, "a rejected key is not retried");
    }

    #[tokio::test]
    async fn refusals_filters_and_cut_off_answers_are_reported_not_parsed() {
        let api = FakeApi::default();
        let llm = api.llm().await;
        api.reply(200, json!({ "choices": [{ "message": { "role": "assistant", "content": null, "refusal": "no" }, "finish_reason": "stop" }] }));
        api.reply(200, json!({ "choices": [{ "message": { "role": "assistant", "content": "{\"half\":" }, "finish_reason": "length" }] }));
        api.reply(200, json!({ "choices": [{ "message": { "role": "assistant", "content": "" }, "finish_reason": "content_filter" }] }));
        api.reply(400, json!({ "error": { "code": "content_filter", "message": "The request was filtered" } }));
        let mut spent = Spent::default();
        assert!(matches!(ask(&llm, &mut spent).await, Err(LlmError::NoAnswer(m)) if m.contains("declined")));
        assert!(matches!(ask(&llm, &mut spent).await, Err(LlmError::NoAnswer(m)) if m.contains("too long")));
        assert!(matches!(ask(&llm, &mut spent).await, Err(LlmError::NoAnswer(m)) if m.contains("declined")));
        assert!(matches!(ask(&llm, &mut spent).await, Err(LlmError::NoAnswer(m)) if m.contains("declined")));
    }
}
