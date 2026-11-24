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
use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::{json, Map, Value as J};
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

/// The type id carried by runtime `JsonError` records. `JsonError` is
/// compiler-known, so it has no declaration id; this one is reserved.
pub const JSON_ERROR_TYPE_ID: DefId = DefId(u32::MAX);

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
}

/// Encode `value` as compact public JSON text: `value.to_json()`.
pub fn encode_public_json_text(value: &Value) -> Result<String, String> {
    encode_public_json(value).map(|json| json.to_string())
}

/// Encode `value` as public JSON. Values with no public JSON form
/// (closures, handles, streams, builders) are reported by name; the
/// checker rejects `to_json` on those types, so this is a backstop
/// for host boundaries.
pub fn encode_public_json(value: &Value) -> Result<J, String> {
    Ok(match value {
        Value::Int(n) => J::from(*n),
        Value::Float(f) => {
            if !f.is_finite() {
                return Err(format!("the Float {f} has no JSON form"));
            }
            J::from(*f)
        }
        Value::String(s) => J::String(s.to_string()),
        Value::Bool(b) => J::Bool(*b),
        Value::Nothing | Value::OptionNone => J::Null,
        Value::OptionSome(inner) => encode_public_json(&inner.get())?,
        Value::Struct(s) => {
            let mut object = Map::new();
            let mut failure = None;
            s.with_fields(|fields| {
                for (name, field) in fields {
                    match encode_public_json(field) {
                        Ok(encoded) => {
                            object.insert(name.clone(), encoded);
                        }
                        Err(reason) => {
                            failure.get_or_insert(reason);
                        }
                    }
                }
            });
            if let Some(reason) = failure {
                return Err(reason);
            }
            J::Object(object)
        }
        Value::List(items) => J::Array(
            items.iter_cloned().iter().map(encode_public_json).collect::<Result<_, _>>()?,
        ),
        Value::Map(map) => {
            let entries = map.entries_cloned();
            if entries.iter().all(|(key, _)| matches!(key, Value::String(_))) {
                let mut object = Map::new();
                for (key, item) in &entries {
                    if let Value::String(key) = key {
                        object.insert(key.to_string(), encode_public_json(item)?);
                    }
                }
                J::Object(object)
            } else {
                J::Array(
                    entries
                        .iter()
                        .map(|(key, item)| {
                            Ok(J::Array(vec![encode_public_json(key)?, encode_public_json(item)?]))
                        })
                        .collect::<Result<_, String>>()?,
                )
            }
        }
        Value::Enum(variant) => json!({
            "tag": "variant",
            "type": variant.type_name(),
            "variant": variant.variant_name(),
            "fields": variant
                .fields_cloned()
                .iter()
                .map(encode_public_json)
                .collect::<Result<Vec<_>, _>>()?,
        }),
        // `tag` repeats which key is present. The contract schema allows it,
        // and existing clients branch on it.
        Value::ResultOk(inner) => json!({ "tag": "ok", "ok": encode_public_json(&inner.get())? }),
        Value::ResultErr(inner) => json!({ "tag": "err", "err": encode_public_json(&inner.get())? }),
        Value::Grounded(grounded) => {
            let sources: Vec<J> = grounded
                .provenance
                .entries
                .iter()
                .map(|entry| {
                    json!({
                        "kind": entry.kind.label(),
                        "name": entry.name,
                        "timestamp_ms": entry.timestamp_ms,
                    })
                })
                .collect();
            json!({ "value": encode_public_json(&grounded.inner.get())?, "sources": sources })
        }
        Value::JsonValue(json) => (**json).clone(),
        Value::Weak(_) => return Err("a weak reference has no JSON form".into()),
        Value::Closure(_) => return Err("a function has no JSON form".into()),
        Value::DbHandle(_) => return Err("a database handle has no JSON form".into()),
        Value::JsonBuilder(_) => {
            return Err("a JSON builder has no JSON form; call json_object_finish".into())
        }
        Value::Stream(_) => return Err("a stream has no single JSON form".into()),
        Value::Partial(_) => return Err("a partial value has no JSON form".into()),
        Value::ResumeToken(_) => return Err("a resume token has no JSON form".into()),
    })
}

#[cfg(test)]
mod tests;
