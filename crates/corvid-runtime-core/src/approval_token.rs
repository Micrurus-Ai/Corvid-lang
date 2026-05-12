//! Approval-token state machine.
//!
//! `ApprovalToken` is the single-use receipt issued by the
//! approval gate when a request is granted. The next dangerous
//! call must present the token to the runtime; `validate`
//! enforces the scope (`OneTime`, `Session`, `AmountLimited`,
//! `TimeLimited`, `ArgumentBound`) and bumps the
//! `uses_remaining` counter on each successful validation.
//!
//! Tokens are inert structs across the wire — no embedded
//! cryptographic signing. The runtime trusts the in-process
//! issuer, so token forgery is bounded by Rust's memory safety
//! model. Cross-process token transfer would require a signed
//! envelope; that's filed as a future invention slice.
//!
//! Lives in `corvid-runtime-core` so the browser playground can
//! issue + validate approval tokens without depending on the
//! native runtime's tokio / DB / OAuth surface. `corvid-runtime`
//! re-exports through `approvals` so existing native consumers
//! see no API change.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ApprovalTokenScope {
    OneTime,
    Session { session_id: String },
    AmountLimited { max_amount: f64 },
    TimeLimited { expires_at_ms: u64 },
    ArgumentBound { args: Vec<serde_json::Value> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalToken {
    pub token_id: String,
    pub label: String,
    pub args: Vec<serde_json::Value>,
    pub scope: ApprovalTokenScope,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub uses_remaining: u32,
}

impl ApprovalToken {
    pub fn validate(
        &mut self,
        label: &str,
        args: &[serde_json::Value],
        now_ms: u64,
        session_id: Option<&str>,
    ) -> Result<(), String> {
        if self.uses_remaining == 0 {
            return Err("token has no remaining uses".into());
        }
        if self.label != label {
            return Err(format!(
                "token label `{}` does not match requested label `{label}`",
                self.label
            ));
        }
        if now_ms > self.expires_at_ms {
            return Err("token expired".into());
        }
        match &self.scope {
            ApprovalTokenScope::OneTime => {}
            ApprovalTokenScope::Session {
                session_id: expected,
            } => {
                if session_id != Some(expected.as_str()) {
                    return Err("token session does not match current session".into());
                }
            }
            ApprovalTokenScope::AmountLimited { max_amount } => {
                let requested = largest_numeric_arg(args).ok_or_else(|| {
                    "amount-limited token requires at least one numeric argument".to_string()
                })?;
                if requested.abs() > *max_amount {
                    return Err(format!(
                        "requested amount {requested} exceeds token limit {max_amount}"
                    ));
                }
            }
            ApprovalTokenScope::TimeLimited {
                expires_at_ms: scoped_expires_at_ms,
            } => {
                if now_ms > *scoped_expires_at_ms {
                    return Err("token time-limited scope expired".into());
                }
            }
            ApprovalTokenScope::ArgumentBound {
                args: expected_args,
            } => {
                if expected_args != args {
                    return Err("token arguments do not match current arguments".into());
                }
            }
        }
        if matches!(self.scope, ApprovalTokenScope::ArgumentBound { .. }) {
            // Argument-bound scopes are still exact-match tokens.
        } else if self.args != args {
            return Err("token arguments do not match issued arguments".into());
        }
        self.uses_remaining = self.uses_remaining.saturating_sub(1);
        Ok(())
    }
}

fn largest_numeric_arg(args: &[serde_json::Value]) -> Option<f64> {
    args.iter()
        .filter_map(|value| value.as_f64())
        .max_by(|left, right| {
            left.abs()
                .partial_cmp(&right.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approval_token_scopes_fail_closed() {
        let mut token = ApprovalToken {
            token_id: "apr_test".into(),
            label: "ChargeCard".into(),
            args: vec![json!("ord_1"), json!(100.0)],
            scope: ApprovalTokenScope::AmountLimited { max_amount: 100.0 },
            issued_at_ms: 10,
            expires_at_ms: 1000,
            uses_remaining: 1,
        };
        assert!(token
            .validate("ChargeCard", &[json!("ord_1"), json!(101.0)], 20, None)
            .is_err());
        assert_eq!(token.uses_remaining, 1);

        token
            .validate("ChargeCard", &[json!("ord_1"), json!(100.0)], 20, None)
            .unwrap();
        assert_eq!(token.uses_remaining, 0);
        assert!(token
            .validate("ChargeCard", &[json!("ord_1"), json!(100.0)], 20, None)
            .is_err());
    }

    #[test]
    fn approval_token_session_time_and_argument_scopes_are_enforced() {
        let mut session_token = ApprovalToken {
            token_id: "apr_session".into(),
            label: "SendEmail".into(),
            args: vec![json!("user@example.com")],
            scope: ApprovalTokenScope::Session {
                session_id: "s-1".into(),
            },
            issued_at_ms: 10,
            expires_at_ms: 1000,
            uses_remaining: 1,
        };
        assert!(session_token
            .validate("SendEmail", &[json!("user@example.com")], 20, Some("s-2"))
            .is_err());

        let mut argument_token = ApprovalToken {
            token_id: "apr_args".into(),
            label: "SendEmail".into(),
            args: vec![json!("user@example.com")],
            scope: ApprovalTokenScope::ArgumentBound {
                args: vec![json!("user@example.com")],
            },
            issued_at_ms: 10,
            expires_at_ms: 1000,
            uses_remaining: 1,
        };
        assert!(argument_token
            .validate("SendEmail", &[json!("other@example.com")], 20, None)
            .is_err());

        let mut time_token = ApprovalToken {
            token_id: "apr_time".into(),
            label: "SendEmail".into(),
            args: vec![json!("user@example.com")],
            scope: ApprovalTokenScope::TimeLimited { expires_at_ms: 30 },
            issued_at_ms: 10,
            expires_at_ms: 1000,
            uses_remaining: 1,
        };
        assert!(time_token
            .validate("SendEmail", &[json!("user@example.com")], 31, None)
            .is_err());
    }
}
