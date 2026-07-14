//! MCP client (slice 46f) — consume Model Context Protocol tool
//! servers as governed Corvid tools.
//!
//! "MCP with governance": every MCP call flows through the ONE
//! executing surface `mcp_call(server, tool, args_json)`, which is
//! a standard stdlib tool dispatch — traced (`tool_call` /
//! `tool_result` events), replay-quarantined (substitute mode
//! never contacts a server), budget-accounted through its effect
//! row, and APPROVAL-GATED: servers are untrusted by default, and
//! an untrusted server's calls go through the runtime `Approver`
//! (interactive stdin in `corvid run`); `trust = "autonomous"` in
//! `corvid.toml` loosens a server explicitly.
//!
//! Transports: stdio (newline-delimited JSON-RPC over a spawned
//! child process, connection cached per server) and HTTP (one
//! JSON-RPC POST per call). Server transport SSE streaming is out
//! of scope in v1.

use crate::errors::RuntimeError;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// One configured MCP server (mirrors `[mcp.servers.<name>]`).
#[derive(Debug, Clone, Default)]
pub struct McpServerConfig {
    /// stdio transport: the command line to spawn.
    pub command: Vec<String>,
    /// HTTP transport: the JSON-RPC endpoint. Takes precedence over
    /// `command` when both are set.
    pub url: Option<String>,
    /// `trust = "autonomous"` — calls skip the runtime approver.
    /// Default FALSE: unknown MCP tools require approval.
    pub trusted: bool,
}

/// A live stdio connection: the child process plus framed pipes.
struct StdioConnection {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Drop for StdioConnection {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Per-runtime MCP state: configs + cached stdio connections.
#[derive(Clone, Default)]
pub struct McpRuntime {
    servers: HashMap<String, McpServerConfig>,
    connections: Arc<Mutex<HashMap<String, StdioConnection>>>,
    next_id: Arc<AtomicU64>,
    http: reqwest::Client,
}

impl std::fmt::Debug for McpRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpRuntime")
            .field("servers", &self.servers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl McpRuntime {
    pub fn new(servers: HashMap<String, McpServerConfig>) -> Self {
        Self {
            servers,
            connections: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("reqwest client builds with default config"),
        }
    }

    pub fn server(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.get(name)
    }

    fn err(server: &str, message: impl std::fmt::Display) -> RuntimeError {
        RuntimeError::ToolFailed {
            tool: format!("mcp:{server}"),
            message: message.to_string(),
        }
    }

    /// Perform `tools/call` against a configured server.
    pub async fn call(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let config = self
            .servers
            .get(server)
            .ok_or_else(|| Self::err(server, "server is not configured in [mcp.servers]"))?
            .clone();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments },
        });
        let response = if let Some(url) = &config.url {
            self.call_http(server, url, request).await?
        } else if !config.command.is_empty() {
            let this = self.clone();
            let server_owned = server.to_string();
            // Blocking pipe I/O off the async executor.
            tokio::task::spawn_blocking(move || {
                this.call_stdio(&server_owned, &request)
            })
            .await
            .map_err(|e| Self::err(server, format!("stdio task failed: {e}")))??
        } else {
            return Err(Self::err(
                server,
                "server config needs either `command` (stdio) or `url` (http)",
            ));
        };

        if let Some(error) = response.get("error") {
            return Err(Self::err(server, format!("JSON-RPC error: {error}")));
        }
        let result = response
            .get("result")
            .cloned()
            .ok_or_else(|| Self::err(server, "response carried no result"))?;
        Ok(extract_content(result))
    }

    /// Perform `tools/list` against a configured server — the
    /// discovery half of `corvid add mcp` / `corvid mcp regen`.
    /// Returns the raw tool entries (`name`, `description`,
    /// `inputSchema`).
    pub async fn list_tools(
        &self,
        server: &str,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let config = self
            .servers
            .get(server)
            .ok_or_else(|| Self::err(server, "server is not configured in [mcp.servers]"))?
            .clone();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": {},
        });
        let response = if let Some(url) = &config.url {
            self.call_http(server, url, request).await?
        } else if !config.command.is_empty() {
            let this = self.clone();
            let server_owned = server.to_string();
            tokio::task::spawn_blocking(move || this.call_stdio(&server_owned, &request))
                .await
                .map_err(|e| Self::err(server, format!("stdio task failed: {e}")))??
        } else {
            return Err(Self::err(
                server,
                "server config needs either `command` (stdio) or `url` (http)",
            ));
        };
        if let Some(error) = response.get("error") {
            return Err(Self::err(server, format!("JSON-RPC error: {error}")));
        }
        let tools = response
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(|t| t.as_array())
            .cloned()
            .ok_or_else(|| Self::err(server, "tools/list response carried no tools array"))?;
        Ok(tools)
    }

    async fn call_http(
        &self,
        server: &str,
        url: &str,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let resp = self
            .http
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Self::err(server, format!("HTTP send failed: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Self::err(server, format!("reading response failed: {e}")))?;
        if !status.is_success() {
            return Err(Self::err(server, format!("HTTP {status}: {text}")));
        }
        serde_json::from_str(&text)
            .map_err(|e| Self::err(server, format!("response is not JSON: {e}")))
    }

    fn call_stdio(
        &self,
        server: &str,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| Self::err(server, "connection cache poisoned"))?;
        if !connections.contains_key(server) {
            let config = self
                .servers
                .get(server)
                .ok_or_else(|| Self::err(server, "server vanished from config"))?;
            connections.insert(server.to_string(), spawn_stdio(server, config)?);
        }
        let conn = connections
            .get_mut(server)
            .expect("connection just inserted");
        match stdio_round_trip(conn, request) {
            Ok(v) => Ok(v),
            Err(e) => {
                // Drop the dead connection so the next call respawns.
                connections.remove(server);
                Err(Self::err(server, e))
            }
        }
    }
}

fn spawn_stdio(
    server: &str,
    config: &McpServerConfig,
) -> Result<StdioConnection, RuntimeError> {
    let mut cmd = std::process::Command::new(&config.command[0]);
    cmd.args(&config.command[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| RuntimeError::ToolFailed {
        tool: format!("mcp:{server}"),
        message: format!("failed to spawn `{}`: {e}", config.command[0]),
    })?;
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut conn = StdioConnection {
        child,
        stdin,
        stdout,
    };
    // MCP handshake: initialize + initialized notification.
    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "corvid", "version": env!("CARGO_PKG_VERSION")},
        },
    });
    stdio_round_trip(&mut conn, &init).map_err(|e| RuntimeError::ToolFailed {
        tool: format!("mcp:{server}"),
        message: format!("initialize failed: {e}"),
    })?;
    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    });
    write_line(&mut conn.stdin, &initialized).map_err(|e| RuntimeError::ToolFailed {
        tool: format!("mcp:{server}"),
        message: e,
    })?;
    Ok(conn)
}

fn write_line(stdin: &mut std::process::ChildStdin, value: &serde_json::Value) -> Result<(), String> {
    let mut line = serde_json::to_string(value).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|e| format!("stdio write failed: {e}"))
}

/// Send one request and read lines until its matching-id response
/// (skipping server-initiated notifications).
fn stdio_round_trip(
    conn: &mut StdioConnection,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    write_line(&mut conn.stdin, request)?;
    let want_id = request.get("id").cloned();
    loop {
        let mut line = String::new();
        let n = conn
            .stdout
            .read_line(&mut line)
            .map_err(|e| format!("stdio read failed: {e}"))?;
        if n == 0 {
            return Err("server closed the pipe".to_string());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue; // non-JSON noise on stdout
        };
        if value.get("id").cloned() == want_id {
            return Ok(value);
        }
        // Notification or unrelated id — keep reading.
    }
}

/// MCP `tools/call` results carry `content: [{type: "text", text}]`
/// blocks and an `isError` flag. Concatenate text blocks; surface
/// isError as a JSON marker the dispatch turns into `Err`.
fn extract_content(result: serde_json::Value) -> serde_json::Value {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let text = match result.get("content").and_then(|c| c.as_array()) {
        Some(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        None => result.to_string(),
    };
    serde_json::json!({ "is_error": is_error, "text": text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_content_concatenates_text_blocks() {
        let result = serde_json::json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "text", "text": "world"},
            ],
            "isError": false,
        });
        let out = extract_content(result);
        assert_eq!(out["text"], "hello\nworld");
        assert_eq!(out["is_error"], false);
    }

    #[test]
    fn extract_content_surfaces_error_flag() {
        let result = serde_json::json!({
            "content": [{"type": "text", "text": "boom"}],
            "isError": true,
        });
        let out = extract_content(result);
        assert_eq!(out["is_error"], true);
    }
}
