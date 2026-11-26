//! The public answer to a handler failure: what a browser, a partner or
//! an assistant may read when a route's handler stops with an error.
//!
//! An error's own text is written for the operator. It can hold a URL a
//! connector refused, a file path, a tool's message or a provider's
//! reply, none of which belongs in a browser. So the public answer is a
//! code naming the kind of stop and a fixed sentence written for that
//! code, holding only numbers the program declared. The full error goes
//! to the server's standard error, tagged with the route, so the operator
//! still sees it.
//!
//! One mapping serves every place a Corvid server answers a failure:
//! the `failed` event that ends a route stream and the `500`/`504` body
//! of an ordinary route in `corvid serve`, and the server
//! `corvid build --target server` generates.

use crate::{InterpErrorKind, RunError, RuntimeError};

/// The public form of a handler failure: `{"error": code, "detail": text}`
/// with the HTTP status an ordinary route answers it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicFailure {
    pub status: u16,
    pub error: &'static str,
    pub detail: String,
}

impl PublicFailure {
    fn new(status: u16, error: &'static str, detail: impl Into<String>) -> Self {
        Self { status, error, detail: detail.into() }
    }

    /// The `{error, detail}` JSON every failure answer carries.
    pub fn body(&self) -> serde_json::Value {
        serde_json::json!({ "error": self.error, "detail": self.detail })
    }
}

/// A stream item the server could not encode as the route's declared
/// item type. The reason names internal shapes, so it is logged, not sent.
pub fn unencodable_item() -> PublicFailure {
    PublicFailure::new(500, "unencodable_item", "a stream item did not match the route's declared item type")
}

/// Map a handler error to its public answer.
pub fn public_failure(error: &RunError) -> PublicFailure {
    match error {
        RunError::Interp(e) => interp_failure(&e.kind),
        _ => handler_failed(),
    }
}

fn interp_failure(kind: &InterpErrorKind) -> PublicFailure {
    match kind {
        InterpErrorKind::Timeout { milliseconds } => PublicFailure::new(
            504,
            "handler_timeout",
            format!("the handler did not finish within its {milliseconds} ms timeout"),
        ),
        InterpErrorKind::BudgetExceeded { budget, .. } => PublicFailure::new(
            500,
            "budget_exceeded",
            format!("the stream reached its declared ${budget:.4} budget"),
        ),
        InterpErrorKind::TokenLimitExceeded { limit, .. } => PublicFailure::new(
            500,
            "token_limit_exceeded",
            format!("the stream reached its declared limit of {limit} tokens"),
        ),
        InterpErrorKind::ConfidenceFloorBreached { floor, .. } => PublicFailure::new(
            500,
            "confidence_below_floor",
            format!("the stream's confidence fell below its declared floor of {floor:.3}"),
        ),
        InterpErrorKind::ApprovalDenied(_) | InterpErrorKind::Runtime(RuntimeError::ApprovalDenied { .. }) => {
            PublicFailure::new(403, "approval_denied", "an approval this request needed was denied")
        }
        InterpErrorKind::Runtime(RuntimeError::ReplayDivergence(_)) => PublicFailure::new(
            500,
            "replay_divergence",
            "the replayed run asked for a step the recording does not hold",
        ),
        _ => handler_failed(),
    }
}

/// The answer to any failure the mapping has no more specific code for.
pub fn handler_failed() -> PublicFailure {
    PublicFailure::new(500, "handler_failed", "the request handler failed; the server log has the details")
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::Span;
    use crate::InterpError;

    fn interp(kind: InterpErrorKind) -> RunError {
        RunError::Interp(InterpError::new(kind, Span::new(0, 0)))
    }

    /// The point of the mapping: an error whose own text holds a URL or a
    /// local path answers with a fixed sentence that holds neither.
    #[test]
    fn a_failure_never_carries_the_errors_own_text() {
        let leaky = [
            InterpErrorKind::Runtime(RuntimeError::ToolFailed {
                tool: "fetch".into(),
                message: "GET http://10.0.0.7:8080/admin refused; see /Users/ops/app/secrets.toml".into(),
            }),
            InterpErrorKind::DispatchFailed("connector call to https://vault.internal/v1 failed".into()),
            InterpErrorKind::Other("/var/lib/app/state.db is locked".into()),
            InterpErrorKind::Marshal("{\"debug\": \"C:\\\\app\\\\tools.dll\"}".into()),
        ];
        for kind in leaky {
            let failure = public_failure(&interp(kind));
            assert_eq!(failure.error, "handler_failed");
            assert_eq!(failure.status, 500);
            for leak in ["http", "/Users", "/var", "vault", "10.0.0.7", "dll", "\\"] {
                assert!(!failure.detail.contains(leak), "{leak} leaked: {}", failure.detail);
            }
        }
    }

    #[test]
    fn each_declared_stop_has_its_own_code() {
        let cases = [
            (InterpErrorKind::Timeout { milliseconds: 250 }, 504, "handler_timeout", "250 ms"),
            (InterpErrorKind::BudgetExceeded { budget: 0.5, used: 0.75 }, 500, "budget_exceeded", "$0.5000"),
            (InterpErrorKind::TokenLimitExceeded { limit: 100, used: 140 }, 500, "token_limit_exceeded", "100 tokens"),
            (InterpErrorKind::ConfidenceFloorBreached { floor: 0.8, actual: 0.2 }, 500, "confidence_below_floor", "0.800"),
            (InterpErrorKind::ApprovalDenied("refund".into()), 403, "approval_denied", "denied"),
        ];
        for (kind, status, code, detail) in cases {
            let failure = public_failure(&interp(kind));
            assert_eq!((failure.status, failure.error), (status, code));
            assert!(failure.detail.contains(detail), "{}", failure.detail);
        }
        assert_eq!(public_failure(&RunError::NoAgents).error, "handler_failed");
    }
}
