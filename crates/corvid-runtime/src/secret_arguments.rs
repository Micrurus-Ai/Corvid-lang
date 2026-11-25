//! Tool arguments that hold a `Secret`.
//!
//! The same two forms carry a connector's file body (B05): the recorded
//! form holds an upload's name, type, size and SHA-256, the revealed form
//! its bytes. Nothing in a file body is a `<redacted>` marker, so nothing
//! is scrubbed from the result.
//!
//! A tool may take a `Secret` parameter, and its implementation must
//! receive the real text. Every record of the call must not: the
//! `ToolCall` event, the replay comparison, an approval request, the
//! observation and OpenTelemetry exports built from the trace. The
//! interpreter therefore builds two forms of the arguments from the
//! declared types. The *recorded* form carries [`REDACTED_SECRET`] at
//! every `Secret` position; every record of the call is built from it.
//! The *revealed* form, held by [`RevealedToolArguments`], carries the
//! text and reaches only the implementation.
//!
//! Replay compares recorded forms on both sides, so a redacted position
//! matches any secret: the replayed run re-reads its secrets from the
//! environment and its recorded form is `<redacted>` at the same
//! positions. Replay never needs, and never sees, the text.
//!
//! A tool that echoes a secret it was given, in its result or in an
//! error message, does not leak it into the record either: the
//! recorded `ToolResult` has every occurrence of each secret's text
//! replaced by [`REDACTED_SECRET`] and carries the
//! [`CORVID_SECRET_SCRUBBED_KEY`] marker. The program receives the
//! tool's result unchanged. Replay substitutes the recorded, scrubbed
//! form, and warns, naming the tool, that it differs from what the live
//! run returned (see `replay::scrubbed_results`).

use serde_json::Value as J;

/// What every trace record writes in place of a secret's text.
pub const REDACTED_SECRET: &str = "<redacted>";

/// The reserved key of a recorded `ToolResult` whose payload had a
/// secret's text replaced. A scrubbed result is recorded as
/// `{"__corvid_secret_scrubbed__": <scrubbed result>}`; a scrubbed error
/// as the error record with this key set to `true`. It sits inside the
/// result payload, as the tool-error key does, so the trace schema is
/// unchanged.
pub const CORVID_SECRET_SCRUBBED_KEY: &str = "__corvid_secret_scrubbed__";

/// The arguments a tool implementation receives when its declared
/// parameters hold a `Secret`, and the secret texts they carry.
#[derive(Clone)]
pub struct RevealedToolArguments {
    args: Vec<J>,
    texts: Vec<String>,
}

impl std::fmt::Debug for RevealedToolArguments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RevealedToolArguments({} arguments, {} secrets)", self.args.len(), self.texts.len())
    }
}

impl RevealedToolArguments {
    /// `revealed` is `recorded` with each `<redacted>` position holding
    /// the secret's text. The texts are found by comparing the two, so
    /// they cannot disagree.
    pub fn new(recorded: &[J], revealed: Vec<J>) -> Self {
        let mut texts = Vec::new();
        for (recorded, revealed) in recorded.iter().zip(&revealed) {
            collect_texts(recorded, revealed, &mut texts);
        }
        texts.retain(|text| !text.is_empty());
        texts.sort();
        texts.dedup();
        // Longest first, so a secret that contains another is replaced whole.
        texts.sort_by_key(|text| std::cmp::Reverse(text.len()));
        Self { args: revealed, texts }
    }

    /// The arguments for the tool implementation.
    pub(crate) fn args(&self) -> &[J] {
        &self.args
    }

    /// `text` with every secret's text replaced by `<redacted>`.
    pub fn scrub_text(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in &self.texts {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), REDACTED_SECRET);
            }
        }
        out
    }

    /// `value` with every secret's text replaced in every string, keys
    /// included. For records of the call only, never the program's value.
    pub fn scrub_json(&self, value: J) -> J {
        match value {
            J::String(text) => J::String(self.scrub_text(&text)),
            J::Array(items) => J::Array(items.into_iter().map(|item| self.scrub_json(item)).collect()),
            J::Object(fields) => {
                J::Object(fields.into_iter().map(|(key, item)| (self.scrub_text(&key), self.scrub_json(item))).collect())
            }
            other => other,
        }
    }

    /// The recorded form of a successful result: unchanged when it holds
    /// no secret's text, otherwise scrubbed and marked.
    pub(crate) fn recorded_result(&self, result: &J) -> J {
        let scrubbed = self.scrub_json(result.clone());
        if &scrubbed == result {
            scrubbed
        } else {
            serde_json::json!({ CORVID_SECRET_SCRUBBED_KEY: scrubbed })
        }
    }

    /// The recorded form of a failed call's error message, marked when
    /// it held a secret's text.
    pub(crate) fn recorded_error(&self, message: &str) -> J {
        let scrubbed = self.scrub_text(message);
        if scrubbed == message {
            serde_json::json!({ crate::replay::CORVID_TOOL_ERROR_KEY: message })
        } else {
            serde_json::json!({
                crate::replay::CORVID_TOOL_ERROR_KEY: scrubbed,
                CORVID_SECRET_SCRUBBED_KEY: true,
            })
        }
    }
}

fn collect_texts(recorded: &J, revealed: &J, texts: &mut Vec<String>) {
    match (recorded, revealed) {
        (J::String(marker), J::String(text)) if marker == REDACTED_SECRET && text != REDACTED_SECRET => {
            texts.push(text.clone());
        }
        (J::Array(a), J::Array(b)) => {
            for (a, b) in a.iter().zip(b) {
                collect_texts(a, b, texts);
            }
        }
        (J::Object(a), J::Object(b)) => {
            for (key, a) in a {
                if let Some(b) = b.get(key) {
                    collect_texts(a, b, texts);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn arguments() -> RevealedToolArguments {
        RevealedToolArguments::new(
            &[json!("payload"), json!({"key": "<redacted>", "label": "live"}), json!(["<redacted>"])],
            vec![json!("payload"), json!({"key": "whsec_abc123", "label": "live"}), json!(["pw-9"])],
        )
    }

    #[test]
    fn finds_the_texts_at_the_redacted_positions() {
        let arguments = arguments();
        assert_eq!(arguments.texts, vec!["whsec_abc123".to_string(), "pw-9".to_string()]);
        assert_eq!(arguments.args()[1]["key"], "whsec_abc123");
    }

    #[test]
    fn records_a_scrubbed_result_and_error_with_the_marker() {
        let arguments = arguments();
        assert_eq!(
            arguments.recorded_result(&json!({"echo": "key=whsec_abc123;pw=pw-9", "n": 1})),
            json!({"__corvid_secret_scrubbed__": {"echo": "key=<redacted>;pw=<redacted>", "n": 1}})
        );
        assert_eq!(arguments.recorded_result(&json!({"ok": true})), json!({"ok": true}), "no echo, no marker");
        assert_eq!(
            arguments.recorded_error("bad key whsec_abc123"),
            json!({"__corvid_tool_error__": "bad key <redacted>", "__corvid_secret_scrubbed__": true})
        );
        assert_eq!(arguments.recorded_error("timed out"), json!({"__corvid_tool_error__": "timed out"}));
    }

    #[test]
    fn debug_never_shows_the_arguments() {
        let shown = format!("{:?}", arguments());
        assert!(!shown.contains("whsec"), "{shown}");
    }
}
