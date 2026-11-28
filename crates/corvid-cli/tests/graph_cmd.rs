//! `corvid graph`: each form is drawn from one checked program, and a bad
//! name is refused.

use std::process::Command;

const SOURCE: &str = r#"effect payment:
    cost: $0.01

tool lookup_order(id: String) -> Int
tool issue_refund(id: String, amount: Int) -> String dangerous uses payment
tool archive_order(id: String) -> String

agent refund_bot(id: String) -> String:
    amount = lookup_order(id)
    approve IssueRefund(id, amount)
    return issue_refund(id, amount)
"#;

fn graph(args: &[&str]) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("refund.cor");
    std::fs::write(&file, SOURCE).unwrap();
    Command::new(env!("CARGO_BIN_EXE_corvid"))
        .arg("graph")
        .arg(&file)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &std::process::Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn text_draws_the_steps_and_what_nothing_reaches() {
    let out = stdout(&graph(&[]));
    assert!(out.contains("agent refund_bot"), "{out}");
    assert!(out.contains("approve IssueRefund") && out.contains("a person decides"), "{out}");
    assert!(out.contains("tool issue_refund (dangerous)") && out.contains("refund.cor:11"), "{out}");
    assert!(out.contains("Never reached:\n  tool archive_order (refund.cor:6)"), "{out}");
}

#[test]
fn mermaid_is_a_flowchart_with_dangerous_tools_marked() {
    let out = stdout(&graph(&["--format", "mermaid"]));
    assert!(out.starts_with("flowchart TD\n"), "{out}");
    assert!(out.contains("[/\"approve IssueRefund"), "approval is a parallelogram: {out}");
    assert!(out.contains("class n4 dangerous"), "{out}");
    assert!(out.contains("subgraph never_reached"), "{out}");
}

#[test]
fn json_is_the_driver_document() {
    let out = stdout(&graph(&["--format", "json"]));
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["schema"], "corvid.agent_graph.v1");
    assert_eq!(doc["entries"][0]["steps"][1]["guards"], "issue_refund");
    assert_eq!(doc["never_reached"][0]["name"], "archive_order");
}

#[test]
fn an_unknown_agent_is_refused() {
    let output = graph(&["--agent", "nobody"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("`nobody` is not an agent"));
}
