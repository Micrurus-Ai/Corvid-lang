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
async fn list_method_batch_end_to_end() {
    // Pins 45f: in-place append/sort/reverse through shared cells
    // (the alias sees the append — 45b reference semantics), Option
    // returns from first/last (generic table returns), clamped
    // slice, join, and range() counted iteration.
    let source = "
agent main() -> String:
    xs = [3, 1, 2]
    xs.append(4)
    xs.sort()
    low = xs[0]
    xs.reverse()
    high = xs[0]

    counted = 0
    for i in range(0, 5):
        counted = counted + i

    names = [\"b\", \"a\", \"c\"]
    names.sort()
    joined = names.join(\"-\")

    sub = range(0, 10).slice(2, 5)
    alias = xs
    alias.append(99)

    ok1 = xs.length() == 5 and xs.contains(99)
    ok2 = low == 1 and high == 4
    ok3 = counted == 10
    ok4 = joined == \"a-b-c\"
    ok5 = sub.first() == Some(2) and sub.last() == Some(4)
    ok6 = range(0, 0).first() == None
    if ok1 and ok2 and ok3 and ok4 and ok5 and ok6:
        return \"ok\"
    return \"FAILED\"
";
    let ir = compile_to_ir(source).expect("45f batch source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45f batch program must run");
    match out {
        Value::String(s) => assert_eq!(s.as_ref(), "ok", "a list-method check failed"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn map_surface_end_to_end() {
    // Pins 45g: dup-key last-wins literals, Option reads (hit +
    // miss), insert-or-update + compound place assignment, aliasing
    // through the shared cell, insertion-order keys, remove.
    let source = "
agent main() -> String:
    m = {\"a\": 1, \"b\": 2, \"a\": 10}
    m[\"c\"] = 3
    m[\"b\"] += 5
    alias = m
    alias[\"d\"] = 4
    ks = m.keys()
    ok1 = m[\"a\"] == Some(10) and m[\"zzz\"] == None
    ok2 = m[\"b\"] == Some(7) and m[\"c\"] == Some(3)
    ok3 = m.length() == 4 and m.contains_key(\"d\")
    ok4 = ks[0] == \"a\" and ks[3] == \"d\"
    ok5 = m.remove(\"a\") == Some(10) and m.get(\"a\") == None
    if ok1 and ok2 and ok3 and ok4 and ok5:
        return \"ok\"
    return \"FAILED\"
";
    let ir = compile_to_ir(source).expect("45g map source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45g map program must run");
    match out {
        Value::String(s) => assert_eq!(s.as_ref(), "ok", "a map check failed"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sum_type_construction_and_equality() {
    // Pins 45h: unit variants as bare values, payload construction,
    // structural equality (same variant + fields), cross-variant and
    // cross-payload inequality, sums inside lists.
    let source = "
type Status:
    | Pending
    | Approved(approver: String)
    | Denied(reason: String, code: Int)

agent main() -> String:
    a = Approved(\"alice\")
    ok1 = a == Approved(\"alice\")
    ok2 = not (a == Approved(\"bob\"))
    ok3 = not (a == Pending)
    ok4 = Denied(\"policy\", 42) == Denied(\"policy\", 42)
    ok5 = not (Denied(\"policy\", 42) == Denied(\"policy\", 7))
    xs = [Pending, Approved(\"x\")]
    ok6 = xs.length() == 2 and xs[0] == Pending
    if ok1 and ok2 and ok3 and ok4 and ok5 and ok6:
        return \"ok\"
    return \"FAILED\"
";
    let ir = compile_to_ir(source).expect("45h source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45h program must run");
    match out {
        Value::String(s) => assert_eq!(s.as_ref(), "ok", "a sum-type check failed"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn match_expression_full_surface() {
    // Pins 45i: variant destructuring, guards, Option/Result
    // patterns, record patterns (literal + shorthand + ..), @
    // bindings, wildcard, literal arms, arm-type unification.
    let source = "
type Status:
    | Pending
    | Approved(approver: String)
    | Denied(reason: String, code: Int)

type Decision:
    refund: Bool
    amount: Float

agent describe(s: Status) -> String:
    return match s:
        Pending -> \"waiting\"
        Approved(who) -> \"approved by \" + who
        Denied(reason, code) -> \"denied: \" + reason + \" #\" + code.to_string()

agent classify(n: Int) -> String:
    return match n:
        0 -> \"zero\"
        x if x > 100 -> \"big\"
        _ -> \"small\"

agent unwrap_or_zero(o: Option<Int>) -> Int:
    return match o:
        Some(x) -> x
        None -> 0

agent settle(r: Result<Float, String>) -> String:
    return match r:
        Ok(v) -> \"got \" + v.to_string()
        Err(msg) -> \"failed: \" + msg

agent decide(d: Decision) -> String:
    return match d:
        Decision { refund: true, amount } -> \"refund \" + amount.to_string()
        Decision { .. } -> \"no refund\"

agent tag(s: Status) -> String:
    return match s:
        v @ Approved(_) -> \"an approval\"
        other -> \"something else\"

agent main() -> String:
    ok1 = describe(Approved(\"alice\")) == \"approved by alice\"
    ok2 = describe(Denied(\"policy\", 7)) == \"denied: policy #7\"
    ok3 = classify(0) == \"zero\" and classify(500) == \"big\" and classify(5) == \"small\"
    ok4 = unwrap_or_zero(Some(42)) == 42 and unwrap_or_zero(None) == 0
    ok5 = settle(Ok(2.5)) == \"got 2.5\" and settle(Err(\"no\")) == \"failed: no\"
    ok6 = decide(Decision(true, 50.0)) == \"refund 50.0\"
    ok7 = decide(Decision(false, 9.0)) == \"no refund\"
    ok8 = tag(Approved(\"x\")) == \"an approval\" and tag(Pending) == \"something else\"
    if ok1 and ok2 and ok3 and ok4 and ok5 and ok6 and ok7 and ok8:
        return \"ok\"
    return \"FAILED\"
";
    let ir = compile_to_ir(source).expect("45i source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45i program must run");
    match out {
        Value::String(s) => assert_eq!(s.as_ref(), "ok", "a match check failed"),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lambda_full_surface_end_to_end() {
    // Slice 45j: map / filter / fold / any / all, first-class
    // closures (stored, annotated, called), capture-by-value of an
    // outer local, and map producing a DIFFERENT element type
    // (Int -> String, exercised through join).
    let source = "
agent main() -> String:
    xs = [1, 2, 3, 4]
    doubled = xs.map(fn (x) -> x * 2)
    evens = xs.filter(fn (x) -> x % 2 == 0)
    total = xs.fold(0, fn (acc, x) -> acc + x)
    has_big = xs.any(fn (x) -> x > 3)
    all_pos = xs.all(fn (x) -> x > 0)
    base = 100
    add_base = fn (n) -> n + base
    shifted = add_base(5)
    scale: (Int) -> Int = fn (m: Int) -> m * 10
    labels = xs.map(fn (x) -> x.to_string())
    ok1 = doubled.length() == 4 and doubled.last() == Some(8)
    ok2 = evens.length() == 2 and total == 10
    ok3 = has_big and all_pos
    ok4 = shifted == 105 and scale(7) == 70
    ok5 = labels.join(\",\") == \"1,2,3,4\"
    if ok1 and ok2 and ok3 and ok4 and ok5:
        return \"LAMBDAS WORK\"
    return \"MISMATCH\"
";
    let ir = compile_to_ir(source).expect("45j e2e source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45j e2e program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "LAMBDAS WORK"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lambda_capture_is_snapshot_but_cells_share() {
    // Rebinding an outer Int AFTER the lambda is created must not
    // change what the closure sees (by-value snapshot, no Python
    // late-binding footgun) — but a captured LIST is a shared cell,
    // so in-place mutation IS visible through the capture.
    let source = "
agent main() -> Int:
    n = 1
    xs = [10]
    f = fn (k) -> k + n + xs.length()
    n = 100
    xs.append(20)
    return f(0)
";
    let ir = compile_to_ir(source).expect("45j capture source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45j capture program must run");
    match out {
        // n snapshots at 1; xs.length() sees the shared cell (2).
        Value::Int(v) => assert_eq!(v, 3, "snapshot Int + shared list cell"),
        other => panic!("expected Int, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn option_result_ergonomics_end_to_end() {
    // Slice 45l: unwrap_or / is_some / is_none / ok_or on Option,
    // unwrap_or / is_ok / is_err / map_err on Result. map_err's
    // closure must run ONLY on the Err side.
    let source = "
agent main() -> String:
    xs = [10, 20, 30]
    first = xs.first().unwrap_or(0)
    missing = xs.slice(0, 0).first().unwrap_or(-1)
    have = xs.first().is_some()
    empty = xs.slice(0, 0).first().is_none()
    converted = xs.first().ok_or(\"empty list\")
    fallback = xs.slice(0, 0).first().ok_or(\"empty list\")
    parsed = \"42\".parse_int()
    bad = \"nope\".parse_int()
    v = parsed.unwrap_or(0)
    w = bad.unwrap_or(-7)
    tagged = bad.map_err(fn (e) -> \"parse failed: \" + e)
    untouched = parsed.map_err(fn (e) -> \"never runs: \" + e)
    ok1 = first == 10 and missing == -1 and have and empty
    ok2 = converted.is_ok() and fallback.is_err()
    ok3 = v == 42 and w == -7 and parsed.is_ok() and bad.is_err()
    ok4 = match tagged:
        Ok(_) -> false
        Err(msg) -> msg.starts_with(\"parse failed:\")
    ok5 = untouched.unwrap_or(0) == 42
    if ok1 and ok2 and ok3 and ok4 and ok5:
        return \"OPTION RESULT ERGONOMICS WORK\"
    return \"MISMATCH\"
";
    let ir = compile_to_ir(source).expect("45l e2e source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45l e2e program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "OPTION RESULT ERGONOMICS WORK"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aliases_literals_destructuring_end_to_end() {
    // Slice 45n: transparent type aliases, named struct literals
    // with shorthand + `..base` spread (a NEW cell — the base is
    // untouched), and irrefutable destructuring bindings with
    // rename + rest.
    let source = "
type CustomerId = String

type Decision:
    refund: Bool
    amount: Float
    customer: CustomerId

agent main() -> String:
    d = Decision { customer: \"c-42\", refund: true, amount: 125.0 }
    amount = 99.5
    d2 = Decision { amount, ..d }
    cid: CustomerId = d.customer
    tail = cid.substring(2, 4)
    Decision { refund, amount: final_amount, .. } = d2
    ok1 = d.refund and d.amount == 125.0 and d.customer == \"c-42\"
    ok2 = d2.amount == 99.5 and d2.refund and d2.customer == \"c-42\"
    ok3 = refund and final_amount == 99.5
    ok4 = tail == \"42\"
    ok5 = d.amount == 125.0
    if ok1 and ok2 and ok3 and ok4 and ok5:
        return \"ALIASES LITERALS DESTRUCTURING WORK\"
    return \"MISMATCH\"
";
    let ir = compile_to_ir(source).expect("45n e2e source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45n e2e program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "ALIASES LITERALS DESTRUCTURING WORK"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elif_unary_plus_annotations_end_to_end() {
    // Slice 45q: elif chains (parser desugar), unary `+` (checked
    // identity, elided at lowering), keyword-named + named-arg
    // annotations, and a prompt mock compiling alongside its target.
    let source = "
@retry(max_attempts: 3, backoff: exponential 250)
@idempotency(key: order_id)
agent process(order_id: String) -> String:
    n = 7
    grade = \"\"
    if n > 8:
        grade = \"high\"
    elif n > 5:
        grade = \"mid\"
    elif n > 2:
        grade = \"low\"
    else:
        grade = \"none\"
    pos = +5
    posf = +2.5
    return grade + pos.to_string() + posf.to_string()

prompt summarize(text: String) -> String:
    \"Summarize {text}\"

mock summarize(text: String) -> String:
    return \"mocked summary\"

agent main() -> String:
    return process(\"o-1\")
";
    let ir = compile_to_ir(source).expect("45q e2e source must compile");
    let agent = ir
        .agents
        .iter()
        .find(|a| a.name == "process")
        .expect("process agent lowered");
    assert_eq!(agent.retry_max_attempts, Some(3));
    assert_eq!(agent.retry_backoff_ms, Some((true, 250)));
    assert_eq!(agent.idempotency_key_param.as_deref(), Some("order_id"));
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45q e2e program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "mid52.5"),
        other => panic!("expected String, got {other:?}"),
    }
}
