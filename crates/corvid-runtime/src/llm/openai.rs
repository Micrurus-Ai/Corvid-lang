//! OpenAI Chat Completions adapter.
//!
//! Speaks `POST /v1/chat/completions` directly. Structured output uses
//! `response_format: {type: "json_schema", json_schema: {...}}` with
//! `strict: true`. The JSON Schema we build (`crate::schema_for`)
//! already meets OpenAI strict-mode requirements: every property listed
//! in `required`, `additionalProperties: false`.
//!
//! Auth: `Authorization: Bearer <key>`.
//!
//! SSE streaming shipped in slice 46d. Out of scope (deferred):
//! function calling, vision, batch,
//! reasoning-effort knobs for o-series.

use crate::errors::RuntimeError;
use crate::llm::{LlmAdapter, LlmRequestRef, LlmResponse, TokenUsage};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::time::Duration;

pub const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com";

pub struct OpenAiAdapter {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAiAdapter {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: OPENAI_DEFAULT_BASE.to_string(),
            client: build_client(),
        }
    }

    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into();
        self
    }
}

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("reqwest client builds with default config")
}

impl LlmAdapter for OpenAiAdapter {
    fn name(&self) -> &str {
        "openai"
    }

    fn supports_tool_turns(&self) -> bool { true }

    fn handles(&self, model: &str) -> bool {
        model.starts_with("gpt-")
            || model.starts_with("o1-")
            || model.starts_with("o3-")
            || model.starts_with("o4-")
            || model == "o1"
            || model == "o3"
            || model == "o4"
    }

    fn stream<'a>(
        &'a self,
        req: &'a LlmRequestRef<'a>,
    ) -> BoxFuture<'a, Result<crate::llm::LlmChunkStream, RuntimeError>> {
        Box::pin(async move {
            let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
            let request = self.client.post(url).bearer_auth(&self.api_key);
            stream_chat(self, req, request, req.model, "openai").await
        })
    }

    fn call<'a>(
        &'a self,
        req: &'a LlmRequestRef<'a>,
    ) -> BoxFuture<'a, Result<LlmResponse, RuntimeError>> {
        Box::pin(async move {
            let url = format!(
                "{}/v1/chat/completions",
                self.base_url.trim_end_matches('/')
            );
            let body = build_request_body(req);
            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| RuntimeError::AdapterFailed {
                    adapter: "openai".into(),
                    message: format!("HTTP send failed: {e}"),
                })?;

            let status = resp.status();
            let body_text = resp.text().await.map_err(|e| RuntimeError::AdapterFailed {
                adapter: "openai".into(),
                message: format!("reading response body failed: {e}"),
            })?;

            if !status.is_success() {
                return Err(RuntimeError::AdapterFailed {
                    adapter: "openai".into(),
                    message: format!("HTTP {status}: {body_text}"),
                });
            }

            let parsed: Value = serde_json::from_str(&body_text).map_err(|e| {
                RuntimeError::AdapterFailed {
                    adapter: "openai".into(),
                    message: format!("response body is not JSON: {e} (body: {body_text})"),
                }
            })?;
            if req.tool_turn.is_some() {
                if let Some(message) = super::openai::tool_call_message(&parsed)? {
                    return Ok(LlmResponse::new(message, super::openai::extract_usage(&parsed)));
                }
            }
            extract_response(&parsed, req.output_schema.is_some())
        })
    }
}

pub(super) async fn stream_chat(
    adapter: &dyn LlmAdapter,
    req: &LlmRequestRef<'_>,
    request: reqwest::RequestBuilder,
    model: &str,
    name: &'static str,
) -> Result<crate::llm::LlmChunkStream, RuntimeError> {
    use crate::llm::LlmStreamChunk;
    use futures::StreamExt;
    if req.output_schema.is_some() {
        let response = adapter.call(req).await?;
        return Ok(Box::pin(futures::stream::once(async move {
            Ok(LlmStreamChunk {
                delta: match response.value { Value::String(s) => s, v => v.to_string() },
                done: true, usage: Some(response.usage), confidence: response.confidence,
            })
        })));
    }
    let mut body = build_request_body(req);
    body["model"] = json!(model);
    body["stream"] = json!(true);
    body["stream_options"] = json!({"include_usage":true});
    let fail = move |message: String| RuntimeError::AdapterFailed { adapter: name.into(), message };
    let response = request.json(&body).send().await.map_err(|e| fail(e.to_string()))?;
    if !response.status().is_success() {
        return Err(fail(format!("HTTP {}: {}", response.status(), response.text().await.unwrap_or_default())));
    }
    Ok(Box::pin(async_stream::try_stream! {
        let lines = super::sse::response_lines(response, name);
        futures::pin_mut!(lines);
        let mut usage = None;
        loop {
            let line = lines.next().await.ok_or_else(|| fail("provider stream ended before [DONE]".into()))??;
            let Some(data) = super::sse::sse_data(&line) else { continue };
            if data.trim() == "[DONE]" {
                yield LlmStreamChunk { delta: String::new(), done: true, usage, confidence: None };
                break;
            }
            let event: Value = serde_json::from_str(data).map_err(|e| fail(format!("invalid stream event: {e}")))?;
            if let Some(error) = event.get("error") { Err(fail(format!("provider stream error: {error}")))?; }
            if event["usage"]["completion_tokens"].is_u64() { usage = Some(extract_usage(&event)); }
            if let Some(text) = event["choices"][0]["delta"]["content"].as_str() {
                if !text.is_empty() {
                    yield LlmStreamChunk { delta: text.into(), done: false, usage: None, confidence: None };
                }
            }
        }
    }))
}

pub(super) fn build_request_body(req: &LlmRequestRef<'_>) -> Value {
    // Multi-message form (46b): OpenAI takes roles verbatim.
    let messages: Value = if req.messages.is_empty() {
        json!([{"role": "user", "content": req.rendered}])
    } else {
        Value::Array(
            req.messages
                .iter()
                .map(message_json)
                .collect(),
        )
    };
    let mut body = json!({
        "model": req.model,
        "messages": messages,
    });
    if let Some(turn) = req.tool_turn {
        body["tools"] = json!(turn.definitions);
        if turn.final_answer_only { body["tool_choice"] = json!("none"); }
        body["messages"].as_array_mut().unwrap().extend(turn.history.iter().cloned());
    }
    if let Some(t) = req.sampling.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = req.sampling.top_p {
        body["top_p"] = json!(p);
    }
    if let Some(mt) = req.sampling.max_tokens {
        body["max_completion_tokens"] = json!(mt);
    }

    if let Some(schema) = &req.output_schema {
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {
                "name": format!("respond_with_{}", req.prompt),
                "strict": true,
                "schema": schema,
            }
        });
    }

    body
}

pub(super) fn tool_call_message(parsed: &Value) -> Result<Option<Value>, RuntimeError> {
    let message = &parsed["choices"][0]["message"];
    let calls = &message["tool_calls"];
    if calls.is_null() { return Ok(None); }
    let calls = calls.as_array().ok_or_else(|| RuntimeError::AdapterFailed {
        adapter: "chat-completions".into(), message: "tool_calls must be an array".into(),
    })?;
    if calls.is_empty() { return Ok(None); }
    Ok(Some(json!({"role": "assistant", "content": message["content"], "tool_calls": calls})))
}

fn extract_response(parsed: &Value, expect_structured: bool) -> Result<LlmResponse, RuntimeError> {
    let content = parsed
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| RuntimeError::AdapterFailed {
            adapter: "openai".into(),
            message: "response missing `choices[0].message.content`".into(),
        })?;

    let usage = extract_usage(parsed);

    if expect_structured {
        let value: Value = serde_json::from_str(content).map_err(|e| {
            RuntimeError::AdapterFailed {
                adapter: "openai".into(),
                message: format!(
                    "structured response was not valid JSON: {e} (content: {content})"
                ),
            }
        })?;
        Ok(LlmResponse::new(value, usage))
    } else {
        Ok(LlmResponse::new(Value::String(content.to_string()), usage))
    }
}

/// Pull the `usage` block out of an OpenAI response. Standard shape:
///   `{"usage": {"prompt_tokens": N, "completion_tokens": N, "total_tokens": N}}`
/// Returns zeros if missing — older endpoints + some compat servers
/// don't report usage; treating absence as "unknown" is safer than
/// failing the call.
pub(crate) fn extract_usage(parsed: &Value) -> TokenUsage {
    let usage = parsed.get("usage");
    let prompt_tokens = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let completion_tokens = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let total_tokens = usage
        .and_then(|u| u.get("total_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or((prompt_tokens + completion_tokens) as u64) as u32;
    TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    }
}

fn message_json(m: &crate::llm::LlmMessage) -> serde_json::Value {
    if m.images.is_empty() { return json!({"role":m.role,"content":m.content}); }
    let mut parts = vec![json!({"type":"text","text":m.content})];
    parts.extend(m.images.iter().map(|image| json!({"type":"image_url","image_url":{
        "url":format!("data:{};base64,{}", image.media_type, image.data_base64)
    }})));
    json!({"role":m.role,"content":parts})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmRequest;

    #[test]
    fn handles_gpt_and_o_series_models() {
        let a = OpenAiAdapter::new("k");
        assert!(a.handles("gpt-4o-mini"));
        assert!(a.handles("gpt-4.1"));
        assert!(a.handles("o1-preview"));
        assert!(a.handles("o3-mini"));
        assert!(a.handles("o4-mini"));
        assert!(!a.handles("claude-opus-4-6"));
        assert!(!a.handles("gemini-pro"));
    }

    #[test]
    fn body_builds_message_array_with_roles() {
        // Slice 46b: OpenAI takes roles verbatim.
        let req = LlmRequest {
            prompt: "ask".into(),
            model: "gpt-4o-mini".into(),
            rendered: "[system] Be terse.
[user] hi".into(),
            args: vec![],
            output_schema: None,
            sampling: Default::default(),
            messages: vec![
                crate::llm::LlmMessage {
                    images: Vec::new(),
                    role: "system".into(),
                    content: "Be terse.".into(),
                },
                crate::llm::LlmMessage {
                    images: Vec::new(),
                    role: "user".into(),
                    content: "hi".into(),
                },
            ],
        };
        let body = build_request_body(&req.as_ref());
        assert_eq!(
            body["messages"],
            json!([
                {"role": "system", "content": "Be terse."},
                {"role": "user", "content": "hi"}
            ])
        );
    }

    #[test]
    fn body_includes_response_format_when_schema_present() {
        let req = LlmRequest {
            prompt: "decide".into(),
            model: "gpt-4o-mini".into(),
            rendered: "decide pls".into(),
            args: vec![],
            sampling: Default::default(),
            messages: Vec::new(),
            output_schema: Some(json!({
                "type": "object",
                "properties": {"x": {"type": "boolean"}},
                "required": ["x"],
                "additionalProperties": false,
            })),
        };
        let body = build_request_body(&req.as_ref());
        let rf = &body["response_format"];
        assert_eq!(rf["type"], "json_schema");
        assert_eq!(rf["json_schema"]["name"], "respond_with_decide");
        assert_eq!(rf["json_schema"]["strict"], true);
        assert!(rf["json_schema"]["schema"].is_object());
    }

    #[test]
    fn body_omits_response_format_when_no_schema() {
        let req = LlmRequest {
            prompt: "chat".into(),
            model: "gpt-4o-mini".into(),
            rendered: "say hi".into(),
            args: vec![],
            sampling: Default::default(),
            messages: Vec::new(),
            output_schema: None,
        };
        let body = build_request_body(&req.as_ref());
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn extract_parses_json_content_for_structured() {
        let parsed = json!({
            "choices": [
                {"message": {"role": "assistant", "content": "{\"should_refund\": true}"}}
            ]
        });
        let resp = extract_response(&parsed, true).unwrap();
        assert_eq!(resp.value, json!({"should_refund": true}));
    }

    #[test]
    fn extract_returns_raw_string_for_unstructured() {
        let parsed = json!({
            "choices": [
                {"message": {"role": "assistant", "content": "hello world"}}
            ]
        });
        let resp = extract_response(&parsed, false).unwrap();
        assert_eq!(resp.value, json!("hello world"));
    }

    #[test]
    fn extract_errors_on_invalid_json_in_structured_response() {
        let parsed = json!({
            "choices": [
                {"message": {"role": "assistant", "content": "this is not json"}}
            ]
        });
        let err = extract_response(&parsed, true).unwrap_err();
        assert!(matches!(err, RuntimeError::AdapterFailed { .. }));
    }
}
