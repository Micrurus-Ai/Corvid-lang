//! `corvid connectors simulate` — counterfactual protocol exploration.
//!
//! The checker proves a protocol is well-formed. These tests cover the
//! different question the simulator answers: what its legal provider
//! behaviours cost you, and which of them an author is unlikely to have
//! considered.

use std::process::Command;

fn write(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("main.cor");
    std::fs::write(&path, body).expect("write source");
    path
}

/// Run the command and return `(exit code, parsed JSON, human report)`.
fn simulate(source: &str, deny: &[&str]) -> (i32, serde_json::Value, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write(dir.path(), source);
    let exe = env!("CARGO_BIN_EXE_corvid");
    let deny_args: Vec<String> = deny
        .iter()
        .flat_map(|kind| ["--deny".to_string(), (*kind).to_string()])
        .collect();

    let json_run = Command::new(exe)
        .args(["connectors", "simulate"])
        .arg(&path)
        .arg("--json")
        .args(&deny_args)
        .output()
        .expect("run simulate --json");
    let text_run = Command::new(exe)
        .args(["connectors", "simulate"])
        .arg(&path)
        .args(&deny_args)
        .output()
        .expect("run simulate");

    let stdout = String::from_utf8_lossy(&json_run.stdout).to_string();
    let parsed = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "simulate --json must emit JSON ({e}); stdout: {stdout}; stderr: {}",
            String::from_utf8_lossy(&json_run.stderr)
        )
    });
    (
        json_run.status.code().unwrap_or(-1),
        parsed,
        String::from_utf8_lossy(&text_run.stdout).to_string(),
    )
}

/// `terminal:` lists `abandoned`, but no transition targets it.
const UNREACHABLE_TERMINAL: &str = r#"
effect http_write:
    cost: 1.0

type Job:
    id: String
    status: String

connector shipping:
    base_url: "https://api.example.com"
    auth: bearer(secret("SHIPPING_TOKEN"))
    modes: [real]
    operation submit(order: String) -> Job dangerous uses http_write:
        POST "/shipments" body order
        async:
            statuses: [queued, completed, failed]
            initial: queued
            terminal: [completed, failed, abandoned]
            deadline: 600s
            deadline_target: failed
            idempotency: intent via header "Idempotency-Key"
            poll GET "/shipments/{id}"
            every: 30s
            on_protocol_change: refuse
            state queued:
                on queued -> queued
                on completed -> completed
                on failed -> failed

agent main() -> Job uses http_write:
    approve Submit("order-1")
    return submit("order-1")
"#;

const HEALTHY: &str = r#"
effect http_write:
    cost: 1.0

type Job:
    id: String
    status: String

connector shipping:
    base_url: "https://api.example.com"
    auth: bearer(secret("SHIPPING_TOKEN"))
    modes: [real]
    operation submit(order: String) -> Job dangerous uses http_write:
        POST "/shipments" body order
        async:
            statuses: [queued, processing, completed, failed]
            initial: queued
            terminal: [completed, failed]
            deadline: 600s
            deadline_target: failed
            idempotency: intent via header "Idempotency-Key"
            poll GET "/shipments/{id}"
            every: 30s
            on_protocol_change: refuse
            state queued:
                on queued -> queued
                on processing -> processing
                on completed -> completed
                on failed -> failed
            state processing:
                on queued -> processing
                on processing -> processing
                on completed -> completed
                on failed -> failed

agent main() -> Job uses http_write:
    approve Submit("order-1")
    return submit("order-1")
"#;

/// The headline finding: a protocol the checker accepts can still be held
/// open indefinitely by a provider that never fails, and the outcome the
/// program then has to handle is the deadline target — the arm nobody
/// writes a test for.
#[test]
fn a_slow_provider_is_reported_as_reaching_the_deadline_target() {
    let (code, json, report) = simulate(HEALTHY, &[]);
    assert_eq!(code, 0, "every reported behaviour is legal, so nothing fails");

    let sim = &json[0];
    assert_eq!(sim["operation"], "shipping.submit");
    let stalls = sim["non_terminating"].as_array().expect("array");
    assert!(
        stalls
            .iter()
            .any(|s| s["status"] == "processing" && s["state"] == "processing"),
        "a self-looping status must be reported as non-terminating; got {stalls:?}"
    );
    assert!(
        report.contains("without the provider ever failing"),
        "the report must make clear no provider fault is required; got:\n{report}"
    );
}

/// The worst case the simulator reports must be the worst case the
/// compiler charges the budget for. A simulator that disagreed with the
/// cost analysis would be worse than none.
#[test]
fn the_reported_worst_case_matches_the_declared_bound() {
    let (_, json, _) = simulate(HEALTHY, &[]);
    assert_eq!(
        json[0]["worst_case_polls"], 20,
        "600s deadline / 30s cadence = 20 observations"
    );
    assert_eq!(json[0]["deadline_target"], "failed");
}

/// The CI surface. Stalling is LEGAL, so the command cannot fail on it by
/// default — but a team can declare it unacceptable for a given protocol,
/// and that is a property the checker cannot prove for them.
#[test]
fn deny_turns_a_legal_but_unwanted_behaviour_into_a_ci_failure() {
    let (clean, _, _) = simulate(HEALTHY, &[]);
    assert_eq!(clean, 0, "informational by default");

    let (denied, _, _) = simulate(HEALTHY, &["non_terminating"]);
    assert_eq!(
        denied, 1,
        "--deny non_terminating must fail a protocol a slow provider can hold open"
    );

    let (unrelated, _, _) = simulate(HEALTHY, &["unreachable_terminal"]);
    assert_eq!(
        unrelated, 0,
        "--deny must only fire on the kind that is actually present"
    );
}

/// The checker already proves reachability, so an unreachable terminal
/// never reaches the simulator — it is refused at compile time. This test
/// pins that division of labour: the simulator reports LEGAL behaviours,
/// the checker rejects malformed ones.
#[test]
fn an_unreachable_terminal_is_refused_by_the_compiler_not_the_simulator() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write(dir.path(), UNREACHABLE_TERMINAL);
    let out = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["connectors", "simulate"])
        .arg(&path)
        .output()
        .expect("run simulate");
    assert!(!out.status.success(), "the file must not compile");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unreachable"),
        "the compiler must name the unreachable state; got: {stderr}"
    );
}

/// A file with no `async:` block is not an error — it has nothing to
/// explore, and the command should say so rather than fail.
#[test]
fn a_file_without_protocols_reports_nothing_to_explore() {
    let (code, json, report) = simulate("agent main() -> String:\n    return \"hello\"\n", &[]);
    assert_eq!(code, 0);
    assert!(json.as_array().expect("array").is_empty());
    assert!(report.contains("no `async:` protocols"));
}
