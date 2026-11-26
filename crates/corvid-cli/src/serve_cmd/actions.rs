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
use corvid_ir::{IrRoutePolicy, IrType};
use corvid_resolve::DefId;
use corvid_types::action_input::{is_caller, whole_input_type, ACTIONS_PATH};
use corvid_types::Type;
use corvid_vm::{decode_public_json, UnknownFields};

use super::{
    approval_runtime, bad_request, capture_pending_invocation_if_queued, enforce_route_policy, finish,
    invalid_body, reply_encoding, urlencoded_record, urlencoded_value, urlencoded_values, ServeState,
};

/// What one action's endpoint needs at request time.
struct ActionEndpoint {
    name: String,
    agent: String,
    path: String,
    policy: IrRoutePolicy,
    /// Who decides an `approve` the body reaches; the checker requires it
    /// whenever one can be reached.
    approval: Option<corvid_ast::ApprovalSpec>,
    /// The input parameters, callers left out.
    params: Vec<(String, Type)>,
    whole_input: Option<Type>,
    /// For each declared parameter, in order: whether it is the caller.
    callers: Vec<bool>,
}

/// One `(path, handler)` per declared action.
pub(super) fn endpoints(state: &ServeState) -> anyhow::Result<Vec<(String, MethodRouter<Arc<ServeState>>)>> {
    let mut endpoints = Vec::new();
    for action in &state.ir.actions {
        let agent = state
            .ir
            .agents
            .iter()
            .find(|agent| agent.id == action.agent)
            .ok_or_else(|| anyhow::anyhow!("action `{}` has no lowered body", action.name))?;
        let callers: Vec<bool> = agent.params.iter().map(|p| is_caller(&p.ty)).collect();
        if callers.iter().any(|caller| *caller) && !action.policy.requires_auth() {
            anyhow::bail!("action `{}` reads its caller but requires no sign-in", action.name);
        }
        let params: Vec<(String, Type)> = agent
            .params
            .iter()
            .filter(|p| !is_caller(&p.ty))
            .map(|p| (p.name.clone(), p.ty.clone()))
            .collect();
        let endpoint = Arc::new(ActionEndpoint {
            name: action.name.clone(),
            agent: agent.name.clone(),
            path: format!("{ACTIONS_PATH}{}", action.name),
            policy: action.policy.clone(),
            approval: action.approval.clone(),
            whole_input: whole_input_type(params.iter().map(|(_, ty)| ty)).cloned(),
            params,
            callers,
        });
        let path = endpoint.path.clone();
        let handler = on(
            MethodFilter::POST,
            move |State(state): State<Arc<ServeState>>, headers: axum::http::HeaderMap, body: Bytes| {
                let endpoint = endpoint.clone();
                async move { run_action(state, endpoint, headers, body).await }
            },
        );
        endpoints.push((path, handler));
    }
    Ok(endpoints)
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
    let actor = enforced.as_ref().map(|e| &e.actor);

    let types_by_id: HashMap<DefId, &IrType> = state.ir.types.iter().map(|t| (t.id, t)).collect();
    let inputs = match decode_input(&endpoint, &headers, &body, &types_by_id) {
        Ok(inputs) => inputs,
        Err(refused) => return refused,
    };
    // Each caller parameter takes the verified actor; startup refused an
    // action that reads its caller without requiring a sign-in.
    let mut inputs = inputs.into_iter();
    let args: Vec<Value> = endpoint
        .callers
        .iter()
        .map(|caller| match (caller, &enforced) {
            (true, Some(enforced)) => enforced.value.clone(),
            _ => inputs.next().unwrap_or(Value::Nothing),
        })
        .collect();

    state.runtime.emit_host_event(
        "action",
        serde_json::json!({
            "action": endpoint.name,
            "surface": "http",
            "actor": actor.as_ref().map(|a| a.id.clone()),
            "tenant": actor.as_ref().map(|a| a.tenant_id.clone()),
        }),
    );
    let route = format!("POST {}", endpoint.path);
    let runtime = approval_runtime(&state, enforced.as_ref(), endpoint.approval.as_ref(), route.clone());
    let outcome = run_ir_with_runtime(&state.ir, Some(&endpoint.agent), args.clone(), &runtime).await;
    capture_pending_invocation_if_queued(&state, &endpoint.agent, &args, &outcome);
    finish(outcome, RouteResponseKind::Json, &state.page_scripts, reply_encoding(&state, &endpoint.agent), route)
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
        }
        "application/x-www-form-urlencoded" => {
            let text = std::str::from_utf8(body).map_err(|_| bad_request("invalid_form", "form body must be UTF-8"))?;
            let refused = |err: String| bad_request("invalid_form", &err);
            match &endpoint.whole_input {
                Some(ty) => urlencoded_record(text, ty, types_by_id, "form").map(|value| vec![value]).map_err(refused),
                // One form field per parameter, read by the same rule as a
                // record's fields.
                None => {
                    let plain = |ty: &Type| corvid_vm::public_json::has_plain_variants(ty, types_by_id);
                    if let Some((name, _)) = endpoint
                        .params
                        .iter()
                        .find(|(_, ty)| corvid_types::text_parameters::query_field_shape(ty, &plain).is_none())
                    {
                        return Err(unsupported(&format!(
                            "parameter `{name}` of `{}` cannot come from a form field; send this action JSON",
                            endpoint.name
                        )));
                    }
                    let given = urlencoded_values(text).map_err(refused)?;
                    endpoint
                        .params
                        .iter()
                        .map(|(name, ty)| urlencoded_value(&given, name, ty, types_by_id, "form").map_err(refused))
                        .collect()
                }
            }
        }
        _ => Err(unsupported("expected application/json or application/x-www-form-urlencoded")),
    }
}

fn decode_json_input(
    endpoint: &ActionEndpoint,
    json: &serde_json::Value,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Vec<Value>, Response> {
    if let Some(ty) = &endpoint.whole_input {
        return decode_public_json(json, ty, types_by_id, UnknownFields::Reject)
            .map(|value| vec![value])
            .map_err(|e| invalid_body(&e));
    }
    let Some(object) = json.as_object() else {
        return Err(bad_request("invalid_input", "the input is a JSON object with one field per parameter"));
    };
    if let Some(unknown) = object.keys().find(|key| !endpoint.params.iter().any(|(name, _)| name == *key)) {
        return Err(bad_request("invalid_input", &format!("`{unknown}` is not a parameter of `{}`", endpoint.name)));
    }
    endpoint
        .params
        .iter()
        .map(|(name, ty)| {
            let given = object.get(name).unwrap_or(&serde_json::Value::Null);
            decode_public_json(given, ty, types_by_id, UnknownFields::Reject).map_err(|e| {
                let mut refused = e.to_json();
                refused["error"] = serde_json::json!("invalid_input");
                refused["parameter"] = serde_json::json!(name);
                refused["detail"] = serde_json::json!(format!("parameter `{name}`: {}", e.message()));
                (StatusCode::BAD_REQUEST, Json(refused)).into_response()
            })
        })
        .collect()
}

fn unsupported(detail: &str) -> Response {
    (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        Json(serde_json::json!({ "error": "unsupported_media_type", "detail": detail })),
    )
        .into_response()
}
