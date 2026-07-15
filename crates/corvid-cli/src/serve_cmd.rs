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
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, on, MethodFilter};
use axum::{Json, Router};
use corvid_ast::HttpMethod;
use corvid_driver::{
    compile_to_ir_with_config_at_path, load_corvid_config_for, render_all_pretty,
    run_ir_with_runtime, InterpErrorKind, RunError, Runtime, RuntimeError, ToolRegistry, Value,
};
use corvid_ir::{IrCallKind, IrExprKind, IrFile, IrLiteral, IrRoute, IrStmt, IrType};
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
    let state = Arc::new(ServeState {
        ir,
        runtime,
        approval_queue,
        pending_invocations: Arc::new(Mutex::new(HashMap::new())),
        host_tools,
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
        );

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
        for plan in &plans {
            // Slice 33Q9: the label "approval-gated -> 202 + queued"
            // is only accurate when the handler agent's body actually
            // contains an `approve` boundary that the dispatch path
            // can reach. Pre-33Q9 every `Dispatch::Body` route was
            // unconditionally labeled approval-gated — misleading for
            // routes whose agent has no syntactic `approve` (the
            // route returns 200/500 directly, never 202). Filed by
            // maintainer-as-reviewer-2026-06-05 P2.1.
            //
            // The check is a recursive IR walk on the handler agent's
            // body, looking for any `IrStmt::Approve` reachable
            // through nested `If` / `For` blocks. It's NOT a call-
            // graph walk: an agent whose body only calls another
            // agent that approves still gets the no-approve label.
            // That's a conservative under-count — false negative
            // possible but no false positive. The opposite direction
            // is the one the trial reviewer hit.
            let agent_name = match &plan.dispatch {
                Dispatch::Literal { agent, .. } | Dispatch::Body { agent, .. } => agent.as_str(),
            };
            let approves = agent_body_contains_approve(&state_for_banner.ir, agent_name);
            let body_suffix = match &plan.dispatch {
                Dispatch::Body { .. } => "body",
                Dispatch::Literal { .. } => "literal",
            };
            let kind = if approves {
                format!("-> {agent_name} ({body_suffix}; approval-gated -> 202 + queued)")
            } else {
                format!("-> {agent_name} ({body_suffix})")
            };
            println!("  {:<6} {}  {kind}", plan.method.as_str(), plan.path);
        }
        println!("  GET    /__approvals                -> list pending approvals");
        println!("  GET    /__approvals/<id>           -> fetch one pending approval");
        println!("  POST   /__approvals/<id>/approve   -> approve + re-execute the original request");
        println!("  POST   /__approvals/<id>/deny      -> deny + drop the pending invocation");
        for (method, path) in &not_served {
            println!("  {method:<6} {path}  -> 501 (not served)");
        }
        axum::serve(listener, app).await.context("serve").map(|_| ())
    })?;
    Ok(0)
}

/// Run a literal-arg handler agent and serialize its result to JSON.
async fn dispatch_literal(state: Arc<ServeState>, agent: String, args: Vec<Value>) -> Response {
    let outcome = run_ir_with_runtime(&state.ir, Some(&agent), args.clone(), &state.runtime).await;
    capture_pending_invocation_if_queued(&state, &agent, &args, &outcome);
    finish(outcome)
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
    let args = vec![body_val];
    let outcome = run_ir_with_runtime(&state.ir, Some(&agent), args.clone(), &state.runtime).await;
    capture_pending_invocation_if_queued(&state, &agent, &args, &outcome);
    finish(outcome)
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

/// 33Q9 helper — return true when `agent_name`'s body contains any
/// reachable `IrStmt::Approve`. Walks nested `If` / `For` blocks
/// recursively; does NOT follow calls into other agents (a
/// conservative under-count). Used by the serve startup banner to
/// accurately label routes as approval-gated vs not — pre-33Q9 every
/// body-dispatch route was labeled approval-gated regardless of
/// what the agent actually did.
fn agent_body_contains_approve(ir: &IrFile, agent_name: &str) -> bool {
    let Some(agent) = ir.agents.iter().find(|a| a.name == agent_name) else {
        return false;
    };
    block_contains_approve(&agent.body)
}

fn block_contains_approve(block: &corvid_ir::IrBlock) -> bool {
    block.stmts.iter().any(stmt_contains_approve)
}

fn stmt_contains_approve(stmt: &IrStmt) -> bool {
    match stmt {
        IrStmt::Approve { .. } => true,
        IrStmt::If {
            then_block,
            else_block,
            ..
        } => {
            block_contains_approve(then_block)
                || else_block
                    .as_ref()
                    .is_some_and(block_contains_approve)
        }
        IrStmt::For { body, .. } => block_contains_approve(body),
        _ => false,
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
