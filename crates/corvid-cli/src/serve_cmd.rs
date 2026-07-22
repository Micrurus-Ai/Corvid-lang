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
//! Approval posture: every `approve` boundary that fires under `corvid
//! serve` creates a pending entry in the existing `ApprovalQueueRuntime`
//! flow (slice `35V2-P42-E0-serve-5`) and answers `202 Accepted` with
//! `{"approval_id": "..."}` + a `Location: /__approvals/<id>` header so
//! the client can poll `GET /__approvals/<id>` for the eventual decision.
//! This replaces the prior deny-by-default `403 approval_required`
//! posture from `E0-serve-4`: the 403 was safe but developer-unusable,
//! since every approval-gated request died at the gate with no way for
//! a reviewer to make a decision. The async-approval model lets the
//! request proceed to a reviewer/queue out of band while the HTTP
//! handler immediately surfaces a polling id. The synchronous `Approver`
//! trait shape is preserved by routing the queued state through a new
//! `RuntimeError::ApprovalQueued { approval_id }` variant (see
//! `crate::serve_approval`); existing approver impls (StdinApprover /
//! ProgrammaticApprover) never produce this variant and need no change.
//!
//! Routes that match neither shape (path params, query types) answer
//! `501` so the gap is explicit rather than a silent `404`.

use std::collections::HashMap;
use std::ffi::CString;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::{RawPathParams, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{on, MethodFilter};
use axum::{Json, Router};
use corvid_ast::HttpMethod;
use corvid_driver::{
    compile_to_application_contract_with_config, compile_to_ir_with_config_at_path,
    load_corvid_config_for, render_all_pretty,
    run_ir_with_runtime, InterpErrorKind, RunError, Runtime, RuntimeError, ToolRegistry, Value,
};
use corvid_ir::{IrFile, IrStmt, IrType};
use corvid_resolve::DefId;
use corvid_runtime::approval_authorization::ApprovalActorContext;
use corvid_runtime::approval_queue::ApprovalQueueRuntime;
use corvid_runtime::approvals::ProgrammaticApprover;
use corvid_runtime::catalog_c_api::{corvid_register_tool, dispatch_host_tool, CorvidToolFn};
use corvid_types::Type;
use corvid_vm::{json_to_value, value_to_json};
use libloading::{Library, Symbol};

use crate::serve_approval::{QueueApprover, SERVE_DEFAULT_TENANT};

/// Actor id every `/__approvals/:id/{approve,deny}` transition runs
/// under at the slice MVP boundary. Per-request reviewer auth is a
/// follow-up; today every reviewer is the same anonymous actor.
const SERVE_REVIEWER_ACTOR: &str = "serve-reviewer";
/// Role the reviewer claims. Must match the `required_role` set in
/// `serve_approval::QueueApprover` (`operator`) for the queue's
/// `authorize_approval_transition` to accept the transition.
const SERVE_REVIEWER_ROLE: &str = "operator";

/// What the serve loop remembers about each in-flight approval so the
/// `/__approvals/:id/approve` handler can re-execute the original
/// agent without the client having to re-POST. Captured when an
/// `approve` boundary surfaces `ApprovalQueued` — the dispatch handler
/// already has the agent name + args at that point and just stashes
/// them under the freshly-minted approval id.
///
/// 33Q2 — `last_handler_error` carries the most recent
/// re-execution failure when an approval was granted but the
/// downstream handler errored. The approval stays `pending` so the
/// reviewer can retry without re-granting; the captured error is
/// surfaced by `GET /__approvals/<id>` and in the 500 body returned
/// by `POST /__approvals/<id>/approve` so the reviewer can decide
/// whether to retry (transient) or `/deny` (permanently broken).
/// Pre-33Q2 a 500 silently consumed the approval — the bug
/// anonymous-2026-06-04 P1.2 reported.
#[derive(Clone)]
struct PendingInvocation {
    agent: String,
    args: Vec<Value>,
    last_handler_error: Option<String>,
}

/// Shared, read-only serving state: the lowered app, the interpreter
/// runtime, and the approval queue the runtime's `QueueApprover` writes
/// into. All three are constructed once at startup and shared across
/// request tasks behind an `Arc`. The approval queue is also reachable
/// directly so the admin endpoints (`GET /__approvals`,
/// `GET /__approvals/<id>`) can list and fetch pending entries without
/// going through the runtime.
struct ServeState {
    ir: IrFile,
    runtime: Runtime,
    approval_queue: Arc<ApprovalQueueRuntime>,
    /// `approval_id -> PendingInvocation` so the transition handler
    /// can re-run the agent on `/approve` without the client re-
    /// POSTing the original request. Populated by the dispatch
    /// handlers on the queued path; drained on either `/approve`
    /// (re-executed) or `/deny` (discarded).
    pending_invocations: Arc<Mutex<HashMap<String, PendingInvocation>>>,
    /// Tool handlers registered at startup — typically populated by
    /// the `--with-tools-cdylib <path>` loader, but could also carry
    /// handlers from future loader paths (e.g. 33Q1b's tools.py
    /// autoloader). Cloned into the `/approve` handler's
    /// `bypass_runtime` builder so the re-executed agent sees the
    /// same tool registry as the original request, instead of
    /// failing with "no handler registered for tool `<name>`" — the
    /// regression anonymous-2026-06-04 round-2 P1.1 documented.
    host_tools: ToolRegistry,
    /// The Application Contract JSON, served at `/.well-known/corvid`
    /// so clients + tooling discover the surface from the live backend
    /// (slice 51r). `{}` when the contract could not be built.
    contract_json: String,
    /// The OpenAPI 3.1 JSON, served at `/openapi.json` (slice 51r).
    openapi_json: String,
}

pub(crate) fn cmd_serve(
    file: &Path,
    listen: &str,
    tools_cdylib: Option<&Path>,
) -> Result<u8> {
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

    // Contract Closure (slice 52b): the running backend proves it
    // implements its own contract, or it refuses to start. Any public
    // route the contract advertises but the interpreter tier cannot yet
    // execute — a streaming/upload/pagination boundary type, or a
    // `requires` policy with no authorization enforcement — is a startup
    // error (E5204) naming the element, never a silent runtime 501.
    let closure_gaps =
        corvid_driver::check_contract_closure(&ir, corvid_driver::RuntimeCapabilities::interpreter_tier());
    if !closure_gaps.is_empty() {
        eprintln!(
            "error: {} is not contract-closed — {} route(s) the contract advertises \
             cannot be executed by this runtime yet:",
            file.display(),
            closure_gaps.len()
        );
        for gap in &closure_gaps {
            eprintln!("  {}", gap.message());
        }
        return Ok(1);
    }

    let addr: SocketAddr = listen
        .parse()
        .with_context(|| format!("invalid --listen address `{listen}` (expected host:port)"))?;

    // `corvid serve` has no interactive approver; instead every
    // `approve` boundary creates a pending entry in the in-memory
    // approval queue via `QueueApprover` and surfaces the queued
    // state as `RuntimeError::ApprovalQueued`. `finish` then answers
    // 202 + approval id. See `crate::serve_approval` for the rationale.
    let approval_queue = Arc::new(
        ApprovalQueueRuntime::open_in_memory()
            .context("open in-memory approval queue for `corvid serve`")?,
    );
    // Tool registry composition (slices 33Q1a + 33Q1b):
    //
    // - `tools.py` autoloader (33Q1b) runs FIRST. If a `tools.py`
    //   sits next to the source (or in the project root), embed
    //   Python via PyO3, import it, and materialise one Rust handler
    //   per `@tool("<name>")`-decorated implementation.
    // - `--with-tools-cdylib <path>` loader (33Q1a) runs SECOND. The
    //   cdylib's `__corvid_tool_<name>` symbols are dlsym'd,
    //   registered via `corvid_register_tool`, and bridged into the
    //   ToolRegistry through `dispatch_host_tool`.
    // - `extend` overwrites same-named entries, so cdylib (the
    //   explicit operator flag) wins precedence over tools.py
    //   (implicit autoload). Mental model: explicit beats implicit.
    //
    // Without either source, `host_tools` stays empty and the
    // interpreter's existing `UnknownTool` error surfaces the gap
    // at call time — the same behaviour the trial documented as
    // P1.1, now an honest signal rather than a silent regression.
    let mut host_tools = corvid_runtime::python_tools::install_python_tools(file)
        .context("autoload tools.py")?;
    if let Some(cdylib_path) = tools_cdylib {
        host_tools.extend(register_cdylib_tool_handlers(&ir, cdylib_path)?);
    }

    let runtime = corvid_driver::apply_env_llm_wiring(
        Runtime::builder()
            .approver(Arc::new(QueueApprover::new(approval_queue.clone())))
            .tool_registry(host_tools.clone()),
    )
    .build();
    // Build the Application Contract + OpenAPI so the live backend
    // exposes its own machine-readable surface (slice 51r). Best-effort:
    // the source already compiled to IR, so this should succeed; if not,
    // fall back to `{}` and keep serving the routes.
    let generated_at =
        std::env::var("CORVID_BUILD_DATE").unwrap_or_else(|_| "unknown".to_string());
    let (contract_json, openapi_json) = match compile_to_application_contract_with_config(
        &source,
        &file.display().to_string(),
        &generated_at,
        config.as_ref(),
    ) {
        Ok(contract) => (
            serde_json::to_string(&contract).unwrap_or_else(|_| "{}".to_string()),
            serde_json::to_string(&corvid_abi::openapi::emit_openapi(&contract))
                .unwrap_or_else(|_| "{}".to_string()),
        ),
        Err(_) => ("{}".to_string(), "{}".to_string()),
    };

    let state = Arc::new(ServeState {
        ir,
        runtime,
        approval_queue,
        pending_invocations: Arc::new(Mutex::new(HashMap::new())),
        host_tools,
        contract_json,
        openapi_json,
    });

    let mut app: Router<Arc<ServeState>> = Router::new()
        .route("/healthz", on(MethodFilter::GET, || async { StatusCode::OK }))
        .route("/readyz", on(MethodFilter::GET, || async { StatusCode::OK }))
        // Admin endpoints (`E0-serve-5`). Read-only at the slice MVP
        // boundary; the transition surface (POST .../approve|deny) is
        // a follow-up. Hidden under `/__approvals` so they cannot
        // collide with an app-declared route.
        .route(
            "/__approvals",
            on(MethodFilter::GET, list_approvals),
        )
        .route(
            "/__approvals/:id",
            on(MethodFilter::GET, get_approval),
        )
        // Transition endpoints (`serve-6`): a reviewer marks an
        // approval granted, the server re-executes the original
        // request and returns the result; or marks it denied and
        // discards the pending invocation. axum 0.7 colon-capture.
        .route(
            "/__approvals/:id/approve",
            on(MethodFilter::POST, approve_approval),
        )
        .route(
            "/__approvals/:id/deny",
            on(MethodFilter::POST, deny_approval),
        )
        // The live backend advertises its own machine-readable surface
        // (slice 51r): the Application Contract at the well-known path
        // and a standard OpenAPI 3.1 document.
        .route("/.well-known/corvid", on(MethodFilter::GET, serve_contract))
        .route("/openapi.json", on(MethodFilter::GET, serve_openapi));

    // Slice 52a: register EVERY declared route and execute its body via
    // the synthetic per-route handler agent — any shape (path params,
    // query struct, typed body). No more `501 not_implemented` for
    // supported shapes. Routes sharing a path merge their methods.
    let mut by_path: std::collections::BTreeMap<String, axum::routing::MethodRouter<Arc<ServeState>>> =
        std::collections::BTreeMap::new();
    for server in &state.ir.servers {
        for route in &server.routes {
            let param_names: Vec<String> = state
                .ir
                .agents
                .iter()
                .find(|a| a.name == route.handler_agent)
                .map(|a| a.params.iter().map(|p| p.name.clone()).collect())
                .unwrap_or_default();
            let meta = Arc::new(RouteMeta {
                handler_agent: route.handler_agent.clone(),
                path_params: route
                    .path_params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone()))
                    .collect(),
                query_ty: route.query_ty.clone(),
                body_ty: route.body_ty.clone(),
                upload_format: route.upload_format.clone(),
                param_names,
            });
            let filter = method_filter(route.method);
            let mr = on(
                filter,
                move |State(state): State<Arc<ServeState>>,
                      raw_path: RawPathParams,
                      RawQuery(raw_query): RawQuery,
                      headers: axum::http::HeaderMap,
                      body: Bytes| {
                    let meta = meta.clone();
                    async move {
                        run_route(
                            state,
                            meta,
                            raw_path,
                            raw_query.unwrap_or_default(),
                            headers,
                            body,
                        )
                        .await
                    }
                },
            );
            let axum_path = corvid_route_to_axum_path(&route.path);
            match by_path.remove(&axum_path) {
                Some(existing) => {
                    by_path.insert(axum_path, existing.merge(mr));
                }
                None => {
                    by_path.insert(axum_path, mr);
                }
            }
        }
    }
    for (path, mr) in by_path {
        app = app.route(&path, mr);
    }

    // Clone the Arc before `with_state` moves it so the startup
    // banner below can still see the IR for the 33Q9 approval-label
    // check.
    let state_for_banner = state.clone();
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
        // Slice 52a: every declared route is served — its body executes
        // through the synthetic per-route handler agent. The
        // approval-gated label (33Q9) is a recursive IR walk of the
        // handler body for a reachable `IrStmt::Approve` (a conservative
        // under-count — no false positives).
        for server in &state_for_banner.ir.servers {
            for route in &server.routes {
                let approves =
                    agent_body_contains_approve(&state_for_banner.ir, &route.handler_agent);
                let tag = if approves {
                    "  (approval-gated -> 202 + queued)"
                } else {
                    ""
                };
                println!("  {:<6} {}{tag}", route.method.as_str(), route.path);
            }
        }
        println!("  GET    /__approvals                -> list pending approvals");
        println!("  GET    /__approvals/<id>           -> fetch one pending approval");
        println!("  POST   /__approvals/<id>/approve   -> approve + re-execute the original request");
        println!("  POST   /__approvals/<id>/deny      -> deny + drop the pending invocation");
        println!("  GET    /.well-known/corvid         -> the Application Contract");
        println!("  GET    /openapi.json               -> OpenAPI 3.1");
        axum::serve(listener, app).await.context("serve").map(|_| ())
    })?;
    Ok(0)
}

/// Run a literal-arg handler agent and serialize its result to JSON.

/// Per-route metadata captured for the general request handler
/// (slice 52a).
struct RouteMeta {
    /// Name of the synthetic per-route handler agent to invoke.
    handler_agent: String,
    /// Declared path params: `(name, type)` in path order.
    path_params: Vec<(String, Type)>,
    query_ty: Option<Type>,
    body_ty: Option<Type>,
    /// The `Upload<Format>` format tag when the body is an upload
    /// (slice 52c-2) — drives accepted-MIME enforcement.
    upload_format: Option<String>,
    /// The handler agent's param names, in order — drives which of
    /// `path`/`query`/`body`/`actor` values are assembled and how.
    param_names: Vec<String>,
}

/// Convert a Corvid route path (`/orders/{id}`) to axum's colon-capture
/// syntax (`/orders/:id`) (slice 52a).
fn corvid_route_to_axum_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            out.push(':');
            for seg in chars.by_ref() {
                if seg == '}' {
                    break;
                }
                out.push(seg);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Coerce a raw request string (path/query param) into a `Value` of the
/// declared scalar type (slice 52a).
fn coerce_str_to_value(raw: &str, ty: &Type) -> Result<Value, String> {
    Ok(match ty {
        Type::Int => Value::Int(raw.parse::<i64>().map_err(|_| format!("`{raw}` is not an Int"))?),
        Type::Float => {
            Value::Float(raw.parse::<f64>().map_err(|_| format!("`{raw}` is not a Float"))?)
        }
        Type::Bool => Value::Bool(
            raw.parse::<bool>().map_err(|_| format!("`{raw}` is not a Bool"))?,
        ),
        // String / TraceId / anything else the boundary carries as text.
        _ => Value::String(raw.into()),
    })
}

/// The unauthenticated `actor` placeholder bound in a route body when
/// the route carries a policy (slice 52a). Real session-derived actors
/// land in slices 52e/52f; until then the fields are empty so a body
/// that reads `actor.id` runs, and authorization is not yet enforced.
fn stub_actor_value() -> Value {
    let empty_list = Value::List(corvid_driver::ListValue::new(std::iter::empty::<Value>()));
    Value::Struct(corvid_driver::StructValue::new(
        DefId(0),
        "actor",
        vec![
            ("id".to_string(), Value::String("".into())),
            ("tenant".to_string(), Value::String("".into())),
            ("display_name".to_string(), Value::String("".into())),
            ("roles".to_string(), empty_list.clone()),
            ("permissions".to_string(), empty_list),
        ],
    ))
}

/// Execute a declared route: parse path params / query struct / typed
/// body from the request, then invoke the route's synthetic handler
/// agent with `[path, query?, body?, actor?]` in its declared param
/// order (slice 52a).
async fn run_route(
    state: Arc<ServeState>,
    meta: Arc<RouteMeta>,
    raw_path: RawPathParams,
    raw_query: String,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    let types_by_id: HashMap<DefId, &IrType> =
        state.ir.types.iter().map(|t| (t.id, t)).collect();

    // Path params → a `path` struct value.
    let path_lookup: HashMap<&str, &str> = raw_path.iter().collect();
    let mut path_fields: Vec<(String, Value)> = Vec::new();
    for (name, ty) in &meta.path_params {
        let raw = path_lookup.get(name.as_str()).copied().unwrap_or("");
        match coerce_str_to_value(raw, ty) {
            Ok(v) => path_fields.push((name.clone(), v)),
            Err(e) => return bad_request("invalid_path_param", &format!("{name}: {e}")),
        }
    }
    let path_value = Value::Struct(corvid_driver::StructValue::new(DefId(0), "path", path_fields));

    // Query string → the declared query struct value.
    let query_value = match &meta.query_ty {
        Some(ty) => match query_string_to_value(&raw_query, ty, &types_by_id) {
            Ok(v) => Some(v),
            Err(e) => return bad_request("invalid_query", &e),
        },
        None => None,
    };

    // Request body → the declared body value. An `Upload<Format>` body
    // is parsed from the multipart request (slice 52c-2); every other
    // body type is typed JSON.
    let body_value = match &meta.body_ty {
        Some(Type::Upload(_)) => {
            match parse_multipart_upload(&headers, &body, meta.upload_format.as_deref()).await {
                Ok(v) => Some(v),
                Err(resp) => return resp,
            }
        }
        Some(ty) => {
            let json: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(j) => j,
                Err(e) => return bad_request("invalid_json", &e.to_string()),
            };
            match json_to_value(json, ty, &types_by_id) {
                Ok(v) => Some(v),
                Err(e) => return bad_request("invalid_body", &format!("{e:?}")),
            }
        }
        None => None,
    };

    // Assemble args in the handler agent's declared param order.
    let mut args: Vec<Value> = Vec::new();
    for name in &meta.param_names {
        match name.as_str() {
            "path" => args.push(path_value.clone()),
            "query" => args.push(query_value.clone().unwrap_or(Value::Nothing)),
            "body" => args.push(body_value.clone().unwrap_or(Value::Nothing)),
            "actor" => args.push(stub_actor_value()),
            _ => {}
        }
    }

    let outcome =
        run_ir_with_runtime(&state.ir, Some(&meta.handler_agent), args.clone(), &state.runtime).await;
    capture_pending_invocation_if_queued(&state, &meta.handler_agent, &args, &outcome);
    finish(outcome)
}

/// Maximum accepted upload size for the interpreter tier (slice 52c-2).
/// A conservative default; per-format `@upload(max_size:)` overrides
/// are a follow-up once the annotation is threaded to the runtime.
const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;

/// Default accepted MIME types for a well-known upload format tag —
/// mirrors the contract's `default_mime_for_format` (slice 51f) so the
/// runtime enforces exactly what the contract advertises. An empty
/// slice means "accept any" (unknown/custom format tag).
fn accepted_mime_for_format(format: Option<&str>) -> &'static [&'static str] {
    match format {
        Some("Pdf") => &["application/pdf"],
        Some("Csv") => &["text/csv"],
        Some("Image") => &["image/png", "image/jpeg", "image/gif", "image/webp"],
        Some("Json") => &["application/json"],
        Some("Text") => &["text/plain"],
        _ => &[],
    }
}

/// Parse a multipart/form-data request into an `Upload<Format>` value
/// (slice 52c-2). Takes the FIRST file part, enforces the format's
/// accepted-MIME set and the max-size limit, and builds a struct value
/// carrying `filename` / `content_type` / `size` / `bytes` for the
/// `Upload` accessor methods. Returns a structured 400 on any
/// violation.
async fn parse_multipart_upload(
    headers: &axum::http::HeaderMap,
    body: &Bytes,
    format: Option<&str>,
) -> Result<Value, Response> {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let boundary = match multer::parse_boundary(content_type) {
        Ok(b) => b,
        Err(_) => {
            return Err(bad_request(
                "invalid_upload",
                "expected a multipart/form-data request with a boundary",
            ))
        }
    };

    // Feed the already-buffered body to multer as a single-chunk stream.
    let owned = body.clone();
    let stream = futures::stream::once(
        async move { Ok::<_, std::convert::Infallible>(owned) },
    );
    let mut multipart = multer::Multipart::new(stream, boundary);

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => {
                return Err(bad_request(
                    "invalid_upload",
                    "no file part found in the multipart request",
                ))
            }
            Err(e) => return Err(bad_request("invalid_upload", &e.to_string())),
        };
        // The upload is the first part that carries a file name; plain
        // form fields are skipped.
        let Some(filename) = field.file_name().map(|s| s.to_string()) else {
            continue;
        };
        let content_type = field
            .content_type()
            .map(|m| m.essence_str().to_string())
            .unwrap_or_default();

        let accepted = accepted_mime_for_format(format);
        if !accepted.is_empty() && !accepted.iter().any(|m| *m == content_type) {
            return Err(bad_request(
                "unsupported_media_type",
                &format!(
                    "`{content_type}` is not accepted for Upload<{}>; expected one of: {}",
                    format.unwrap_or("Format"),
                    accepted.join(", ")
                ),
            ));
        }

        let data = match field.bytes().await {
            Ok(b) => b,
            Err(e) => return Err(bad_request("invalid_upload", &e.to_string())),
        };
        if data.len() > MAX_UPLOAD_BYTES {
            return Err(bad_request(
                "upload_too_large",
                &format!(
                    "upload is {} bytes; the limit is {MAX_UPLOAD_BYTES} bytes",
                    data.len()
                ),
            ));
        }

        let bytes_list = Value::List(corvid_driver::ListValue::new(
            data.iter().map(|b| Value::Int(*b as i64)),
        ));
        let upload = corvid_driver::StructValue::new(
            DefId(0),
            "Upload",
            vec![
                ("filename".to_string(), Value::String(filename.into())),
                (
                    "content_type".to_string(),
                    Value::String(content_type.into()),
                ),
                ("size".to_string(), Value::Int(data.len() as i64)),
                ("bytes".to_string(), bytes_list),
            ],
        );
        return Ok(Value::Struct(upload));
    }
}

/// Parse a URL query string (`a=1&b=x`) into the declared query struct
/// value, coercing each field from its string form (slice 52a).
fn query_string_to_value(
    query: &str,
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Value, String> {
    let Type::Struct(def_id) = ty else {
        return Err("query type must be a struct".to_string());
    };
    let ir_type = types_by_id
        .get(def_id)
        .copied()
        .ok_or_else(|| "unknown query struct type".to_string())?;
    let pairs: HashMap<String, String> = query
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((
                urldecode(k),
                urldecode(v),
            ))
        })
        .collect();
    let mut fields: Vec<(String, Value)> = Vec::new();
    for field in &ir_type.fields {
        let raw = pairs
            .get(&field.name)
            .cloned()
            .ok_or_else(|| format!("missing query param `{}`", field.name))?;
        fields.push((field.name.clone(), coerce_str_to_value(&raw, &field.ty)?));
    }
    Ok(Value::Struct(corvid_driver::StructValue::new(
        ir_type.id,
        ir_type.name.clone(),
        fields,
    )))
}

/// Minimal percent-decoding for query keys/values (slice 52a).
fn urldecode(s: &str) -> String {
    let bytes = s.replace('+', " ");
    let mut out = String::with_capacity(bytes.len());
    let mut chars = bytes.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(hi), Some(lo)) = (hi, lo) {
                if let Ok(byte) = u8::from_str_radix(&format!("{hi}{lo}"), 16) {
                    out.push(byte as char);
                    continue;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// If the dispatch outcome surfaced an `ApprovalQueued` from the
/// runtime, stash the agent name + args under that approval id so
/// `/__approvals/:id/approve` can re-execute the original request.
/// No-op for any other outcome (200 success, 403 deny, 500 error).
fn capture_pending_invocation_if_queued(
    state: &Arc<ServeState>,
    agent: &str,
    args: &[Value],
    outcome: &Result<Value, RunError>,
) {
    let approval_id = match outcome {
        Err(RunError::Interp(e)) => match approval_queued_id(&e.kind) {
            Some(id) => id.to_string(),
            None => return,
        },
        _ => return,
    };
    state.pending_invocations.lock().unwrap().insert(
        approval_id,
        PendingInvocation {
            agent: agent.to_string(),
            args: args.to_vec(),
            last_handler_error: None,
        },
    );
}

/// Map a handler outcome to an HTTP response: 200 + JSON on success,
/// 202 + `{"approval_id":"..."}` when an `approve` boundary queued
/// (slice `E0-serve-5`), 403 when an `approve` boundary denied (kept
/// for the case where a non-queue approver is wired in — defensive),
/// 500 otherwise.
fn finish(outcome: Result<Value, RunError>) -> Response {
    match outcome {
        // A Stream-returning handler responds as Server-Sent Events:
        // each chunk flushes to the client the moment it arrives
        // (`data: <json>` per chunk, `event: done` terminator) — the
        // modern AI-app transport, straight from the language's
        // Stream type with zero glue.
        Ok(Value::Stream(stream)) => {
            let sse = async_stream::stream! {
                loop {
                    match stream.next().await {
                        Some(Ok(chunk)) => {
                            let payload = value_to_json(&chunk);
                            yield Ok::<_, std::convert::Infallible>(
                                axum::response::sse::Event::default()
                                    .data(payload.to_string()),
                            );
                        }
                        Some(Err(err)) => {
                            yield Ok(axum::response::sse::Event::default()
                                .event("error")
                                .data(format!("{err:?}")));
                            break;
                        }
                        None => {
                            yield Ok(axum::response::sse::Event::default()
                                .event("done")
                                .data(""));
                            break;
                        }
                    }
                }
            };
            axum::response::Sse::new(sse).into_response()
        }
        Ok(value) => (StatusCode::OK, Json(value_to_json(&value))).into_response(),
        Err(RunError::Interp(e)) => {
            if let Some(approval_id) = approval_queued_id(&e.kind) {
                let approval_id = approval_id.to_string();
                return (
                    StatusCode::ACCEPTED,
                    [(
                        axum::http::header::LOCATION,
                        format!("/__approvals/{approval_id}"),
                    )],
                    Json(serde_json::json!({
                        "approval_id": approval_id,
                        "status": "pending",
                        "poll": format!("/__approvals/{approval_id}"),
                        "detail": "this write is approval-gated; a pending approval has been queued. Poll the `poll` URL for the decision.",
                    })),
                )
                    .into_response();
            }
            if is_approval_denied(&e.kind) {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "approval_required",
                        "detail": "this write is approval-gated and the approver denied it.",
                    })),
                )
                    .into_response();
            }
            // 33Q10: emit user-facing detail (no IR byte-span prefix)
            // so clients don't see `[1227..1269] ...` they can't act on.
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "handler_failed",
                    "detail": RunError::Interp(e).user_facing_detail(),
                })),
            )
                .into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "handler_failed",
                "detail": err.user_facing_detail(),
            })),
        )
            .into_response(),
    }
}

/// If the interpreter error carries an `ApprovalQueued` from the
/// runtime, return the approval id so `finish` can answer 202.
fn approval_queued_id(kind: &InterpErrorKind) -> Option<&str> {
    match kind {
        InterpErrorKind::Runtime(RuntimeError::ApprovalQueued { approval_id }) => {
            Some(approval_id.as_str())
        }
        _ => None,
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

/// `GET /__approvals` — list pending approvals for the `corvid serve`
/// default tenant. Read-only at the slice MVP boundary.
async fn list_approvals(State(state): State<Arc<ServeState>>) -> Response {
    match state.approval_queue.list_by_tenant(SERVE_DEFAULT_TENANT) {
        Ok(records) => {
            let entries: Vec<serde_json::Value> = records
                .iter()
                .filter(|r| r.status == "pending")
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "action": r.action,
                        "status": r.status,
                        "tenant_id": r.tenant_id,
                        "requester_actor_id": r.requester_actor_id,
                        "created_ms": r.created_ms,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "approvals": entries })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "approval_queue_list_failed",
                "detail": e.to_string(),
            })),
        )
            .into_response(),
    }
}

/// `POST /__approvals/:id/approve` — slice `serve-6`. Reviewer marks the
/// approval granted; the server transitions the queue record, looks
/// up the pending invocation captured at queue time, re-runs the
/// agent under an always-yes approver (the approval is already
/// granted at this layer, so the inner `approve` boundary must pass
/// without re-queuing), and returns the agent's result. Errors:
/// 404 if the approval doesn't exist, 409 if already decided or no
/// pending invocation linked, 500 if the queue transition or the
/// re-execution itself failed.
async fn approve_approval(
    State(state): State<Arc<ServeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    // Pre-check the record exists and is still pending so we can
    // distinguish "unknown id" (404), "already decided" (409), and
    // queue-runtime IO errors (500) — `queue.approve()` collapses
    // these into a single Err.
    match state.approval_queue.get(&id) {
        Ok(Some(record)) if record.status == "pending" => {}
        Ok(Some(record)) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "approval_already_decided",
                    "id": id,
                    "status": record.status,
                })),
            )
                .into_response();
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "approval_not_found", "id": id })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "approval_queue_get_failed",
                    "detail": e.to_string(),
                })),
            )
                .into_response();
        }
    }

    // 33Q2 — peek the pending invocation BEFORE transitioning the
    // queue. If the handler errors, the approval stays at `pending`
    // so the reviewer can retry without re-granting; the invocation
    // stays in `pending_invocations` for the retry path. Pre-33Q2,
    // `queue.approve()` ran first and then the invocation was
    // pop'd unconditionally — a handler 500 left the approval
    // permanently `approved` AND the invocation gone, so neither
    // /approve (409 already-decided) nor a re-POST of the original
    // request (would create a NEW approval — silently double-
    // billing the reviewer's authorization) could recover. That's
    // the regression anonymous-2026-06-04 P1.2 documented.
    //
    // The invocation is CLONED, not removed; removal only happens
    // after the queue transition succeeds (which itself only runs
    // after the handler succeeds).
    let invocation = match state.pending_invocations.lock().unwrap().get(&id).cloned() {
        Some(inv) => inv,
        None => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "no_pending_invocation",
                    "id": id,
                    "detail": "approval is pending but no pending invocation is linked in this serve process — re-execution is not possible. This happens when the approval id is not one this server queued.",
                })),
            )
                .into_response();
        }
    };

    // Re-run the agent under a fresh runtime whose approver is
    // `ProgrammaticApprover::always_yes()`. The granted approval
    // lives at the HTTP/queue layer; the inner `approve` boundary
    // must pass at this layer or the agent would re-queue forever.
    // The IR / approval queue / pending-invocation map are all
    // shared, so the re-execution can still hit other approval
    // boundaries — those would re-queue normally. (Slice MVP
    // assumes a single approve boundary per route — multi-step
    // approval chains are a follow-up.)
    //
    // Tool registry is cloned from the host's startup config
    // (`ServeState::host_tools`) so the re-executed agent sees the
    // same `--with-tools-cdylib` / `tools.py` handlers as the
    // original request. Pre-33Q1a this was an empty default
    // registry, which is the bug anonymous-2026-06-04 P1.1 hit.
    let bypass_runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool_registry(state.host_tools.clone())
        .build();
    let outcome = run_ir_with_runtime(
        &state.ir,
        Some(&invocation.agent),
        invocation.args,
        &bypass_runtime,
    )
    .await;

    match outcome {
        Ok(value) => {
            // Handler succeeded — NOW transition the queue and pop
            // the pending invocation. The 200 response carries the
            // re-executed agent's result.
            let actor = serve_reviewer_actor();
            if let Err(e) = state.approval_queue.approve(
                &id,
                SERVE_DEFAULT_TENANT,
                &actor,
                Some("approved via /__approvals/:id/approve"),
            ) {
                // Queue transition failed AFTER successful handler
                // execution — the side effect already happened, so
                // we surface the queue error but the action is done.
                // Leave the pending invocation in place so the
                // operator can inspect/retry the queue transition if
                // needed. (This is a very rare edge case — SQLite
                // write failure mid-handler.)
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "approval_transition_failed",
                        "detail": e.to_string(),
                        "handler_outcome": "succeeded",
                    })),
                )
                    .into_response();
            }
            state.pending_invocations.lock().unwrap().remove(&id);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "approved",
                    "result": value_to_json(&value),
                })),
            )
                .into_response()
        }
        Err(err) => {
            // Handler errored — KEEP the approval pending, keep the
            // pending invocation, capture the error for diagnostic
            // surfacing via GET. The reviewer can /approve again to
            // retry (transient failure) or /deny to terminate
            // (permanent failure). Adversarial: a permanently-broken
            // handler creates a replayable approval but the reviewer
            // can /deny to exit; nothing here bypasses the original
            // approve boundary because retrying /approve still runs
            // the same `ProgrammaticApprover::always_yes` bypass
            // runtime that's local to this handler call.
            //
            // 33Q10: user_facing_detail strips the IR `[start..end]`
            // byte-span prefix from interpreter errors so 500 bodies
            // (and the `last_handler_error` field that surfaces in
            // GET) don't leak internal compiler artifacts to clients.
            let detail = err.user_facing_detail();
            {
                let mut pending = state.pending_invocations.lock().unwrap();
                if let Some(stored) = pending.get_mut(&id) {
                    stored.last_handler_error = Some(detail.clone());
                }
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "approved_execution_failed",
                    "detail": detail,
                    "approval_status": "pending",
                    "retry": {
                        "possible": true,
                        "url": format!("/__approvals/{id}/approve"),
                        "note": "approval was not consumed; POST again to retry, or POST /__approvals/<id>/deny to terminate the pending invocation if the handler is permanently broken",
                    },
                })),
            )
                .into_response()
        }
    }
}

/// `POST /__approvals/:id/deny` — slice `serve-6`. Reviewer marks the
/// approval denied; the server transitions the queue record and
/// drops the pending invocation. No re-execution happens. 404 if
/// the approval doesn't exist, 409 if already decided.
async fn deny_approval(
    State(state): State<Arc<ServeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    match state.approval_queue.get(&id) {
        Ok(Some(record)) if record.status == "pending" => {}
        Ok(Some(record)) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "approval_already_decided",
                    "id": id,
                    "status": record.status,
                })),
            )
                .into_response();
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "approval_not_found", "id": id })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "approval_queue_get_failed",
                    "detail": e.to_string(),
                })),
            )
                .into_response();
        }
    }

    let actor = serve_reviewer_actor();
    if let Err(e) = state.approval_queue.deny(
        &id,
        SERVE_DEFAULT_TENANT,
        &actor,
        Some("denied via /__approvals/:id/deny"),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "approval_transition_failed",
                "detail": e.to_string(),
            })),
        )
            .into_response();
    }

    // Drop the pending invocation; it will never re-execute.
    state.pending_invocations.lock().unwrap().remove(&id);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "denied",
            "id": id,
        })),
    )
        .into_response()
}

/// Reviewer actor context the `/approve` and `/deny` transitions run
/// under. Per-request reviewer auth is a slice follow-up; today
/// every reviewer is the same anonymous actor distinct from the
/// requester (the queue's `authorize_approval_transition` rejects
/// self-approval, so requester and reviewer ids must differ — they
/// do: `serve-anonymous` vs `serve-reviewer`).
fn serve_reviewer_actor() -> ApprovalActorContext {
    ApprovalActorContext {
        actor_id: SERVE_REVIEWER_ACTOR.to_string(),
        tenant_id: SERVE_DEFAULT_TENANT.to_string(),
        role: SERVE_REVIEWER_ROLE.to_string(),
    }
}

/// `GET /__approvals/<id>` — fetch one queued approval.
async fn get_approval(
    State(state): State<Arc<ServeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    match state.approval_queue.get(&id) {
        Ok(Some(record)) => {
            // 33Q2 — surface the captured `last_handler_error` (if any)
            // from `PendingInvocation` so a reviewer probing why an
            // approval is still `pending` after they POSTed /approve
            // can see "the handler errored with <message>; you may
            // retry or deny" instead of guessing.
            let last_handler_error = state
                .pending_invocations
                .lock()
                .unwrap()
                .get(&id)
                .and_then(|inv| inv.last_handler_error.clone());
            let mut body = serde_json::json!({
                "id": record.id,
                "action": record.action,
                "status": record.status,
                "tenant_id": record.tenant_id,
                "requester_actor_id": record.requester_actor_id,
                "created_ms": record.created_ms,
                "updated_ms": record.updated_ms,
            });
            if let Some(err) = last_handler_error {
                body["last_handler_error"] = serde_json::json!(err);
                body["retry_possible"] = serde_json::json!(true);
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "approval_not_found", "id": id })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "approval_queue_get_failed",
                "detail": e.to_string(),
            })),
        )
            .into_response(),
    }
}

fn bad_request(error: &str, detail: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": error, "detail": detail })),
    )
        .into_response()
}

/// Serve the Application Contract JSON (slice 51r).
async fn serve_contract(State(state): State<Arc<ServeState>>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        state.contract_json.clone(),
    )
        .into_response()
}

/// Serve the OpenAPI 3.1 JSON (slice 51r).
async fn serve_openapi(State(state): State<Arc<ServeState>>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        state.openapi_json.clone(),
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

/// Load the host's tools cdylib and register one interpreter
/// `ToolHandler` per declared tool that bridges through the runtime's
/// C-ABI tool registry — the unblocker for slice 33Q1a's
/// "Surface 3 (approval-gated dangerous tool) is undemonstrable over
/// HTTP" gap surfaced by the anonymous-2026-06-04 round-2 trial.
///
/// **Two layers of registration** because the runtime keeps two distinct
/// tool registries:
///
/// 1. **C-ABI registry** (`crates/corvid-runtime/src/catalog_c_api/tool_bridge.rs`).
///    A global static keyed by name → fn pointer. Populated via
///    `corvid_register_tool`. This is the registry every cdylib build
///    targets when it self-registers, and what `corvid_invoke_tool`
///    (the native-codegen tool-dispatch path) reads.
/// 2. **Rust `ToolRegistry`** (`crates/corvid-runtime/src/tools.rs`).
///    A `HashMap<String, ToolHandler>` on each `Runtime` instance, used
///    by the *interpreter* tier (which `corvid serve` runs). We populate
///    each app-declared tool with a Rust handler that internally calls
///    `dispatch_host_tool` — the public Rust-callable shim over the
///    C registry's dispatch core.
///
/// **Cdylib lifetime.** The loaded `Library` handle is leaked
/// (`Box::leak`) so the cdylib stays mapped for the rest of the
/// process. Unloading mid-serve would invalidate the fn pointers we
/// just registered into the C registry, and the next tool invocation
/// would crash inside `corvid_invoke_tool` with no Rust-side recovery
/// path. The leak is one-time at startup and bounded by the cdylib's
/// own footprint; the trade-off favours operator safety over the
/// memory we don't reclaim on shutdown (which `corvid serve` doesn't
/// gracefully execute today anyway — Ctrl-C tears the process down
/// before any handle cleanup would run).
///
/// **Missing symbols are not fatal.** An app may declare tools that
/// the cdylib doesn't implement (e.g. some are host-supplied
/// elsewhere, or the cdylib is a subset). Missing tools log a startup
/// warning and stay unregistered; the interpreter's existing
/// `UnknownTool` error surfaces the gap at call time, which is the
/// honest behaviour. If the operator wants startup-fail-fast
/// semantics, the right future surface is `--require-tools-cdylib`
/// (filed under 33Q1's slice notes; not in this commit).
fn register_cdylib_tool_handlers(ir: &IrFile, cdylib_path: &Path) -> Result<ToolRegistry> {
    if !cdylib_path.exists() {
        anyhow::bail!(
            "--with-tools-cdylib `{}` does not exist — build your tools crate first \
             (`cargo build -p <tools-crate> --release` with `crate-type = [\"cdylib\"]` \
             in its Cargo.toml)",
            cdylib_path.display()
        );
    }

    // SAFETY: dlopen / LoadLibrary on a path supplied by the operator.
    // `libloading::Library::new` is documented unsafe because loading a
    // shared library executes its static constructors in the host
    // process. The operator running `corvid serve --with-tools-cdylib`
    // explicitly chose to trust that file; that is the trust boundary.
    let library = unsafe { Library::new(cdylib_path) }
        .with_context(|| format!("dlopen `{}`", cdylib_path.display()))?;

    let mut registry = ToolRegistry::default();
    let mut registered_tools: Vec<String> = Vec::new();
    let mut missing_tools: Vec<String> = Vec::new();

    for tool in &ir.tools {
        let symbol_name = format!("__corvid_tool_{}", tool.name);
        // SAFETY: dlsym for a symbol name string. The signature
        // `CorvidToolFn` is the ABI the `#[tool]` proc-macro emits — see
        // `crates/corvid-runtime/src/catalog_c_api/tool_bridge.rs::CorvidToolFn`.
        let fn_ptr: Result<Symbol<CorvidToolFn>, libloading::Error> =
            unsafe { library.get(symbol_name.as_bytes()) };

        let Ok(symbol) = fn_ptr else {
            missing_tools.push(tool.name.clone());
            continue;
        };

        // Copy the raw fn pointer OUT of the `Symbol<'_>` lifetime
        // wrapper. The `Symbol` borrow's lifetime is tied to the
        // `Library`, but we're about to leak the library anyway, so
        // promoting to a raw `CorvidToolFn` is the cleanest fit for
        // the C-ABI `corvid_register_tool` signature.
        let raw_fn: CorvidToolFn = *symbol;

        // Register in the C-ABI registry. `corvid_register_tool` is the
        // CLI's statically-linked corvid-runtime entry; calling it
        // updates the CLI process's global tool registry that
        // `dispatch_host_tool` reads. The cdylib's own copy of the
        // registry (it also statically links corvid-runtime) is a
        // separate static and stays empty here — that's expected.
        let name_c = CString::new(tool.name.clone()).with_context(|| {
            format!(
                "tool name `{}` contains an interior NUL byte (cannot pass to C ABI)",
                tool.name
            )
        })?;
        unsafe {
            corvid_register_tool(name_c.as_ptr(), Some(raw_fn), std::ptr::null_mut());
        }

        // Register a Rust handler in the returned `ToolRegistry` that
        // bridges through `dispatch_host_tool` back into the C
        // registry we just populated. The interpreter calls
        // `runtime.tools.call(name, args)` for every tool invocation;
        // without this Rust handler the interpreter would still
        // return `UnknownTool` even though the C registry has the
        // fn pointer. Returning a `ToolRegistry` rather than mutating
        // a `RuntimeBuilder` lets `cmd_serve` clone the same
        // registry into both the main runtime and the `/approve`
        // bypass runtime so the re-executed agent sees the same
        // handler set as the original request.
        let tool_name_owned = tool.name.clone();
        registry.register(tool_name_owned.clone(), move |args| {
            let tool_name = tool_name_owned.clone();
            async move {
                let args_json = serde_json::to_string(&args).map_err(|e| {
                    RuntimeError::ToolFailed {
                        tool: tool_name.clone(),
                        message: format!("serialize args to JSON: {e}"),
                    }
                })?;
                match dispatch_host_tool(&tool_name, &args_json) {
                    Some(result_json) => {
                        serde_json::from_str(&result_json).map_err(|e| {
                            RuntimeError::ToolFailed {
                                tool: tool_name.clone(),
                                message: format!("parse host tool result JSON: {e}"),
                            }
                        })
                    }
                    None => Err(RuntimeError::UnknownTool(tool_name)),
                }
            }
        });

        registered_tools.push(tool.name.clone());
    }

    // Leak the library so it stays mapped for the process lifetime.
    // See doc comment above for rationale.
    Box::leak(Box::new(library));

    eprintln!(
        "corvid serve: linked {}/{} tool(s) from `{}`",
        registered_tools.len(),
        ir.tools.len(),
        cdylib_path.display()
    );
    if !registered_tools.is_empty() {
        registered_tools.sort();
        eprintln!("  registered: {}", registered_tools.join(", "));
    }
    if !missing_tools.is_empty() {
        missing_tools.sort();
        eprintln!(
            "  declared in app but missing from cdylib: {} (will return UnknownTool at call time)",
            missing_tools.join(", ")
        );
    }

    Ok(registry)
}

/// 33Q9 helper — return true when executing `agent_name` can reach an
/// `IrStmt::Approve`, following agent calls transitively. Used by the
/// serve startup banner to label routes as approval-gated vs not.
///
/// Slice 52a made every route dispatch through a synthetic handler
/// agent whose body is `return <handler>(...)`, so the `approve`
/// boundary usually lives one call deeper than the handler agent
/// itself — the detection must follow agent calls or it under-reports
/// every real route. A `visited` set bounds recursion through cycles.
/// Exotic expression forms (stream combinators, replay) fall through
/// to `false`: a conservative under-count with no false positives,
/// matching the label's original contract.
fn agent_body_contains_approve(ir: &IrFile, agent_name: &str) -> bool {
    let mut visited = std::collections::HashSet::new();
    agent_reaches_approve(ir, agent_name, &mut visited)
}

fn agent_reaches_approve(
    ir: &IrFile,
    agent_name: &str,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    if !visited.insert(agent_name.to_string()) {
        return false;
    }
    match ir.agents.iter().find(|a| a.name == agent_name) {
        Some(agent) => block_reaches_approve(ir, &agent.body, visited),
        None => false,
    }
}

fn block_reaches_approve(
    ir: &IrFile,
    block: &corvid_ir::IrBlock,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    block
        .stmts
        .iter()
        .any(|s| stmt_reaches_approve(ir, s, visited))
}

fn stmt_reaches_approve(
    ir: &IrFile,
    stmt: &IrStmt,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    match stmt {
        IrStmt::Approve { .. } => true,
        IrStmt::Let { value, .. }
        | IrStmt::Yield { value, .. }
        | IrStmt::Destructure { value, .. }
        | IrStmt::Assign { value, .. } => expr_reaches_approve(ir, value, visited),
        IrStmt::Expr { expr, .. } => expr_reaches_approve(ir, expr, visited),
        IrStmt::Return { value, .. } => {
            value.as_ref().is_some_and(|e| expr_reaches_approve(ir, e, visited))
        }
        IrStmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            expr_reaches_approve(ir, cond, visited)
                || block_reaches_approve(ir, then_block, visited)
                || else_block
                    .as_ref()
                    .is_some_and(|b| block_reaches_approve(ir, b, visited))
        }
        IrStmt::For { iter, body, .. } => {
            expr_reaches_approve(ir, iter, visited) || block_reaches_approve(ir, body, visited)
        }
        IrStmt::While { cond, body, .. } => {
            expr_reaches_approve(ir, cond, visited) || block_reaches_approve(ir, body, visited)
        }
        IrStmt::Parallel { arms, .. } => arms
            .iter()
            .any(|arm| expr_reaches_approve(ir, &arm.call, visited)),
        IrStmt::Break { .. }
        | IrStmt::Continue { .. }
        | IrStmt::Pass { .. }
        | IrStmt::Dup { .. }
        | IrStmt::Drop { .. } => false,
    }
}

fn expr_reaches_approve(
    ir: &IrFile,
    expr: &corvid_ir::IrExpr,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    use corvid_ir::{IrCallKind, IrExprKind};
    match &expr.kind {
        IrExprKind::Call {
            kind: IrCallKind::Agent { .. },
            callee_name,
            args,
        } => {
            agent_reaches_approve(ir, callee_name, visited)
                || args.iter().any(|a| expr_reaches_approve(ir, a, visited))
        }
        IrExprKind::Call { args, .. } => {
            args.iter().any(|a| expr_reaches_approve(ir, a, visited))
        }
        IrExprKind::BuiltinMethod { receiver, args, .. } => {
            expr_reaches_approve(ir, receiver, visited)
                || args.iter().any(|a| expr_reaches_approve(ir, a, visited))
        }
        IrExprKind::FieldAccess { target, .. } => expr_reaches_approve(ir, target, visited),
        IrExprKind::Index { target, index } => {
            expr_reaches_approve(ir, target, visited) || expr_reaches_approve(ir, index, visited)
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            expr_reaches_approve(ir, left, visited) || expr_reaches_approve(ir, right, visited)
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            expr_reaches_approve(ir, operand, visited)
        }
        IrExprKind::Match { scrutinee, arms } => {
            expr_reaches_approve(ir, scrutinee, visited)
                || arms.iter().any(|arm| {
                    arm.guard
                        .as_ref()
                        .is_some_and(|g| expr_reaches_approve(ir, g, visited))
                        || expr_reaches_approve(ir, &arm.body, visited)
                })
        }
        IrExprKind::StructLiteral { fields, spread, .. } => {
            fields
                .iter()
                .any(|(_, e)| expr_reaches_approve(ir, e, visited))
                || spread
                    .as_ref()
                    .is_some_and(|s| expr_reaches_approve(ir, s, visited))
        }
        IrExprKind::Lambda { body, .. } => expr_reaches_approve(ir, body, visited),
        IrExprKind::MapLiteral { keys, values } => {
            keys.iter().chain(values.iter()).any(|e| expr_reaches_approve(ir, e, visited))
        }
        IrExprKind::List { items } => {
            items.iter().any(|e| expr_reaches_approve(ir, e, visited))
        }
        IrExprKind::UnwrapGrounded { value } => expr_reaches_approve(ir, value, visited),
        IrExprKind::ResultOk { inner }
        | IrExprKind::ResultErr { inner }
        | IrExprKind::OptionSome { inner }
        | IrExprKind::TryPropagate { inner }
        | IrExprKind::TrustBoundary { inner } => expr_reaches_approve(ir, inner, visited),
        IrExprKind::Ask { prompt, .. } => expr_reaches_approve(ir, prompt, visited),
        IrExprKind::Choose { options } => expr_reaches_approve(ir, options, visited),
        IrExprKind::TryRetry { body, .. } => expr_reaches_approve(ir, body, visited),
        // Literals, locals, decls, option-none, weak/stream combinators,
        // and replay fall through: no agent call reachable through them
        // in a route handler (conservative under-count, no false positive).
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::Span;
    use corvid_ir::IrField;

    #[test]
    fn axum_path_translates_single_brace_param() {
        assert_eq!(corvid_route_to_axum_path("/orders/{id}"), "/orders/:id");
    }

    #[test]
    fn axum_path_translates_multiple_params() {
        assert_eq!(
            corvid_route_to_axum_path("/tenants/{tenant}/orders/{id}"),
            "/tenants/:tenant/orders/:id"
        );
    }

    #[test]
    fn axum_path_leaves_static_paths_untouched() {
        assert_eq!(corvid_route_to_axum_path("/health"), "/health");
    }

    #[test]
    fn coerce_parses_scalar_types() {
        assert!(matches!(coerce_str_to_value("42", &Type::Int), Ok(Value::Int(42))));
        assert!(matches!(
            coerce_str_to_value("3.5", &Type::Float),
            Ok(Value::Float(f)) if (f - 3.5).abs() < 1e-9
        ));
        assert!(matches!(
            coerce_str_to_value("true", &Type::Bool),
            Ok(Value::Bool(true))
        ));
        assert!(matches!(
            coerce_str_to_value("hello", &Type::String),
            Ok(Value::String(s)) if &*s == "hello"
        ));
    }

    #[test]
    fn coerce_rejects_malformed_int() {
        assert!(coerce_str_to_value("notanint", &Type::Int).is_err());
    }

    #[test]
    fn urldecode_handles_percent_and_plus() {
        assert_eq!(urldecode("a%20b+c"), "a b c");
        assert_eq!(urldecode("plain"), "plain");
    }

    #[test]
    fn query_string_coerces_struct_fields() {
        let ty = IrType {
            id: DefId(9),
            name: "Filter".into(),
            fields: vec![
                IrField {
                    name: "limit".into(),
                    ty: Type::Int,
                    refinement: None,
                    span: Span::new(0, 0),
                },
                IrField {
                    name: "q".into(),
                    ty: Type::String,
                    refinement: None,
                    span: Span::new(0, 0),
                },
            ],
            variants: vec![],
            span: Span::new(0, 0),
        };
        let mut types_by_id: HashMap<DefId, &IrType> = HashMap::new();
        types_by_id.insert(DefId(9), &ty);
        let value =
            query_string_to_value("limit=10&q=widgets", &Type::Struct(DefId(9)), &types_by_id)
                .expect("coerces");
        let Value::Struct(s) = value else {
            panic!("expected struct")
        };
        assert!(matches!(s.get_field("limit"), Some(Value::Int(10))));
        assert!(matches!(s.get_field("q"), Some(Value::String(v)) if &*v == "widgets"));
    }

    #[test]
    fn query_string_reports_missing_field() {
        let ty = IrType {
            id: DefId(9),
            name: "Filter".into(),
            fields: vec![IrField {
                name: "limit".into(),
                ty: Type::Int,
                refinement: None,
                span: Span::new(0, 0),
            }],
            variants: vec![],
            span: Span::new(0, 0),
        };
        let mut types_by_id: HashMap<DefId, &IrType> = HashMap::new();
        types_by_id.insert(DefId(9), &ty);
        assert!(query_string_to_value("", &Type::Struct(DefId(9)), &types_by_id).is_err());
    }
}
