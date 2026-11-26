//! `corvid build` builds natively by default, and the retired Python
//! target says why it is gone. A program native code cannot compile yet
//! is refused by name, never translated in part.

use std::path::Path;
use std::process::Command;

fn corvid(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_corvid")).args(args).current_dir(dir).output().expect("run corvid");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn the_python_target_is_retired_with_its_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("main.cor"), "agent main() -> Int:\n    return 1\n").expect("write");
    let (ok, text) = corvid(dir.path(), &["build", "main.cor", "--target", "python"]);
    assert!(!ok, "{text}");
    assert!(text.contains("the `python` build target was retired"), "{text}");
    assert!(text.contains("corvid build main.cor"), "the message names the native build: {text}");
    assert!(!dir.path().join("target").join("py").exists(), "nothing is written");
}

/// A table is not native yet, so the default build refuses the program
/// and names the reason, where the Python default used to write a
/// partial translation.
#[test]
fn the_default_build_is_native_and_refuses_what_native_cannot_compile() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("main.cor"),
        "database library:\n    engine: sqlite\n    location: \"library.sqlite\"\n\ntable Shelf:\n    key shelf: Int\n    title: String\n\nagent main() -> Int:\n    return match Shelf.get(1):\n        Some(row) -> row.shelf\n        None -> 0\n",
    )
    .expect("write");
    let (ok, text) = corvid(dir.path(), &["build", "main.cor"]);
    assert!(!ok, "a program native code cannot compile is refused: {text}");
    assert!(!dir.path().join("target").join("py").exists(), "no Python is written: {text}");
    assert!(!text.contains("built:"), "{text}");
}
