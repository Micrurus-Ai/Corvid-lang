//! Recording and replaying table operations (B09 P1b).
//!
//! A table operation is recorded like a tool call: a `ToolCall` event
//! named `table:<Table>.<op>` with its arguments, then a `ToolResult`
//! with the outcome. Replay serves the recorded outcome through the
//! same cursor tool calls use, so a replayed run reproduces the rows it
//! read and the outcomes of its writes without touching the database,
//! and a run whose operations differ from the trace diverges loudly.

use super::Runtime;
use crate::errors::RuntimeError;
use crate::tracing::now_ms;
use corvid_trace_schema::TraceEvent;
use serde_json::Value as J;
use std::future::Future;

impl Runtime {
    /// Run `live` and record it, or serve it from the replay trace.
    pub(crate) async fn recorded_table_call<F, Fut>(
        &self,
        name: &str,
        args: Vec<J>,
        live: F,
    ) -> Result<J, RuntimeError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<J, RuntimeError>>,
    {
        if let Some(replay) = self.replay_source()? {
            return replay.replay_tool_call(name, &args);
        }
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: name.to_string(),
                args,
            });
        }
        let outcome = live().await;
        if self.tracer.is_enabled() {
            let result = match &outcome {
                Ok(value) => value.clone(),
                Err(error) => serde_json::json!({ crate::replay::CORVID_TOOL_ERROR_KEY: error.to_string() }),
            };
            self.tracer.emit(TraceEvent::ToolResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: name.to_string(),
                result,
            });
        }
        outcome
    }
}
