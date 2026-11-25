//! Replaying a tool result whose record had a secret's text scrubbed.
//!
//! A tool that echoes a secret it was given has the text replaced by
//! `<redacted>` in its recorded `ToolResult`, marked with
//! [`CORVID_SECRET_SCRUBBED_KEY`], while the live program received the
//! real result. Replay cannot reproduce the text, which the trace never
//! held, so it substitutes the recorded, scrubbed form and says so: each
//! such substitution prints a warning naming the tool and keeps it for
//! `Runtime::replay_secret_scrub_warnings`. The replay never differs
//! from the recording silently.

use crate::secret_arguments::CORVID_SECRET_SCRUBBED_KEY;
use corvid_trace_schema::TraceEvent;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(crate) struct ReplayScrubbedResults {
    warnings: Arc<Mutex<Vec<String>>>,
}

impl ReplayScrubbedResults {
    /// `event` with a scrubbed tool result unwrapped to its recorded,
    /// scrubbed form, warning when it was scrubbed. Any other event is
    /// returned unchanged.
    pub(crate) fn unwrap(&self, tool: &str, event: TraceEvent) -> TraceEvent {
        let TraceEvent::ToolResult { ts_ms, run_id, tool: recorded_tool, mut result } = event else {
            return event;
        };
        let Some(marker) = result.as_object_mut().and_then(|fields| fields.remove(CORVID_SECRET_SCRUBBED_KEY)) else {
            return TraceEvent::ToolResult { ts_ms, run_id, tool: recorded_tool, result };
        };
        let what = if marker == serde_json::Value::Bool(true) {
            // An error record: the remaining fields are the error.
            "error message"
        } else {
            result = marker;
            "result"
        };
        let warning = format!(
            "warning: replaying tool `{tool}` with its recorded {what}, in which a secret the tool echoed was replaced by `<redacted>`; the live run received the real {what}, so this replay can differ from it from here on"
        );
        eprintln!("{warning}");
        self.warnings.lock().expect("scrub warning lock").push(warning);
        TraceEvent::ToolResult { ts_ms, run_id, tool: recorded_tool, result }
    }

    pub(crate) fn warnings(&self) -> Vec<String> {
        self.warnings.lock().expect("scrub warning lock").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(value: serde_json::Value) -> TraceEvent {
        TraceEvent::ToolResult { ts_ms: 0, run_id: "r".into(), tool: "echo".into(), result: value }
    }

    #[test]
    fn unwraps_a_scrubbed_result_and_warns() {
        let results = ReplayScrubbedResults::default();
        let event = results.unwrap("echo", result(json!({"__corvid_secret_scrubbed__": "you sent <redacted>"})));
        let TraceEvent::ToolResult { result: value, .. } = event else { panic!() };
        assert_eq!(value, json!("you sent <redacted>"));
        let warnings = results.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("`echo`") && warnings[0].contains("recorded result"), "{}", warnings[0]);
    }

    #[test]
    fn unwraps_a_scrubbed_error_and_leaves_others_alone() {
        let results = ReplayScrubbedResults::default();
        let event = results.unwrap(
            "fail",
            result(json!({"__corvid_tool_error__": "bad <redacted>", "__corvid_secret_scrubbed__": true})),
        );
        let TraceEvent::ToolResult { result: value, .. } = event else { panic!() };
        assert_eq!(value, json!({"__corvid_tool_error__": "bad <redacted>"}));
        assert!(results.warnings()[0].contains("recorded error message"));
        let plain = results.unwrap("echo", result(json!({"ok": 1})));
        let TraceEvent::ToolResult { result: value, .. } = plain else { panic!() };
        assert_eq!(value, json!({"ok": 1}));
        assert_eq!(results.warnings().len(), 1);
    }
}
