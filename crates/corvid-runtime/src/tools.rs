//! Tool registry and dispatch.
//!
//! Tools are registered by name with an async handler. The handler takes a
//! JSON array of arguments and returns a JSON value. Speaking JSON at the
//! boundary keeps the runtime independent of the interpreter's `Value`
//! type and matches the wire format of every real LLM tool protocol.

use crate::errors::RuntimeError;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::sync::Arc;

/// Type-erased async tool handler. Cheap to clone (it's an `Arc`).
pub type ToolHandler = Arc<
    dyn Fn(Vec<serde_json::Value>) -> BoxFuture<'static, Result<serde_json::Value, RuntimeError>>
        + Send
        + Sync,
>;

/// Map of tool name → handler.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolHandler>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool. The handler is any closure returning a future.
    ///
    /// ```ignore
    /// registry.register("get_order", |args| async move {
    ///     let id = args[0].as_str().unwrap_or("");
    ///     Ok(serde_json::json!({ "id": id, "amount": 19.99 }))
    /// });
    /// ```
    pub fn register<F, Fut>(&mut self, name: impl Into<String>, handler: F)
    where
        F: Fn(Vec<serde_json::Value>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<serde_json::Value, RuntimeError>>
            + Send
            + 'static,
    {
        let boxed: ToolHandler = Arc::new(move |args| Box::pin(handler(args)));
        self.tools.insert(name.into(), boxed);
    }

    /// Dispatch a tool call. A name with no handler here is looked up
    /// among the host's tools: each `#[tool]` linked into a native binary
    /// registers there at startup, and an embedding host registers its
    /// own. Returns `UnknownTool` when neither holds it.
    pub async fn call(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, RuntimeError> {
        match self.tools.get(name).cloned() {
            Some(handler) => handler(args).await,
            None if crate::catalog_c_api::host_tool_registered(name) => call_host_tool(name, args).await,
            None => Err(RuntimeError::UnknownTool(name.to_string())),
        }
    }

    /// Whether a handler is registered for `name`. Useful for diagnostics.
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Names of all registered tools, alphabetized. Used by tracing.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tools.keys().cloned().collect();
        v.sort();
        v
    }

    /// Merge `other`'s handlers into `self`. Entries in `other`
    /// OVERWRITE same-named entries in `self`. Used by `corvid serve`
    /// to compose `tools.py` autoloaded handlers (registered first)
    /// with `--with-tools-cdylib` handlers (registered second), so
    /// the explicit operator flag wins precedence over the implicit
    /// autoload — see `crates/corvid-cli/src/serve_cmd.rs::cmd_serve`
    /// for the call site.
    pub fn extend(&mut self, other: ToolRegistry) {
        for (name, handler) in other.tools {
            self.tools.insert(name, handler);
        }
    }
}

/// Run a host tool. Its entry point takes the arguments as one JSON
/// array and may wait on the tokio runtime itself (a `#[tool]` runs its
/// `async fn` with `block_on`), so it runs on a blocking thread, never
/// on the task that awaits it.
async fn call_host_tool(name: &str, args: Vec<serde_json::Value>) -> Result<serde_json::Value, RuntimeError> {
    let tool = name.to_string();
    let in_flight = HostToolInFlight::start();
    tokio::task::spawn_blocking(move || {
        let _in_flight = in_flight;
        crate::catalog_c_api::call_registered_host_tool(&tool, &args)
    })
        .await
        .map_err(|e| RuntimeError::ToolFailed { tool: name.to_string(), message: format!("the implementation stopped: {e}") })?
}

/// Host tool calls still running. A call a `try ... timeout` gave up on
/// keeps running on its blocking thread, as a dropped future's blocking
/// task does on the interpreter; its work may have side effects, so it
/// is never cut off midway, and the runtime waits for it before shutting
/// down rather than tearing the tokio runtime out from under it.
static HOST_TOOLS_IN_FLIGHT: (std::sync::Mutex<usize>, std::sync::Condvar) =
    (std::sync::Mutex::new(0), std::sync::Condvar::new());

struct HostToolInFlight;

impl HostToolInFlight {
    fn start() -> Self {
        *HOST_TOOLS_IN_FLIGHT.0.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        HostToolInFlight
    }
}

impl Drop for HostToolInFlight {
    fn drop(&mut self) {
        let mut count = HOST_TOOLS_IN_FLIGHT.0.lock().unwrap_or_else(|p| p.into_inner());
        *count -= 1;
        HOST_TOOLS_IN_FLIGHT.1.notify_all();
    }
}

/// Wait until every host tool call still running has finished.
pub(crate) fn wait_for_host_tools() {
    let mut count = HOST_TOOLS_IN_FLIGHT.0.lock().unwrap_or_else(|p| p.into_inner());
    while *count > 0 {
        count = HOST_TOOLS_IN_FLIGHT.1.wait(count).unwrap_or_else(|p| p.into_inner());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn register_and_call() {
        let mut reg = ToolRegistry::new();
        reg.register("double", |args| async move {
            let n = args[0].as_i64().unwrap();
            Ok(json!(n * 2))
        });
        let out = reg.call("double", vec![json!(21)]).await.unwrap();
        assert_eq!(out, json!(42));
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let reg = ToolRegistry::new();
        let err = reg.call("missing", vec![]).await.unwrap_err();
        assert!(matches!(err, RuntimeError::UnknownTool(ref s) if s == "missing"));
    }

    #[tokio::test]
    async fn handler_error_propagates() {
        let mut reg = ToolRegistry::new();
        reg.register("nope", |_| async {
            Err(RuntimeError::ToolFailed {
                tool: "nope".into(),
                message: "boom".into(),
            })
        });
        let err = reg.call("nope", vec![]).await.unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::ToolFailed { ref tool, .. } if tool == "nope"
        ));
    }
}
