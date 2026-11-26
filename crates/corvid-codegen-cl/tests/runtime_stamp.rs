//! A native build links only a runtime archive that holds the runtime the
//! compiler was built against. Its own test binary, because it sets the
//! process-wide `CORVID_RUNTIME_STATICLIB_OVERRIDE`.

use corvid_codegen_cl::build_native_to_disk;
use corvid_resolve::resolve;
use corvid_syntax::{lex, parse_file};
use corvid_types::typecheck;
use std::process::Command;

fn ir_of(src: &str) -> corvid_ir::IrFile {
    let tokens = lex(src).expect("lex");
    let (file, parse_errors) = parse_file(&tokens);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let resolved = resolve(&file);
    let checked = typecheck(&file, &resolved);
    assert!(checked.errors.is_empty(), "type errors: {:?}", checked.errors);
    corvid_ir::lower(&file, &resolved, &checked)
}

/// An archive built from other runtime sources is refused before the
/// link, with an error naming it and how to rebuild it; an archive that
/// holds this runtime links and runs.
#[test]
fn a_runtime_archive_from_other_sources_is_refused_and_the_current_one_links() {
    let ir = ir_of("agent main() -> Int:\n    return 7\n");
    let dir = tempfile::tempdir().unwrap();

    // An `ar` archive whose symbol index names another runtime's stamp.
    let index = b"\0\0\0\x01corvid_runtime_stamp_00000000000000000000000000000000\0";
    let mut stale = b"!<arch>\n".to_vec();
    stale.extend(format!("{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", "/", 0, 0, 0, 644, index.len()).as_bytes());
    stale.extend(index);
    let stale_path = dir.path().join("libstale_runtime.a");
    std::fs::write(&stale_path, &stale).unwrap();
    unsafe { std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", &stale_path) };
    let err = build_native_to_disk(&ir, "stamp_stale", &dir.path().join("stale_bin"), &[])
        .expect_err("a stale runtime archive must not link");
    let message = format!("{err:?}");
    assert!(message.contains("holds a different corvid-runtime"), "{message}");
    assert!(message.contains("libstale_runtime.a"), "{message}");

    // The test tools library bundles the runtime this compiler was built with.
    let status = Command::new("cargo")
        .args(["build", "-q", "-p", "corvid-test-tools", "--release"])
        .current_dir(corvid_test_paths::workspace_root())
        .status()
        .expect("build corvid-test-tools");
    assert!(status.success());
    let current = corvid_test_paths::test_tools_staticlib();
    unsafe { std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", &current) };
    let bin = build_native_to_disk(&ir, "stamp_current", &dir.path().join("current_bin"), &[])
        .expect("the current runtime archive links");
    let output = Command::new(bin).output().expect("run");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "7");
}
