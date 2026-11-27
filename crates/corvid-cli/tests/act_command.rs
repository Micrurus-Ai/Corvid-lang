//! B10-4 — `corvid act` calls a declared action from the command line,
//! reading its input by the rules every other caller uses and printing
//! the result as public JSON.

use std::process::Command;

const BOARD: &str = r#"type Column:
    | Todo
    | Done

type MoveCard:
    card: Int
    to: Column
    position: Int where between(0, 500)

type Moved:
    card: Int
    position: Int

action move_card(move: MoveCard) -> Moved requires anyone:
    return Moved(move.card, move.position)

action label(card: Int, labels: List<String>, note: Option<String>) -> String requires anyone:
    shown = match note:
        Some(text) -> text
        None -> "no note"
    return card.to_string() + ": " + labels.join(", ") + " (" + shown + ")"

action watch(from: Int) -> Stream<Int> requires anyone:
    yield from
    yield from + 1
"#;

const GUARDED: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        editor: "board.edit"

action archive(card: Int) -> Int requires permission("board.edit"):
    return card
"#;

fn act(source: &str, args: &[&str]) -> (i32, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cor");
    std::fs::write(&file, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .arg("act")
        .arg("--file")
        .arg(&file)
        .args(args)
        .current_dir(dir.path())
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn flags_read_like_form_fields_and_the_result_is_json() {
    let (code, out, err) = act(BOARD, &["move_card", "--card", "3", "--to", "Done", "--position", "7"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap(), serde_json::json!({"card": 3, "position": 7}));

    let (code, out, err) = act(BOARD, &["label", "--card=4", "--labels", "urgent", "--labels", "blocked"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "\"4: urgent, blocked (no note)\"", "a repeated flag fills a list; an optional may be left out");
}

#[test]
fn json_input_and_streams() {
    let (code, out, err) = act(BOARD, &["--json", r#"{"card": 1, "to": {"tag": "variant", "type": "Column", "variant": "Todo", "fields": []}, "position": 0}"#, "move_card"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("\"card\": 1"), "{out}");

    let (code, out, err) = act(BOARD, &["watch", "--from", "5"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.lines().collect::<Vec<_>>(), ["5", "6"], "a stream prints one JSON value per item");
}

#[test]
fn a_refused_input_names_the_field_and_nothing_runs() {
    let (code, _, err) = act(BOARD, &["move_card", "--card", "3", "--to", "Done", "--position", "900"]);
    assert_eq!(code, 1);
    assert!(err.contains("position"), "{err}");

    let (code, _, err) = act(BOARD, &["move_card", "--card", "3", "--to", "Done", "--position", "1", "--colour", "red"]);
    assert_eq!(code, 1);
    assert!(err.contains("takes no `--colour`"), "{err}");

    let (code, _, err) = act(BOARD, &["move_cards"]);
    assert_eq!(code, 1);
    assert!(err.contains("its actions are: move_card, label, watch"), "{err}");
}

/// `corvid run`'s positional arguments read by the same request-text
/// rule, so a type beyond Int, Float, Bool and String is accepted.
#[test]
fn run_arguments_read_every_text_type() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cor");
    std::fs::write(&file, "agent main(price: Decimal, qty: Int) -> Decimal:\n    return price * qty\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["run", "--target", "interpreter"])
        .arg(&file)
        .args(["2.50", "3"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(out.trim().starts_with("7.5"), "{out}");
}

#[test]
fn an_action_that_requires_a_signed_in_caller_is_refused_by_name() {
    let (code, out, err) = act(GUARDED, &["archive", "--card", "3"]);
    assert_eq!(code, 1);
    assert!(out.is_empty(), "the body does not run: {out}");
    assert!(err.contains("requires a signed-in caller") && err.contains("B11"), "{err}");
}
