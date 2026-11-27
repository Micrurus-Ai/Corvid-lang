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
        for (severity, detail) in reached_type_changes(old, base, head) {
            push(severity, detail);
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

/// Where a declared type is reached: what a caller sends, what it reads,
/// or both. The direction decides what breaks: a caller that sends a
/// record breaks on a new required field, one that reads a choice breaks
/// on a new variant it does not handle.
#[derive(Clone, Copy, PartialEq)]
enum Reached {
    Sent,
    Read,
}

/// The declared types a type name mentions (`Option<List<Card>>` names
/// `Card`), by the contract's own type list.
fn named_types<'a>(type_name: &str, contract: &'a ApplicationContract) -> Vec<&'a ContractType> {
    type_name
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .filter_map(|word| contract.types.iter().find(|t| t.name == word))
        .collect()
}

/// Every declared type reached from `roots`, following fields and variant
/// payloads, each once.
fn reach<'a>(roots: Vec<&'a ContractType>, contract: &'a ApplicationContract) -> Vec<&'a ContractType> {
    let mut seen: Vec<&ContractType> = Vec::new();
    let mut pending = roots;
    while let Some(ty) = pending.pop() {
        if seen.iter().any(|t| t.name == ty.name) {
            continue;
        }
        seen.push(ty);
        let fields = ty.fields.iter().chain(ty.variants.iter().flat_map(|v| v.fields.iter()));
        for field in fields {
            pending.extend(named_types(&field.type_name, contract));
        }
    }
    seen
}

/// Changes inside the declared types an action's input and result hold.
/// The input's own record is compared by `input_changes`; this covers
/// what it and the result reach.
fn reached_type_changes(old: &ContractAction, base: &ApplicationContract, head: &ApplicationContract) -> Vec<(ChangeSeverity, String)> {
    let sent_roots: Vec<&ContractType> = match &old.input_type {
        Some(record) => base
            .types
            .iter()
            .find(|t| &t.name == record)
            .map(|t| t.fields.iter().flat_map(|f| named_types(&f.type_name, base)).collect())
            .unwrap_or_default(),
        None => old.callable.inputs.iter().flat_map(|p| named_types(&p.type_name, base)).collect(),
    };
    let read_roots = named_types(&old.callable.output_type, base);
    let mut changes = Vec::new();
    for (reached, roots) in [(Reached::Sent, sent_roots), (Reached::Read, read_roots)] {
        for before in reach(roots, base) {
            // A type renamed or removed shows up where it is named, as a
            // changed field or result type.
            let Some(after) = head.types.iter().find(|t| t.name == before.name) else { continue };
            changes.extend(type_changes(before, after, reached));
        }
    }
    changes
}

fn type_changes(before: &ContractType, after: &ContractType, reached: Reached) -> Vec<(ChangeSeverity, String)> {
    let name = &before.name;
    let place = match reached {
        Reached::Sent => "in its input",
        Reached::Read => "in its result",
    };
    let mut changes = Vec::new();
    let fields = |ty: &ContractType| -> BTreeMap<String, (String, String)> {
        ty.fields.iter().map(|f| (f.key().to_string(), (f.type_name.clone(), bounds(f)))).collect()
    };
    let (old_fields, new_fields) = (fields(before), fields(after));
    for (key, (ty, limits)) in &old_fields {
        match new_fields.get(key) {
            None => changes.push((ChangeSeverity::Breaking, format!("`{name}.{key}` {place} was removed"))),
            Some((now, _)) if now != ty => {
                changes.push((ChangeSeverity::Breaking, format!("`{name}.{key}` {place} changed from `{ty}` to `{now}`")))
            }
            Some((_, now)) if now != limits && reached == Reached::Sent => changes.push((
                ChangeSeverity::Breaking,
                format!("`{name}.{key}` {place} has different bounds, so a value accepted before can be refused"),
            )),
            _ => {}
        }
    }
    for (key, (ty, _)) in &new_fields {
        if old_fields.contains_key(key) {
            continue;
        }
        let severity = if reached == Reached::Sent && !ty.starts_with("Option<") {
            ChangeSeverity::Breaking
        } else {
            ChangeSeverity::Additive
        };
        changes.push((severity, format!("`{name}` {place} gained the field `{key}`")));
    }
    let variants = |ty: &ContractType| -> Vec<String> { ty.variants.iter().map(|v| v.name.clone()).collect() };
    let (old_variants, new_variants) = (variants(before), variants(after));
    for variant in old_variants.iter().filter(|v| !new_variants.contains(v)) {
        // A caller that sends a removed variant is refused; one that reads
        // it simply never receives it again.
        let severity = if reached == Reached::Sent { ChangeSeverity::Breaking } else { ChangeSeverity::Informational };
        changes.push((severity, format!("`{name}` {place} no longer has the variant `{variant}`")));
    }
    for variant in new_variants.iter().filter(|v| !old_variants.contains(v)) {
        // A reader that handles every variant meets one it does not know.
        let severity = if reached == Reached::Read { ChangeSeverity::Breaking } else { ChangeSeverity::Additive };
        changes.push((severity, format!("`{name}` {place} gained the variant `{variant}`")));
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
