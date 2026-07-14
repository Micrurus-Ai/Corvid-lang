//! End-to-end pins for the governed-cron scheduler runner.
//!
//! `corvid schedule run` registers every `schedule` declaration as a
//! durable-queue schedule manifest and fires due jobs through the
//! worker pool. The happy path uses an every-second cron so a short
//! bounded run observes real fires executing the target agent; the
//! refusal paths pin the literal-args contract and the empty-source
//! diagnostic.

use std::path::Path;
use std::process::Command;

fn run_corvid(args: &[&str], cwd: &Path) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_corvid");
    Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run corvid")
}

#[test]
fn schedule_run_fires_due_jobs_through_the_worker_pool() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("sched.cor"),
        "agent heartbeat(label: String) -> String:\n    return label + \"-fired\"\n\nschedule \"* * * * * *\" zone \"UTC\" -> heartbeat(\"tick\")\n",
    )
    .expect("write source");

    let output = run_corvid(
        &[
            "schedule",
            "run",
            "--source",
            "sched.cor",
            "--state",
            "jobs.sqlite",
            "--max-runtime-ms",
            "3500",
            "--poll-ms",
            "250",
        ],
        dir.path(),
    );
    assert!(
        output.status.success(),
        "schedule run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("schedule registered: sched_heartbeat_0"),
        "manifest registration must be reported; got: {stdout}"
    );
    // An every-second cron over a ~3.5s window must fire at least
    // once, and every fire must execute the agent successfully (the
    // executor unwraps the schedule-fire envelope into positional
    // args).
    let succeeded: u64 = stdout
        .lines()
        .find_map(|line| line.strip_prefix("result: succeeded="))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("missing result line in: {stdout}"));
    assert!(
        succeeded >= 1,
        "at least one scheduled fire must succeed; got: {stdout}"
    );
    assert!(
        stdout.contains("failed=0"),
        "no fire may fail; got: {stdout}"
    );
}

#[test]
fn schedule_run_refuses_non_literal_arguments() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("bad.cor"),
        "agent heartbeat(label: String) -> String:\n    return label\n\nagent make_label() -> String:\n    return \"x\"\n\nschedule \"* * * * * *\" zone \"UTC\" -> heartbeat(make_label())\n",
    )
    .expect("write source");

    let output = run_corvid(
        &[
            "schedule",
            "run",
            "--source",
            "bad.cor",
            "--state",
            "jobs.sqlite",
            "--max-runtime-ms",
            "500",
        ],
        dir.path(),
    );
    assert!(
        !output.status.success(),
        "non-literal schedule args must refuse"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("argument 1 is not a literal"),
        "diagnostic must name the offending argument; got: {stderr}"
    );
    assert!(
        stderr.contains("move computation"),
        "diagnostic must state the fix; got: {stderr}"
    );
}

#[test]
fn schedule_run_refuses_source_without_schedules() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("none.cor"),
        "agent main() -> String:\n    return \"no schedules\"\n",
    )
    .expect("write source");

    let output = run_corvid(
        &[
            "schedule",
            "run",
            "--source",
            "none.cor",
            "--state",
            "jobs.sqlite",
        ],
        dir.path(),
    );
    assert!(!output.status.success(), "no schedules must refuse");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("declares no `schedule` blocks"),
        "diagnostic must state the reason; got: {stderr}"
    );
}
