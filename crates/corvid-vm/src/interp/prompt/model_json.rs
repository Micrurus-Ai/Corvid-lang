//! The JSON a prompt exchanges with a model.
//!
//! Every value a prompt shows a model, and every reply it reads back, is
//! the public JSON form (`corvid_public_json`): the form `T.to_json()`
//! writes, HTTP bodies carry and the reply schema (`schema_for`)
//! describes. An `Option` is its value or `null`, a sum type is
//! `{"tag": "variant", "type": …, "variant": …, "fields": [...]}`. So the
//! model sees what a frontend sees, the schema it is given matches what
//! it is asked to produce, and native code, whose decoders are written
//! from the same mapping, reads the same replies.
//!
//! Two prompt-specific rules sit on top of the mapping:
//!
//! - A `Grounded<T>` argument is shown as its `T`. Its provenance stays
//!   with the runtime; the model has no use for the source list.
//! - A `Partial<T>` reply is a record of per-field envelopes:
//!   `{"tag": "complete", "value": …}` for a finished field and
//!   `{"tag": "streaming"}` (or an absent field) for one still arriving.
//!   The values inside are the public form. A partial value shown back
//!   to a model (a continuation, a trace) is written the same way.

use std::collections::HashMap;

use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::Value as J;

use crate::public_json::{decode_public_json, encode_public_json};
use crate::value::{PartialFieldValue, PartialValue, Value};
use corvid_public_json::UnknownFields;

/// A value as a model is shown it, given its declared type.
pub(crate) fn to_model_json(value: &Value, ty: &Type, types: &HashMap<DefId, &IrType>) -> Result<J, String> {
    match (value, ty) {
        (Value::Grounded(grounded), Type::Grounded(inner) | Type::Tainted(inner)) => {
            to_model_json(&grounded.inner.get(), inner, types)
        }
        (Value::Grounded(grounded), _) => to_model_json(&grounded.inner.get(), ty, types),
        (_, Type::Grounded(inner) | Type::Tainted(inner)) => to_model_json(value, inner, types),
        (Value::Partial(partial), Type::Partial(inner)) => partial_to_model_json(partial, inner, types),
        _ => encode_public_json(value, ty, types),
    }
}

fn partial_to_model_json(partial: &PartialValue, inner: &Type, types: &HashMap<DefId, &IrType>) -> Result<J, String> {
    let ir_type = record_type(inner, types)?;
    partial.with_fields(|fields| {
        let mut out = serde_json::Map::new();
        for field in &ir_type.fields {
            let envelope = match fields.get(&field.name) {
                Some(PartialFieldValue::Complete(value)) => {
                    serde_json::json!({ "tag": "complete", "value": encode_public_json(value, &field.ty, types)? })
                }
                _ => serde_json::json!({ "tag": "streaming" }),
            };
            out.insert(field.key().to_string(), envelope);
        }
        Ok(J::Object(out))
    })
}

fn record_type<'t>(inner: &Type, types: &HashMap<DefId, &'t IrType>) -> Result<&'t IrType, String> {
    let def_id = match inner {
        Type::Struct(def_id) => *def_id,
        Type::ImportedStruct(imported) => imported.def_id,
        other => return Err(format!("a partial value must describe a record, not {}", other.display_name())),
    };
    types.get(&def_id).copied().ok_or_else(|| format!("unknown record type {def_id:?}"))
}

/// A value as text in a prompt: its JSON form, so a `String` is quoted
/// and escaped.
pub(crate) fn to_model_text(value: &Value, ty: &Type, types: &HashMap<DefId, &IrType>) -> Result<String, String> {
    to_model_json(value, ty, types).map(|json| json.to_string())
}

/// Read a model's reply as a value of type `ty`. Undeclared keys are
/// ignored, as `T.from_json` ignores them: a model that adds a field has
/// not produced a wrong value.
pub(crate) fn from_model_json(json: J, ty: &Type, types: &HashMap<DefId, &IrType>) -> Result<Value, String> {
    let json = corvid_public_json::model_reply(json, ty);
    match ty {
        Type::Partial(inner) => partial_from_model_json(json, inner, types),
        Type::Grounded(inner) | Type::Tainted(inner) => from_model_json(json, inner, types),
        _ => decode_public_json(&json, ty, types, UnknownFields::Ignore).map_err(|error| error.to_string()),
    }
}

fn partial_from_model_json(json: J, inner: &Type, types: &HashMap<DefId, &IrType>) -> Result<Value, String> {
    let ir_type = record_type(inner, types)?;
    let J::Object(map) = json else {
        return Err(format!("a partial `{}` must be an object, got {json}", ir_type.name));
    };
    let mut fields = HashMap::new();
    for field in &ir_type.fields {
        let state = match map.get(field.key()) {
            None => PartialFieldValue::Streaming,
            Some(J::Object(envelope)) if envelope.get("tag").and_then(J::as_str) == Some("streaming") => {
                PartialFieldValue::Streaming
            }
            Some(J::Object(envelope)) if envelope.get("tag").and_then(J::as_str) == Some("complete") => {
                let raw = envelope
                    .get("value")
                    .ok_or_else(|| format!("`{}.{}` is complete but has no `value`", ir_type.name, field.name))?;
                let value = decode_public_json(raw, &field.ty, types, UnknownFields::Ignore)
                    .map_err(|error| format!("`{}.{}`: {error}", ir_type.name, field.name))?;
                PartialFieldValue::Complete(value)
            }
            Some(other) => {
                return Err(format!(
                    "`{}.{}` must be {{\"tag\": \"complete\", \"value\": …}} or {{\"tag\": \"streaming\"}}, got {other}",
                    ir_type.name, field.name
                ))
            }
        };
        fields.insert(field.name.clone(), state);
    }
    Ok(Value::Partial(PartialValue::new(ir_type.id, ir_type.name.clone(), fields)))
}
