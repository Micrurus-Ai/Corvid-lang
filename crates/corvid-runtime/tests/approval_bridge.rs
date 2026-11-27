use std::fs;
use std::path::Path;

use corvid_runtime::approver_bridge::{
    clear_registered_approver, evaluate_registered_approver, register_approver_from_source,
    simulate_approver_source, ApprovalSiteInput, CorvidApproverLoadStatus,
};
use serde_json::json;

fn write_source(dir: &Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, source).expect("write approver source");
    path
}

fn sample_site(label: &str) -> ApprovalSiteInput {
    ApprovalSiteInput {
        site_name: label.to_string(),
        agent_context: "refund_bot".to_string(),
        declared_at_file: "examples/refund.cor".to_string(),
        declared_at_line: 42,
        budget_remaining_usd: 1.0,
        trace_run_id: "run-1".to_string(),
    }
}

#[test]
fn well_typed_approver_registers_and_evaluates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(
        dir.path(),
        "approver.cor",
        r#"
@budget($0.05)
agent approve_site(site: ApprovalSite, args: ApprovalArgs, ctx: ApprovalContext) -> ApprovalDecision:
    if args.values[0] == "\"vip\"":
        return ApprovalDecision(true, "approved")
    return ApprovalDecision(false, "rejected")
"#,
    );

    register_approver_from_source(&path, 1.0).expect("register approver");
    let decision = evaluate_registered_approver(&sample_site("IssueRefund"), &[json!("vip")])
        .expect("evaluate approver")
        .expect("registered approver");
    assert!(decision.accepted);
    assert_eq!(decision.rationale.as_deref(), Some("approved"));

    clear_registered_approver();
}

#[test]
fn missing_approve_site_agent_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(
        dir.path(),
        "missing.cor",
        r#"
agent something_else() -> Bool:
    return true
"#,
    );

    let err = register_approver_from_source(&path, 1.0).expect_err("missing approve_site");
    assert_eq!(err.status, CorvidApproverLoadStatus::MissingAgent);
    assert!(err.message.contains("approve_site"));
}

#[test]
fn over_budget_approver_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(
        dir.path(),
        "over_budget.cor",
        r#"
@budget($2.00)
agent approve_site(site: ApprovalSite, args: ApprovalArgs, ctx: ApprovalContext) -> ApprovalDecision:
    return ApprovalDecision(true, "approved")
"#,
    );

    let err = register_approver_from_source(&path, 1.0).expect_err("over-budget approver");
    assert_eq!(err.status, CorvidApproverLoadStatus::OverBudget);
    assert!(err.message.contains("exceeds host ceiling"));
}

#[test]
fn simulate_returns_decision_and_rationale() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(
        dir.path(),
        "simulate.cor",
        r#"
agent approve_site(site: ApprovalSite, args: ApprovalArgs, ctx: ApprovalContext) -> ApprovalDecision:
    if site.label == "IssueRefund":
        return ApprovalDecision(false, "manual review")
    return ApprovalDecision(true, "approved")
"#,
    );

    let decision =
        simulate_approver_source(&path, "IssueRefund", "[1000]", 1.0).expect("simulate approver");
    assert!(!decision.accepted);
    assert_eq!(decision.rationale, "manual review");
}

/// An approver the approval evaluator cannot run is refused when it is
/// loaded, by registration and by simulation alike, naming the construct.
/// Before, it loaded and then failed its first real approval.
#[test]
fn an_approver_the_evaluator_cannot_run_is_refused_at_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let duration = write_source(
        dir.path(),
        "duration.cor",
        r#"
agent approve_site(site: ApprovalSite, args: ApprovalArgs, ctx: ApprovalContext) -> ApprovalDecision:
    limit = 5 minutes
    if limit == 5 minutes:
        return ApprovalDecision(true, "approved")
    return ApprovalDecision(false, "rejected")
"#,
    );
    let err = register_approver_from_source(&duration, 1.0).expect_err("a duration is refused at load");
    assert_eq!(err.status, CorvidApproverLoadStatus::CompileError);
    assert!(err.message.contains("the duration `5 minutes`"), "{}", err.message);
    let err = simulate_approver_source(&duration, "IssueRefund", "[]", 1.0).expect_err("and when simulated");
    assert!(err.message.contains("the duration `5 minutes`"), "{}", err.message);

    let call = write_source(
        dir.path(),
        "call.cor",
        r#"
agent approve_site(site: ApprovalSite, args: ApprovalArgs, ctx: ApprovalContext) -> ApprovalDecision:
    return ApprovalDecision(site.label.contains("Refund"), "checked")
"#,
    );
    let err = register_approver_from_source(&call, 1.0).expect_err("a method call is refused at load");
    assert!(err.message.contains("a method call"), "{}", err.message);
}

/// Approver arithmetic is checked, as everywhere in Corvid: an overflow is
/// an error the host sees, never a wrapped value or a panic.
#[test]
fn approver_arithmetic_overflow_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_source(
        dir.path(),
        "overflow.cor",
        r#"
agent approve_site(site: ApprovalSite, args: ApprovalArgs, ctx: ApprovalContext) -> ApprovalDecision:
    most = 9223372036854775807
    big = most + most
    return ApprovalDecision(big > 0, "checked")
"#,
    );
    let err = simulate_approver_source(&path, "IssueRefund", "[]", 1.0).expect_err("overflow is an error");
    assert!(err.message.contains("integer overflow"), "{}", err.message);
}
