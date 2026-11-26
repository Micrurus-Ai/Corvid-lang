//! Replaying `random_token`, whose token the trace never holds.
//!
//! A token is a `Secret`, so its recorded `ToolResult` is `<redacted>`.
//! Replay cannot substitute it, and refusing to replay would make every
//! program that issues a session or an API key unreplayable, so replay
//! steps past the recorded call, draws a fresh token, and says so: each
//! re-draw prints a warning and keeps it for
//! `Runtime::replay_redrawn_token_warnings`. The token feeds only
//! secret-typed positions, which every record holds as `<redacted>`, so
//! a replayed call that takes it (such as `hash_password`) still matches
//! its recording and is substituted. Where the fresh token changes what
//! the run computes, such as `verify_password` against a hash recorded
//! from the old token, replay reports the divergence where the
//! recording stops matching. The replay never differs silently.

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(crate) struct ReplayRedrawnTokens {
    warnings: Arc<Mutex<Vec<String>>>,
}

impl ReplayRedrawnTokens {
    /// Prints and keeps the warning for one re-drawn token.
    pub(crate) fn note(&self, tool: &str) {
        let warning = format!(
            "warning: replay drew a fresh token for `{tool}`, because the recorded token was never stored in the trace; anything this run computes from the token's value can differ from the recording from here on"
        );
        eprintln!("{warning}");
        self.warnings.lock().expect("token warning lock").push(warning);
    }

    pub(crate) fn warnings(&self) -> Vec<String> {
        self.warnings.lock().expect("token warning lock").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_redraw_is_kept_as_a_warning_naming_the_tool() {
        let tokens = ReplayRedrawnTokens::default();
        tokens.note("random_token");
        tokens.note("random_token");
        let warnings = tokens.warnings();
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("`random_token`") && warnings[0].contains("never stored"), "{}", warnings[0]);
    }
}
