//! End-to-end integration test for L-3: native prompt returns of
//! `Type::Struct(_)`.
//!
//! Compiles a Corvid source that declares a struct with one field of
//! each supported scalar type (`Int`, `Bool`, `Float`, `String`),
//! returns it from a prompt, runs the resulting native binary against
//! a mock LLM whose JSON reply matches the struct schema, and asserts
//! the agent observes the decoded fields.
//!
//! Three observations the test pins simultaneously:
//!
//! 1. The codegen-emitted decoder reads every field type correctly
//!    (the agent body extracts the `Int` field for stdout, but the
//!    struct destructor walks every field on scope-exit so the
//!    `String` field's refcount must be valid for the program to
//!    exit cleanly without a crash).
//! 2. The schema literal embedded in the binary is well-formed
//!    JSON Schema — if the schema's compact serialisation broke,
//!    the system prompt would carry malformed text and the LLM
//!    would never produce a parseable reply (parse retries would
//!    exhaust and the bridge would panic).
//! 3. The decoder/bridge integration round-trips: bridge calls
//!    decoder, decoder returns non-zero ptr, bridge returns ptr,
//!    agent body reads field, prints it.
//!
//! The agent's entry-boundary return type is `Int` rather than the
//! struct itself because the entry-boundary check still rejects
//! struct-typed entry agents until phase 20n-C commit 5 lifts that
//! restriction. Returning a scalar extracted from the struct is the
//! correct shape to test the prompt-bridge struct path in isolation.

use corvid_codegen_cl::build_native_to_disk;
use corvid_ir::lower;
use corvid_resolve::resolve;
use corvid_syntax::{lex, parse_file};
use corvid_types::typecheck;
use std::path::PathBuf;
use std::process::Command;

const STRUCT_PROMPT_SRC: &str = r#"
type Decision:
    code: Int
    confidence: Float
    approved: Bool
    reason: String

prompt classify(amount: Int) -> Decision:
    """Classify amount {amount}"""

agent run(amount: Int) -> Int:
    d = classify(amount)
    return d.code
"#;

fn ir_of(src: &str) -> corvid_ir::IrFile {
    let tokens = lex(src).expect("lex");
    let (file, parse_errors) = parse_file(&tokens);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let resolved = resolve(&file);
    assert!(
        resolved.errors.is_empty(),
        "resolve errors: {:?}",
        resolved.errors
    );
    let checked = typecheck(&file, &resolved);
    assert!(
        checked.errors.is_empty(),
        "type errors: {:?}",
        checked.errors
    );
    lower(&file, &resolved, &checked)
}

fn test_tools_lib_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf();
    let name = if cfg!(windows) {
        "corvid_test_tools.lib"
    } else {
        "libcorvid_test_tools.a"
    };
    let path = workspace_root.join("target").join("release").join(name);
    let status = Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("corvid-test-tools")
        .arg("--release")
        .current_dir(&workspace_root)
        .status()
        .expect("build corvid-test-tools");
    assert!(status.success(), "building corvid-test-tools failed");
    // Override the runtime staticlib path so the linker pulls a
    // single Rust-bundled artifact, matching the precedent set by
    // `tests/record_native.rs` for MSVC duplicate-`std` avoidance.
    unsafe {
        std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", &path);
    }
    path
}

#[test]
fn native_prompt_struct_return_decodes_all_scalar_fields() {
    // Build the static-libs side-effect first so the override env
    // var is set before `build_native_to_disk` runs.
    let _ = test_tools_lib_path();

    let ir = ir_of(STRUCT_PROMPT_SRC);

    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_path = tmp.path().join("struct_prompt_return");
    let produced = build_native_to_disk(
        &ir,
        "struct_prompt_return",
        &bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary");

    // Mock LLM reply: a JSON object matching the schema for
    // `Decision`. The decoder should read every field, the agent
    // should return d.code = 7, and stdout should contain "7".
    //
    // The String field "reason" exercises the refcount path on
    // scope-exit destructor — if the decoder mishandled the
    // CorvidString (e.g. forgot to retain it before storing in the
    // struct slot, or the destructor double-free'd it), the binary
    // would either crash or emit a leak warning.
    let mock_reply = r#"{"code":7,"confidence":0.95,"approved":true,"reason":"approved on policy"}"#;
    let mock_replies_json = serde_json::json!({ "classify": [mock_reply] }).to_string();

    let output = Command::new(&produced)
        .arg("100") // the Int amount param to the entry agent
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", mock_replies_json)
        .env("CORVID_MODEL", "mock-1")
        .output()
        .expect("run native binary");

    assert!(
        output.status.success(),
        "native run failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_trimmed = stdout.trim();
    assert_eq!(
        stdout_trimmed, "7",
        "agent should return d.code = 7 from the decoded struct; got: {stdout_trimmed:?}",
    );
}

#[test]
fn native_entry_struct_return_prints_full_json() {
    // Phase 20n-C commit 5: the entry-boundary now accepts
    // `Type::Struct(_)` returns. The agent declares its return as
    // `Decision`, the binary's `main` calls the per-struct
    // `corvid_<Name>__<DefId>_to_json` encoder on the agent's
    // return value, and prints the resulting JSON via
    // `print_string`. The encoder iterates fields in source order
    // matching `IrType.fields`, so stdout should reflect the
    // declaration order: code, confidence, approved, reason.
    let _ = test_tools_lib_path();

    // Re-declare the source with an entry agent that returns the
    // struct directly, exercising the entry-boundary lift.
    let entry_struct_src = r#"
type Decision:
    code: Int
    confidence: Float
    approved: Bool
    reason: String

prompt classify(amount: Int) -> Decision:
    """Classify amount {amount}"""

agent run(amount: Int) -> Decision:
    return classify(amount)
"#;
    let ir = ir_of(entry_struct_src);

    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_path = tmp.path().join("entry_struct_return");
    let produced = build_native_to_disk(
        &ir,
        "entry_struct_return",
        &bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary");

    let mock_reply =
        r#"{"code":13,"confidence":0.42,"approved":false,"reason":"declined"}"#;
    let mock_replies_json = serde_json::json!({ "classify": [mock_reply] }).to_string();

    let output = Command::new(&produced)
        .arg("100")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", mock_replies_json)
        .env("CORVID_MODEL", "mock-1")
        .output()
        .expect("run native binary");

    assert!(
        output.status.success(),
        "native run failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Stdout should be the JSON object built by the encoder, in
    // source-declaration field order, plus print_string's trailing
    // newline.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_trimmed = stdout.trim();
    let expected = r#"{"code":13,"confidence":0.42,"approved":false,"reason":"declined"}"#;
    assert_eq!(
        stdout_trimmed, expected,
        "entry struct print should match the source-order JSON encoding; got: {stdout_trimmed:?}",
    );
}

#[test]
fn native_grounded_struct_prompt_return_attaches_attestation_then_unwraps() {
    // Phase 20n-C commit 6: `Grounded<Struct>` returns from prompts
    // wire the new `corvid_grounded_attest_struct` runtime bridge,
    // which keys the attestation by the struct's heap pointer in
    // the same `pointer_attestations` map used for
    // `Grounded<String>`. The agent's `unwrap g` is a codegen no-op
    // (the value is already the struct ptr), so the test asserts
    // the unwrapped value's field is observable end-to-end.
    let _ = test_tools_lib_path();

    let grounded_struct_src = r#"
type Decision:
    code: Int
    confidence: Float
    approved: Bool
    reason: String

prompt classify(amount: Int) -> Grounded<Decision>:
    """Classify amount {amount}"""

agent run(amount: Int) -> Int:
    g = classify(amount)
    d = g.unwrap_discarding_sources()
    return d.code
"#;
    let ir = ir_of(grounded_struct_src);

    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_path = tmp.path().join("grounded_struct_return");
    let produced = build_native_to_disk(
        &ir,
        "grounded_struct_return",
        &bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary");

    let mock_reply = r#"{"code":42,"confidence":0.9,"approved":true,"reason":"verified"}"#;
    let mock_replies_json = serde_json::json!({ "classify": [mock_reply] }).to_string();

    let output = Command::new(&produced)
        .arg("100")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", mock_replies_json)
        .env("CORVID_MODEL", "mock-1")
        .output()
        .expect("run native binary");

    assert!(
        output.status.success(),
        "native run failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_trimmed = stdout.trim();
    assert_eq!(
        stdout_trimmed, "42",
        "agent should unwrap the Grounded<Decision> and return d.code = 42; got: {stdout_trimmed:?}",
    );
}

#[test]
fn native_prompt_struct_return_retries_on_decoder_failure_then_succeeds() {
    let _ = test_tools_lib_path();

    let ir = ir_of(STRUCT_PROMPT_SRC);

    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_path = tmp.path().join("struct_prompt_retry");
    let produced = build_native_to_disk(
        &ir,
        "struct_prompt_retry",
        &bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary");

    // First reply is missing the required `code` field — decoder
    // returns 0, bridge retries. Second reply is missing `reason`.
    // Third reply is well-formed; bridge returns the decoded ptr.
    //
    // The `field_present` check inside the decoder is what makes
    // the first two attempts fail rather than silently returning
    // garbage; if the present-check were missing, the decoder
    // would happily build a struct with zero-sentinel field values
    // and the test would observe "0" instead of "11".
    let mock_replies_json = serde_json::json!({
        "classify": [
            r#"{"confidence":0.5,"approved":false,"reason":"no code field"}"#,
            r#"{"code":99,"confidence":0.5,"approved":false}"#,
            r#"{"code":11,"confidence":0.99,"approved":true,"reason":"third try"}"#,
        ]
    })
    .to_string();

    let output = Command::new(&produced)
        .arg("100")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", mock_replies_json)
        .env("CORVID_MODEL", "mock-1")
        .output()
        .expect("run native binary");

    assert!(
        output.status.success(),
        "native run with retries failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_trimmed = stdout.trim();
    assert_eq!(
        stdout_trimmed, "11",
        "agent should return d.code = 11 from the third (well-formed) reply; got: {stdout_trimmed:?}",
    );
}
