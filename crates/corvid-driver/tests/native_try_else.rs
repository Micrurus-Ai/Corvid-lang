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
fn a_try_that_reaches_a_prompt_names_the_prompt() {
    let src = r#"
prompt summarize(text: String) -> String:
    "Summarize {text}"

agent main() -> String:
    return try summarize("notes") else "no summary"
"#;
    let reason = ability(src).expect_err("prompts still stop a native program");
    assert!(reason.contains("runs on the interpreter") && reason.contains("prompt `summarize`"), "{reason}");
}

#[test]
fn a_try_with_a_timeout_names_the_timeout() {
    let src = r#"
agent main() -> Int:
    return try 1 + 1 timeout 50 else 0
"#;
    let reason = ability(src).expect_err("native calls have no cancellation point");
    assert!(reason.contains("timeout"), "{reason}");
}
