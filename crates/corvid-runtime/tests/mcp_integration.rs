//! MCP client integration tests (slice 46f).
//!
//! Pinned behavior:
//! 1. `mcp_call` against a TRUSTED HTTP server round-trips
//!    JSON-RPC `tools/call` and returns the text content as Ok.
//! 2. An UNTRUSTED server's call goes through the runtime
//!    approver; denial returns an Err VALUE naming the loosening
//!    path — never a trap, and no transport I/O happens.
//! 3. Tool-side `isError` results surface as Err values.

use corvid_runtime::mcp::McpServerConfig;
use corvid_runtime::{ApprovalDecision, ProgrammaticApprover, Runtime};
use std::collections::HashMap;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn servers(url: &str, trusted: bool) -> HashMap<String, McpServerConfig> {
    let mut map = HashMap::new();
    map.insert(
        "notes".to_string(),
        McpServerConfig {
            command: Vec::new(),
            url: Some(url.to_string()),
            trusted,
        },
    );
    map
}

#[tokio::test]
async fn trusted_http_server_round_trips() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [{"type": "text", "text": "three notes found"}],
                "isError": false,
            },
        })))
        .mount(&server)
        .await;

    let rt = Runtime::builder()
        .mcp_servers(servers(&format!("{}/rpc", server.uri()), true))
        .build();
    let result = rt
        .call_tool(
            "mcp_call",
            vec![
                serde_json::json!("notes"),
                serde_json::json!("search"),
                serde_json::json!("{\"query\": \"corvid\"}"),
            ],
        )
        .await
        .expect("dispatch succeeds");
    assert_eq!(result["tag"], "ok");
    assert_eq!(result["ok"], "three notes found");
}

#[tokio::test]
async fn untrusted_server_denial_is_an_err_value() {
    // No wiremock mount: denial must short-circuit BEFORE any
    // transport I/O, so no request may arrive.
    let rt = Runtime::builder()
        .mcp_servers(servers("http://127.0.0.1:9/rpc", false))
        .approver(Arc::new(ProgrammaticApprover::new(|_req| {
            ApprovalDecision::Deny
        })))
        .build();
    let result = rt
        .call_tool(
            "mcp_call",
            vec![
                serde_json::json!("notes"),
                serde_json::json!("search"),
                serde_json::json!("{}"),
            ],
        )
        .await
        .expect("denial is a value, not a trap");
    assert_eq!(result["tag"], "err");
    let message = result["err"].as_str().unwrap_or_default();
    assert!(
        message.contains("approval denied") && message.contains("autonomous"),
        "denial names the loosening path: {message}"
    );
}

#[tokio::test]
async fn tool_side_error_surfaces_as_err() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [{"type": "text", "text": "no such directory"}],
                "isError": true,
            },
        })))
        .mount(&server)
        .await;

    let rt = Runtime::builder()
        .mcp_servers(servers(&format!("{}/rpc", server.uri()), true))
        .build();
    let result = rt
        .call_tool(
            "mcp_call",
            vec![
                serde_json::json!("notes"),
                serde_json::json!("list"),
                serde_json::json!("{}"),
            ],
        )
        .await
        .expect("dispatch succeeds");
    assert_eq!(result["tag"], "err");
    assert_eq!(result["err"], "no such directory");
}
