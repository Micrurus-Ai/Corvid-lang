use super::*;

#[test]
fn lowers_simple_agent() {
    let src = "\
tool greet(name: String) -> String

agent hello(name: String) -> String:
    message = greet(name)
    return message
";
    let ir = lower_src(src);
    assert_eq!(ir.tools.len(), 1);
    assert_eq!(ir.agents.len(), 1);
    assert_eq!(ir.agents[0].body.stmts.len(), 2);
    assert!(matches!(ir.agents[0].body.stmts[0], IrStmt::Let { .. }));
    assert!(matches!(ir.agents[0].body.stmts[1], IrStmt::Return { .. }));
}

#[test]
fn approve_is_structured_in_ir() {
    let src = "\
tool send_email(to: String, body: String) -> Nothing dangerous

agent do_it(to: String) -> Nothing:
    approve SendEmail(to, to)
    return send_email(to, to)
";
    let ir = lower_src(src);
    let agent = &ir.agents[0];
    match &agent.body.stmts[0] {
        IrStmt::Approve { label, args, .. } => {
            assert_eq!(label, "SendEmail");
            assert_eq!(args.len(), 2);
        }
        other => panic!("expected Approve, got {other:?}"),
    }
}

#[test]
fn break_continue_pass_lower_to_dedicated_variants() {
    // The typechecker gives loop var `x` a real type
    // (String, for String iteration), so `if x:` is now a Bool
    // mismatch. Use an explicit comparison.
    let src = "\
agent loop_stuff(xs: String) -> String:
    for x in xs:
        if x == \"a\":
            break
        continue
    pass
    return xs
";
    let ir = lower_src(src);
    let agent = &ir.agents[0];
    // First stmt: a For loop containing Break and Continue.
    match &agent.body.stmts[0] {
        IrStmt::For { body, .. } => {
            // body has: if-stmt (containing Break), continue
            match &body.stmts[0] {
                IrStmt::If { then_block, .. } => {
                    assert!(matches!(then_block.stmts[0], IrStmt::Break { .. }));
                }
                other => panic!("expected If, got {other:?}"),
            }
            assert!(matches!(body.stmts[1], IrStmt::Continue { .. }));
        }
        other => panic!("expected For, got {other:?}"),
    }
    // Second stmt at top level: Pass.
    assert!(matches!(agent.body.stmts[1], IrStmt::Pass { .. }));
}

#[test]
fn tool_call_ir_identifies_the_tool() {
    let src = "\
tool get_order(id: String) -> Order
type Order:
    id: String

agent a(id: String) -> Order:
    return get_order(id)
";
    let ir = lower_src(src);
    let agent = &ir.agents[0];
    match &agent.body.stmts[0] {
        IrStmt::Return { value: Some(e), .. } => match &e.kind {
            IrExprKind::Call {
                kind, callee_name, ..
            } => {
                assert_eq!(callee_name, "get_order");
                assert!(matches!(kind, IrCallKind::Tool { .. }));
            }
            other => panic!("expected Call, got {other:?}"),
        },
        other => panic!("expected Return, got {other:?}"),
    }
}

#[test]
fn fixture_and_mock_lower_to_test_ir() {
    let src = r#"
tool lookup(id: String) -> Int

fixture sample_id() -> String:
    return "ord_42"

mock lookup(id: String) -> Int:
    return 42

test mocked_lookup:
    id = sample_id()
    value = lookup(id)
    assert value == 42
"#;
    let ir = lower_src(src);
    assert_eq!(ir.fixtures.len(), 1);
    assert_eq!(ir.mocks.len(), 1);
    assert_eq!(ir.tests.len(), 1);
    match &ir.tests[0].body.stmts[0] {
        IrStmt::Let { value, .. } => match &value.kind {
            IrExprKind::Call {
                kind, callee_name, ..
            } => {
                assert_eq!(callee_name, "sample_id");
                assert!(matches!(kind, IrCallKind::Fixture { .. }));
            }
            other => panic!("expected fixture call, got {other:?}"),
        },
        other => panic!("expected let, got {other:?}"),
    }
    assert_eq!(ir.mocks[0].target_name, "lookup");
    assert_eq!(ir.mocks[0].return_ty, corvid_types::Type::Int);
}

#[test]
fn lowers_result_option_constructors_and_none() {
    let src = "\
agent build(flag: Bool) -> Result<Option<String>, String>:
    if flag:
        return Ok(Some(\"hi\"))
    return Ok(None)
";
    let ir = lower_src(src);
    let agent = &ir.agents[0];
    match &agent.body.stmts[0] {
        IrStmt::If {
            then_block,
            else_block: None,
            ..
        } => match &then_block.stmts[0] {
            IrStmt::Return {
                value: Some(expr), ..
            } => match &expr.kind {
                IrExprKind::ResultOk { inner } => match &inner.kind {
                    IrExprKind::OptionSome { .. } => {}
                    other => panic!("expected OptionSome, got {other:?}"),
                },
                other => panic!("expected ResultOk, got {other:?}"),
            },
            other => panic!("expected Return, got {other:?}"),
        },
        other => panic!("expected If, got {other:?}"),
    }
    match &agent.body.stmts[1] {
        IrStmt::Return {
            value: Some(expr), ..
        } => match &expr.kind {
            IrExprKind::ResultOk { inner } => {
                assert!(matches!(inner.kind, IrExprKind::OptionNone));
            }
            other => panic!("expected ResultOk, got {other:?}"),
        },
        other => panic!("expected Return, got {other:?}"),
    }
}

#[test]
fn lowers_try_propagate_and_retry() {
    let src = "\
tool fetch(id: String) -> Result<String, String>
tool lookup(id: String) -> Result<String, String>

agent load(id: String) -> Result<String, String>:
    value = fetch(id)?
    stable = try lookup(id) on error retry 3 times backoff exponential 40
    joined = stable?
    return Ok(value + joined)
";
    let ir = lower_src(src);
    let agent = &ir.agents[0];
    match &agent.body.stmts[0] {
        IrStmt::Let { value, .. } => {
            assert!(matches!(value.kind, IrExprKind::TryPropagate { .. }));
        }
        other => panic!("expected Let, got {other:?}"),
    }
    match &agent.body.stmts[1] {
        IrStmt::Let { value, .. } => match &value.kind {
            IrExprKind::TryRetry {
                attempts, backoff, ..
            } => {
                assert_eq!(*attempts, 3);
                assert_eq!(*backoff, Backoff::Exponential(40));
            }
            other => panic!("expected TryRetry, got {other:?}"),
        },
        other => panic!("expected Let, got {other:?}"),
    }
    match &agent.body.stmts[2] {
        IrStmt::Let { value, .. } => {
            assert!(matches!(value.kind, IrExprKind::TryPropagate { .. }));
        }
        other => panic!("expected Let, got {other:?}"),
    }
}

#[test]
fn lowers_evals_into_ir_nodes() {
    let src = "\
tool get_order(id: String) -> String
tool issue_refund(id: String) -> String dangerous

eval refund_process:
    order = get_order(\"ord_42\")
    assert called get_order before issue_refund
    assert approved IssueRefund
    assert cost < $0.50
    assert order == order with confidence 0.95 over 50 runs
";
    let ir = lower_src(src);
    assert_eq!(ir.evals.len(), 1);
    let eval = &ir.evals[0];
    assert_eq!(eval.name, "refund_process");
    assert_eq!(eval.body.stmts.len(), 1);
    assert_eq!(eval.assertions.len(), 4);
    assert!(matches!(eval.assertions[0], IrEvalAssert::Ordering { .. }));
    assert!(matches!(eval.assertions[1], IrEvalAssert::Approved { .. }));
    assert!(matches!(eval.assertions[2], IrEvalAssert::Cost { .. }));
    match &eval.assertions[3] {
        IrEvalAssert::Value {
            confidence, runs, ..
        } => {
            assert_eq!(*confidence, Some(0.95));
            assert_eq!(*runs, Some(50));
        }
        other => panic!("expected value assertion, got {other:?}"),
    }
}

#[test]
fn lowers_tests_into_ir_nodes() {
    let src = "\
tool get_order(id: String) -> String

test refund_contract:
    order = get_order(\"ord_42\")
    assert called get_order
    assert_snapshot order
    assert order == \"ord_42\"

test refund_trace from_trace \"traces/refund.jsonl\":
    assert called get_order
";
    let ir = lower_src(src);
    assert_eq!(ir.tests.len(), 2);
    let test = &ir.tests[0];
    assert_eq!(test.name, "refund_contract");
    assert_eq!(test.body.stmts.len(), 1);
    assert_eq!(test.assertions.len(), 3);
    assert!(matches!(test.assertions[0], IrEvalAssert::Called { .. }));
    assert!(matches!(test.assertions[1], IrEvalAssert::Snapshot { .. }));
    assert!(matches!(test.assertions[2], IrEvalAssert::Value { .. }));
    assert_eq!(
        ir.tests[1].trace_fixture.as_deref(),
        Some("traces/refund.jsonl")
    );
}

#[test]
fn lowers_yield_to_dedicated_ir_stmt() {
    let src = "\
agent chunks(text: String) -> Stream<String>:
    yield text
";
    let ir = lower_src(src);
    let agent = &ir.agents[0];
    match &agent.body.stmts[0] {
        IrStmt::Yield { value, .. } => {
            assert_eq!(value.ty.display_name(), "String");
        }
        other => panic!("expected Yield, got {other:?}"),
    }
}

#[test]
fn wrapping_agent_lowers_integer_arithmetic_to_explicit_nodes() {
    let src = "\
@wrapping
agent hash_step(n: Int) -> Int:
    return -(n * 1099511628211)
";
    let ir = lower_src(src);
    let agent = ir.agents.iter().find(|a| a.name == "hash_step").unwrap();
    assert!(agent.wrapping_arithmetic);
    let ret = agent
        .body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            IrStmt::Return {
                value: Some(value), ..
            } => Some(value),
            _ => None,
        })
        .expect("return value");
    match &ret.kind {
        IrExprKind::WrappingUnOp { operand, .. } => {
            assert!(matches!(operand.kind, IrExprKind::WrappingBinOp { .. }));
        }
        other => panic!("expected WrappingUnOp, got {other:?}"),
    }
}

#[test]
fn lowers_server_block_into_ir_routes() {
    let src = "\
public type Manifest:
    name: String

public type Req:
    id: String

public type Resp:
    id: String

agent make_manifest() -> Manifest:
    return Manifest(\"svc\")

agent handle(req: Req) -> Resp:
    return Resp(req.id)

server demo_api:
    route GET \"/schema\" -> json Manifest:
        return make_manifest()
    route POST \"/do\" body Req -> json Resp:
        return handle(body)
";
    let ir = lower_src(src);
    assert_eq!(ir.servers.len(), 1, "expected one server block");
    let server = &ir.servers[0];
    assert_eq!(server.name, "demo_api");
    assert_eq!(server.routes.len(), 2);

    // GET /schema -> json Manifest: return make_manifest()
    let get = &server.routes[0];
    assert_eq!(get.method, corvid_ast::HttpMethod::Get);
    assert_eq!(get.path, "/schema");
    assert!(get.body_ty.is_none(), "GET route has no body type");
    let get_ret = get
        .body
        .stmts
        .iter()
        .find_map(|s| match s {
            IrStmt::Return { value: Some(v), .. } => Some(v),
            _ => None,
        })
        .expect("GET handler return");
    match &get_ret.kind {
        IrExprKind::Call { callee_name, args, .. } => {
            assert_eq!(callee_name, "make_manifest");
            assert!(args.is_empty(), "zero-arg GET handler");
        }
        other => panic!("expected Call, got {other:?}"),
    }

    // POST /do body Req -> json Resp: return handle(body)
    let post = &server.routes[1];
    assert_eq!(post.method, corvid_ast::HttpMethod::Post);
    assert_eq!(post.path, "/do");
    assert!(post.body_ty.is_some(), "POST route declares a body type");
    let post_ret = post
        .body
        .stmts
        .iter()
        .find_map(|s| match s {
            IrStmt::Return { value: Some(v), .. } => Some(v),
            _ => None,
        })
        .expect("POST handler return");
    match &post_ret.kind {
        IrExprKind::Call { callee_name, args, .. } => {
            assert_eq!(callee_name, "handle");
            assert_eq!(args.len(), 1, "POST handler passes the body");
            assert!(
                matches!(&args[0].kind, IrExprKind::Local { name, .. } if name == "body"),
                "POST handler arg is the `body` binding, got {:?}",
                args[0].kind
            );
        }
        other => panic!("expected Call, got {other:?}"),
    }
}

#[test]
fn lowers_direct_upload_policy_into_the_route_ir() {
    let ir = lower_src(
        r#"
server imports:
    @upload(max_mb: 3, mime: "text/csv, application/csv")
    route POST "/imports" body Upload<Csv> -> json Int:
        return body.size()
"#,
    );
    let route = &ir.servers[0].routes[0];
    assert_eq!(route.upload_format.as_deref(), Some("Csv"));
    let policy = route.upload_policy.as_ref().expect("lowered upload policy");
    assert_eq!(policy.max_bytes, Some(3 * 1024 * 1024));
    assert_eq!(
        policy.mime,
        vec!["text/csv".to_string(), "application/csv".to_string()]
    );
}
