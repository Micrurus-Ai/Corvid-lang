//! The assistant endpoint (B10-3b): `POST /mcp`, the Model Context
//! Protocol over HTTP, for actions with an `assistants:` line.
//!
//! An assistant acts only inside the person's own signed-in session: the
//! endpoint authenticates as an action call does (session, CSRF for a
//! cookie session), so it can never do what the person could not.
//! `tools/list` returns the offered actions the caller's `requires`
//! allows; any other name is an unknown tool, so the endpoint never
//! confirms an action exists. `tools/call` validates the input through
//! the action's types and records a pending confirmation for that same
//! person; the action runs only when they `POST
//! /__confirmations/<id>/confirm` from the page. A confirmation is the
//! person agreeing to what their own assistant asked, not the approval
//! queue's separate reviewer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{on, MethodFilter, MethodRouter};
use axum::Json;
use corvid_abi::app_contract::ApplicationContract;
use corvid_ast::HttpMethod;
use corvid_driver::Value;
use corvid_ir::{IrRoutePolicy, IrType};
use corvid_resolve::DefId;
use serde_json::{json, Value as Json_};

use super::actions::{decode_json_input, execute, ActionEndpoint};
use super::{auth_error, enforce_route_policy, epoch_ms, EnforcedRouteActor, ServeState};

/// Where the assistant endpoint is served.
pub(super) const MCP_PATH: &str = "/mcp";
/// The MCP version this endpoint speaks when the client names none.
const PROTOCOL_VERSION: &str = "2025-06-18";
/// How long a pending confirmation waits for the person. Fixed: a short
/// window only removes risk, and a longer one is a decision for
/// on-behalf-of authority (B11).
const CONFIRMATION_WINDOW_MS: u64 = 15 * 60 * 1000;

/// One assistant call waiting for its person.
pub(super) struct PendingConfirmation {
    action: String,
    actor_id: String,
    tenant_id: String,
    inputs: Vec<Value>,
    expires_at_ms: u64,
}

pub(super) type Confirmations = Arc<Mutex<HashMap<String, PendingConfirmation>>>;

/// An action offered to assistants, with what `tools/list` shows for it.
struct Offered {
    endpoint: Arc<ActionEndpoint>,
    description: String,
    input_schema: Json_,
}

/// The endpoint and the confirmation routes, or none when no action has
/// an `assistants:` line.
pub(super) fn routes(
    catalog: &[Arc<ActionEndpoint>],
    contract: &ApplicationContract,
) -> Vec<(String, MethodRouter<Arc<ServeState>>)> {
    let components = json!({ "schemas": corvid_abi::openapi::component_schemas(&contract.types) });
    let offered: Vec<Offered> = catalog
        .iter()
        .filter_map(|endpoint| {
            let line = endpoint.assistants.as_ref()?;
            let action = contract.actions.iter().find(|a| a.callable.name == endpoint.name)?;
            // The schema's `$ref`s resolve against the components it carries.
            let mut input_schema = corvid_abi::openapi::action_input_schema(action);
            input_schema["components"] = components.clone();
            Some(Offered { endpoint: endpoint.clone(), description: line.description.clone(), input_schema })
        })
        .collect();
    if offered.is_empty() {
        return Vec::new();
    }
    let offered = Arc::new(offered);
    let mcp = {
        let offered = offered.clone();
        on(MethodFilter::POST, move |State(state): State<Arc<ServeState>>, headers: axum::http::HeaderMap, body: Bytes| {
            let offered = offered.clone();
            async move { handle(&state, &offered, &headers, &body).await }
        })
    };
    let decide = |confirm: bool| {
        let offered = offered.clone();
        on(MethodFilter::POST, move |State(state): State<Arc<ServeState>>, Path(id): Path<String>, headers: axum::http::HeaderMap| {
            let offered = offered.clone();
            async move { decide(&state, &offered, &id, &headers, confirm).await }
        })
    };
    vec![
        (MCP_PATH.to_string(), mcp),
        ("/__confirmations/:id/confirm".to_string(), decide(true)),
        ("/__confirmations/:id/decline".to_string(), decide(false)),
    ]
}

/// Any signed-in person.
fn signed_in() -> IrRoutePolicy {
    IrRoutePolicy { authenticated: true, roles: Vec::new(), permissions: Vec::new() }
}

/// Whether this person meets an action's `requires`, read fresh, without
/// recording a decision: listing is not a call.
fn may_call(state: &ServeState, enforced: &EnforcedRouteActor, policy: &IrRoutePolicy) -> bool {
    if !policy.requires_auth() {
        return true;
    }
    let Some(ctx) = &state.auth_context else { return false };
    let Ok(roles) = ctx.auth.actor_roles(&enforced.actor.id) else { return false };
    let requirement = corvid_runtime::RoutePolicyRequirement {
        authenticated: policy.authenticated,
        roles: policy.roles.clone(),
        permissions: policy.permissions.clone(),
    };
    matches!(
        corvid_runtime::authorize_route(&roles, &ctx.role_permissions, &requirement),
        corvid_runtime::RouteAuthzOutcome::Allowed
    )
}

async fn handle(state: &Arc<ServeState>, offered: &[Offered], headers: &axum::http::HeaderMap, body: &[u8]) -> Response {
    let enforced = match enforce_route_policy(state, &signed_in(), HttpMethod::Post, MCP_PATH, headers) {
        Ok(enforced) => enforced,
        Err(refused) => return refused,
    };
    let Ok(request) = serde_json::from_slice::<Json_>(body) else {
        return rpc_error(Json_::Null, -32700, "the request is not JSON");
    };
    let id = request.get("id").cloned();
    let method = request["method"].as_str().unwrap_or_default();
    let Some(id) = id else {
        // A notification, such as `notifications/initialized`, has no reply.
        return StatusCode::ACCEPTED.into_response();
    };
    let callable: Vec<&Offered> = offered.iter().filter(|o| may_call(state, &enforced, &o.endpoint.policy)).collect();
    match method {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": request["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "corvid", "version": env!("CARGO_PKG_VERSION") },
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(
            id,
            json!({
                "tools": callable.iter().map(|o| json!({
                    "name": o.endpoint.name,
                    "description": o.description,
                    "inputSchema": o.input_schema,
                })).collect::<Vec<_>>()
            }),
        ),
        "tools/call" => {
            let name = request["params"]["name"].as_str().unwrap_or_default();
            // An action not offered, or one this person may not call, is
            // unknown: the answer never confirms it exists.
            let Some(tool) = callable.iter().find(|o| o.endpoint.name == name) else {
                return rpc_error(id, -32602, &format!("Unknown tool: {name}"));
            };
            let arguments = request["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
            let types_by_id: HashMap<DefId, &IrType> = state.ir.types.iter().map(|t| (t.id, t)).collect();
            let inputs = match decode_json_input(&tool.endpoint, &arguments, &types_by_id) {
                Ok(inputs) => inputs,
                Err(refused) => {
                    let detail = refused["detail"].as_str().unwrap_or("the input was refused").to_string();
                    return rpc_result(id, json!({ "isError": true, "content": [{ "type": "text", "text": detail }], "structuredContent": refused }));
                }
            };
            rpc_result(id, propose(state, tool, &enforced, inputs))
        }
        _ => rpc_error(id, -32601, &format!("Method not found: {method}")),
    }
}

/// Record a pending confirmation for this person and say what happens next.
fn propose(state: &ServeState, tool: &Offered, enforced: &EnforcedRouteActor, inputs: Vec<Value>) -> Json_ {
    let now = epoch_ms();
    let id = crate::serve_auth::pkce::random_token();
    {
        let mut pending = state.confirmations.lock().unwrap();
        pending.retain(|_, waiting| waiting.expires_at_ms > now);
        pending.insert(
            id.clone(),
            PendingConfirmation {
                action: tool.endpoint.name.clone(),
                actor_id: enforced.actor.id.clone(),
                tenant_id: enforced.actor.tenant_id.clone(),
                inputs,
                expires_at_ms: now + CONFIRMATION_WINDOW_MS,
            },
        );
    }
    trace_confirmation(state, &id, &tool.endpoint.name, &enforced.actor.id, "proposed");
    json!({
        "content": [{ "type": "text", "text": format!(
            "`{}` waits for the person you act for to confirm it in the page; it expires in 15 minutes.",
            tool.endpoint.name
        ) }],
        "structuredContent": {
            "status": "confirmation_required",
            "confirmation_id": id,
            "confirm": format!("/__confirmations/{id}/confirm"),
            "decline": format!("/__confirmations/{id}/decline"),
        },
    })
}

/// The person confirms or declines their assistant's call. Only the same
/// person in the same tenant may; anyone else leaves it waiting.
async fn decide(
    state: &Arc<ServeState>,
    offered: &[Offered],
    id: &str,
    headers: &axum::http::HeaderMap,
    confirm: bool,
) -> Response {
    let path = format!("/__confirmations/{id}");
    let enforced = match enforce_route_policy(state, &signed_in(), HttpMethod::Post, &path, headers) {
        Ok(enforced) => enforced,
        Err(refused) => return refused,
    };
    let pending = {
        let mut pending = state.confirmations.lock().unwrap();
        match pending.get(id) {
            None => None,
            Some(waiting) if waiting.actor_id != enforced.actor.id || waiting.tenant_id != enforced.actor.tenant_id => {
                return auth_error(StatusCode::FORBIDDEN, "only the person the assistant acts for may decide");
            }
            Some(_) => pending.remove(id),
        }
    };
    let Some(pending) = pending else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "unknown_confirmation" }))).into_response();
    };
    if pending.expires_at_ms <= epoch_ms() {
        trace_confirmation(state, id, &pending.action, &pending.actor_id, "expired");
        return (StatusCode::GONE, Json(json!({ "error": "confirmation_expired" }))).into_response();
    }
    if !confirm {
        trace_confirmation(state, id, &pending.action, &pending.actor_id, "declined");
        return Json(json!({ "status": "declined" })).into_response();
    }
    let Some(tool) = offered.iter().find(|o| o.endpoint.name == pending.action) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "unknown_confirmation" }))).into_response();
    };
    // The action's own `requires` is checked again, fresh, as the call runs.
    let enforced = if tool.endpoint.policy.requires_auth() {
        match enforce_route_policy(state, &tool.endpoint.policy, HttpMethod::Post, &tool.endpoint.path, headers) {
            Ok(enforced) => enforced,
            Err(refused) => return refused,
        }
    } else {
        enforced
    };
    trace_confirmation(state, id, &pending.action, &pending.actor_id, "confirmed");
    execute(state, &tool.endpoint, Some(&enforced), pending.inputs, json!({ "surface": "assistant", "confirmation": id })).await
}

fn trace_confirmation(state: &ServeState, id: &str, action: &str, actor: &str, status: &str) {
    state.runtime.emit_host_event(
        "assistant.confirmation",
        json!({ "confirmation": id, "action": action, "actor": actor, "status": status }),
    );
}

fn rpc_result(id: Json_, result: Json_) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_error(id: Json_, code: i64, message: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })).into_response()
}
