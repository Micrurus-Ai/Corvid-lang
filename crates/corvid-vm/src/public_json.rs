//! The public JSON mapping for interpreter values.
//!
//! The mapping itself, and its decoder, live in `corvid-public-json`
//! so the native runtime decodes with the same rules; this module
//! builds interpreter [`Value`]s for that decoder and encodes them.
//!
//! Prompt structured output, tool arguments and recorded traces use
//! the internal envelope in [`crate::conv`] instead. That format is
//! lossless for every runtime value (including nested options and
//! provenance), matches the schema sent to model providers, and is
//! what existing recordings replay against, so it stays separate.
//! The two mappings agree on records, lists, scalars and sum types;
//! they differ only on `Option`, `Result` and `Grounded`.

use crate::value::{BoxedValue, EnumValue, ListValue, MapValue, StructValue, Value};
use corvid_public_json::text_scalar::TextScalar;
use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::{json, Value as J};
use std::collections::HashMap;
use std::sync::Arc;

pub use corvid_public_json::{JsonDecodeError, UnknownFields};

/// The Corvid `JsonError` record `T.from_json` returns, built from a
/// decode failure with the fields the checker types it with.
pub trait JsonErrorValue {
    fn to_value(&self) -> Value;
}

impl JsonErrorValue for JsonDecodeError {
    fn to_value(&self) -> Value {
        let message = self.message();
        let fields = corvid_types::json_error_fields().map(|(name, _)| {
            let text = match name {
                "path" => self.path.as_str(),
                "expected" => self.expected.as_str(),
                "found" => self.found.as_str(),
                "message" => message.as_str(),
                other => unreachable!("JsonError field `{other}` has no runtime value"),
            };
            (name.to_string(), Value::String(Arc::from(text)))
        });
        Value::Struct(StructValue::new(JSON_ERROR_TYPE_ID, "JsonError", fields))
    }
}

pub use corvid_types::JSON_ERROR_TYPE_ID;

/// Decode `json` into a value of type `expected` using the public mapping.
pub fn decode_public_json(
    json: &J,
    expected: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    unknown_fields: UnknownFields,
) -> Result<Value, JsonDecodeError> {
    corvid_public_json::decode(json, expected, types_by_id, unknown_fields, &mut InterpreterValues)
}

/// Decode JSON `text` into a value of type `expected`: the
/// `T.from_json(text)` entry point. Text that is not JSON fails at the
/// root path.
pub fn decode_public_json_text(
    text: &str,
    expected: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    unknown_fields: UnknownFields,
) -> Result<Value, JsonDecodeError> {
    corvid_public_json::decode_text(text, expected, types_by_id, unknown_fields, &mut InterpreterValues)
}

/// Builds interpreter values for the shared decoder.
struct InterpreterValues;

impl corvid_public_json::ValueBuilder for InterpreterValues {
    type Value = Value;

    fn int(&mut self, value: i64) -> Value {
        Value::Int(value)
    }
    fn float(&mut self, value: f64) -> Value {
        Value::Float(value)
    }
    fn string(&mut self, value: &str) -> Value {
        Value::String(Arc::from(value))
    }
    fn bool(&mut self, value: bool) -> Value {
        Value::Bool(value)
    }
    fn nothing(&mut self) -> Value {
        Value::Nothing
    }
    fn none(&mut self, _option: &Type) -> Value {
        Value::OptionNone
    }
    fn some(&mut self, _option: &Type, value: Value) -> Value {
        Value::OptionSome(BoxedValue::new(value))
    }
    fn list(&mut self, _list: &Type, items: Vec<Value>) -> Value {
        Value::List(ListValue::new(items))
    }
    fn map(&mut self, _map: &Type, entries: Vec<(Value, Value)>) -> Value {
        Value::Map(MapValue::new(entries))
    }
    fn ok(&mut self, _result: &Type, value: Value) -> Value {
        Value::ResultOk(BoxedValue::new(value))
    }
    fn err(&mut self, _result: &Type, value: Value) -> Value {
        Value::ResultErr(BoxedValue::new(value))
    }
    fn record(&mut self, record: &IrType, fields: Vec<Value>) -> Value {
        let named: HashMap<String, Value> =
            record.fields.iter().map(|field| field.name.clone()).zip(fields).collect();
        Value::Struct(StructValue::new(record.id, record.name.clone(), named))
    }
    fn variant(&mut self, sum: &IrType, index: usize, fields: Vec<Value>) -> Value {
        let variant = &sum.variants[index];
        Value::Enum(EnumValue::new(sum.id, sum.name.clone(), index as u32, variant.name.clone(), fields))
    }
    fn json(&mut self, json: &J) -> Value {
        Value::JsonValue(Arc::new(json.clone()))
    }
    fn text_scalar(&mut self, value: TextScalar) -> Value {
        value_of_text_scalar(value)
    }
}

/// B06 — the interpreter value of a parsed text scalar. Tool and prompt
/// JSON (`conv.rs`) and the public mapping both go through this pair.
pub(crate) fn value_of_text_scalar(value: TextScalar) -> Value {
    match value {
        TextScalar::Decimal(d) => Value::Decimal(d),
        TextScalar::Duration(d) => Value::Duration(d),
        TextScalar::Instant(i) => Value::Instant(i),
        TextScalar::TimeZone(z) => Value::TimeZone(z),
        TextScalar::ZonedTime(z) => Value::ZonedTime(z),
        TextScalar::Currency(c) => Value::Currency(c),
        TextScalar::Locale(l) => Value::Locale(Arc::new(l)),
    }
}

/// B06 — the text scalar an interpreter value holds, if it is one.
pub(crate) fn text_scalar_of_value(value: &Value) -> Option<TextScalar> {
    Some(match value {
        Value::Decimal(d) => TextScalar::Decimal(*d),
        Value::Duration(d) => TextScalar::Duration(*d),
        Value::Instant(i) => TextScalar::Instant(*i),
        Value::TimeZone(z) => TextScalar::TimeZone(*z),
        Value::ZonedTime(z) => TextScalar::ZonedTime(*z),
        Value::Currency(c) => TextScalar::Currency(*c),
        Value::Locale(l) => TextScalar::Locale((**l).clone()),
        _ => return None,
    })
}

/// Encode `value`, of type `ty`, as compact public JSON text with sorted
/// object keys: `value.to_json()`.
pub fn encode_public_json_text(
    value: &Value,
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<String, String> {
    corvid_public_json::encode_text(value, ty, types_by_id, &mut InterpreterValues)
}

/// Encode `value`, of type `ty`, as public JSON. The walk follows the
/// type, the way native code must, so both tiers produce the same JSON.
/// Types with no public JSON form (functions, handles, streams,
/// builders) are reported by name; the checker rejects `to_json` on
/// them, so this is a backstop for host boundaries.
pub fn encode_public_json(
    value: &Value,
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<J, String> {
    corvid_public_json::encode(value, ty, types_by_id, &mut InterpreterValues)
}

fn mismatch(expected: &str, value: &Value) -> String {
    format!("the value is {} where its type says {expected}", value.type_name())
}

impl corvid_public_json::ValueReader for InterpreterValues {
    type Value = Value;

    fn int(&mut self, value: &Value) -> Result<i64, String> {
        match value {
            Value::Int(n) => Ok(*n),
            other => Err(mismatch("Int", other)),
        }
    }
    fn float(&mut self, value: &Value) -> Result<f64, String> {
        match value {
            Value::Float(f) => Ok(*f),
            Value::Int(n) => Ok(*n as f64),
            other => Err(mismatch("Float", other)),
        }
    }
    fn bool(&mut self, value: &Value) -> Result<bool, String> {
        match value {
            Value::Bool(b) => Ok(*b),
            other => Err(mismatch("Bool", other)),
        }
    }
    fn string(&mut self, value: &Value) -> Result<String, String> {
        match value {
            Value::String(s) => Ok(s.to_string()),
            other => Err(mismatch("String", other)),
        }
    }
    fn option(&mut self, _option: &Type, value: &Value) -> Result<Option<Value>, String> {
        Ok(match value {
            Value::OptionNone | Value::Nothing => None,
            Value::OptionSome(inner) => Some(inner.get()),
            // A payload stored without its `Some` wrapper.
            other => Some(other.clone()),
        })
    }
    fn list(&mut self, _list: &Type, value: &Value) -> Result<Vec<Value>, String> {
        match value {
            Value::List(items) => Ok(items.iter_cloned()),
            other => Err(mismatch("a List", other)),
        }
    }
    fn map(&mut self, _map: &Type, value: &Value) -> Result<Vec<(Value, Value)>, String> {
        match value {
            Value::Map(map) => Ok(map.entries_cloned()),
            other => Err(mismatch("a Map", other)),
        }
    }
    fn result(&mut self, _result: &Type, value: &Value) -> Result<(bool, Value), String> {
        match value {
            Value::ResultOk(inner) => Ok((true, inner.get())),
            Value::ResultErr(inner) => Ok((false, inner.get())),
            other => Err(mismatch("a Result", other)),
        }
    }
    fn record(&mut self, record: &IrType, value: &Value) -> Result<Vec<Value>, String> {
        let Value::Struct(fields) = value else { return Err(mismatch(&record.name, value)) };
        record
            .fields
            .iter()
            .map(|field| match fields.get_field(&field.name) {
                Some(field_value) => Ok(field_value),
                None if matches!(field.ty, Type::Option(_)) => Ok(Value::OptionNone),
                None => Err(format!("the `{}` value has no `{}` field", record.name, field.name)),
            })
            .collect()
    }
    fn variant(&mut self, sum: &IrType, value: &Value) -> Result<(usize, Vec<Value>), String> {
        match value {
            Value::Enum(variant) => Ok((variant.variant_index() as usize, variant.fields_cloned())),
            other => Err(mismatch(&sum.name, other)),
        }
    }
    fn grounded(&mut self, _grounded: &Type, value: &Value) -> Result<(Value, Vec<J>), String> {
        let Value::Grounded(grounded) = value else { return Ok((value.clone(), Vec::new())) };
        let sources = grounded
            .provenance
            .entries
            .iter()
            .map(|entry| json!({ "kind": entry.kind.label(), "name": entry.name, "timestamp_ms": entry.timestamp_ms }))
            .collect();
        Ok((grounded.inner.get(), sources))
    }
    fn json(&mut self, value: &Value) -> Result<J, String> {
        match value {
            Value::JsonValue(json) => Ok((**json).clone()),
            other => Err(mismatch("JsonValue", other)),
        }
    }
    fn text_scalar(&mut self, ty: &Type, value: &Value) -> Result<TextScalar, String> {
        text_scalar_of_value(value).ok_or_else(|| mismatch(&ty.display_name(), value))
    }
}

#[cfg(test)]
mod tests;
