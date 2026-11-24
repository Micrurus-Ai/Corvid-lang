//! The parity harness for the text-held scalars (`Decimal`, the time
//! types, `Currency` and `Locale`). Every program runs on the
//! interpreter and as a compiled binary; both must return the same
//! text, and the binary must release every allocation. Traps must stop
//! both tiers with the same message.

use super::{assert_no_leaks, entry_name, ir_of, run_with_leak_detector, test_tools_lib_path};
use corvid_codegen_cl::build_native_to_disk;
use corvid_runtime::{ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::sync::Arc;

fn interpret(ir: &corvid_ir::IrFile) -> Result<Value, corvid_vm::InterpError> {
    let runtime = Runtime::builder().approver(Arc::new(ProgrammaticApprover::always_yes())).build();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { corvid_vm::run_agent(ir, entry_name(ir), vec![], &runtime).await })
}

fn compile(ir: &corvid_ir::IrFile, dir: &tempfile::TempDir) -> std::path::PathBuf {
    build_native_to_disk(ir, "corvid_parity_test", &dir.path().join("prog"), &[test_tools_lib_path().as_path()])
        .expect("compile + link")
}

/// Run `src`, whose entry agent returns a String, an Int or a text-held
/// scalar, on both tiers and return the text both produced.
#[track_caller]
pub(super) fn assert_parity_text(src: &str) -> String {
    let ir = ir_of(src);
    let interpreted = match interpret(&ir).expect("interpreter run") {
        Value::String(text) => text.to_string(),
        Value::Decimal(decimal) => decimal.to_string(),
        Value::Duration(duration) => duration.to_string(),
        Value::Instant(instant) => instant.to_string(),
        Value::ZonedTime(zoned) => zoned.to_string(),
        Value::TimeZone(zone) => zone.to_string(),
        Value::Currency(currency) => currency.to_string(),
        Value::Locale(locale) => locale.to_string(),
        Value::Int(n) => n.to_string(),
        other => panic!("interpreter returned {other:?}, not a String, Int or text-held scalar; src=\n{src}"),
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let (stdout, stderr, status) = run_with_leak_detector(&compile(&ir, &tmp));
    assert!(status.success(), "compiled binary failed: {status:?} stderr={stderr} src=\n{src}");
    let compiled = stdout.trim_end_matches(['\r', '\n']).to_string();
    assert_eq!(compiled, interpreted, "the tiers disagree; src=\n{src}");
    assert_no_leaks(&stderr, src);
    compiled
}

/// Both tiers stop with an arithmetic error carrying `message`.
#[track_caller]
pub(super) fn assert_parity_trap(src: &str, message: &str) {
    let ir = ir_of(src);
    let error = interpret(&ir).expect_err("the interpreter must trap");
    assert!(
        matches!(&error.kind, corvid_vm::InterpErrorKind::Arithmetic(text) if text == message),
        "interpreter error {error} is not the arithmetic error `{message}`; src=\n{src}"
    );
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = std::process::Command::new(compile(&ir, &tmp)).output().expect("run compiled binary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "the compiled binary must trap; stdout={}", String::from_utf8_lossy(&output.stdout));
    assert!(
        stderr.contains(&format!("corvid: runtime error: {message}")),
        "native stderr `{stderr}` does not carry `{message}`; src=\n{src}"
    );
}

