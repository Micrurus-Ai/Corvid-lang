//! Slices 45c/45d — end-to-end pins for the builtin-method surface.
//!
//! 45c pilot: `String.length()` counts Unicode scalar values
//! (Python's `len(str)`), not UTF-8 bytes.
//! 45d batch: the nine string methods, including the decided edge
//! semantics (empty-separator split traps; substring clamps to
//! bounds and returns "" when start >= end; casing is full
//! Unicode).

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn string_length_counts_unicode_scalars() {
    // "héllo" is 5 Unicode scalars but 6 UTF-8 bytes — the assertion
    // fails if length() ever regresses to byte counting.
    let source = "
agent main() -> Int:
    ascii = \"hello, Corvid\"
    accented = \"héllo\"
    return ascii.length() + accented.length()
";
    let ir = compile_to_ir(source).expect("45c e2e source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45c e2e program must run");
    match out {
        Value::Int(n) => assert_eq!(n, 18, "length must count Unicode scalars, not bytes"),
        other => panic!("expected Int, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn string_method_batch_end_to_end() {
    let source = "
agent main() -> String:
    s = \"  Hello, Corvid!  \"
    trimmed = s.trim()
    upper = trimmed.to_upper()
    lower = trimmed.to_lower()
    parts = \"a,b,c\".split(\",\")
    joined_count = parts[0] + parts[1] + parts[2]
    replaced = trimmed.replace(\"Corvid\", \"World\")
    sub = trimmed.substring(0, 5)
    clamped = trimmed.substring(10, 999)
    empty = trimmed.substring(5, 2)
    uni = \"héllo\".to_upper()

    ok1 = trimmed == \"Hello, Corvid!\"
    ok2 = upper == \"HELLO, CORVID!\"
    ok3 = lower == \"hello, corvid!\"
    ok4 = joined_count == \"abc\"
    ok5 = trimmed.contains(\"Corvid\") and trimmed.starts_with(\"Hello\") and trimmed.ends_with(\"!\")
    ok6 = replaced == \"Hello, World!\"
    ok7 = sub == \"Hello\"
    ok8 = clamped == \"vid!\"
    ok9 = empty == \"\"
    ok10 = uni == \"HÉLLO\"
    if ok1 and ok2 and ok3 and ok4 and ok5 and ok6 and ok7 and ok8 and ok9 and ok10:
        return \"ok\"
    return \"FAILED\"
";
    let ir = compile_to_ir(source).expect("45d batch source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45d batch program must run");
    match out {
        Value::String(s) => assert_eq!(s.as_ref(), "ok", "a string-method check failed"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conversion_batch_end_to_end() {
    // Pins the 45e semantics: Python-style Float rendering (42.0 ->
    // "42.0", never bare "42"), truncation toward zero, whitespace-
    // trimming parses returning Result, and the "count: " + n
    // papercut being dead.
    let source = "
agent parse_pair() -> Result<Float, String>:
    n = \" 42 \".parse_int()?
    f = \"2.5\".parse_float()?
    return Ok(n.to_float() + f)

agent main() -> String:
    n = 42
    whole = 42.0
    neg = 0.0 - 3.9

    ok1 = (\"count: \" + n.to_string()) == \"count: 42\"
    ok2 = whole.to_string() == \"42.0\"
    ok3 = true.to_string() == \"true\"
    ok4 = n.to_float() == 42.0
    ok5 = 3.9.to_int_truncated() == 3
    ok6 = neg.to_int_truncated() == -3
    ok7 = parse_pair() == Ok(44.5)
    ok8 = \"nope\".parse_int() == Err(\"not an integer: `nope`\")
    if ok1 and ok2 and ok3 and ok4 and ok5 and ok6 and ok7 and ok8:
        return \"ok\"
    return \"FAILED\"
";
    let ir = compile_to_ir(source).expect("45e batch source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45e batch program must run");
    match out {
        Value::String(s) => assert_eq!(s.as_ref(), "ok", "a conversion check failed"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn to_int_truncated_traps_on_nan() {
    let source = "
agent main() -> Int:
    zero = 0.0
    nan = zero / zero
    return nan.to_int_truncated()
";
    let ir = compile_to_ir(source).expect("source must compile");
    let runtime = Runtime::builder().build();
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await;
    match result {
        Err(err) => assert!(
            format!("{err:?}").contains("to_int_truncated"),
            "expected the truncation trap, got {err:?}"
        ),
        // If Float division 0.0/0.0 itself traps (checked-arithmetic
        // rule), that is an acceptable earlier trap — but it must
        // NOT produce an Int.
        Ok(v) => panic!("NaN truncation must not succeed, got {v:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn split_with_empty_separator_traps() {
    let source = "
agent main() -> Int:
    parts = \"abc\".split(\"\")
    return 0
";
    let ir = compile_to_ir(source).expect("source must compile");
    let runtime = Runtime::builder().build();
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await;
    let err = result.expect_err("empty-separator split must trap");
    assert!(
        format!("{err:?}").contains("non-empty separator"),
        "expected the empty-separator trap, got {err:?}"
    );
}
