//! The outage rule `try ... retry` uses (and `try ... else` will): a
//! failing tool or an unusable reply is retried; a bug or a decision is
//! raised at once, without another attempt. A counting tool called at the
//! start of each attempt shows how many attempts ran.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A runtime whose `attempt` tool counts its calls, plus `extra`.
fn counting(builder: corvid_runtime::RuntimeBuilder) -> (Runtime, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let runtime = builder
        .tool("attempt", move |_| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(json!(1))
            }
        })
        .build();
    (runtime, calls)
}

const RISKY: &str = "\
type Receipt:
    id: String

tool attempt() -> Int
tool flaky() -> Int
tool wrong_shape() -> Int
tool issue_refund(id: String) -> Receipt dangerous

agent divide(zero: Int) -> Result<Int, String>:
    first = attempt()
    return Ok(10 / zero)

agent failing() -> Result<Int, String>:
    first = attempt()
    return Ok(flaky())

agent unusable() -> Result<Int, String>:
    first = attempt()
    return Ok(wrong_shape())

agent refund(id: String) -> Result<Receipt, String>:
    first = attempt()
    approve IssueRefund(id)
    return Ok(issue_refund(id))

agent retry_divide() -> Result<Int, String>:
    return try divide(0) on error retry 3 times backoff linear 0

agent retry_failing() -> Result<Int, String>:
    return try failing() on error retry 3 times backoff linear 0

agent retry_unusable() -> Result<Int, String>:
    return try unusable() on error retry 3 times backoff linear 0

agent retry_refund() -> Result<Receipt, String>:
    return try refund(\"ord_1\") on error retry 3 times backoff linear 0
";

fn builder() -> corvid_runtime::RuntimeBuilder {
    Runtime::builder()
        .tool("flaky", |_| async move {
            Err(RuntimeError::ToolFailed { tool: "flaky".into(), message: "connection reset".into() })
        })
        .tool("wrong_shape", |_| async move { Ok(json!("not a number")) })
        .tool("issue_refund", |_| async move { Ok(json!({"id": "should_never_happen"})) })
}

#[tokio::test]
async fn a_bug_is_raised_at_once_and_never_retried() {
    let ir = ir_of(RISKY);
    let (rt, calls) = counting(builder().approver(Arc::new(ProgrammaticApprover::always_yes())));
    let err = run_agent(&ir, "retry_divide", vec![], &rt).await.unwrap_err();
    assert!(matches!(err.kind, InterpErrorKind::Arithmetic(_)), "got {err:?}");
    assert!(!err.is_outage());
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a division by zero fails the same way every time");
}

#[tokio::test]
async fn a_denied_approval_is_never_asked_again() {
    let ir = ir_of(RISKY);
    let (rt, calls) = counting(builder().approver(Arc::new(ProgrammaticApprover::always_no())));
    let err = run_agent(&ir, "retry_refund", vec![], &rt).await.unwrap_err();
    assert!(matches!(err.kind, InterpErrorKind::Runtime(RuntimeError::ApprovalDenied { .. })), "got {err:?}");
    assert!(!err.is_outage());
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a person who said no is not asked again");
}

#[tokio::test]
async fn a_failing_tool_is_an_outage_and_is_retried() {
    let ir = ir_of(RISKY);
    let (rt, calls) = counting(builder());
    let err = run_agent(&ir, "retry_failing", vec![], &rt).await.unwrap_err();
    assert!(matches!(err.kind, InterpErrorKind::Runtime(RuntimeError::ToolFailed { .. })), "got {err:?}");
    assert!(err.is_outage());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn an_unusable_reply_is_an_outage_and_is_retried() {
    let ir = ir_of(RISKY);
    let (rt, calls) = counting(builder());
    let err = run_agent(&ir, "retry_unusable", vec![], &rt).await.unwrap_err();
    assert!(matches!(err.kind, InterpErrorKind::UnusableReply(_)), "got {err:?}");
    assert!(err.is_outage());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn a_recorded_outage_is_still_one_in_replay() {
    let outage = RuntimeError::DatabaseFailed { database: "board".into(), message: "connection refused".into() };
    let recorded = corvid_runtime::replay::recorded_failure(&outage);
    let replayed = corvid_runtime::replay::replayed_failure(&recorded).expect("a recorded failure");
    assert!(replayed.is_outage(), "the same rule, the same answer");
    assert_eq!(replayed.to_string(), "database `board`: connection refused");

    let bug = RuntimeError::UnknownTool("missing".into());
    let replayed = corvid_runtime::replay::replayed_failure(&corvid_runtime::replay::recorded_failure(&bug)).unwrap();
    assert!(!replayed.is_outage());

    // A trace written before failures recorded their kind.
    let old = json!({ corvid_runtime::replay::CORVID_TOOL_ERROR_KEY: "tool `x` failed: reset" });
    assert!(!corvid_runtime::replay::replayed_failure(&old).unwrap().is_outage());
}
