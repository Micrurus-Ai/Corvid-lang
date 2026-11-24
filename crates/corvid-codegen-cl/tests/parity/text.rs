//! B03 text methods on the native tier. Every program runs on the
//! interpreter and as a compiled binary; both must return the same
//! text, including the messages decoders put in `Err`, and the binary
//! must release every allocation.

use super::{assert_no_leaks, entry_name, ir_of, run_with_leak_detector, test_tools_lib_path};
use corvid_codegen_cl::build_native_to_disk;
use corvid_runtime::{ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::sync::Arc;

/// Run `src`, whose entry agent returns a String, on both tiers and
/// return the text both produced.
#[track_caller]
fn assert_parity_text(src: &str) -> String {
    let ir = ir_of(src);
    let runtime = Runtime::builder().approver(Arc::new(ProgrammaticApprover::always_yes())).build();
    let interpreted = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { corvid_vm::run_agent(&ir, entry_name(&ir), vec![], &runtime).await })
        .expect("interpreter run");
    let Value::String(interpreted) = interpreted else {
        panic!("interpreter returned {interpreted:?}, not a String; src=\n{src}")
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let produced = build_native_to_disk(&ir, "corvid_parity_test", &tmp.path().join("prog"), &[test_tools_lib_path().as_path()])
        .expect("compile + link");
    let (stdout, stderr, status) = run_with_leak_detector(&produced);
    assert!(status.success(), "compiled binary failed: {status:?} stderr={stderr} src=\n{src}");
    let compiled = stdout.trim_end_matches(['\r', '\n']).to_string();
    assert_eq!(compiled, interpreted.as_ref(), "the tiers disagree; src=\n{src}");
    assert_no_leaks(&stderr, src);
    compiled
}

const SHOW: &str = r#"
agent show(result: Result<String, String>) -> String:
    return match result:
        Ok(text) -> "ok:" + text
        Err(problem) -> "err:" + problem
"#;

#[test]
fn encoders_agree_on_both_tiers() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    word = "café ☕ a+b/c"
    return word.encode_base64() + "|" + word.encode_base64_url() + "|" + word.encode_hex() + "|" + word.encode_url_component()
"#,
    );
    assert_eq!(out, "Y2Fmw6kg4piVIGErYi9j|Y2Fmw6kg4piVIGErYi9j|636166c3a920e2989520612b622f63|caf%C3%A9%20%E2%98%95%20a%2Bb%2Fc");
}

#[test]
fn decoders_return_the_same_ok_and_err_values_on_both_tiers() {
    let out = assert_parity_text(&format!(
        r#"{SHOW}
agent main() -> String:
    word = "naïve résumé"
    ok = show(word.encode_base64().decode_base64()) + show(word.encode_base64_url().decode_base64_url()) + show(word.encode_hex().decode_hex()) + show(word.encode_url_component().decode_url_component())
    bad = show("Zm9v!A==".decode_base64()) + show("4g".decode_hex()) + show("100%".decode_url_component()) + show("/w==".decode_base64()) + show("a+b".decode_url_component())
    return ok + "|" + bad
"#
    ));
    assert!(out.starts_with("ok:naïve résuméok:naïve résuméok:naïve résuméok:naïve résumé|"), "{out}");
    assert!(out.contains("err:not base64: unexpected character `!` at position 4"), "{out}");
    assert!(out.ends_with("ok:a+b"), "{out}");
}

#[test]
fn parse_uuid_agrees_on_both_tiers() {
    let out = assert_parity_text(&format!(
        r#"{SHOW}
agent main() -> String:
    return show("{{F81D4FAE-7DEC-11D0-A765-00A0C91E6BF6}}".parse_uuid()) + "|" + show("nope".parse_uuid())
"#
    ));
    assert_eq!(
        out,
        "ok:f81d4fae-7dec-11d0-a765-00a0c91e6bf6|err:not a UUID: `nope` (expected 32 hex digits, optionally hyphenated 8-4-4-4-12)"
    );
}

#[test]
fn a_decoder_result_propagates_with_question_mark() {
    let out = assert_parity_text(&format!(
        r#"{SHOW}
agent token(text: String) -> Result<String, String>:
    inner = text.decode_base64_url()?
    return inner.decode_hex()

agent main() -> String:
    return show(token("NjE2NDYx")) + "|" + show(token("!!"))
"#
    ));
    assert_eq!(out, "ok:ada|err:not URL-safe base64: unexpected character `!` at position 0");
}
