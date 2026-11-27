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
    approval_runtime, bad_request, capture_pending_invocation_if_queued, enforce_route_policy, finish,
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
    let enforced = if endpoint.policy.requires_auth() {
        match enforce_route_policy(&state, &endpoint.policy, HttpMethod::Post, &endpoint.path, &headers) {
            Ok(enforced) => Some(enforced),
            Err(refused) => return refused,
        }
    } else {
        None
    };
    let types_by_id: HashMap<DefId, &IrType> = state.ir.types.iter().map(|t| (t.id, t)).collect();
    let inputs = match decode_input(&endpoint, &headers, &body, &types_by_id) {
        Ok(inputs) => inputs,
        Err(refused) => return refused,
    };
    execute(&state, &endpoint, enforced.as_ref(), inputs, serde_json::json!({ "surface": "http" })).await
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
    let runtime = approval_runtime(state, enforced, endpoint.approval.as_ref(), route.clone());
    let outcome = run_ir_with_runtime(&state.ir, Some(&endpoint.agent), args.clone(), &runtime).await;
    capture_pending_invocation_if_queued(state, &endpoint.agent, &args, &outcome);
    finish(outcome, RouteResponseKind::Json, &state.page_scripts, reply_encoding(state, &endpoint.agent), route)
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
            let given = urlencoded_values(text).map_err(|err| bad_request("invalid_form", &err))?;
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
