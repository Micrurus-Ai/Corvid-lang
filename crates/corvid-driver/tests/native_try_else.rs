//! The tier picker sends a `try` to native code exactly when native code
//! catches its outages as the interpreter does (recoverable failures
//! TE-3), and otherwise names why it runs on the interpreter. It asks the
//! same outage plan native lowering uses.

use corvid_driver::{compile_to_ir, native_ability};

fn ability(src: &str) -> Result<(), String> {
    let ir = compile_to_ir(src).unwrap_or_else(|d| panic!("compile: {d:?}"));
    native_ability(&ir).map_err(|reason| reason.to_string())
}

#[test]
fn a_try_native_code_catches_exactly_runs_natively() {
    let src = r#"
agent total(items: List<Int>) -> Int:
    return items.length() + 1

agent main() -> Int:
    first = try total([1, 2]) else 0
    second = try total([]) on error retry 2 times backoff linear 1 else -1
    return first + second
"#;
    assert_eq!(ability(src), Ok(()));
}

#[test]
fn a_try_over_a_prompt_runs_natively() {
    let src = r#"
prompt summarize(text: String) -> String:
    "Summarize {text}"

agent main() -> String:
    return try summarize("notes") else "no summary"
"#;
    assert_eq!(ability(src), Ok(()));
}

#[test]
fn a_try_over_a_failing_guard_runs_natively() {
    let src = r#"
prompt summarize(text: String) -> String:
    "Summarize {text}"

agent pick(text: Option<String>) -> String:
    return match text:
        Some(t) if summarize(t) == "" -> "empty"
        _ -> ""

agent main() -> String:
    return try pick(Some("notes")) else "no summary"
"#;
    assert_eq!(ability(src), Ok(()));
}

#[test]
fn a_try_with_a_timeout_runs_natively() {
    let src = r#"
agent main() -> Int:
    return try 1 + 1 timeout 50 else 0
"#;
    assert_eq!(ability(src), Ok(()));
}

#[test]
fn a_timed_try_whose_body_uses_question_mark_runs_natively() {
    let src = r#"
agent half(n: Int) -> Option<Int>:
    if n > 0:
        return Some(n)
    return None

agent main() -> Option<Int>:
    return Some(try half(4)? timeout 50 else 0)
"#;
    assert_eq!(ability(src), Ok(()));
}
