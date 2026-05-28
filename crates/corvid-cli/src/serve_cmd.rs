//! `corvid serve` — run a Corvid app's `server` block over HTTP.
//!
//! Loads the app, builds the same interpreter `Runtime` `corvid run`
//! uses, and dispatches each route to its handler agent via
//! [`corvid_driver::run_ir_with_runtime`]. The route surface comes from
//! `IrFile.servers` (lowered in slice `35V2-P42-E0-serve-1`).
//!
//! Dispatch shapes:
//! - **Literal** (`E0-serve-2`): a handler that is `return
//!   <agent>(<literal args>)` — covers `/schema`, `/config`, the
//!   `/…/mock` reads, and the literal-arg auth reads. Served on the
//!   route's method.
//! - **Body** (`E0-serve-4`): a handler that is `return <agent>(body)`
//!   on a route declaring a body type. The request JSON is deserialized
//!   into the body struct via `json_to_value` and passed to the handler.
//!
//! Approval posture: `corvid serve` has no interactive approver, so it
//! denies by default. A body route whose handler crosses an `approve`
//! boundary (every `execute_approved_*` does) is denied and answers
//! `403 approval_required` — the safe, honest behavior given Corvid's
//! synchronous `approve` semantics. Executing approval-gated writes over
//! HTTP (create-pending-approval → 202, reviewer executes) is the
//! developer-facing end state, filed as `35V2-P42-E0-serve-5`.
//!
//! Routes that match neither shape (path params, query types) answer
//! `501` so the gap is explicit rather than a silent `404`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, on, MethodFilter};
use axum::{Json, Router};
use corvid_ast::HttpMethod;
use corvid_driver::{
    compile_to_ir_with_config_at_path, load_corvid_config_for, render_all_pretty,
    run_ir_with_runtime, InterpErrorKind, RunError, Runtime, RuntimeError, Value,
};
use corvid_ir::{IrCallKind, IrExprKind, IrFile, IrLiteral, IrRoute, IrStmt, IrType};
use corvid_resolve::DefId;
use corvid_runtime::approvals::ProgrammaticApprover;
use corvid_types::Type;
use corvid_vm::{json_to_value, value_to_json};

/// Shared, read-only serving state: the lowered app + the interpreter
/// runtime. Both are constructed once at startup and shared across
/// request tasks behind an `Arc`.
struct ServeState {
    ir: IrFile,
    runtime: Runtime,
}

/// How a route's handler is invoked per request.
enum Dispatch {
    /// `return <agent>(<literal args>)` — call the agent with the
    /// pre-evaluated literal arguments.
    Literal { agent: String, args: Vec<Value> },
    /// `return <agent>(body)` — deserialize the request JSON into
    /// `body_ty` and pass it as the single argument.
    Body { agent: String, body_ty: Type },
}

/// A dispatchable route: its method/path plus how to invoke the handler.
struct RoutePlan {
    method: HttpMethod,
    path: String,
    dispatch: Dispatch,
}

pub(crate) fn cmd_serve(file: &Path, listen: &str) -> Result<u8> {
    let source =
        std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let config = load_corvid_config_for(file);
    let ir = match compile_to_ir_with_config_at_path(&source, file, config.as_ref()) {
        Ok(ir) => ir,
        Err(diags) => {
            eprint!("{}", render_all_pretty(&diags, file, &source));
            return Ok(1);
        }
    };
    if ir.servers.is_empty() {
        eprintln!("error: no `server` block found in {}", file.display());
        return Ok(1);
    }

    let mut plans: Vec<RoutePlan> = Vec::new();
    let mut not_served: Vec<(String, String)> = Vec::new();
    for server in &ir.servers {
        for route in &server.routes {
            match dispatch_for(route) {
                Some(plan) => plans.push(plan),
                None => not_served.push((route.method.as_str().to_string(), route.path.clone())),
            }
        }
    }

    let addr: SocketAddr = listen
        .parse()
        .with_context(|| format!("invalid --listen address `{listen}` (expected host:port)"))?;

    // `corvid serve` has no interactive approver, so it denies by
    // default: approval-gated writes hit their `approve` boundary, get
    // denied, and answer 403 (see module docs).
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_no()))
        .build();
    let state = Arc::new(ServeState { ir, runtime });

    let mut app: Router<Arc<ServeState>> = Router::new()
        .route("/healthz", on(MethodFilter::GET, || async { StatusCode::OK }))
        .route("/readyz", on(MethodFilter::GET, || async { StatusCode::OK }));

    for plan in plans.iter() {
        let filter = method_filter(plan.method);
        let handler = match &plan.dispatch {
            Dispatch::Literal { agent, args } => {
                let agent = agent.clone();
                let args = args.clone();
                on(filter, move |State(state): State<Arc<ServeState>>| {
                    let agent = agent.clone();
                    let args = args.clone();
                    async move { dispatch_literal(state, agent, args).await }
                })
            }
            Dispatch::Body { agent, body_ty } => {
                let agent = agent.clone();
                let body_ty = body_ty.clone();
                on(
                    filter,
                    move |State(state): State<Arc<ServeState>>, body: Bytes| {
                        let agent = agent.clone();
                        let body_ty = body_ty.clone();
                        async move { dispatch_body(state, agent, body_ty, body).await }
                    },
                )
            }
        };
        app = app.route(&plan.path, handler);
    }
    for (_method, path) in not_served.iter() {
        if plans.iter().any(|p| &p.path == path) {
            continue;
        }
        app = app.route(path, any(not_implemented));
    }

    let app = app.with_state(state);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind {addr}"))?;
        println!("corvid serve: listening on http://{addr}");
        for plan in &plans {
            let kind = match &plan.dispatch {
                Dispatch::Literal { agent, .. } => format!("-> {agent}"),
                Dispatch::Body { agent, .. } => format!("-> {agent} (body; approval-gated -> 403)"),
            };
            println!("  {:<6} {}  {kind}", plan.method.as_str(), plan.path);
        }
        for (method, path) in &not_served {
            println!("  {method:<6} {path}  -> 501 (not served)");
        }
        axum::serve(listener, app).await.context("serve").map(|_| ())
    })?;
    Ok(0)
}

/// Run a literal-arg handler agent and serialize its result to JSON.
async fn dispatch_literal(state: Arc<ServeState>, agent: String, args: Vec<Value>) -> Response {
    finish(run_ir_with_runtime(&state.ir, Some(&agent), args, &state.runtime).await)
}

/// Deserialize the request body into the route's body type, then run the
/// handler agent with it.
async fn dispatch_body(
    state: Arc<ServeState>,
    agent: String,
    body_ty: Type,
    body: Bytes,
) -> Response {
    let json: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(j) => j,
        Err(e) => {
            return bad_request("invalid_json", &e.to_string());
        }
    };
    let types_by_id: HashMap<DefId, &IrType> =
        state.ir.types.iter().map(|t| (t.id, t)).collect();
    let body_val = match json_to_value(json, &body_ty, &types_by_id) {
        Ok(v) => v,
        Err(e) => {
            return bad_request("invalid_body", &format!("{e:?}"));
        }
    };
    finish(run_ir_with_runtime(&state.ir, Some(&agent), vec![body_val], &state.runtime).await)
}

/// Map a handler outcome to an HTTP response: 200 + JSON on success,
/// 403 when an `approve` boundary denied, 500 otherwise.
fn finish(outcome: Result<Value, RunError>) -> Response {
    match outcome {
        Ok(value) => (StatusCode::OK, Json(value_to_json(&value))).into_response(),
        Err(RunError::Interp(e)) if is_approval_denied(&e.kind) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "approval_required",
                "detail": "this write is approval-gated; `corvid serve` has no interactive approver. Approve out-of-band or wait for the HTTP approval queue (E0-serve-5).",
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "handler_failed", "detail": err.to_string() })),
        )
            .into_response(),
    }
}

/// An `approve` boundary denial — either the interpreter's own
/// `ApprovalDenied` or the corvid-runtime approver's `ApprovalDenied`
/// (a deny bubbles up through the runtime boundary as the latter).
fn is_approval_denied(kind: &InterpErrorKind) -> bool {
    matches!(kind, InterpErrorKind::ApprovalDenied(_))
        || matches!(
            kind,
            InterpErrorKind::Runtime(RuntimeError::ApprovalDenied { .. })
        )
}

fn bad_request(error: &str, detail: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": error, "detail": detail })),
    )
        .into_response()
}

async fn not_implemented() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "not_implemented",
            "detail": "this route shape is not served yet (path-param / query routes)",
        })),
    )
        .into_response()
}

fn method_filter(m: HttpMethod) -> MethodFilter {
    match m {
        HttpMethod::Get => MethodFilter::GET,
        HttpMethod::Post => MethodFilter::POST,
        HttpMethod::Put => MethodFilter::PUT,
        HttpMethod::Patch => MethodFilter::PATCH,
        HttpMethod::Delete => MethodFilter::DELETE,
    }
}

/// Decide how (if at all) a route can be dispatched. Returns `None` for
/// shapes not served yet (path-param / query routes, or handlers that
/// aren't a single `return <agent>(...)`).
fn dispatch_for(route: &IrRoute) -> Option<RoutePlan> {
    if !route.path_params.is_empty() || route.query_ty.is_some() {
        return None;
    }
    for stmt in &route.body.stmts {
        if let IrStmt::Return {
            value: Some(expr), ..
        } = stmt
        {
            let IrExprKind::Call {
                kind: IrCallKind::Agent { .. },
                callee_name,
                args,
            } = &expr.kind
            else {
                return None;
            };
            // All-literal args → Literal dispatch.
            if let Some(vals) = args.iter().map(literal_value).collect::<Option<Vec<_>>>() {
                return Some(RoutePlan {
                    method: route.method,
                    path: route.path.clone(),
                    dispatch: Dispatch::Literal {
                        agent: callee_name.clone(),
                        args: vals,
                    },
                });
            }
            // Single `body` argument + a declared body type → Body dispatch.
            if args.len() == 1 {
                if let (IrExprKind::Local { .. }, Some(body_ty)) =
                    (&args[0].kind, route.body_ty.as_ref())
                {
                    return Some(RoutePlan {
                        method: route.method,
                        path: route.path.clone(),
                        dispatch: Dispatch::Body {
                            agent: callee_name.clone(),
                            body_ty: body_ty.clone(),
                        },
                    });
                }
            }
            return None;
        }
    }
    None
}

fn literal_value(arg: &corvid_ir::IrExpr) -> Option<Value> {
    match &arg.kind {
        IrExprKind::Literal(lit) => Some(match lit {
            IrLiteral::Int(n) => Value::Int(*n),
            IrLiteral::Float(f) => Value::Float(*f),
            IrLiteral::String(s) => Value::String(s.as_str().into()),
            IrLiteral::Bool(b) => Value::Bool(*b),
            IrLiteral::Nothing => Value::Nothing,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::{RouteResponseKind, Span};
    use corvid_ir::{IrBlock, IrExpr};
    use corvid_resolve::LocalId;

    fn span() -> Span {
        Span::new(0, 0)
    }

    fn lit(s: &str) -> IrExpr {
        IrExpr {
            kind: IrExprKind::Literal(IrLiteral::String(s.into())),
            ty: Type::String,
            span: span(),
        }
    }

    fn local(name: &str) -> IrExpr {
        IrExpr {
            kind: IrExprKind::Local {
                local_id: LocalId(0),
                name: name.into(),
            },
            ty: Type::String,
            span: span(),
        }
    }

    fn route(method: HttpMethod, agent: &str, args: Vec<IrExpr>, body_ty: Option<Type>) -> IrRoute {
        let call = IrExpr {
            kind: IrExprKind::Call {
                kind: IrCallKind::Agent { def_id: DefId(0) },
                callee_name: agent.into(),
                args,
            },
            ty: Type::String,
            span: span(),
        };
        IrRoute {
            method,
            path: "/x".into(),
            path_params: vec![],
            query_ty: None,
            body_ty,
            response_kind: RouteResponseKind::Json,
            response_ty: Type::String,
            effect_names: vec![],
            body: IrBlock {
                stmts: vec![IrStmt::Return {
                    value: Some(call),
                    span: span(),
                }],
                span: span(),
            },
            span: span(),
        }
    }

    #[test]
    fn get_zero_arg_handler_is_literal_dispatch() {
        let plan = dispatch_for(&route(HttpMethod::Get, "make_manifest", vec![], None))
            .expect("dispatchable");
        match plan.dispatch {
            Dispatch::Literal { agent, args } => {
                assert_eq!(agent, "make_manifest");
                assert!(args.is_empty());
            }
            _ => panic!("expected Literal"),
        }
    }

    #[test]
    fn get_literal_arg_handler_is_literal_dispatch() {
        let plan = dispatch_for(&route(
            HttpMethod::Get,
            "auth_status",
            vec![lit("user-1"), lit("tenant-1")],
            None,
        ))
        .expect("dispatchable");
        match plan.dispatch {
            Dispatch::Literal { agent, args } => {
                assert_eq!(agent, "auth_status");
                assert_eq!(args.len(), 2);
                assert!(matches!(&args[0], Value::String(s) if &**s == "user-1"));
            }
            _ => panic!("expected Literal"),
        }
    }

    #[test]
    fn post_body_handler_is_body_dispatch() {
        let plan = dispatch_for(&route(
            HttpMethod::Post,
            "execute_approved_share",
            vec![local("body")],
            Some(Type::Struct(DefId(7))),
        ))
        .expect("dispatchable");
        assert_eq!(plan.method, HttpMethod::Post);
        match plan.dispatch {
            Dispatch::Body { agent, body_ty } => {
                assert_eq!(agent, "execute_approved_share");
                assert!(matches!(body_ty, Type::Struct(DefId(7))));
            }
            _ => panic!("expected Body"),
        }
    }

    #[test]
    fn body_arg_without_body_type_is_not_dispatchable() {
        // A `body` local but no declared body type — can't deserialize.
        assert!(dispatch_for(&route(HttpMethod::Post, "handle", vec![local("body")], None)).is_none());
    }

    #[test]
    fn path_param_route_is_not_dispatchable() {
        let mut r = route(HttpMethod::Get, "make", vec![], None);
        r.path_params.push(corvid_ir::IrRoutePathParam {
            name: "id".into(),
            ty: Type::String,
            span: span(),
        });
        assert!(dispatch_for(&r).is_none());
    }
}
