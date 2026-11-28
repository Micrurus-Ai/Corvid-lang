//! Grants of authority to an assistant (B11-2b): `POST /__delegations`,
//! `GET /__delegations` and `DELETE /__delegations/:id`.
//!
//! A signed-in person grants an assistant the actions they choose, each at
//! `confirm` or `allow`, for at most the source's `delegation: up_to` cap.
//! The answer carries a bearer token, shown once and stored only as a hash.
//! The routes exist only when the source declares the cap: without it no
//! grant can be issued.
//!
//! A grant authorizes nothing by itself. Each call made with it is checked
//! against what the grant names, what the source offers to assistants, and
//! what the person may do at that moment (see `enforce_grant`).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{on, MethodFilter, MethodRouter};
use axum::Json;
use corvid_ast::{AssistantMode, HttpMethod};
use corvid_ir::IrRoutePolicy;
use corvid_runtime::{DelegatedAction, DelegationCreate, DelegationMode, DelegationRecord};
use serde_json::{json, Value as Json_};

use super::actions::ActionEndpoint;
use super::{enforce_route_policy, epoch_ms, ServeState};

pub(super) const DELEGATIONS_PATH: &str = "/__delegations";

/// The three grant routes, or none when the source declares no cap.
pub(super) fn routes(catalog: &[Arc<ActionEndpoint>], cap_secs: Option<u64>) -> Vec<(String, MethodRouter<Arc<ServeState>>)> {
    let Some(cap_secs) = cap_secs else { return Vec::new() };
    let offered: Arc<Vec<Arc<ActionEndpoint>>> = Arc::new(catalog.iter().filter(|e| e.assistants.is_some()).cloned().collect());
    let collection = on(MethodFilter::GET, |State(state): State<Arc<ServeState>>, headers: axum::http::HeaderMap| async move {
        list(&state, &headers)
    })
    .on(MethodFilter::POST, move |State(state): State<Arc<ServeState>>, headers: axum::http::HeaderMap, body: Bytes| {
        let offered = offered.clone();
        async move { create(&state, &offered, cap_secs, &headers, &body) }
    });
    let one = on(MethodFilter::DELETE, move |State(state): State<Arc<ServeState>>, Path(id): Path<String>, headers: axum::http::HeaderMap| async move {
        revoke(&state, &id, &headers)
    });
    vec![(DELEGATIONS_PATH.to_string(), collection), (format!("{DELEGATIONS_PATH}/:id"), one)]
}

/// Any signed-in person, through their session (and CSRF for changes).
fn signed_in() -> IrRoutePolicy {
    IrRoutePolicy { authenticated: true, roles: Vec::new(), permissions: Vec::new() }
}

fn refused(status: StatusCode, error: &str, detail: &str) -> Response {
    (status, Json(json!({ "error": error, "detail": detail }))).into_response()
}

/// `POST /__delegations` with `{"label", "actions": [{"action", "mode"}],
/// "expires_in_secs"}`. Each action must be offered to assistants, at no
/// more than the mode its `assistants:` line declares, and the person must
/// be able to call it now; the lifetime is at most the declared cap.
fn create(state: &ServeState, offered: &[Arc<ActionEndpoint>], cap_secs: u64, headers: &axum::http::HeaderMap, body: &[u8]) -> Response {
    let enforced = match enforce_route_policy(state, &signed_in(), HttpMethod::Post, DELEGATIONS_PATH, headers) {
        Ok(enforced) => enforced,
        Err(refused) => return refused,
    };
    let Some(ctx) = &state.auth_context else { return refused(StatusCode::UNAUTHORIZED, "authentication_required", "") };
    let Ok(request) = serde_json::from_slice::<Json_>(body) else {
        return refused(StatusCode::BAD_REQUEST, "invalid_json", "the body is not JSON");
    };
    let label = request["label"].as_str().unwrap_or_default().trim().to_string();
    if label.is_empty() || label.chars().count() > 80 {
        return refused(StatusCode::BAD_REQUEST, "invalid_label", "`label` names the assistant, in 1 to 80 characters");
    }
    let Some(expires_in) = request["expires_in_secs"].as_u64().filter(|secs| *secs > 0) else {
        return refused(StatusCode::BAD_REQUEST, "invalid_expiry", "`expires_in_secs` is how long the grant lasts, in seconds");
    };
    if expires_in > cap_secs {
        return refused(
            StatusCode::BAD_REQUEST,
            "expiry_beyond_cap",
            &format!("a grant may last at most {cap_secs} seconds (`delegation: up_to`)"),
        );
    }
    let Some(asked) = request["actions"].as_array().filter(|a| !a.is_empty()) else {
        return refused(StatusCode::BAD_REQUEST, "invalid_actions", "`actions` lists at least one action and its mode");
    };
    let roles = ctx.auth.actor_roles(&enforced.actor.id).unwrap_or_default();
    let mut actions = Vec::new();
    for entry in asked {
        let name = entry["action"].as_str().unwrap_or_default();
        let Some(mode) = entry["mode"].as_str().and_then(DelegationMode::parse) else {
            return refused(StatusCode::BAD_REQUEST, "invalid_mode", &format!("`{name}`: `mode` is `confirm` or `allow`"));
        };
        // An action not offered to assistants is unknown, as on `/mcp`.
        let Some(endpoint) = offered.iter().find(|e| e.name == name) else {
            return refused(StatusCode::BAD_REQUEST, "unknown_action", &format!("`{name}` is not offered to assistants"));
        };
        let declared = endpoint.assistants.as_ref().map(|line| line.mode);
        if mode == DelegationMode::Allow && declared != Some(AssistantMode::Allow) {
            return refused(
                StatusCode::BAD_REQUEST,
                "mode_beyond_source",
                &format!("`{name}` is offered with `confirm` only, so a grant cannot `allow` it"),
            );
        }
        let requirement = corvid_runtime::RoutePolicyRequirement {
            authenticated: endpoint.policy.authenticated,
            roles: endpoint.policy.roles.clone(),
            permissions: endpoint.policy.permissions.clone(),
        };
        if !matches!(corvid_runtime::authorize_route(&roles, &ctx.role_permissions, &requirement), corvid_runtime::RouteAuthzOutcome::Allowed) {
            return refused(StatusCode::FORBIDDEN, "beyond_your_authority", &format!("you may not call `{name}`, so you cannot grant it"));
        }
        actions.push(DelegatedAction { action: name.to_string(), mode });
    }
    let now = epoch_ms();
    let token = format!("cdg_{}", crate::serve_auth::pkce::random_token());
    let record = match ctx.auth.create_delegation(DelegationCreate {
        id: format!("grant_{}", crate::serve_auth::pkce::random_token()),
        actor_id: enforced.actor.id.clone(),
        tenant_id: enforced.actor.tenant_id.clone(),
        label,
        actions,
        raw_token: token.clone(),
        issued_ms: now,
        expires_ms: now + expires_in * 1000,
        max_lifetime_ms: cap_secs * 1000,
    }) {
        Ok(record) => record,
        Err(error) => return refused(StatusCode::BAD_REQUEST, "invalid_grant", &error.to_string()),
    };
    trace(state, "created", &record);
    let mut answer = describe(&record, now);
    // The token is shown once, here, and never again.
    answer["token"] = json!(token);
    (StatusCode::CREATED, Json(answer)).into_response()
}

/// `GET /__delegations`: the person's grants, without their tokens.
fn list(state: &ServeState, headers: &axum::http::HeaderMap) -> Response {
    let enforced = match enforce_route_policy(state, &signed_in(), HttpMethod::Get, DELEGATIONS_PATH, headers) {
        Ok(enforced) => enforced,
        Err(refused) => return refused,
    };
    let Some(ctx) = &state.auth_context else { return refused(StatusCode::UNAUTHORIZED, "authentication_required", "") };
    let now = epoch_ms();
    match ctx.auth.list_delegations(&enforced.actor.id) {
        Ok(grants) => Json(json!({ "delegations": grants.iter().map(|g| describe(g, now)).collect::<Vec<_>>() })).into_response(),
        Err(error) => refused(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", &error.to_string()),
    }
}

/// `DELETE /__delegations/:id`: the person revokes one of their grants.
/// Every call made with it stops at once; someone else's grant is unknown.
fn revoke(state: &ServeState, id: &str, headers: &axum::http::HeaderMap) -> Response {
    let path = format!("{DELEGATIONS_PATH}/{id}");
    let enforced = match enforce_route_policy(state, &signed_in(), HttpMethod::Delete, &path, headers) {
        Ok(enforced) => enforced,
        Err(refused) => return refused,
    };
    let Some(ctx) = &state.auth_context else { return refused(StatusCode::UNAUTHORIZED, "authentication_required", "") };
    match ctx.auth.revoke_delegation(id, &enforced.actor.id, epoch_ms()) {
        Ok(true) => {
            state.runtime.emit_host_event("delegation", json!({ "status": "revoked", "delegation": id, "actor": enforced.actor.id }));
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => refused(StatusCode::NOT_FOUND, "unknown_delegation", ""),
        Err(error) => refused(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", &error.to_string()),
    }
}

fn describe(grant: &DelegationRecord, now_ms: u64) -> Json_ {
    let status = if grant.revoked_ms.is_some() {
        "revoked"
    } else if grant.is_live(now_ms) {
        "live"
    } else {
        "expired"
    };
    json!({
        "id": grant.id,
        "label": grant.label,
        "actions": grant.actions.iter().map(|a| json!({ "action": a.action, "mode": a.mode.as_str() })).collect::<Vec<_>>(),
        "issued_ms": grant.issued_ms,
        "expires_ms": grant.expires_ms,
        "status": status,
    })
}

fn trace(state: &ServeState, status: &str, grant: &DelegationRecord) {
    state.runtime.emit_host_event(
        "delegation",
        json!({
            "status": status,
            "delegation": grant.id,
            "label": grant.label,
            "actor": grant.actor_id,
            "actions": grant.actions.iter().map(|a| json!({ "action": a.action, "mode": a.mode.as_str() })).collect::<Vec<_>>(),
            "expires_ms": grant.expires_ms,
        }),
    );
}
