//! Integration tests for `AnthropicAdapter` against a wiremock server.
//! Proves request shape, header presence, and structured-output extraction
//! without touching the real Anthropic API.

use corvid_runtime::{AnthropicAdapter, LlmAdapter, LlmRequest};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn structured_call_sends_tool_definition_and_extracts_input() {
    let server = MockServer::start().await;

    let response = ResponseTemplate::new(200).set_body_json(json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "content": [
            {
                "type": "tool_use",
                "id": "tu_1",
                "name": "respond_with_decide",
                "input": { "should_refund": true, "reason": "legit complaint" }
            }
        ],
        "model": "claude-haiku-4-5",
        "stop_reason": "tool_use"
    }));

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(header("x-api-key", "sk-test-anthropic"))
        .respond_with(response)
        .mount(&server)
        .await;

    let adapter =
        AnthropicAdapter::new("sk-test-anthropic").with_base_url(server.uri());
    let req = LlmRequest {
        prompt: "decide".into(),
        model: "claude-haiku-4-5".into(),
        rendered: "Decide.".into(),
        args: vec![],
        sampling: Default::default(),
        messages: Vec::new(),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "should_refund": {"type": "boolean"},
                "reason": {"type": "string"}
            },
            "required": ["should_refund", "reason"],
            "additionalProperties": false
        })),
    };

    let resp = adapter.call(&req.as_ref()).await.expect("adapter call");
    assert_eq!(
        resp.value,
        json!({"should_refund": true, "reason": "legit complaint"})
    );

    // Inspect the request the adapter actually sent.
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let body: serde_json::Value =
        serde_json::from_slice(&received[0].body).expect("request body is JSON");
    assert_eq!(body["model"], "claude-haiku-4-5");
    assert_eq!(body["messages"][0]["content"], "Decide.");
    assert_eq!(body["tools"][0]["name"], "respond_with_decide");
    assert_eq!(body["tool_choice"]["name"], "respond_with_decide");
}

#[tokio::test]
async fn unstructured_call_concatenates_text_blocks() {
    let server = MockServer::start().await;
    let response = ResponseTemplate::new(200).set_body_json(json!({
        "content": [
            {"type": "text", "text": "hello "},
            {"type": "text", "text": "world"}
        ]
    }));
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(response)
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("k").with_base_url(server.uri());
    let req = LlmRequest {
        prompt: "chat".into(),
        model: "claude-haiku-4-5".into(),
        rendered: "say hi".into(),
        args: vec![],
        sampling: Default::default(),
        messages: Vec::new(),
        output_schema: None,
    };
    let resp = adapter.call(&req.as_ref()).await.unwrap();
    assert_eq!(resp.value, json!("hello world"));
}

#[tokio::test]
async fn http_error_surfaces_as_adapter_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("bad").with_base_url(server.uri());
    let req = LlmRequest {
        prompt: "x".into(),
        model: "claude-haiku-4-5".into(),
        rendered: "".into(),
        args: vec![],
        sampling: Default::default(),
        messages: Vec::new(),
        output_schema: None,
    };
    let err = adapter.call(&req.as_ref()).await.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("anthropic"));
    assert!(msg.contains("401"));
}

#[tokio::test]
async fn streams_sse_deltas_incrementally() {
    // Slice 46d: real SSE parsing — three content_block_delta
    // events + message_stop become three deltas and a done marker.
    use futures::StreamExt;
    let server = MockServer::start().await;
    let sse_body = concat!(
        "event: message_start
",
        "data: {\"type\":\"message_start\"}
",
        "
",
        "event: content_block_delta
",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}
",
        "
",
        "event: content_block_delta
",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo \"}}
",
        "
",
        "event: content_block_delta
",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}
",
        "
",
        "event: message_stop
",
        "data: {\"type\":\"message_stop\"}
",
    );
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse_body, "text/event-stream"),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("sk-test").with_base_url(server.uri());
    let req = LlmRequest {
        prompt: "chat".into(),
        model: "claude-haiku-4-5".into(),
        rendered: "say hi".into(),
        args: vec![],
        output_schema: None,
        sampling: Default::default(),
        messages: Vec::new(),
    };
    let req_ref = req.as_ref();
    let mut stream = adapter.stream(&req_ref).await.expect("stream setup");
    let mut deltas = Vec::new();
    let mut done = false;
    while let Some(item) = stream.next().await {
        let chunk = item.expect("chunk ok");
        if !chunk.delta.is_empty() {
            deltas.push(chunk.delta.clone());
        }
        if chunk.done {
            done = true;
            break;
        }
    }
    assert_eq!(deltas, vec!["Hel", "lo ", "world"]);
    assert!(done, "message_stop must yield a done chunk");
}
