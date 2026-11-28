//! `POST /actions/<name>`: one endpoint per `action` declaration (B10-2).
//!
//! The order is the same for every caller: the action's `requires`
//! policy first (a missing or invalid session is `401`, too little
//! authority or a failed CSRF check `403`), then the input decoded
//! through the action's types (a refused input is `400` naming the
//! field), then the `action` trace event, then the body. The body
//! never sees an unauthorized caller or an input its types refuse.
//!
//! The input's shape comes from `corvid_types::action_input`, the rule
//! the contract and OpenAPI publish: one parameter of a declared type is
//! the whole input, anything else is an object with one field per
//! parameter.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{on, MethodFilter, MethodRouter};
use axum::Json;
use corvid_ast::{HttpMethod, RouteResponseKind};
use corvid_driver::{run_ir_with_runtime, Value};
use corvid_ir::IrType;
use corvid_resolve::DefId;

pub(super) use crate::action_calls::{catalog, decode_json_input, ActionEndpoint};
use crate::action_calls::{decode_text_input, TextInputRefusal};
use crate::request_text::urlencoded_values;

use super::{
    bad_request, capture_pending_invocation_if_queued, enforce_route_policy, finish, request_runtime, with_run_header,
    reply_encoding, EnforcedRouteActor, ServeState,
};

/// One `(path, handler)` per declared action.
pub(super) fn endpoints(
    catalog: &[Arc<ActionEndpoint>],
) -> Vec<(String, MethodRouter<Arc<ServeState>>)> {
    catalog
        .iter()
        .map(|endpoint| {
            let endpoint = endpoint.clone();
            let path = endpoint.path.clone();
            let handler = on(
                MethodFilter::POST,
                move |State(state): State<Arc<ServeState>>, headers: axum::http::HeaderMap, body: Bytes| {
                    let endpoint = endpoint.clone();
                    async move { run_action(state, endpoint, headers, body).await }
                },
            );
            (path, handler)
        })
        .collect()
}

async fn run_action(
    state: Arc<ServeState>,
    endpoint: Arc<ActionEndpoint>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    let answer = answer_action(&state, &endpoint, &headers, &body).await;
    // A form a person submitted from a page gets a page back (B10-2b-5):
    // `303` to that page on success, or a page naming the refusal.
    if super::browser_form::wants_page(&headers) {
        let base = state.auth_context.as_ref().and_then(|ctx| ctx.public_base_url.as_deref());
        return super::browser_form::page_answer(answer, &headers, base, &endpoint.name).await;
    }
    answer
}

async fn answer_action(
    state: &Arc<ServeState>,
    endpoint: &ActionEndpoint,
    headers: &axum::http::HeaderMap,
    body: &Bytes,
) -> Response {
    // B11-3: an assistant presenting a person's grant. The call runs only
    // where the grant, the source and the person all agree: the grant
    // names the action at `allow`, the source offers it with `assistants:
    // allow`, and the person may call it now (checked fresh). A narrower
    // mode needs the person's confirmation, which `/mcp` asks for.
    if let Some(granted) = super::enforce_grant(state, &endpoint.policy, &endpoint.path, headers) {
        let (enforced, grant) = match granted {
            Ok(granted) => granted,
            Err(refused) => return refused,
        };
        let source = endpoint.assistants.as_ref().map(|line| line.mode);
        match (grant.mode_for(&endpoint.name), source) {
            (Some(corvid_runtime::DelegationMode::Allow), Some(corvid_ast::AssistantMode::Allow)) => {}
            (Some(_), Some(_)) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "confirmation_required",
                        "detail": format!("`{}` needs the person's confirmation for this grant; call it through {}", endpoint.name, super::assistant_endpoint::MCP_PATH),
                    })),
                )
                    .into_response();
            }
            _ => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({ "error": "not_granted", "detail": format!("the grant does not cover `{}`", endpoint.name) })),
                )
                    .into_response();
            }
        }
        let types_by_id: HashMap<DefId, &IrType> = state.ir.types.iter().map(|t| (t.id, t)).collect();
        let inputs = match decode_input(endpoint, headers, body, &types_by_id) {
            Ok(inputs) => inputs,
            Err(refused) => return refused,
        };
        let surface = serde_json::json!({ "surface": "assistant", "mode": "allow", "delegation": grant.id, "label": grant.label });
        return execute(state, endpoint, Some(&enforced), inputs, surface).await;
    }
    // A plain form carries its CSRF token in the reserved field; the one
    // double-submit check reads it from the header either way.
    let policy_headers = super::browser_form::with_form_csrf(headers, body);
    let enforced = if endpoint.policy.requires_auth() {
        match enforce_route_policy(state, &endpoint.policy, HttpMethod::Post, &endpoint.path, &policy_headers) {
            Ok(enforced) => Some(enforced),
            Err(refused) => return refused,
        }
    } else {
        None
    };
    let types_by_id: HashMap<DefId, &IrType> = state.ir.types.iter().map(|t| (t.id, t)).collect();
    let inputs = match decode_input(endpoint, headers, body, &types_by_id) {
        Ok(inputs) => inputs,
        Err(refused) => return refused,
    };
    execute(state, endpoint, enforced.as_ref(), inputs, serde_json::json!({ "surface": "http" })).await
}

/// Run an action with its decoded inputs, the same way for every surface:
/// each caller parameter takes the verified actor, the `action` trace
/// event records the surface, and the reply follows the route answers.
/// Startup refused an action that reads its caller without a sign-in.
pub(super) async fn execute(
    state: &Arc<ServeState>,
    endpoint: &ActionEndpoint,
    enforced: Option<&EnforcedRouteActor>,
    inputs: Vec<Value>,
    trace: serde_json::Value,
) -> Response {
    let mut inputs = inputs.into_iter();
    let args: Vec<Value> = endpoint
        .callers
        .iter()
        .map(|caller| match (caller, enforced) {
            (true, Some(enforced)) => enforced.value.clone(),
            _ => inputs.next().unwrap_or(Value::Nothing),
        })
        .collect();
    let mut event = serde_json::json!({
        "action": endpoint.name,
        "actor": enforced.map(|e| e.actor.id.clone()),
        "tenant": enforced.map(|e| e.actor.tenant_id.clone()),
    });
    if let (Some(event), Some(extra)) = (event.as_object_mut(), trace.as_object()) {
        event.extend(extra.clone());
    }
    state.runtime.emit_host_event("action", event);
    let route = format!("POST {}", endpoint.path);
    let runtime = request_runtime(state, enforced, endpoint.approval.as_ref(), route.clone());
    let outcome = run_ir_with_runtime(&state.ir, Some(&endpoint.agent), args.clone(), &runtime).await;
    capture_pending_invocation_if_queued(state, &endpoint.agent, &args, &outcome);
    let response = finish(outcome, RouteResponseKind::Json, &state.page_scripts, reply_encoding(state, &endpoint.agent), route);
    with_run_header(state, &runtime, response)
}

/// The action's arguments, in declaration order, from a JSON or form body.
fn decode_input(
    endpoint: &ActionEndpoint,
    headers: &axum::http::HeaderMap,
    body: &[u8],
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Vec<Value>, Response> {
    let media_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match media_type.as_str() {
        "application/json" => {
            let json: serde_json::Value = if body.is_empty() && endpoint.params.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_slice(body).map_err(|e| bad_request("invalid_json", &e.to_string()))?
            };
            decode_json_input(endpoint, &json, types_by_id)
                .map_err(|refused| (StatusCode::BAD_REQUEST, Json(refused)).into_response())
        }
        "application/x-www-form-urlencoded" => {
            let text = std::str::from_utf8(body).map_err(|_| bad_request("invalid_form", "form body must be UTF-8"))?;
            let mut given = urlencoded_values(text).map_err(|err| bad_request("invalid_form", &err))?;
            // The CSRF field is the form's, not the action's input; no
            // action input may take its name.
            given.remove(corvid_types::action_input::CSRF_FORM_FIELD);
            decode_text_input(endpoint, &given, types_by_id, "form").map_err(|refusal| match refusal {
                TextInputRefusal::Unsupported(detail) => unsupported(&detail),
                TextInputRefusal::Refused(detail) => bad_request("invalid_form", &detail),
            })
        }
        _ => Err(unsupported("expected application/json or application/x-www-form-urlencoded")),
    }
}

fn unsupported(detail: &str) -> Response {
    (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        Json(serde_json::json!({ "error": "unsupported_media_type", "detail": detail })),
    )
        .into_response()
}
