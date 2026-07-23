//! Real-mode connector HTTP dispatch on `Runtime` (slice 52g-3c-4).
//!
//! Reached from the connector branch of [`Runtime::call_tool`] only in
//! `real` mode (a replay source short-circuits earlier, so replay never
//! performs a real request). Resolves the credential from the secret
//! store at the last moment, sends the request through the egress-gated
//! HTTP client, and hands the response body back for the VM to decode.
//!
//! The credential value appears ONLY in the outgoing request header —
//! never in the ToolCall arguments, the ToolResult body, or an error.

use super::Runtime;
use crate::connectors::{build_connector_request, ConnectorHttpSpec};
use crate::errors::RuntimeError;

impl Runtime {
    pub(super) async fn dispatch_connector_http(
        &self,
        spec: &ConnectorHttpSpec,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, RuntimeError> {
        // Resolve credentials from the secret store at the last moment.
        // The resolver hands the value ONLY to the request builder, which
        // places it in a header; it is never returned or recorded.
        let resolve = |name: &str| -> Option<String> {
            self.secrets.read_env(name).ok().and_then(|read| read.value)
        };
        let request =
            build_connector_request(spec, args, &resolve).map_err(|e| e.into_runtime_error())?;

        // Outbound egress gate: the always-on SSRF block plus the
        // configured `[http] allow` list. A connector never reaches an
        // unpermitted host, even though the base URL is fixed in source.
        self.http_policy.check(&request.url)?;

        let response = self.http.send(&request).await?;

        // A 2xx response body is decoded to the operation's return type
        // by the VM. A non-2xx status without a declared `on status`
        // mapping is a transport-level failure. Typed status->error
        // mapping (turning a mapped status into a typed error variant)
        // lands in the next sub-slice.
        if (200..300).contains(&response.status) {
            let body: serde_json::Value =
                serde_json::from_str(&response.body).unwrap_or(serde_json::Value::Null);
            Ok(body)
        } else {
            Err(RuntimeError::ToolFailed {
                tool: spec.operation.clone(),
                message: format!("provider returned HTTP {}", response.status),
            })
        }
    }
}
