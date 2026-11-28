//! A computed argument outside its parameter's refinement stops the call
//! when it starts (PR-1), with a message naming the parameter, the value
//! and the refinement, and before a tool's implementation runs.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const SRC: &str = "\
tool fetch_page(limit: Int where between(1, 50)) -> Int

agent page(limit: Int where between(1, 50)) -> Int:
    return limit

agent call_agent(n: Int) -> Int:
    return page(n)

agent call_tool(n: Int) -> Int:
    return fetch_page(n)
";

#[tokio::test]
async fn a_computed_argument_outside_the_refinement_stops_the_call_before_it_runs() {
    let ir = ir_of(SRC);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let rt = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("fetch_page", move |args| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(args[0].clone())
            }
        })
        .build();

    assert_eq!(run_agent(&ir, "call_agent", vec![Value::Int(7)], &rt).await.unwrap(), Value::Int(7));
    let error = run_agent(&ir, "call_agent", vec![Value::Int(500)], &rt).await.unwrap_err();
    assert_eq!(error.kind.to_string(), "`limit` is 500, outside `between(1, 50)`, in this call to `page`");
    assert!(!error.is_outage(), "a caller's bug, never caught by `try ... else`");

    let error = run_agent(&ir, "call_tool", vec![Value::Int(0)], &rt).await.unwrap_err();
    assert!(matches!(error.kind, InterpErrorKind::ArgumentOutOfRange { ref callable, .. } if callable == "fetch_page"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the tool's implementation never saw the value");
    assert_eq!(run_agent(&ir, "call_tool", vec![Value::Int(3)], &rt).await.unwrap(), Value::Int(3));
}
