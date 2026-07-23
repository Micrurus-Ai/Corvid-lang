//! Connector-operation dispatch tests (slice 52g-3c).
//!
//! A connector `operation` lowers to an `IrTool`, so calling one runs
//! through the VM's normal Tool-arm pipeline (effect / approval / budget
//! / taint / provenance / trace). In the deployment-selected `mock`
//! mode the VM evaluates the operation's COMPILED `mock` expression with
//! the operation's parameters bound to the typed call arguments — the
//! real evaluator, real `Value`s, no JSON bypass.

use super::*;
use corvid_ast::ConnectorMode;

fn mock_mode_runtime() -> Runtime {
    Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .connector_mode(Some(ConnectorMode::Mock))
        .build()
}

const READ_CONNECTOR: &str = r#"
effect http_read:
    cost: 1.0

type Repo:
    name: String

connector github:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_TOKEN"))
    modes: [mock, replay, real]
    operation get_repo(owner: String, repo: String) -> Repo uses http_read:
        GET "/repos/{owner}/{repo}"
        mock: Repo(repo)

agent main() -> String:
    r = get_repo("micrurus", "corvid")
    return r.name
"#;

#[tokio::test]
async fn connector_operation_dispatches_in_mock_mode_with_typed_args() {
    let ir = ir_of(READ_CONNECTOR);
    let rt = mock_mode_runtime();
    let v = run_agent(&ir, "main", vec![], &rt).await.expect("run");
    // The compiled mock `Repo(repo)` was evaluated with `repo` bound to
    // the typed argument `"corvid"` — so `.name` is "corvid". If the
    // mock were built from raw JSON instead of the compiled expression,
    // the parameter reference could not resolve.
    assert_eq!(v, Value::String(Arc::from("corvid")));
}

const DANGEROUS_CONNECTOR: &str = r#"
effect http_write:
    cost: 2.0

type Issue:
    id: Int
type NewIssue:
    title: String

connector github:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_TOKEN"))
    modes: [mock]
    operation create_issue(owner: String, req: NewIssue) -> Issue dangerous uses http_write:
        POST "/repos/{owner}/issues" body req
        mock: Issue(1)

agent main() -> Int:
    approve CreateIssue("micrurus", NewIssue("bug"))
    i = create_issue("micrurus", NewIssue("bug"))
    return i.id
"#;

#[tokio::test]
async fn a_dangerous_connector_operation_cannot_bypass_the_approval_gate() {
    // A connector operation is a tool: its dispatch runs INSIDE the Tool
    // arm's approval gate. With a denying approver, the operation is
    // refused at runtime — a connector call cannot smuggle a dangerous
    // effect past approval. (The compile-time gate already forces the
    // `approve`; this proves the runtime gate fires too.)
    let ir = ir_of(DANGEROUS_CONNECTOR);
    let rt = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_no()))
        .connector_mode(Some(ConnectorMode::Mock))
        .build();
    let err = run_agent(&ir, "main", vec![], &rt).await.unwrap_err();
    let text = format!("{err:?}").to_lowercase();
    assert!(
        text.contains("approval") || text.contains("denied"),
        "a denied dangerous connector op must error at the approval gate, got: {err:?}"
    );
}

#[tokio::test]
async fn a_dangerous_connector_operation_dispatches_once_approved() {
    let ir = ir_of(DANGEROUS_CONNECTOR);
    let rt = mock_mode_runtime();
    let v = run_agent(&ir, "main", vec![], &rt).await.expect("run");
    assert_eq!(v, Value::Int(1));
}
