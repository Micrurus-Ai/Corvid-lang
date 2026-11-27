//! Calling a declared `action` from outside Corvid code: what each
//! action takes, and its input decoded from JSON or from text. `corvid
//! serve` (`POST /actions/<name>`, the assistant endpoint) and `corvid
//! act` all read through here, so every surface accepts and refuses the
//! same input.
//!
//! The input's shape comes from `corvid_types::action_input`, the rule
//! the contract and OpenAPI publish: one parameter of a declared type is
//! the whole input, anything else is one field per parameter. A `Caller`
//! parameter is never input.

use std::collections::HashMap;
use std::sync::Arc;

use corvid_driver::Value;
use corvid_ir::{IrRoutePolicy, IrType};
use corvid_resolve::DefId;
use corvid_types::action_input::{is_caller, whole_input_type, ACTIONS_PATH};
use corvid_types::Type;
use corvid_vm::{decode_public_json, UnknownFields};

use crate::request_text::{record_from_values, urlencoded_value};

/// What one action needs at call time, whichever surface calls it.
pub(crate) struct ActionEndpoint {
    pub(crate) name: String,
    pub(crate) agent: String,
    pub(crate) path: String,
    pub(crate) policy: IrRoutePolicy,
    /// Who decides an `approve` the body reaches; the checker requires it
    /// whenever one can be reached.
    pub(crate) approval: Option<corvid_ast::ApprovalSpec>,
    /// The `assistants:` line, when an assistant is offered the action.
    pub(crate) assistants: Option<corvid_ast::AssistantExposure>,
    /// The input parameters, callers left out.
    pub(crate) params: Vec<(String, Type)>,
    pub(crate) whole_input: Option<Type>,
    /// For each declared parameter, in order: whether it is the caller.
    pub(crate) callers: Vec<bool>,
}

/// Every declared action.
pub(crate) fn catalog(ir: &corvid_ir::IrFile) -> anyhow::Result<Vec<Arc<ActionEndpoint>>> {
    let mut catalog = Vec::new();
    for action in &ir.actions {
        let agent = ir
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
        catalog.push(Arc::new(ActionEndpoint {
            name: action.name.clone(),
            agent: agent.name.clone(),
            path: format!("{ACTIONS_PATH}{}", action.name),
            policy: action.policy.clone(),
            approval: action.approval.clone(),
            assistants: action.assistants.clone(),
            whole_input: whole_input_type(params.iter().map(|(_, ty)| ty)).cloned(),
            params,
            callers,
        }));
    }
    Ok(catalog)
}

/// The action's inputs from a JSON value, or the refusal body naming the
/// field or parameter. The HTTP endpoint answers it as `400`; the
/// assistant endpoint returns it as a tool error; `corvid act` prints it.
pub(crate) fn decode_json_input(
    endpoint: &ActionEndpoint,
    json: &serde_json::Value,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Vec<Value>, serde_json::Value> {
    let refusal = |error: &str, detail: String| serde_json::json!({ "error": error, "detail": detail });
    if let Some(ty) = &endpoint.whole_input {
        return decode_public_json(json, ty, types_by_id, UnknownFields::Reject).map(|value| vec![value]).map_err(|e| {
            let mut refused = e.to_json();
            refused["error"] = serde_json::json!("invalid_body");
            refused["detail"] = serde_json::json!(e.message());
            refused
        });
    }
    let Some(object) = json.as_object() else {
        return Err(refusal("invalid_input", "the input is a JSON object with one field per parameter".into()));
    };
    if let Some(unknown) = object.keys().find(|key| !endpoint.params.iter().any(|(name, _)| name == *key)) {
        return Err(refusal("invalid_input", format!("`{unknown}` is not a parameter of `{}`", endpoint.name)));
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
                refused
            })
        })
        .collect()
}

/// Why text could not become an action's input.
pub(crate) enum TextInputRefusal {
    /// A parameter no single text value can carry, such as a record beside
    /// other parameters: send this action JSON.
    Unsupported(String),
    /// A value that does not read as its type, naming the field.
    Refused(String),
}

/// The action's inputs from text values collected by name (form fields or
/// command-line flags), by the request-text rule: the record's fields
/// when it takes one record, otherwise one value per parameter. `source`
/// names where the values came from in messages (`form`, `flag`).
pub(crate) fn decode_text_input(
    endpoint: &ActionEndpoint,
    given: &HashMap<String, Vec<String>>,
    types_by_id: &HashMap<DefId, &IrType>,
    source: &str,
) -> Result<Vec<Value>, TextInputRefusal> {
    if let Some(ty) = &endpoint.whole_input {
        return record_from_values(given, ty, types_by_id, source)
            .map(|value| vec![value])
            .map_err(TextInputRefusal::Refused);
    }
    let plain = |ty: &Type| corvid_vm::public_json::has_plain_variants(ty, types_by_id);
    if let Some((name, _)) = endpoint
        .params
        .iter()
        .find(|(_, ty)| corvid_types::text_parameters::query_field_shape(ty, &plain).is_none())
    {
        return Err(TextInputRefusal::Unsupported(format!(
            "parameter `{name}` of `{}` cannot come from a {source} value; send this action JSON",
            endpoint.name
        )));
    }
    endpoint
        .params
        .iter()
        .map(|(name, ty)| urlencoded_value(given, name, ty, types_by_id, source).map_err(TextInputRefusal::Refused))
        .collect()
}
