//! The public JSON mapping between Corvid values and ordinary JSON.
//!
//! This is the shape Corvid promises to code outside the program:
//! HTTP request and response bodies, the published OpenAPI schemas,
//! and `T.from_json(text)` / `value.to_json()`. It follows the JSON
//! conventions other ecosystems use:
//!
//! | Corvid                 | JSON                                              |
//! | ---------------------- | ------------------------------------------------- |
//! | `Int`                  | integer (a fraction or out-of-range value fails)  |
//! | `Float`                | number (integers are accepted)                    |
//! | `String`, `Bool`       | string, boolean                                   |
//! | `Nothing`              | `null`                                            |
//! | record                 | object; an `Option` field may be `null` or absent |
//! | `Option<T>`            | the `T` value, or `null` for `None`               |
//! | `List<T>`              | array                                             |
//! | `Map<String, V>`       | object                                            |
//! | `Map<K, V>` (other K)  | array of `[key, value]` pairs                     |
//! | `Result<T, E>`         | `{"tag": "ok", "ok": T}` or `{"tag": "err", "err": E}`; `tag` is optional on input |
//! | sum type               | `{"tag": "variant", "type": …, "variant": …, "fields": [...]}` |
//! | `Grounded<T>`          | `{"value": T, "sources": [...]}` (encode only)    |
//! | `JsonValue`            | the JSON itself                                   |
//!
//! Prompt structured output, tool arguments and recorded traces use
//! the internal envelope in [`crate::conv`] instead. That format is
//! lossless for every runtime value (including nested options and
//! provenance), matches the schema sent to model providers, and is
//! what existing recordings replay against, so it stays separate.
//! The two mappings agree on records, lists, scalars and sum types;
//! they differ only on `Option`, `Result` and `Grounded`.
//!
//! Decoding never fabricates provenance: a `Grounded<T>` target is
//! rejected, because JSON text cannot prove where a value came from.

use crate::conv::{check_field_refinement, ConvError};
use crate::value::{BoxedValue, EnumValue, ListValue, MapValue, StructValue, Value};
use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::{json, Map, Value as J};
use std::collections::HashMap;
use std::sync::Arc;

/// How a record decode treats object keys that are not declared fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownFields {
    /// Skip undeclared keys. The default for `T.from_json`, because
    /// external APIs add fields without notice.
    Ignore,
    /// Fail on the first undeclared key. Route bodies use this, as
    /// the published contract declares `additionalProperties: false`.
    Reject,
}

/// A decode failure located at a JSON path such as `items[2].price`.
/// The root is the empty path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonDecodeError {
    pub path: String,
    pub expected: String,
    pub found: String,
}

impl JsonDecodeError {
    fn new(path: &str, expected: impl Into<String>, found: impl Into<String>) -> Self {
        Self { path: path.to_string(), expected: expected.into(), found: found.into() }
    }

    /// A one-line message for people: ``at `tag`: expected String, found number``.
    pub fn message(&self) -> String {
        if self.path.is_empty() {
            format!("expected {}, found {}", self.expected, self.found)
        } else {
            format!("at `{}`: expected {}, found {}", self.path, self.expected, self.found)
        }
    }

    /// The structured form returned to HTTP clients.
    pub fn to_json(&self) -> J {
        json!({ "path": self.path, "expected": self.expected, "found": self.found })
    }
}

impl std::fmt::Display for JsonDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for JsonDecodeError {}

/// Decode `json` into a value of type `expected` using the public mapping.
pub fn decode_public_json(
    json: &J,
    expected: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    unknown_fields: UnknownFields,
) -> Result<Value, JsonDecodeError> {
    Decoder { types_by_id, unknown_fields }.decode(json, expected, &mut String::new())
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

struct Decoder<'a> {
    types_by_id: &'a HashMap<DefId, &'a IrType>,
    unknown_fields: UnknownFields,
}

impl Decoder<'_> {
    fn decode(&self, json: &J, expected: &Type, path: &mut String) -> Result<Value, JsonDecodeError> {
        match expected {
            Type::Int => match json {
                J::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        Ok(Value::Int(i))
                    } else if n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0) {
                        Err(JsonDecodeError::new(path, "Int", format!("the out-of-range integer {n}")))
                    } else {
                        Err(JsonDecodeError::new(path, "Int", format!("the fractional number {n}")))
                    }
                }
                other => Err(self.mismatch(path, "Int", other)),
            },
            Type::Float => match json {
                J::Number(n) => n
                    .as_f64()
                    .map(Value::Float)
                    .ok_or_else(|| JsonDecodeError::new(path, "Float", format!("the number {n}"))),
                other => Err(self.mismatch(path, "Float", other)),
            },
            Type::String => match json {
                J::String(s) => Ok(Value::String(Arc::from(s.as_str()))),
                other => Err(self.mismatch(path, "String", other)),
            },
            Type::Bool => match json {
                J::Bool(b) => Ok(Value::Bool(*b)),
                other => Err(self.mismatch(path, "Bool", other)),
            },
            Type::Nothing => match json {
                J::Null => Ok(Value::Nothing),
                other => Err(self.mismatch(path, "null", other)),
            },
            Type::Option(inner) => match json {
                J::Null => Ok(Value::OptionNone),
                other => Ok(Value::OptionSome(BoxedValue::new(self.decode(other, inner, path)?))),
            },
            Type::List(element) => match json {
                J::Array(items) => {
                    let mut out = Vec::with_capacity(items.len());
                    for (index, item) in items.iter().enumerate() {
                        let mark = path.len();
                        path.push_str(&format!("[{index}]"));
                        out.push(self.decode(item, element, path)?);
                        path.truncate(mark);
                    }
                    Ok(Value::List(ListValue::new(out)))
                }
                other => Err(self.mismatch(path, &self.type_name(expected), other)),
            },
            Type::Map(key, item) => self.decode_map(json, expected, key, item, path),
            Type::Result(ok, err) => self.decode_result(json, expected, ok, err, path),
            // `Tainted<T>` is a `T` to the checker; taint is tracked
            // statically, not carried in the JSON.
            Type::Tainted(inner) => self.decode(json, inner, path),
            Type::Struct(def_id) | Type::ImportedStruct(corvid_types::types::ImportedStructType { def_id, .. }) => {
                let ir_type = self.types_by_id.get(def_id).copied().ok_or_else(|| {
                    JsonDecodeError::new(path, "a declared type", "an unknown type identifier")
                })?;
                if ir_type.variants.is_empty() {
                    self.decode_record(json, ir_type, path)
                } else {
                    self.decode_variant(json, ir_type, path)
                }
            }
            Type::JsonValue => Ok(Value::JsonValue(Arc::new(json.clone()))),
            Type::Grounded(_) => Err(JsonDecodeError::new(
                path,
                "a grounded value produced by a retrieval",
                "JSON text, which cannot carry verified provenance",
            )),
            other => Err(JsonDecodeError::new(
                path,
                format!("a type with a JSON form (`{}` has none)", self.type_name(other)),
                json_kind(json),
            )),
        }
    }

    fn decode_map(
        &self,
        json: &J,
        expected: &Type,
        key: &Type,
        item: &Type,
        path: &mut String,
    ) -> Result<Value, JsonDecodeError> {
        let mut entries = Vec::new();
        match (key, json) {
            (Type::String, J::Object(object)) => {
                for (name, raw) in object {
                    let mark = path.len();
                    push_key(path, name);
                    entries.push((Value::String(Arc::from(name.as_str())), self.decode(raw, item, path)?));
                    path.truncate(mark);
                }
            }
            (_, J::Array(pairs)) if !matches!(key, Type::String) => {
                for (index, pair) in pairs.iter().enumerate() {
                    let mark = path.len();
                    path.push_str(&format!("[{index}]"));
                    let J::Array(kv) = pair else {
                        return Err(self.mismatch(path, "a [key, value] pair", pair));
                    };
                    if kv.len() != 2 {
                        return Err(JsonDecodeError::new(
                            path,
                            "a [key, value] pair",
                            format!("an array of length {}", kv.len()),
                        ));
                    }
                    path.push_str("[0]");
                    let decoded_key = self.decode(&kv[0], key, path)?;
                    path.truncate(mark);
                    path.push_str(&format!("[{index}][1]"));
                    let decoded_item = self.decode(&kv[1], item, path)?;
                    path.truncate(mark);
                    entries.push((decoded_key, decoded_item));
                }
            }
            (_, other) => return Err(self.mismatch(path, &self.type_name(expected), other)),
        }
        Ok(Value::Map(MapValue::new(entries)))
    }

    fn decode_result(
        &self,
        json: &J,
        expected: &Type,
        ok: &Type,
        err: &Type,
        path: &mut String,
    ) -> Result<Value, JsonDecodeError> {
        let J::Object(object) = json else {
            return Err(self.mismatch(path, &self.type_name(expected), json));
        };
        let has_ok = object.contains_key("ok");
        let has_err = object.contains_key("err");
        let mark = path.len();
        if let Some(tag) = object.get("tag") {
            let expected_tag = if has_ok && !has_err { "\"ok\"" } else { "\"err\"" };
            let agrees = (has_ok && !has_err && tag == "ok") || (has_err && !has_ok && tag == "err");
            if !agrees && has_ok != has_err {
                push_key(path, "tag");
                let error = JsonDecodeError::new(path, expected_tag, tag.to_string());
                path.truncate(mark);
                return Err(error);
            }
        }
        if self.unknown_fields == UnknownFields::Reject {
            if let Some(extra) = object.keys().find(|k| !matches!(k.as_str(), "tag" | "ok" | "err")) {
                push_key(path, extra);
                let error = JsonDecodeError::new(path, "no field (a Result has only `ok` or `err`)", "a value");
                path.truncate(mark);
                return Err(error);
            }
        }
        let decoded = match (has_ok, has_err) {
            (true, false) => {
                push_key(path, "ok");
                Value::ResultOk(BoxedValue::new(self.decode(&object["ok"], ok, path)?))
            }
            (false, true) => {
                push_key(path, "err");
                Value::ResultErr(BoxedValue::new(self.decode(&object["err"], err, path)?))
            }
            _ => {
                return Err(JsonDecodeError::new(
                    path,
                    "an object with exactly one of `ok` or `err`",
                    if has_ok { "both `ok` and `err`" } else { "neither `ok` nor `err`" },
                ))
            }
        };
        path.truncate(mark);
        Ok(decoded)
    }

    fn decode_record(&self, json: &J, ir_type: &IrType, path: &mut String) -> Result<Value, JsonDecodeError> {
        let J::Object(object) = json else {
            return Err(self.mismatch(path, &ir_type.name, json));
        };
        if self.unknown_fields == UnknownFields::Reject {
            if let Some(extra) = object.keys().find(|k| !ir_type.fields.iter().any(|f| &f.name == *k)) {
                let mark = path.len();
                push_key(path, extra);
                let error = JsonDecodeError::new(path, format!("no field (`{}` declares no `{extra}`)", ir_type.name), "a value");
                path.truncate(mark);
                return Err(error);
            }
        }
        let mut fields = HashMap::with_capacity(ir_type.fields.len());
        for field in &ir_type.fields {
            let mark = path.len();
            push_key(path, &field.name);
            let value = match object.get(&field.name) {
                Some(raw) => self.decode(raw, &field.ty, path)?,
                None if matches!(field.ty, Type::Option(_)) => Value::OptionNone,
                None => return Err(JsonDecodeError::new(path, self.type_name(&field.ty), "no value (the field is missing)")),
            };
            check_field_refinement(&ir_type.name, field, &value).map_err(|e| refinement_error(path, e))?;
            path.truncate(mark);
            fields.insert(field.name.clone(), value);
        }
        Ok(Value::Struct(StructValue::new(ir_type.id, ir_type.name.clone(), fields)))
    }

    fn decode_variant(&self, json: &J, ir_type: &IrType, path: &mut String) -> Result<Value, JsonDecodeError> {
        let J::Object(object) = json else {
            return Err(self.mismatch(path, &ir_type.name, json));
        };
        for (key, required) in [("tag", "variant"), ("type", ir_type.name.as_str())] {
            if let Some(actual) = object.get(key) {
                if actual.as_str() != Some(required) {
                    let mark = path.len();
                    push_key(path, key);
                    let error = JsonDecodeError::new(path, format!("\"{required}\""), actual.to_string());
                    path.truncate(mark);
                    return Err(error);
                }
            }
        }
        let names = ir_type.variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", ");
        let mark = path.len();
        push_key(path, "variant");
        let variant_name = match object.get("variant") {
            Some(J::String(name)) => name.as_str(),
            Some(other) => return Err(self.mismatch(path, &format!("one of {names}"), other)),
            None => return Err(JsonDecodeError::new(path, format!("one of {names}"), "no value (the field is missing)")),
        };
        let Some((index, variant)) = ir_type.variants.iter().enumerate().find(|(_, v)| v.name == variant_name) else {
            return Err(JsonDecodeError::new(path, format!("one of {names}"), format!("\"{variant_name}\"")));
        };
        path.truncate(mark);
        if self.unknown_fields == UnknownFields::Reject {
            if let Some(extra) = object.keys().find(|k| !matches!(k.as_str(), "tag" | "type" | "variant" | "fields")) {
                push_key(path, extra);
                let error = JsonDecodeError::new(path, format!("no field (a `{}` variant has no `{extra}`)", ir_type.name), "a value");
                path.truncate(mark);
                return Err(error);
            }
        }
        push_key(path, "fields");
        let payload: &[J] = match object.get("fields") {
            Some(J::Array(items)) => items,
            None => &[],
            Some(other) => return Err(self.mismatch(path, &format!("an array of fields for `{variant_name}`"), other)),
        };
        if payload.len() != variant.fields.len() {
            return Err(JsonDecodeError::new(
                path,
                format!("{} fields for `{variant_name}`", variant.fields.len()),
                format!("{} fields", payload.len()),
            ));
        }
        let mut fields = Vec::with_capacity(payload.len());
        for (position, (raw, field)) in payload.iter().zip(&variant.fields).enumerate() {
            let field_mark = path.len();
            path.push_str(&format!("[{position}]"));
            let value = self.decode(raw, &field.ty, path)?;
            check_field_refinement(variant_name, field, &value).map_err(|e| refinement_error(path, e))?;
            path.truncate(field_mark);
            fields.push(value);
        }
        path.truncate(mark);
        Ok(Value::Enum(EnumValue::new(ir_type.id, ir_type.name.clone(), index as u32, variant.name.clone(), fields)))
    }

    fn mismatch(&self, path: &str, expected: &str, found: &J) -> JsonDecodeError {
        JsonDecodeError::new(path, expected, json_kind(found))
    }

    /// A type name for messages, naming records and sum types by their
    /// declaration (`Type::display_name` renders every record as `struct`).
    fn type_name(&self, ty: &Type) -> String {
        match ty {
            Type::Struct(def_id) => self
                .types_by_id
                .get(def_id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| ty.display_name()),
            Type::List(inner) => format!("List<{}>", self.type_name(inner)),
            Type::Option(inner) => format!("Option<{}>", self.type_name(inner)),
            Type::Map(k, v) => format!("Map<{}, {}>", self.type_name(k), self.type_name(v)),
            Type::Result(ok, err) => format!("Result<{}, {}>", self.type_name(ok), self.type_name(err)),
            other => other.display_name(),
        }
    }
}

fn push_key(path: &mut String, key: &str) {
    let simple = !key.is_empty()
        && key.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !key.starts_with(|c: char| c.is_ascii_digit());
    if simple {
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(key);
    } else {
        path.push_str(&format!("[{}]", J::String(key.to_string())));
    }
}

fn refinement_error(path: &str, error: ConvError) -> JsonDecodeError {
    match error {
        ConvError::RefinementViolated { refinement, got, .. } => {
            JsonDecodeError::new(path, format!("a value {refinement}"), got)
        }
        other => JsonDecodeError::new(path, "a valid value", other.to_string()),
    }
}

fn json_kind(json: &J) -> &'static str {
    match json {
        J::Null => "null",
        J::Bool(_) => "a boolean",
        J::Number(_) => "a number",
        J::String(_) => "a string",
        J::Array(_) => "an array",
        J::Object(_) => "an object",
    }
}


#[cfg(test)]
mod tests;
