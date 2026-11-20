//! Regression test for 20l-A: `corvid check` resolves imports.
//!
//! Before 20l-A, `cmd_check` called the path-less driver entry
//! `compile_with_config`, so a missing or misnamed import was not
//! caught — `corvid check` reported "ok" on programs whose `corvid
//! build` would fail. Editor / pre-commit / LSP integrations all
//! reported false positives.
//!
//! This test reproduces the reporter's exact case: a file whose
//! single import does not resolve. `corvid check` must reject it
//! with a "could not be found" diagnostic and a non-zero exit code.

use std::process::Command;
use tempfile::tempdir;

#[test]
fn new_project_checks_and_runs_with_an_incomplete_installation() {
    let tmp = tempdir().unwrap();
    let install = tmp.path().join("install");
    std::fs::create_dir_all(install.join("std")).unwrap();
    std::fs::write(install.join("std/extra.cor"), "# local module\n").unwrap();
    let cli = |cwd: &std::path::Path, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(cwd)
            .env("CORVID_HOME", &install)
            .args(args)
            .output()
            .unwrap()
    };
    let project = tmp.path().join("hello");
    let created = cli(tmp.path(), &["new", "hello"]);
    assert!(created.status.success(), "{created:?}");
    assert!(project.join("src/std/extra.cor").exists());
    let checked = cli(&project, &["check"]);
    assert!(checked.status.success(), "{checked:?}");
    let greeting = cli(&project, &["run"]);
    assert!(greeting.status.success(), "{greeting:?}");
    assert!(String::from_utf8_lossy(&greeting.stdout).contains("Hello, Corvid!"));
    std::fs::write(project.join("src/main.cor"), "agent main() -> Int:\n    return \"bad\"\n").unwrap();
    let rejected = cli(&project, &["check"]);
    assert!(!rejected.status.success());
    let diagnostic = String::from_utf8_lossy(&rejected.stderr);
    assert!(diagnostic.contains("main.cor:2:") && diagnostic.contains("Int"), "{diagnostic}");
}

#[test]
fn check_rejects_missing_import() {
    let tmp = tempdir().expect("create tempdir");
    let main = tmp.path().join("main.cor");
    std::fs::write(
        &main,
        concat!(
            "import \"./does_not_exist\" use Anything\n",
            "\n",
            "agent main() -> Bool:\n",
            "    return true\n",
        ),
    )
    .expect("write main.cor");

    let out = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["check"])
        .arg(&main)
        .output()
        .expect("spawn corvid");

    assert!(
        !out.status.success(),
        "check should reject missing import; got status {:?}\nstdout={}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let combined = String::from_utf8_lossy(&out.stdout).into_owned()
        + &String::from_utf8_lossy(&out.stderr);
    assert!(
        combined.contains("could not be found"),
        "expected import-not-found diagnostic, got:\n{combined}"
    );
}
