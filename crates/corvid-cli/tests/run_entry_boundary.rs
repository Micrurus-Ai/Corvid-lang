//! `corvid run` runs a program whose entry agent native code cannot carry
//! across the command line (a `List` result, a record parameter) on the
//! interpreter, naming why, instead of failing its native build. The
//! reason is the code generator's own rule, so the two never disagree.

use std::path::PathBuf;
use std::process::Command;

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

#[test]
fn a_list_result_runs_on_the_interpreter_and_native_refuses_it_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("list.cor");
    std::fs::write(&program, "agent main() -> List<Int>:\n    return [1, 2]\n").unwrap();

    let auto = Command::new(corvid_bin()).arg("run").arg(&program).output().expect("corvid run");
    let stderr = String::from_utf8_lossy(&auto.stderr);
    assert!(auto.status.success(), "corvid run failed: {stderr}");
    assert!(stderr.contains("running via interpreter: entry agent return of type `List<Int>`"), "{stderr}");
    assert_eq!(String::from_utf8_lossy(&auto.stdout).trim(), "[1, 2]");

    let native = Command::new(corvid_bin()).args(["run", "--target=native"]).arg(&program).output().expect("corvid run");
    assert!(!native.status.success());
    let stderr = String::from_utf8_lossy(&native.stderr);
    assert!(stderr.contains("`--target=native` refused: entry agent return of type `List<Int>`"), "{stderr}");
}
