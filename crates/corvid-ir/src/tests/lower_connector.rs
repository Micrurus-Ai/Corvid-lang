//! Lowering tests for `connector` blocks (slice 52g-3).
//!
//! A connector's `operation`s lower into two places at once: each
//! becomes an ordinary callable `IrTool` (so a call to it types and
//! dispatches exactly like a hand-written tool, and its effect row
//! composes with budgets / approval / replay / taint), and the
//! connector's HTTP dispatch metadata is carried in an `IrConnector`
//! keyed back to those tools by DefId.

use super::*;

const GITHUB_CONNECTOR: &str = r#"
effect http_read:
    cost: 1.0
effect http_write:
    cost: 2.0

type Repo:
    name: String
type Issue:
    id: Int
type NewIssue:
    title: String
type GithubError:
    | NotFound
    | ValidationFailed

connector github:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_TOKEN"))
    retry: 3
    rate_limit: 60 per 60s
    circuit_breaker: 5
    modes: [mock, real]
    operation get_repo(owner: String, repo: String) -> Repo uses http_read:
        GET "/repos/{owner}/{repo}"
        mock: Repo(repo)
    operation create_issue(owner: String, req: NewIssue) -> Result<Issue, GithubError> dangerous uses http_write:
        POST "/repos/{owner}/issues" body req
        on status 404 -> NotFound
        on status 422 -> ValidationFailed
        mock: Ok(Issue(1))
"#;

#[test]
fn each_operation_lowers_to_a_callable_tool() {
    let ir = lower_src(GITHUB_CONNECTOR);
    // Both operations appear as ordinary tools, so the interpreter's
    // tool-call machinery can dispatch them by name.
    let get_repo = ir
        .tools
        .iter()
        .find(|t| t.name == "get_repo")
        .expect("get_repo lowered to a tool");
    assert!(matches!(get_repo.return_ty, corvid_types::Type::Struct(_)));
    assert!(matches!(get_repo.effect, Effect::Safe));
    assert_eq!(get_repo.effect_names, vec!["http_read".to_string()]);

    let create = ir
        .tools
        .iter()
        .find(|t| t.name == "create_issue")
        .expect("create_issue lowered to a tool");
    // The `dangerous` marker rides through unchanged — this is what
    // makes the approval gate fire on a connector operation exactly as
    // it does on a hand-written dangerous tool.
    assert!(matches!(create.effect, Effect::Dangerous));
    assert_eq!(create.effect_names, vec!["http_write".to_string()]);
}

#[test]
fn connector_dispatch_metadata_is_carried_and_keyed_to_the_tools() {
    let ir = lower_src(GITHUB_CONNECTOR);
    assert_eq!(ir.connectors.len(), 1);
    let c = &ir.connectors[0];
    assert_eq!(c.name, "github");
    assert_eq!(c.base_url, "https://api.github.com");
    assert_eq!(c.retry, Some(3));
    assert_eq!(
        c.rate_limit.map(|r| (r.limit, r.window_secs)),
        Some((60, 60))
    );
    assert_eq!(c.circuit_breaker, Some(5));

    // The allowed execution modes carry through (never a default —
    // the checker rejects an undeclared set).
    assert_eq!(
        c.modes,
        vec![
            corvid_ast::ConnectorMode::Mock,
            corvid_ast::ConnectorMode::Real
        ]
    );

    // Credentials survive as the secret reference NAME, never a value —
    // so a trace can name which secret was used without revealing it.
    assert_eq!(
        c.auth,
        Some(IrConnectorAuth::Bearer {
            secret: "GITHUB_TOKEN".to_string()
        })
    );

    assert_eq!(c.operations.len(), 2);

    let get_repo = c
        .operations
        .iter()
        .find(|o| o.name == "get_repo")
        .expect("get_repo op metadata");
    assert!(matches!(get_repo.method, corvid_ast::HttpMethod::Get));
    assert_eq!(get_repo.path, "/repos/{owner}/{repo}");
    assert!(get_repo.body.is_none());
    assert!(get_repo.error_map.is_empty());
    // The `mock:` payload lowered to an evaluable expression.
    assert!(get_repo.mock.is_some(), "get_repo mock lowered");
    // The dispatch record points at the same DefId the callable tool
    // carries, so the runtime can resolve a tool call to its connector.
    let get_repo_tool = ir.tools.iter().find(|t| t.name == "get_repo").unwrap();
    assert_eq!(get_repo.tool_id, get_repo_tool.id);

    let create = c
        .operations
        .iter()
        .find(|o| o.name == "create_issue")
        .expect("create_issue op metadata");
    assert!(matches!(create.method, corvid_ast::HttpMethod::Post));
    let body = create.body.as_ref().expect("create_issue has a body");
    assert_eq!(body.param_name, "req");
    assert!(matches!(body.encoding, corvid_ast::BodyEncoding::Json));
    assert_eq!(
        create.error_map,
        vec![
            IrStatusErrorMapping {
                status: 404,
                variant: "NotFound".to_string()
            },
            IrStatusErrorMapping {
                status: 422,
                variant: "ValidationFailed".to_string()
            },
        ]
    );
}
