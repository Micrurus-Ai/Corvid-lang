use std::{fs, process::Command};

#[test]
fn format_a_project_then_check_run_and_test_it() {
    let tmp = tempfile::tempdir().unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(tmp.path().join("app"))
            .args(args)
            .output()
            .unwrap()
    };
    let created = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .current_dir(tmp.path())
        .args(["new", "app"])
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    let root = tmp.path().join("app");
    let main = root.join("src/main.cor");
    let math = root.join("src/math.cor");
    let unit = root.join("tests/unit.cor");
    fs::create_dir(unit.parent().unwrap()).unwrap();
    let source = "import \"./math\" use double\nagent main()->String:\n  if double( 4 )==8:\n    return \"x:  -  y\"  # keep this\n  return \"failed\"";
    fs::write(&main, source).unwrap();
    fs::write(&math, "public fn double(n:Int)->Int:\r\n  return n*2\r\n").unwrap();
    fs::write(&unit, "test broken(:").unwrap();
    let invalid = cli(&["fmt"]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unit.cor:1:"));
    assert_eq!(fs::read_to_string(&main).unwrap(), source);
    fs::write(
        &unit,
        "import \"../src/math\" use double\ntest math_works:\n  assert double(4)==8",
    )
    .unwrap();
    let drift = cli(&["fmt", "--check"]);
    assert_eq!(drift.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&main).unwrap(), source);
    assert!(cli(&["fmt"]).status.success());
    assert_eq!(
        fs::read_to_string(&math).unwrap(),
        "public fn double(n: Int) -> Int:\r\n    return n * 2\r\n"
    );
    assert!(fs::read_to_string(&main)
        .unwrap()
        .contains("    if double(4) == 8:\n        return \"x:  -  y\"  # keep this\n"));
    for args in [
        &["fmt", "--check"][..],
        &["check"],
        &["test", "tests/unit.cor"],
    ] {
        let out = cli(args);
        assert!(out.status.success(), "{args:?} {out:?}");
    }
    let run = cli(&["run"]);
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "x:  -  y");
}
