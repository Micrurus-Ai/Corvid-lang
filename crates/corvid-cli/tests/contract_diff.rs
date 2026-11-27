//! B10-5 — `corvid contract app --diff BASE` reports action changes and
//! the callers each reaches, and exits 1 on a breaking one, so a CI step
//! can stop a release that would break a deployed client.

use std::process::Command;

const BASE: &str = "type MoveCard:\n    card: Int\n    to: String\n\naction move_card(move: MoveCard) -> Int requires anyone:\n    return move.card\n";

fn corvid(dir: &std::path::Path, args: &[&str]) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_corvid")).args(args).current_dir(dir).output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), text)
}

#[test]
fn a_breaking_change_is_reported_with_its_callers_and_fails_the_step() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.cor"), BASE).unwrap();
    let (code, out) = corvid(dir.path(), &["contract", "app", "main.cor", "--out", "base.json"]);
    assert_eq!(code, 0, "{out}");

    let (code, out) = corvid(dir.path(), &["contract", "app", "main.cor", "--diff", "base.json"]);
    assert_eq!((code, out.trim()), (0, "no action changed"), "an unchanged program passes");

    std::fs::write(dir.path().join("main.cor"), BASE.replace("    to: String", "    column: String")).unwrap();
    let (code, out) = corvid(dir.path(), &["contract", "app", "main.cor", "--diff", "base.json"]);
    assert_eq!(code, 1, "a breaking change fails the step: {out}");
    assert!(out.contains("breaking: action `move_card`: its input no longer has `to`"), "{out}");
    assert!(out.contains("reaches TypeScript client `api.move_card`"), "{out}");

    std::fs::write(dir.path().join("main.cor"), BASE.replace("    to: String", "    to: String\n    note: Option<String>")).unwrap();
    let (code, out) = corvid(dir.path(), &["contract", "app", "main.cor", "--diff", "base.json"]);
    assert_eq!(code, 0, "an additive change passes: {out}");
    assert!(out.contains("additive: action `move_card`: its input gained the optional `note`"), "{out}");
}
