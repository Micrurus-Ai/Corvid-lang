//! What changed in an application's actions between two contracts, and
//! which callers each change reaches (B10-5).
//!
//! A Corvid caller of a changed action is a compile error at the call;
//! this report is for the callers the compiler cannot see: an HTTP
//! client, the generated TypeScript client in an app already shipped, a
//! saved `corvid act` script, an assistant. Each change is breaking (an
//! existing caller can now be refused or misread), additive (nothing an
//! existing caller does changes) or informational.

use crate::app_contract::{ApplicationContract, ContractAction, ContractField, ContractType};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSeverity {
    Breaking,
    Additive,
    Informational,
}

/// One change to one action.
#[derive(Debug, Clone, Serialize)]
pub struct ActionChange {
    pub action: String,
    pub severity: ChangeSeverity,
    /// What changed, in a sentence.
    pub detail: String,
    /// Every outside surface the action is reached through, each of
    /// which this change affects.
    pub surfaces: Vec<String>,
}

/// The changes from `base` to `head`, breaking ones first, then by action.
pub fn action_changes(base: &ApplicationContract, head: &ApplicationContract) -> Vec<ActionChange> {
    let mut changes = Vec::new();
    let named = |contract: &ApplicationContract| -> BTreeMap<String, ContractAction> {
        contract.actions.iter().map(|a| (a.callable.name.clone(), a.clone())).collect()
    };
    let (before, after) = (named(base), named(head));
    for (name, old) in &before {
        let mut push = |severity, detail: String| {
            let reached = after.get(name).unwrap_or(old);
            changes.push(ActionChange { action: name.clone(), severity, detail, surfaces: surfaces(reached, old) });
        };
        let Some(new) = after.get(name) else {
            push(ChangeSeverity::Breaking, format!("`{name}` was removed"));
            continue;
        };
        for (severity, detail) in input_changes(old, base, new, head) {
            push(severity, detail);
        }
        if old.callable.output_type != new.callable.output_type {
            push(
                ChangeSeverity::Breaking,
                format!("its result changed from `{}` to `{}`", old.callable.output_type, new.callable.output_type),
            );
        }
        for (severity, detail) in policy_changes(old, new) {
            push(severity, detail);
        }
        match (&old.approval, &new.approval) {
            (None, Some(policy)) => push(
                ChangeSeverity::Breaking,
                format!("it now waits for a `{}` reviewer's approval, so a caller can get `202` instead of a result", policy.role),
            ),
            (Some(_), None) => push(ChangeSeverity::Additive, "it no longer waits for an approval".into()),
            (Some(a), Some(b)) if a.role != b.role => {
                push(ChangeSeverity::Informational, format!("its reviewer role changed from `{}` to `{}`", a.role, b.role))
            }
            _ => {}
        }
        match (&old.assistants, &new.assistants) {
            (Some(_), None) => push(ChangeSeverity::Breaking, "it is no longer offered to assistants".into()),
            (None, Some(_)) => push(ChangeSeverity::Additive, "it is now offered to assistants".into()),
            (Some(a), Some(b)) if a.description != b.description => {
                push(ChangeSeverity::Informational, "its description for assistants changed".into())
            }
            _ => {}
        }
    }
    for (name, new) in &after {
        if !before.contains_key(name) {
            changes.push(ActionChange {
                action: name.clone(),
                severity: ChangeSeverity::Additive,
                detail: format!("`{name}` was added"),
                surfaces: surfaces(new, new),
            });
        }
    }
    changes.sort_by(|a, b| (a.severity, &a.action).cmp(&(b.severity, &b.action)));
    changes
}

/// The surfaces an action is reached through. `old` adds the assistant
/// tool when the change removed the action's `assistants:` line.
fn surfaces(action: &ContractAction, old: &ContractAction) -> Vec<String> {
    let name = &action.callable.name;
    let mut reached = vec![
        format!("POST {}", action.path()),
        format!("TypeScript client `api.{name}`"),
        format!("`corvid act {name}`"),
    ];
    if action.assistants.is_some() || old.assistants.is_some() {
        reached.push(format!("assistant tool `{name}` (MCP)"));
    }
    reached
}

/// An input part a caller sends: its type and whether it may be left out.
#[derive(PartialEq)]
struct InputPart {
    type_name: String,
    optional: bool,
    /// Bounds a refinement puts on it, as the contract carries them.
    bounds: String,
}

/// The parts of an action's input by the name a caller sends: the
/// record's fields (by wire key) when it takes one, otherwise its
/// parameters. `None` when the record is not in the contract.
fn input_parts(action: &ContractAction, contract: &ApplicationContract) -> Option<BTreeMap<String, InputPart>> {
    let part = |type_name: &str, bounds: String| InputPart {
        type_name: type_name.to_string(),
        optional: type_name.starts_with("Option<"),
        bounds,
    };
    match &action.input_type {
        Some(record) => {
            let ty: &ContractType = contract.types.iter().find(|t| &t.name == record)?;
            Some(ty.fields.iter().map(|f| (f.key().to_string(), part(&f.type_name, bounds(f)))).collect())
        }
        None => Some(action.callable.inputs.iter().map(|p| (p.name.clone(), part(&p.type_name, String::new()))).collect()),
    }
}

fn bounds(field: &ContractField) -> String {
    let mut text = serde_json::to_value(field).unwrap_or_default();
    if let Some(object) = text.as_object_mut() {
        for key in ["name", "type", "json_key"] {
            object.remove(key);
        }
    }
    text.to_string()
}

fn input_changes(
    old: &ContractAction,
    base: &ApplicationContract,
    new: &ContractAction,
    head: &ApplicationContract,
) -> Vec<(ChangeSeverity, String)> {
    let mut changes = Vec::new();
    if old.input_type.is_some() != new.input_type.is_some() {
        changes.push((
            ChangeSeverity::Breaking,
            "its input changed shape between one record and one field per parameter".to_string(),
        ));
        return changes;
    }
    let (Some(before), Some(after)) = (input_parts(old, base), input_parts(new, head)) else {
        return changes;
    };
    for (name, part) in &before {
        match after.get(name) {
            None => changes.push((
                ChangeSeverity::Breaking,
                format!("its input no longer has `{name}`, so a caller that sends it is refused"),
            )),
            Some(now) if now.type_name != part.type_name => changes.push((
                ChangeSeverity::Breaking,
                format!("input `{name}` changed from `{}` to `{}`", part.type_name, now.type_name),
            )),
            Some(now) if now.bounds != part.bounds => changes.push((
                ChangeSeverity::Breaking,
                format!("input `{name}` has different bounds, so a value accepted before can be refused"),
            )),
            Some(_) => {}
        }
    }
    for (name, part) in &after {
        if before.contains_key(name) {
            continue;
        }
        if part.optional {
            changes.push((ChangeSeverity::Additive, format!("its input gained the optional `{name}`")));
        } else {
            changes.push((
                ChangeSeverity::Breaking,
                format!("its input gained the required `{name}`, which existing callers do not send"),
            ));
        }
    }
    changes
}

fn policy_changes(old: &ContractAction, new: &ContractAction) -> Vec<(ChangeSeverity, String)> {
    let mut changes = Vec::new();
    let required = |action: &ContractAction| -> (bool, Vec<String>) {
        match &action.policy {
            None => (false, Vec::new()),
            Some(policy) => {
                let mut names: Vec<String> = policy.roles.iter().map(|r| format!("role `{r}`")).collect();
                names.extend(policy.permissions.iter().map(|p| format!("permission `{p}`")));
                (true, names)
            }
        }
    };
    let ((signed_in_before, before), (signed_in_after, after)) = (required(old), required(new));
    if !signed_in_before && signed_in_after {
        changes.push((ChangeSeverity::Breaking, "it now requires a signed-in caller".into()));
    } else if signed_in_before && !signed_in_after {
        changes.push((ChangeSeverity::Additive, "it no longer requires a signed-in caller".into()));
    }
    for gained in after.iter().filter(|name| !before.contains(name)) {
        changes.push((ChangeSeverity::Breaking, format!("it now requires {gained}")));
    }
    for dropped in before.iter().filter(|name| !after.contains(name)) {
        changes.push((ChangeSeverity::Additive, format!("it no longer requires {dropped}")));
    }
    changes
}

/// The report as text, one change per paragraph, with a closing count.
pub fn render_action_changes(changes: &[ActionChange]) -> String {
    if changes.is_empty() {
        return "no action changed\n".to_string();
    }
    let mut out = String::new();
    for change in changes {
        let label = match change.severity {
            ChangeSeverity::Breaking => "breaking",
            ChangeSeverity::Additive => "additive",
            ChangeSeverity::Informational => "note",
        };
        out.push_str(&format!("{label}: action `{}`: {}\n", change.action, change.detail));
        for surface in &change.surfaces {
            out.push_str(&format!("    reaches {surface}\n"));
        }
    }
    let breaking = changes.iter().filter(|c| c.severity == ChangeSeverity::Breaking).count();
    out.push_str(&format!("{} change(s), {breaking} breaking\n", changes.len()));
    out
}
