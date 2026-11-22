//! The language model behind plain-English strategy compile. Two services are
//! supported, and they speak different protocols:
//!
//! - `anthropic`: Claude through the Claude API (Messages API). Development and test.
//! - `azure`: a model deployed in Azure OpenAI (Responses API). Production.
//!
//! Settings:
//! - `LLM_PROVIDER`: `anthropic` or `azure`. Default: whichever has a key set (Azure first).
//! - `LLM_MODEL`: model id. On Azure this is the deployment name and has no default.
//! - `LLM_EFFORT`: `low` | `medium` | `high` | …, or `none` to leave it out. Default `medium`
//!   for Claude; left out on Azure, where only reasoning models accept it.
//! - anthropic: `ANTHROPIC_API_KEY`, optional `ANTHROPIC_BASE_URL`.
//! - azure: `AZURE_OPENAI_API_KEY`, and `AZURE_OPENAI_URL`: the full address of the
//!   responses endpoint, `https://{resource}.openai.azure.com/openai/responses?api-version=…`.

use serde_json::{json, Value};
use std::time::Duration;

const API_VERSION: &str = "2026-10-21";
const DEFAULT_MODEL: &str = "claude-opus-5-5";
const MAX_TOKENS: u32 = 16_000;
const ATTEMPTS: u32 = 3;
/// Models whose safety refusals can be re-run on another model by the Claude API itself.
const FALLBACK_MODELS: [&str; 5] = ["claude-fable-5-2", "claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];

const DECLINED: &str = "The model declined to answer this description";
const TOO_LONG: &str = "The answer was too long; try a shorter description";

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
    /// The full address requests are posted to.
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
            None if var("AZURE_OPENAI_API_KEY").is_some() => Provider::Azure,
            None if var("ANTHROPIC_API_KEY").is_some() => Provider::Anthropic,
            None => return Ok(None),
        };
        let effort = var("LLM_EFFORT").filter(|e| e != "none");
        match provider {
            Provider::Anthropic => {
                let base = var("ANTHROPIC_BASE_URL").unwrap_or_else(|| "https://api.anthropic.com".into());
                let key = var("ANTHROPIC_API_KEY").ok_or("LLM_PROVIDER is anthropic, so ANTHROPIC_API_KEY must be set")?;
                let effort = if var("LLM_EFFORT").is_none() { Some("medium".into()) } else { effort };
                Llm::new(provider, &base, key, var("LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into()), effort).map(Some)
            }
            Provider::Azure => {
                let url = var("AZURE_OPENAI_URL").ok_or("the Azure language model needs AZURE_OPENAI_URL (https://….openai.azure.com/openai/responses?api-version=…)")?;
                let key = var("AZURE_OPENAI_API_KEY").ok_or("the Azure language model needs AZURE_OPENAI_API_KEY")?;
                let model = var("LLM_MODEL").ok_or("the Azure language model needs LLM_MODEL: the name of the deployment")?;
                Llm::new(provider, &url, key, model, effort).map(Some)
            }
        }
    }

    /// `address` is the base URL for the Claude API and the full responses URL for Azure.
    pub fn new(provider: Provider, address: &str, key: String, model: String, effort: Option<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| format!("could not set up the HTTP client for the language model: {e}"))?;
        let url = match provider {
            Provider::Anthropic => format!("{}/v1/messages", address.trim_end_matches('/')),
            Provider::Azure => address.to_string(),
        };
        Ok(Llm { http, provider, url, key, model, effort })
    }

    pub fn describe(&self) -> String {
        match self.provider {
            Provider::Anthropic => format!("the Claude API ({})", self.model),
            Provider::Azure => format!("Azure OpenAI (deployment {})", self.model),
        }
    }

    fn body(&self, system: &str, messages: &[Value]) -> Value {
        match self.provider {
            Provider::Anthropic => {
                let mut body = json!({ "model": self.model, "max_tokens": MAX_TOKENS, "system": system, "messages": messages });
                if let Some(effort) = &self.effort {
                    body["output_config"] = json!({ "effort": effort });
                }
                if FALLBACK_MODELS.contains(&self.model.as_str()) {
                    body["fallbacks"] = json!("default");
                }
                body
            }
            Provider::Azure => {
                // `store: false`: users' descriptions are not kept by the service after the answer.
                let mut body = json!({ "model": self.model, "max_output_tokens": MAX_TOKENS, "instructions": system, "input": messages, "store": false });
                if let Some(effort) = &self.effort {
                    body["reasoning"] = json!({ "effort": effort });
                }
                body
            }
        }
    }

    /// Sends one conversation and returns the text of the answer. `messages`
    /// are `{"role": "user" | "assistant", "content": "text"}`, which both services accept.
    pub async fn complete(&self, system: &str, messages: &[Value]) -> Result<String, LlmError> {
        let body = self.body(system, messages);
        let mut wait = Duration::from_millis(500);
        let mut last = String::new();
        for attempt in 1..=ATTEMPTS {
            let mut request = self.http.post(&self.url).json(&body);
            request = match self.provider {
                Provider::Anthropic => request.header("x-api-key", &self.key).header("anthropic-version", API_VERSION),
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
            let request_id = ["request-id", "x-request-id", "apim-request-id"]
                .iter()
                .find_map(|name| response.headers().get(*name).and_then(|v| v.to_str().ok()))
                .unwrap_or("-")
                .to_string();
            let text = response.text().await.unwrap_or_default();
            if status.is_success() {
                let answer: Value = serde_json::from_str(&text).map_err(|e| LlmError::Unavailable(format!("answer is not JSON ({e}), request {request_id}")))?;
                return match self.provider {
                    Provider::Anthropic => read_message(&answer),
                    Provider::Azure => read_response(&answer),
                };
            }
            // Azure's content filter turns a description down with a 400 before the model sees it.
            if self.provider == Provider::Azure && status.as_u16() == 400 && text.contains("content_filter") {
                return Err(LlmError::NoAnswer(DECLINED.into()));
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

fn non_empty(text: String) -> Result<String, LlmError> {
    if text.trim().is_empty() {
        return Err(LlmError::NoAnswer("The model gave an empty answer".into()));
    }
    Ok(text)
}

/// The text of a Messages API answer (Claude), or why there is none.
fn read_message(message: &Value) -> Result<String, LlmError> {
    match message["stop_reason"].as_str() {
        Some("refusal") => return Err(LlmError::NoAnswer(DECLINED.into())),
        Some("max_tokens") => return Err(LlmError::NoAnswer(TOO_LONG.into())),
        _ => {}
    }
    let blocks = message["content"].as_array().map(Vec::as_slice).unwrap_or_default();
    non_empty(blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect())
}

/// The text of a Responses API answer (Azure OpenAI), or why there is none.
fn read_response(response: &Value) -> Result<String, LlmError> {
    match response["status"].as_str() {
        Some("incomplete") => {
            return Err(LlmError::NoAnswer(match response["incomplete_details"]["reason"].as_str() {
                Some("content_filter") => DECLINED.into(),
                _ => TOO_LONG.into(),
            }))
        }
        Some("failed") => return Err(LlmError::Unavailable(format!("the response failed: {}", response["error"]))),
        _ => {}
    }
    let items = response["output"].as_array().map(Vec::as_slice).unwrap_or_default();
    let parts: Vec<&Value> = items.iter().filter(|i| i["type"] == "message").filter_map(|i| i["content"].as_array()).flatten().collect();
    if parts.iter().any(|p| p["type"] == "refusal") {
        return Err(LlmError::NoAnswer(DECLINED.into()));
    }
    non_empty(parts.iter().filter(|p| p["type"] == "output_text").filter_map(|p| p["text"].as_str()).collect())
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
            let router = Router::new().route("/v1/messages", post(handle)).route("/openai/responses", post(handle)).with_state(self.clone());
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

    fn azure_says(text: &str) -> Value {
        json!({ "status": "completed", "output": [
            { "type": "reasoning", "summary": [] },
            { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": text }] },
        ] })
    }

    #[tokio::test]
    async fn azure_gets_the_responses_format_and_its_own_key_header() {
        let api = FakeApi::default();
        api.reply(200, azure_says("hello"));
        let url = format!("{}/openai/responses?api-version=2025-04-01-preview", api.start().await);
        let llm = Llm::new(Provider::Azure, &url, "azure-key".into(), "my-deployment".into(), None).unwrap();
        assert_eq!(llm.complete("be brief", &user("hi")).await, Ok("hello".into()));

        let seen = api.seen.lock().unwrap();
        let (headers, body) = &seen[0];
        assert_eq!(headers["api-key"], "azure-key");
        assert!(headers.get("x-api-key").is_none() && headers.get("anthropic-version").is_none() && headers.get("anthropic-beta").is_none());
        assert_eq!(
            body,
            &json!({
                "model": "my-deployment", "max_output_tokens": 16000, "instructions": "be brief",
                "input": [{ "role": "user", "content": "hi" }], "store": false,
            })
        );
    }

    #[tokio::test]
    async fn azure_refusals_filters_and_cut_off_answers_are_reported() {
        let api = FakeApi::default();
        api.reply(200, json!({ "status": "completed", "output": [{ "type": "message", "content": [{ "type": "refusal", "refusal": "no" }] }] }));
        api.reply(200, json!({ "status": "incomplete", "incomplete_details": { "reason": "max_output_tokens" }, "output": [] }));
        api.reply(400, json!({ "error": { "code": "content_filter", "message": "The response was filtered" } }));
        let url = format!("{}/openai/responses", api.start().await);
        let llm = Llm::new(Provider::Azure, &url, "k".into(), "d".into(), Some("low".into())).unwrap();
        assert!(matches!(llm.complete("s", &user("hi")).await, Err(LlmError::NoAnswer(m)) if m.contains("declined")));
        assert!(matches!(llm.complete("s", &user("hi")).await, Err(LlmError::NoAnswer(m)) if m.contains("too long")));
        assert!(matches!(llm.complete("s", &user("hi")).await, Err(LlmError::NoAnswer(m)) if m.contains("declined")));
        assert_eq!(api.seen.lock().unwrap()[0].1["reasoning"], json!({ "effort": "low" }));
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
        let llm = Llm::new(Provider::Anthropic, &api.start().await, "k".into(), "claude-haiku-5-5".into(), None).unwrap();
        assert!(matches!(llm.complete("s", &user("hi")).await, Err(LlmError::NoAnswer(m)) if m.contains("declined")));
        assert!(matches!(llm.complete("s", &user("hi")).await, Err(LlmError::NoAnswer(m)) if m.contains("too long")));
    }
}
