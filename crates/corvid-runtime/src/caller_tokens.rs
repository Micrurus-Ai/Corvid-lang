//! A person's provider token, kept at sign-in for `by.token("<provider>")`
//! (user credentials C2).
//!
//! Sign-in keeps the access token only for a provider the identity block
//! declares with `keep_token: scopes(...)`. The token is held here, in
//! memory only: a restart, or another instance, sees no token until the
//! person signs in again. Keeping tokens across restarts waits on
//! encryption at rest. Each read is recorded like a tool call
//! (`caller:token`), noting only whether a token was present, never the
//! token, so a replay takes the same branches without holding the secret.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use corvid_crypto::Secret;
use corvid_trace_schema::TraceEvent;

use crate::errors::RuntimeError;
use crate::tracing::now_ms;
use crate::Runtime;

/// The tool name a token read is recorded under.
pub const CALLER_TOKEN_CALL: &str = "caller:token";

/// What a replayed present token reads as. Any call that would use it is
/// itself served from the trace, so it never reaches a provider.
const REPLAYED_TOKEN: &str = "<a token the recorded run held>";

struct Kept {
    token: Secret,
    /// When the provider said it expires, if it did.
    expires_ms: Option<u64>,
}

/// The tokens kept for signed-in people, by person and provider.
#[derive(Clone, Default)]
pub struct CallerTokens {
    kept: Arc<Mutex<HashMap<(String, String), Kept>>>,
}

impl CallerTokens {
    /// Keep `token` for `actor`'s sign-in with `provider`, replacing an
    /// older one.
    pub fn keep(&self, actor: &str, provider: &str, token: Secret, expires_ms: Option<u64>) {
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        kept.insert((actor.to_string(), provider.to_string()), Kept { token, expires_ms });
    }

    /// The person's live token for `provider`, if one is kept.
    pub fn get(&self, actor: &str, provider: &str, now_ms: u64) -> Option<Secret> {
        let kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        kept.get(&(actor.to_string(), provider.to_string()))
            .filter(|k| k.expires_ms.is_none_or(|expires| now_ms < expires))
            .map(|k| k.token.clone())
    }
}

impl Runtime {
    /// The tokens this runtime keeps, which sign-in fills.
    pub fn caller_tokens(&self) -> &CallerTokens {
        &self.caller_tokens
    }

    /// `by.token("<provider>")`: the person's kept token, or `None` when it
    /// expired, was never kept, or was kept by another process. Recorded
    /// as presence only; a replay answers from the record.
    pub fn caller_token(&self, actor: &str, provider: &str) -> Result<Option<Secret>, RuntimeError> {
        let args = vec![serde_json::json!(provider)];
        if let Some(replay) = self.replay_source()? {
            let recorded = replay.replay_tool_call(CALLER_TOKEN_CALL, &args)?;
            let present = recorded.get("present").and_then(|p| p.as_bool()).unwrap_or(false);
            return Ok(present.then(|| Secret::from_text(REPLAYED_TOKEN.to_string())));
        }
        let token = self.caller_tokens.get(actor, provider, now_ms());
        if self.tracer().is_enabled() {
            self.tracer().emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer().run_id().to_string(),
                tool: CALLER_TOKEN_CALL.to_string(),
                args,
            });
            self.tracer().emit(TraceEvent::ToolResult {
                ts_ms: now_ms(),
                run_id: self.tracer().run_id().to_string(),
                tool: CALLER_TOKEN_CALL.to_string(),
                result: serde_json::json!({ "present": token.is_some() }),
            });
        }
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_token_is_read_back_until_it_expires() {
        let tokens = CallerTokens::default();
        tokens.keep("ana", "github", Secret::from_text("gho_abc".to_string()), Some(1_000));
        assert_eq!(tokens.get("ana", "github", 999).and_then(|s| s.expose_text().map(str::to_string)), Some("gho_abc".into()));
        assert!(tokens.get("ana", "github", 1_000).is_none(), "an expired token is gone");
        assert!(tokens.get("bo", "github", 0).is_none(), "another person's token is never read");
        assert!(tokens.get("ana", "google", 0).is_none());
    }

    #[test]
    fn a_read_is_recorded_without_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.jsonl");
        let runtime = Runtime::builder().tracer(crate::tracing::Tracer::open_path(&path, "run-token")).build();
        runtime.caller_tokens().keep("ana", "github", Secret::from_text("gho_secret_value".to_string()), None);
        assert!(runtime.caller_token("ana", "github").unwrap().is_some());
        assert!(runtime.caller_token("bo", "github").unwrap().is_none());
        drop(runtime);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("gho_secret_value"), "the token never enters the trace: {text}");
        assert!(text.contains("\"present\":true") && text.contains("\"present\":false"), "{text}");
    }
}
