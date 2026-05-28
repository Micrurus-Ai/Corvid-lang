//! `corvid serve` — run a Corvid app's `server` block over HTTP.
//!
//! Loads the app, builds the same interpreter `Runtime` `corvid run`
//! uses, and dispatches each route to its handler agent via
//! [`corvid_driver::run_ir_with_runtime`]. The route surface comes from
//! `IrFile.servers` (lowered in slice `35V2-P42-E0-serve-1`).
//!
//! This slice (`E0-serve-2`) serves GET routes whose handler is a direct
//! agent call with literal arguments — that covers `/schema`, `/config`,
//! the `/…/mock` reads, and the auth-status reads in the reference apps.
//! Other shapes (`POST` body routes, path-param routes) are listed at
//! startup as not-yet-served and answer `501` so the gap is explicit
//! rather than a silent `404`. Struct-body `POST` dispatch lands in
//! `E0-serve-4`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use corvid_ast::HttpMethod;
use corvid_driver::{
    compile_to_ir_with_config_at_path, load_corvid_config_for, render_all_pretty,
    run_ir_with_runtime, Runtime, Value,
};
use corvid_ir::{IrCallKind, IrExprKind, IrFile, IrLiteral, IrRoute, IrStmt};
use corvid_runtime::approvals::ProgrammaticApprover;
use corvid_vm::value_to_json;

/// Shared, read-only serving state: the lowered app + the interpreter
/// runtime. Both are constructed once at startup and shared across
/// request tasks behind an `Arc`.
struct ServeState {
    ir: IrFile,
    runtime: Runtime,
}

/// A route the server can dispatch today: `GET` whose handler is
/// `return <agent>(<literal args>)`.
struct Dispatchable {
    path: String,
    agent: String,
    args: Vec<Value>,
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

    let mut dispatchable: Vec<Dispatchable> = Vec::new();
    let mut not_served: Vec<(String, String)> = Vec::new();
    for server in &ir.servers {
        for route in &server.routes {
            match dispatch_for(route) {
                Some(d) => dispatchable.push(d),
                None => not_served.push((route.method.as_str().to_string(), route.path.clone())),
            }
        }
    }

    let addr: SocketAddr = listen
        .parse()
        .with_context(|| format!("invalid --listen address `{listen}` (expected host:port)"))?;

    // Serve never reads stdin, so an interactive approver would hang any
    // approval-gated route. Use a programmatic approver; approval-gated
    // POST routes are not dispatched in this slice regardless.
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .build();
    let state = Arc::new(ServeState { ir, runtime });

    let mut app: Router<Arc<ServeState>> = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/readyz", get(|| async { StatusCode::OK }));

    for d in dispatchable.iter() {
        let agent = d.agent.clone();
        let args = d.args.clone();
        app = app.route(
            &d.path,
            get(move |State(state): State<Arc<ServeState>>| {
                let agent = agent.clone();
                let args = args.clone();
                async move { dispatch(state, agent, args).await }
            }),
        );
    }
    for (_method, path) in not_served.iter() {
        // Register a catch-all 501 for each not-yet-served path so the
        // surface is explicit (501, not 404). Skip paths already served
        // by a GET handler above to avoid an axum duplicate-route panic.
        if dispatchable.iter().any(|d| &d.path == path) {
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
        for d in &dispatchable {
            println!("  GET  {}  -> {}", d.path, d.agent);
        }
        for (method, path) in &not_served {
            println!("  {method:<4} {path}  -> 501 (not served in this slice)");
        }
        axum::serve(listener, app)
            .await
            .context("serve")
            .map(|_| ())
    })?;
    Ok(0)
}

/// Run the route's handler agent and serialize its result to JSON.
async fn dispatch(state: Arc<ServeState>, agent: String, args: Vec<Value>) -> Response {
    match run_ir_with_runtime(&state.ir, Some(&agent), args, &state.runtime).await {
        Ok(value) => (StatusCode::OK, Json(value_to_json(&value))).into_response(),
        Err(err) => {
            let body = serde_json::json!({
                "error": "handler_failed",
                "detail": err.to_string(),
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
        }
    }
}

async fn not_implemented() -> Response {
    let body = serde_json::json!({
        "error": "not_implemented",
        "detail": "this route is not served yet (body/path-param dispatch lands in a later slice)",
    });
    (StatusCode::NOT_IMPLEMENTED, Json(body)).into_response()
}

/// Decide whether a route is dispatchable today and, if so, extract the
/// handler agent name + literal argument values. Returns `None` for any
/// non-`GET` route or any handler that isn't a single
/// `return <agent>(<literals>)`.
fn dispatch_for(route: &IrRoute) -> Option<Dispatchable> {
    if route.method != HttpMethod::Get {
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
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                match &a.kind {
                    IrExprKind::Literal(lit) => vals.push(literal_value(lit)),
                    // Non-literal arg (body/path/field access) — not this slice.
                    _ => return None,
                }
            }
            return Some(Dispatchable {
                path: route.path.clone(),
                agent: callee_name.clone(),
                args: vals,
            });
        }
    }
    None
}

fn literal_value(lit: &IrLiteral) -> Value {
    match lit {
        IrLiteral::Int(n) => Value::Int(*n),
        IrLiteral::Float(f) => Value::Float(*f),
        IrLiteral::String(s) => Value::String(s.as_str().into()),
        IrLiteral::Bool(b) => Value::Bool(*b),
        IrLiteral::Nothing => Value::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::{RouteResponseKind, Span};
    use corvid_ir::{IrBlock, IrExpr};
    use corvid_resolve::DefId;
    use corvid_types::Type;

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
                local_id: corvid_resolve::LocalId(0),
                name: name.into(),
            },
            ty: Type::String,
            span: span(),
        }
    }

    fn route(method: HttpMethod, agent: &str, args: Vec<IrExpr>) -> IrRoute {
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
            body_ty: None,
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
    fn get_zero_arg_handler_is_dispatchable() {
        let d = dispatch_for(&route(HttpMethod::Get, "make_manifest", vec![])).expect("dispatchable");
        assert_eq!(d.agent, "make_manifest");
        assert!(d.args.is_empty());
    }

    #[test]
    fn get_literal_arg_handler_is_dispatchable() {
        let d = dispatch_for(&route(
            HttpMethod::Get,
            "auth_status",
            vec![lit("user-1"), lit("tenant-1")],
        ))
        .expect("dispatchable");
        assert_eq!(d.agent, "auth_status");
        assert_eq!(d.args.len(), 2);
        assert!(matches!(&d.args[0], Value::String(s) if &**s == "user-1"));
    }

    #[test]
    fn post_route_is_not_dispatchable() {
        assert!(dispatch_for(&route(HttpMethod::Post, "execute", vec![])).is_none());
    }

    #[test]
    fn get_with_nonliteral_arg_is_not_dispatchable() {
        // `return handle(body)` — the `body` local is not a literal, so
        // this route waits for struct-body dispatch (E0-serve-4).
        assert!(dispatch_for(&route(HttpMethod::Get, "handle", vec![local("body")])).is_none());
    }
}
