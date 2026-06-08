//! Slice 33S1-fix-naming — end-to-end test that proves a real
//! Corvid program (compiled through the driver, run through the
//! interpreter) reaches the executing file-I/O dispatch.
//!
//! This is the test that was MISSING from the original 33S1
//! umbrella. Pre-fix, the dispatch interception in
//! `Runtime::call_tool` matched names with `io.` (dotted) prefix,
//! but `corvid-ir::lower::lower_expr` produces bare `callee_name =
//! "io_read_text"` (no module prefix). The literal-name tests
//! in 33S1b passed because they wrote `"io.write_text"` directly,
//! bypassing the IR. This test compiles a real `.cor` source
//! through `compile_to_ir_with_config_at_path` and runs it
//! through `run_ir_with_runtime` — the same path `corvid run`
//! takes — and asserts the executing tool's side effect actually
//! lands on disk. If this test passes, real Corvid programs can
//! call the executing file-I/O tools.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{IoToolPolicy, Runtime};
use corvid_vm::Value;
use std::fs;

/// 33S1-fix-naming load-bearing acceptance — compile + run a
/// Corvid program that calls `io_write_text`, assert the file
/// got written through the dispatch. This is the proof that the
/// rename from `io.` to `io_` actually wires real Corvid code
/// to the executing surface.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_corvid_program_writes_file_through_executing_io_dispatch() {
    let project = tempfile::tempdir().expect("tempdir");
    // Lay out the project: src/main.cor + the std modules it
    // imports + a corvid.toml carrying [io] root = ".".
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    // main.cor lives at src/main.cor and imports `./std/io`, so
    // the resolver looks for src/std/io.cor. Mirror the
    // `corvid new` scaffold convention: vendor the std modules
    // under src/std/.
    fs::create_dir_all(project.path().join("src").join("std")).unwrap();
    fs::copy(
        repo.join("std").join("io.cor"),
        project.path().join("src").join("std").join("io.cor"),
    )
    .unwrap();
    fs::copy(
        repo.join("std").join("effects.cor"),
        project.path().join("src").join("std").join("effects.cor"),
    )
    .unwrap();
    fs::write(
        project.path().join("corvid.toml"),
        "[io]\nroot = \".\"\n",
    )
    .unwrap();

    // The program: import the executing tool, call it, return a
    // marker the test can assert on. The `io_write_text` call is
    // the load-bearing line — pre-fix it would have been routed
    // to `tools.call("io_write_text", ...)` and errored with
    // UnknownTool; post-fix it hits the dispatch interception
    // (strip_prefix `"io_"` -> `"write_text"`).
    let source = r#"
import "./std/io" use io_write_text

agent main() -> Int:
    io_write_text("note.txt", "hello from real corvid")
    return 42
"#;
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, source).unwrap();

    // Compile through the driver — same path `corvid run` uses.
    let ir = compile_to_ir_with_config_at_path(source, &main_path, None)
        .expect("real corvid source should compile");

    // Build a Runtime with the IoToolPolicy anchored at the
    // project dir (mirrors how `corvid run` wires it through
    // `load_io_tool_policy`).
    let policy = IoToolPolicy::new(Some("."), Some(project.path()));
    let runtime = Runtime::builder().io_policy(policy).build();

    // Run the program. If the dispatch interception works, the
    // file gets written and `main` returns 42.
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("real corvid program should run end-to-end through the executing surface");

    // The return value should be the marker Int 42.
    match result {
        Value::Int(42) => {}
        other => panic!(
            "expected main to return Int(42), got {other:?} — \
             the dispatch may have errored silently"
        ),
    }

    // Load-bearing assertion: the file written by `io_write_text`
    // must exist on disk with the right contents. Pre-fix, the
    // dispatch was bypassed and this assertion would FAIL.
    let written = fs::read_to_string(project.path().join("note.txt"))
        .expect("io_write_text must have written note.txt under the configured root");
    assert_eq!(
        written, "hello from real corvid",
        "file contents must match what the Corvid program wrote"
    );
}
