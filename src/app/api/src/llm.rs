//! The language model behind plain-English strategy compile: Claude, reached
//! either through the Claude API (an API key; development and test) or
//! through Claude in Microsoft Foundry (an Azure endpoint; production). Both
//! speak the same Messages API; only the address and the key header differ.
//!
//! Settings:
//! - `LLM_PROVIDER`: `anthropic` or `azure`. Default: whichever has a key set.
//! - `LLM_MODEL`: model id (on Azure: the deployment name). Default `claude-opus-5-5`.
//! - `LLM_EFFORT`: `low` | `medium` | `high` | …, or `none` to leave it out. Default `medium`.
//! - anthropic: `ANTHROPIC_API_KEY`, optional `ANTHROPIC_BASE_URL`.
//! - azure: `ANTHROPIC_FOUNDRY_API_KEY`, and `ANTHROPIC_FOUNDRY_RESOURCE` (the
//!   resource name) or `ANTHROPIC_FOUNDRY_BASE_URL` (`https://…/anthropic`).

use serde_json::{json, Value};
use std::time::Duration;

const API_VERSION: &str = "2026-10-21";
const DEFAULT_MODEL: &str = "claude-opus-5-5";
const MAX_TOKENS: u32 = 16_000;
const ATTEMPTS: u32 = 3;
/// Models whose safety refusals can be re-run on another model by the Claude API itself.
const FALLBACK_MODELS: [&str; 5] = ["claude-fable-5-2", "claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Provider {
    Anthropic,
    Azure,
}

#[derive(Debug, PartialEq)]
pub enum LlmError {
    /// The model declined, or ran out of room: nothing usable came back.
    NoAnswer(String),
    /// The service could not be used (credentials, limits, outage). Logged; the user sees a general message.
    Unavailable(String),
}

#[derive(Clone)]
pub struct Llm {
    http: reqwest::Client,
    provider: Provider,
    /// `…/v1/messages`
    url: String,
    key: String,
    model: String,
    effort: Option<String>,
}

impl Llm {
    /// `Ok(None)` when no provider is configured: the feature is switched off.
    pub fn from_env() -> Result<Option<Self>, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let provider = match var("LLM_PROVIDER").as_deref() {
            Some("anthropic") => Provider::Anthropic,
            Some("azure") => Provider::Azure,
            Some(other) => return Err(format!("LLM_PROVIDER must be \"anthropic\" or \"azure\", got \"{other}\"")),
            None if var("ANTHROPIC_FOUNDRY_API_KEY").is_some() => Provider::Azure,
            None if var("ANTHROPIC_API_KEY").is_some() => Provider::Anthropic,
            None => return Ok(None),
        };
        let (base, key) = match provider {
            Provider::Anthropic => (
                var("ANTHROPIC_BASE_URL").unwrap_or_else(|| "https://api.anthropic.com".into()),
                var("ANTHROPIC_API_KEY").ok_or("LLM_PROVIDER is anthropic, so ANTHROPIC_API_KEY must be set")?,
            ),
            Provider::Azure => {
                let base = match (var("ANTHROPIC_FOUNDRY_BASE_URL"), var("ANTHROPIC_FOUNDRY_RESOURCE")) {
                    (Some(url), _) => url,
                    (None, Some(resource)) => format!("https://{resource}.services.ai.azure.com/anthropic"),
                    (None, None) => return Err("LLM_PROVIDER is azure, so ANTHROPIC_FOUNDRY_RESOURCE or ANTHROPIC_FOUNDRY_BASE_URL must be set".into()),
                };
                (base, var("ANTHROPIC_FOUNDRY_API_KEY").ok_or("LLM_PROVIDER is azure, so ANTHROPIC_FOUNDRY_API_KEY must be set")?)
            }
        };
        let effort = match var("LLM_EFFORT") {
            Some(e) if e == "none" => None,
            Some(e) => Some(e),
            None => Some("medium".into()),
        };
        Llm::new(provider, &base, key, var("LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into()), effort).map(Some)
    }

    pub fn new(provider: Provider, base_url: &str, key: String, model: String, effort: Option<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| format!("could not set up the HTTP client for the language model: {e}"))?;
        Ok(Llm { http, provider, url: format!("{}/v1/messages", base_url.trim_end_matches('/')), key, model, effort })
    }

    pub fn describe(&self) -> String {
        format!("{:?} ({})", self.provider, self.model)
    }

    fn body(&self, system: &str, messages: &[Value]) -> Value {
        let mut body = json!({ "model": self.model, "max_tokens": MAX_TOKENS, "system": system, "messages": messages });
        if let Some(effort) = &self.effort {
            body["output_config"] = json!({ "effort": effort });
        }
        // Foundry has no server-side fallback; there a refusal is simply reported.
        if self.provider == Provider::Anthropic && FALLBACK_MODELS.contains(&self.model.as_str()) {
            body["fallbacks"] = json!("default");
        }
        body
    }

    /// Sends one conversation and returns the text of the answer.
    pub async fn complete(&self, system: &str, messages: &[Value]) -> Result<String, LlmError> {
        let body = self.body(system, messages);
        let mut wait = Duration::from_millis(500);
        let mut last = String::new();
        for attempt in 1..=ATTEMPTS {
            let mut request = self.http.post(&self.url).header("anthropic-version", API_VERSION).json(&body);
            request = match self.provider {
                Provider::Anthropic => request.header("x-api-key", &self.key),
                Provider::Azure => request.header("api-key", &self.key),
            };
            if body.get("fallbacks").is_some() {
                request = request.header("anthropic-beta", "server-side-fallback-2026-07-01");
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(e) => {
                    last = format!("request failed: {e}");
                    if attempt < ATTEMPTS {
                        tokio::time::sleep(wait).await;
                        wait *= 4;
                    }
                    continue;
                }
            };
            let status = response.status();
            let request_id = response.headers().get("request-id").and_then(|v| v.to_str().ok()).unwrap_or("-").to_string();
            let text = response.text().await.unwrap_or_default();
            if status.is_success() {
                let message: Value = serde_json::from_str(&text).map_err(|e| LlmError::Unavailable(format!("answer is not JSON ({e}), request {request_id}")))?;
                return read_answer(&message);
            }
            let detail: String = text.chars().take(500).collect();
            last = format!("HTTP {status}, request {request_id}: {detail}");
            // Too many requests, overloaded, or a server error: worth another try. Anything else is ours to fix.
            let retryable = status.as_u16() == 429 || status.is_server_error();
            if !retryable {
                break;
            }
            if attempt < ATTEMPTS {
                tokio::time::sleep(wait).await;
                wait *= 4;
            }
        }
        Err(LlmError::Unavailable(last))
    }
}

/// The text of a Messages API answer, or why there is none.
fn read_answer(message: &Value) -> Result<String, LlmError> {
    match message["stop_reason"].as_str() {
        Some("refusal") => return Err(LlmError::NoAnswer("The model declined to answer this description".into())),
        Some("max_tokens") => return Err(LlmError::NoAnswer("The answer was too long; try a shorter description".into())),
        _ => {}
    }
    let text: String = message["content"]
        .as_array()
        .map(|blocks| blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect())
        .unwrap_or_default();
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

    /// A stand-in for the Messages API: answers with the queued replies in
    /// order and records what it was sent.
    #[derive(Clone, Default)]
    pub struct FakeApi {
        replies: Arc<Mutex<VecDeque<(u16, Value)>>>,
        pub seen: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    }

    impl FakeApi {
        pub fn reply(&self, status: u16, body: Value) -> &Self {
            self.replies.lock().unwrap().push_back((status, body));
            self
        }

        /// Queues a normal answer whose text is `text`.
        pub fn say(&self, text: &str) -> &Self {
            self.reply(200, json!({ "type": "message", "stop_reason": "end_turn", "content": [{ "type": "thinking", "thinking": "" }, { "type": "text", "text": text }] }))
        }

        /// Starts listening on a free local port; returns the base URL.
        pub async fn start(&self) -> String {
            async fn handle(State(api): State<FakeApi>, headers: HeaderMap, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
                api.seen.lock().unwrap().push((headers, body));
                let (status, reply) = api.replies.lock().unwrap().pop_front().unwrap_or((500, json!({ "error": "no reply queued" })));
                (StatusCode::from_u16(status).unwrap(), Json(reply))
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            // Both providers' paths end in /v1/messages.
            let router = Router::new().route("/v1/messages", post(handle)).route("/anthropic/v1/messages", post(handle)).with_state(self.clone());
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            format!("http://{address}")
        }
    }

    fn user(text: &str) -> Vec<Value> {
        vec![json!({ "role": "user", "content": text })]
    }

    #[tokio::test]
    async fn the_claude_api_gets_its_key_version_and_fallbacks() {
        let api = FakeApi::default();
        api.say("hello");
        let llm = Llm::new(Provider::Anthropic, &api.start().await, "sk-test".into(), DEFAULT_MODEL.into(), Some("medium".into())).unwrap();
        assert_eq!(llm.complete("be brief", &user("hi")).await, Ok("hello".into()));

        let seen = api.seen.lock().unwrap();
        let (headers, body) = &seen[0];
        assert_eq!(headers["x-api-key"], "sk-test");
        assert_eq!(headers["anthropic-version"], API_VERSION);
        assert_eq!(headers["anthropic-beta"], "server-side-fallback-2026-07-01");
        assert!(headers.get("api-key").is_none());
        assert_eq!(
            body,
            &json!({
                "model": "claude-opus-5-5", "max_tokens": 16000, "system": "be brief",
                "messages": [{ "role": "user", "content": "hi" }],
                "output_config": { "effort": "medium" }, "fallbacks": "default",
            })
        );
    }

    #[tokio::test]
    async fn azure_gets_its_own_key_header_and_no_fallbacks() {
        let api = FakeApi::default();
        api.say("hello");
        let base = format!("{}/anthropic/", api.start().await);
        let llm = Llm::new(Provider::Azure, &base, "azure-key".into(), "my-deployment".into(), None).unwrap();
        assert_eq!(llm.complete("s", &user("hi")).await, Ok("hello".into()));

        let seen = api.seen.lock().unwrap();
        let (headers, body) = &seen[0];
        assert_eq!(headers["api-key"], "azure-key");
        assert_eq!(headers["anthropic-version"], API_VERSION);
        assert!(headers.get("x-api-key").is_none() && headers.get("anthropic-beta").is_none());
        assert_eq!(body["model"], "my-deployment");
        assert!(body.get("fallbacks").is_none() && body.get("output_config").is_none());
    }

    #[tokio::test]
    async fn busy_servers_are_retried_and_bad_requests_are_not() {
        let api = FakeApi::default();
        api.reply(529, json!({ "type": "error", "error": { "type": "overloaded_error" } })).say("after a retry");
        let llm = Llm::new(Provider::Anthropic, &api.start().await, "k".into(), DEFAULT_MODEL.into(), None).unwrap();
        assert_eq!(llm.complete("s", &user("hi")).await, Ok("after a retry".into()));
        assert_eq!(api.seen.lock().unwrap().len(), 2);

        api.reply(401, json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } })).say("never reached");
        let error = llm.complete("s", &user("hi")).await.unwrap_err();
        assert!(matches!(&error, LlmError::Unavailable(m) if m.contains("401") && m.contains("invalid x-api-key")), "{error:?}");
        assert_eq!(api.seen.lock().unwrap().len(), 3, "a rejected key is not retried");
    }

    #[tokio::test]
    async fn refusals_and_cut_off_answers_are_reported_not_parsed() {
        let api = FakeApi::default();
        api.reply(200, json!({ "stop_reason": "refusal", "content": [] }));
        api.reply(200, json!({ "stop_reason": "max_tokens", "content": [{ "type": "text", "text": "{\"half\":" }] }));
        let llm = Llm::new(Provider::Azure, &api.start().await, "k".into(), "d".into(), None).unwrap();
        assert!(matches!(llm.complete("s", &user("hi")).await, Err(LlmError::NoAnswer(m)) if m.contains("declined")));
        assert!(matches!(llm.complete("s", &user("hi")).await, Err(LlmError::NoAnswer(m)) if m.contains("too long")));
    }
}
