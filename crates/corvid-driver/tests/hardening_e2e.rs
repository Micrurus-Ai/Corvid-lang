//! Hardening pins for semantics the book promises but no test held:
//! checked integer arithmetic (trap on overflow, wrap only under the
//! `wrapping` attribute), float→int conversion overflow, and
//! recursive struct types (nominal `Type::Struct(DefId)` indirection
//! supports them — this file gives the representation its first
//! end-to-end coverage through the interpreter tier).

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

/// The book's arithmetic chapter: integer overflow is CHECKED —
/// `i64::MAX + 1` traps with an arithmetic error, in every build
/// mode. (The implementation always checks; there is no
/// release-mode saturation and no `--overflow` flag.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn int_overflow_traps_checked() {
    let source = "
agent main() -> Int:
    big = 9223372036854775807
    return big + 1
";
    let ir = compile_to_ir(source).expect("overflow source must compile");
    let runtime = Runtime::builder().build();
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await;
    match result {
        Err(err) => {
            let msg = format!("{err:?}");
            assert!(
                msg.to_lowercase().contains("overflow"),
                "the trap must name overflow; got {msg}"
            );
        }
        Ok(v) => panic!("i64::MAX + 1 must trap, got {v:?}"),
    }
}

/// The `wrapping` agent attribute opts into two's-complement
/// wrapping: `i64::MAX + 1` becomes `i64::MIN` instead of trapping.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrapping_attribute_wraps_instead_of_trapping() {
    let source = "
@wrapping
agent main() -> Int:
    big = 9223372036854775807
    return big + 1
";
    let ir = compile_to_ir(source).expect("wrapping source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("wrapping arithmetic must not trap");
    match out {
        Value::Int(n) => assert_eq!(
            n,
            i64::MIN,
            "wrapping add must produce two's-complement wraparound"
        ),
        other => panic!("expected Int, got {other:?}"),
    }
}

/// Division by zero is checked and traps with a diagnostic naming
/// the condition (never a wrapped or sentinel value).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn division_by_zero_traps() {
    let source = "
agent main() -> Int:
    zero = 0
    return 1 / zero
";
    let ir = compile_to_ir(source).expect("div source must compile");
    let runtime = Runtime::builder().build();
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await;
    match result {
        Err(err) => {
            let msg = format!("{err:?}");
            assert!(
                msg.contains("division by zero"),
                "the trap must name division by zero; got {msg}"
            );
        }
        Ok(v) => panic!("1 / 0 must trap, got {v:?}"),
    }
}

/// Float→Int conversion overflow: a float far outside the i64 range
/// must trap, not saturate or wrap.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn to_int_truncated_traps_on_out_of_range_float() {
    let source = "
agent main() -> Int:
    huge = 1000000000000.0 * 1000000000000.0
    return huge.to_int_truncated()
";
    let ir = compile_to_ir(source).expect("conversion source must compile");
    let runtime = Runtime::builder().build();
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await;
    match result {
        Err(err) => {
            let msg = format!("{err:?}");
            assert!(
                msg.contains("to_int_truncated") || msg.to_lowercase().contains("overflow"),
                "the trap must name the conversion or overflow; got {msg}"
            );
        }
        Ok(v) => panic!("out-of-range float truncation must trap, got {v:?}"),
    }
}

/// Recursive struct type through `Option<Self>` — the nominal
/// indirection the representation promises. Build a two-node linked
/// list, traverse it, and sum the values.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recursive_struct_via_option_builds_and_traverses() {
    let source = "
type Node:
    value: Int
    next: Option<Node>

agent main() -> Int:
    tail = Node(2, None)
    head = Node(1, Some(tail))
    second = head.next.unwrap_or(Node(0, None))
    return head.value + second.value
";
    let ir = compile_to_ir(source).expect("recursive struct source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("recursive struct program must run");
    match out {
        Value::Int(n) => assert_eq!(n, 3, "head.value + tail.value through the recursion"),
        other => panic!("expected Int, got {other:?}"),
    }
}

/// Mutually recursive structs (A contains Option<B>, B contains
/// Option<A>) — the indirection must hold across two nominal types.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutually_recursive_structs_typecheck_and_run() {
    let source = "
type Ping:
    label: String
    pong: Option<Pong>

type Pong:
    label: String
    ping: Option<Ping>

agent main() -> String:
    inner = Ping(\"inner\", None)
    pong = Pong(\"pong\", Some(inner))
    ping = Ping(\"outer\", Some(pong))
    middle = ping.pong.unwrap_or(Pong(\"missing\", None))
    return ping.label + \"-\" + middle.label
";
    let ir = compile_to_ir(source).expect("mutually recursive source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("mutually recursive program must run");
    match out {
        Value::String(s) => assert_eq!(s.as_ref(), "outer-pong"),
        other => panic!("expected String, got {other:?}"),
    }
}
