//! Slice 45k — end-to-end pins for `while` loops and the promoted
//! `break`/`continue`/`pass` statements.
//!
//! Pinned semantics:
//! 1. The condition re-evaluates before EVERY iteration, so state
//!    mutated by the body (including through shared list cells)
//!    controls termination.
//! 2. `break` exits the innermost loop only; `continue` skips to
//!    the next condition check. Both compose with `for`.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn while_loop_full_surface() {
    let source = "
agent main() -> String:
    n = 0
    total = 0
    while n < 10:
        n += 1
        if n % 2 == 0:
            continue
        if n > 7:
            break
        total += n

    xs = [1, 2, 3]
    drained = 0
    while xs.length() > 0:
        xs = xs.slice(1, xs.length())
        drained += 1

    hits = 0
    m = 0
    while m < 3:
        m += 1
        for k in range(0, m):
            if k == 99:
                pass
            hits += 1

    ok1 = total == 16 and n == 9
    ok2 = drained == 3
    ok3 = hits == 6
    if ok1 and ok2 and ok3:
        return \"WHILE WORKS\"
    return \"MISMATCH\"
";
    let ir = compile_to_ir(source).expect("45k e2e source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45k e2e program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "WHILE WORKS"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn break_exits_innermost_loop_only() {
    let source = "
agent main() -> Int:
    outer = 0
    i = 0
    while i < 3:
        i += 1
        j = 0
        while j < 10:
            j += 1
            if j == 2:
                break
        outer += j
    return outer
";
    let ir = compile_to_ir(source).expect("45k nesting source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45k nesting program must run");
    match out {
        // Inner loop always breaks at j == 2; outer runs 3 times.
        Value::Int(v) => assert_eq!(v, 6, "break must exit only the innermost loop"),
        other => panic!("expected Int, got {other:?}"),
    }
}
