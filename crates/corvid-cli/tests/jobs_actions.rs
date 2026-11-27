//! B10-4b end to end through the `corvid` binary: `jobs enqueue
//! --source` checks an action's payload before storing it, refuses an
//! action that needs a signed-in caller, and `jobs run` runs the stored
//! action job through the action path.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PROGRAM: &str = "identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    roles:
        member: \"cards.edit\"

type Label:
    card: Int
    label: String

action add_label(input: Label) -> String requires anyone:
    return input.label + \" on \" + input.card.to_string()

action claim(card: Int, by: Caller) -> Int requires permission(\"cards.edit\"):
    return card
";

fn corvid(dir: &Path, args: &[&str]) -> Output {
    Command::new(PathBuf::from(env!("CARGO_BIN_EXE_corvid")))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("corvid runs")
}

fn enqueue(dir: &Path, task: &str, payload: &str, source: bool) -> Output {
    let mut args = vec!["jobs", "enqueue", "--state", "jobs.sqlite", "--task", task, "--payload", payload, "--max-retries", "0"];
    if source {
        args.extend(["--source", "app.cor"]);
    }
    corvid(dir, &args)
}

#[test]
fn an_action_job_is_checked_when_enqueued_and_run_by_a_worker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.cor"), PROGRAM).unwrap();
    let stderr = |out: &Output| String::from_utf8_lossy(&out.stderr).to_string();

    let good = enqueue(dir.path(), "add_label", r#"{"card": 7, "label": "urgent"}"#, true);
    assert!(good.status.success(), "{}", stderr(&good));

    let bad = enqueue(dir.path(), "add_label", r#"{"card": "seven", "label": "x"}"#, true);
    assert_eq!(bad.status.code(), Some(1));
    assert!(stderr(&bad).contains("not `add_label`'s input"), "{}", stderr(&bad));

    let signed_in = enqueue(dir.path(), "claim", r#"{"card": 1}"#, true);
    assert_eq!(signed_in.status.code(), Some(1));
    assert!(stderr(&signed_in).contains("requires a signed-in caller"), "{}", stderr(&signed_in));

    // Without the source nothing is checked at enqueue; the worker refuses.
    let unchecked = enqueue(dir.path(), "claim", r#"{"card": 1}"#, false);
    assert!(unchecked.status.success(), "{}", stderr(&unchecked));

    let run = corvid(
        dir.path(),
        &["jobs", "run", "--state", "jobs.sqlite", "--source", "app.cor", "--workers", "1", "--max-runtime-ms", "2000"],
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "{}{}", stdout, stderr(&run));
    assert!(stdout.contains("succeeded=1"), "the action job ran: {stdout}");
    assert!(stdout.contains("failed=1"), "the signed-in action was refused: {stdout}");
}
