//! The decoder: JSON plus a Corvid type in, a built value or a located
//! [`JsonDecodeError`] out.

use crate::error::{parse_json_text, JsonDecodeError, UnknownFields};
use corvid_ir::{IrField, IrType};
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::Value as J;
use std::collections::HashMap;

/// Builds values of one representation as the decoder accepts JSON.
///
/// Every method receives values the decoder has already checked
/// against the type, so building cannot fail. When a later part of the
/// same JSON fails, the decoder hands each value it has built back
/// through [`ValueBuilder::discard`], so a representation that counts
/// references can release them.
pub trait ValueBuilder {
    type Value;

    fn int(&mut self, value: i64) -> Self::Value;
    fn float(&mut self, value: f64) -> Self::Value;
    fn string(&mut self, value: &str) -> Self::Value;
    fn bool(&mut self, value: bool) -> Self::Value;
    fn nothing(&mut self) -> Self::Value;
    /// `None` of the `Option` type `option`.
    fn none(&mut self, option: &Type) -> Self::Value;
    /// `Some(value)` of the `Option` type `option`.
    fn some(&mut self, option: &Type, value: Self::Value) -> Self::Value;
    fn list(&mut self, list: &Type, items: Vec<Self::Value>) -> Self::Value;
    fn map(&mut self, map: &Type, entries: Vec<(Self::Value, Self::Value)>) -> Self::Value;
    fn ok(&mut self, result: &Type, value: Self::Value) -> Self::Value;
    fn err(&mut self, result: &Type, value: Self::Value) -> Self::Value;
    /// A record, with `fields` in declaration order.
    fn record(&mut self, record: &IrType, fields: Vec<Self::Value>) -> Self::Value;
    /// Variant `index` of the sum type `sum`, with its fields in order.
    fn variant(&mut self, sum: &IrType, index: usize, fields: Vec<Self::Value>) -> Self::Value;
    /// A `JsonValue`: the JSON itself.
    fn json(&mut self, json: &J) -> Self::Value;
    /// Release a value built for a decode that then failed.
    fn discard(&mut self, value: Self::Value) {
        drop(value);
    }
}

/// Decode `json` into a value of type `expected` using the public mapping.
pub fn decode<B: ValueBuilder>(
    json: &J,
    expected: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    unknown_fields: UnknownFields,
    builder: &mut B,
) -> Result<B::Value, JsonDecodeError> {
    Decoder { types_by_id, unknown_fields, builder }.decode(json, expected, &mut String::new())
}

/// Decode JSON `text` into a value of type `expected`: the
/// `T.from_json(text)` entry point. Text that is not JSON fails at the
/// root path.
pub fn decode_text<B: ValueBuilder>(
    text: &str,
    expected: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    unknown_fields: UnknownFields,
    builder: &mut B,
) -> Result<B::Value, JsonDecodeError> {
    let json = parse_json_text(text)?;
    decode(&json, expected, types_by_id, unknown_fields, builder)
}

struct Decoder<'a, B: ValueBuilder> {
    types_by_id: &'a HashMap<DefId, &'a IrType>,
    unknown_fields: UnknownFields,
    builder: &'a mut B,
}

impl<B: ValueBuilder> Decoder<'_, B> {
    fn decode(&mut self, json: &J, expected: &Type, path: &mut String) -> Result<B::Value, JsonDecodeError> {
        match expected {
            Type::Int => match json {
                J::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        Ok(self.builder.int(i))
                    } else if n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0) {
                        Err(JsonDecodeError::new(path, "Int", format!("the out-of-range integer {n}")))
                    } else {
                        Err(JsonDecodeError::new(path, "Int", format!("the fractional number {n}")))
                    }
                }
                other => Err(mismatch(path, "Int", other)),
            },
            Type::Float => match json {
                J::Number(n) => match n.as_f64() {
                    Some(f) => Ok(self.builder.float(f)),
                    None => Err(JsonDecodeError::new(path, "Float", format!("the number {n}"))),
                },
                other => Err(mismatch(path, "Float", other)),
            },
            Type::String => match json {
                J::String(s) => Ok(self.builder.string(s)),
                other => Err(mismatch(path, "String", other)),
            },
            Type::Bool => match json {
                J::Bool(b) => Ok(self.builder.bool(*b)),
                other => Err(mismatch(path, "Bool", other)),
            },
            Type::Nothing => match json {
                J::Null => Ok(self.builder.nothing()),
                other => Err(mismatch(path, "null", other)),
            },
            Type::Option(inner) => match json {
                J::Null => Ok(self.builder.none(expected)),
                other => {
                    let value = self.decode(other, inner, path)?;
                    Ok(self.builder.some(expected, value))
                }
            },
            Type::List(element) => match json {
                J::Array(items) => {
                    let mut out = Vec::with_capacity(items.len());
                    for (index, item) in items.iter().enumerate() {
                        let mark = path.len();
                        path.push_str(&format!("[{index}]"));
                        match self.decode(item, element, path) {
                            Ok(value) => out.push(value),
                            Err(error) => return Err(self.discard_all(out, error)),
                        }
                        path.truncate(mark);
                    }
                    Ok(self.builder.list(expected, out))
                }
                other => Err(mismatch(path, &self.type_name(expected), other)),
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
            Type::JsonValue => Ok(self.builder.json(json)),
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
        &mut self,
        json: &J,
        expected: &Type,
        key: &Type,
        item: &Type,
        path: &mut String,
    ) -> Result<B::Value, JsonDecodeError> {
        let mut entries: Vec<(B::Value, B::Value)> = Vec::new();
        match (key, json) {
            (Type::String, J::Object(object)) => {
                for (name, raw) in object {
                    let mark = path.len();
                    push_key(path, name);
                    match self.decode(raw, item, path) {
                        Ok(value) => {
                            let key = self.builder.string(name);
                            entries.push((key, value));
                        }
                        Err(error) => return Err(self.discard_entries(entries, error)),
                    }
                    path.truncate(mark);
                }
            }
            (_, J::Array(pairs)) if !matches!(key, Type::String) => {
                for (index, pair) in pairs.iter().enumerate() {
                    let mark = path.len();
                    path.push_str(&format!("[{index}]"));
                    let J::Array(kv) = pair else {
                        let error = mismatch(path, "a [key, value] pair", pair);
                        return Err(self.discard_entries(entries, error));
                    };
                    if kv.len() != 2 {
                        let error = JsonDecodeError::new(
                            path,
                            "a [key, value] pair",
                            format!("an array of length {}", kv.len()),
                        );
                        return Err(self.discard_entries(entries, error));
                    }
                    path.push_str("[0]");
                    let decoded_key = match self.decode(&kv[0], key, path) {
                        Ok(value) => value,
                        Err(error) => return Err(self.discard_entries(entries, error)),
                    };
                    path.truncate(mark);
                    path.push_str(&format!("[{index}][1]"));
                    let decoded_item = match self.decode(&kv[1], item, path) {
                        Ok(value) => value,
                        Err(error) => {
                            self.builder.discard(decoded_key);
                            return Err(self.discard_entries(entries, error));
                        }
                    };
                    path.truncate(mark);
                    entries.push((decoded_key, decoded_item));
                }
            }
            (_, other) => return Err(mismatch(path, &self.type_name(expected), other)),
        }
        Ok(self.builder.map(expected, entries))
    }

    fn decode_result(
        &mut self,
        json: &J,
        expected: &Type,
        ok: &Type,
        err: &Type,
        path: &mut String,
    ) -> Result<B::Value, JsonDecodeError> {
        let J::Object(object) = json else {
            return Err(mismatch(path, &self.type_name(expected), json));
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
                let value = self.decode(&object["ok"], ok, path)?;
                self.builder.ok(expected, value)
            }
            (false, true) => {
                push_key(path, "err");
                let value = self.decode(&object["err"], err, path)?;
                self.builder.err(expected, value)
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

    fn decode_record(&mut self, json: &J, ir_type: &IrType, path: &mut String) -> Result<B::Value, JsonDecodeError> {
        let J::Object(object) = json else {
            return Err(mismatch(path, &ir_type.name, json));
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
        let mut fields = Vec::with_capacity(ir_type.fields.len());
        for field in &ir_type.fields {
            let mark = path.len();
            push_key(path, &field.name);
            let value = match object.get(&field.name) {
                Some(raw) => match self.decode(raw, &field.ty, path) {
                    Ok(value) => {
                        if let Some(error) = refinement_violation(path, field, raw) {
                            self.builder.discard(value);
                            return Err(self.discard_all(fields, error));
                        }
                        value
                    }
                    Err(error) => return Err(self.discard_all(fields, error)),
                },
                None if matches!(field.ty, Type::Option(_)) => self.builder.none(&field.ty),
                None => {
                    let error = JsonDecodeError::new(path, self.type_name(&field.ty), "no value (the field is missing)");
                    return Err(self.discard_all(fields, error));
                }
            };
            path.truncate(mark);
            fields.push(value);
        }
        Ok(self.builder.record(ir_type, fields))
    }

    fn decode_variant(&mut self, json: &J, ir_type: &IrType, path: &mut String) -> Result<B::Value, JsonDecodeError> {
        let J::Object(object) = json else {
            return Err(mismatch(path, &ir_type.name, json));
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
            Some(other) => return Err(mismatch(path, &format!("one of {names}"), other)),
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
            Some(other) => return Err(mismatch(path, &format!("an array of fields for `{variant_name}`"), other)),
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
            match self.decode(raw, &field.ty, path) {
                Ok(value) => {
                    if let Some(error) = refinement_violation(path, field, raw) {
                        self.builder.discard(value);
                        return Err(self.discard_all(fields, error));
                    }
                    fields.push(value);
                }
                Err(error) => return Err(self.discard_all(fields, error)),
            }
            path.truncate(field_mark);
        }
        path.truncate(mark);
        Ok(self.builder.variant(ir_type, index, fields))
    }

    fn discard_all(&mut self, values: Vec<B::Value>, error: JsonDecodeError) -> JsonDecodeError {
        for value in values {
            self.builder.discard(value);
        }
        error
    }

    fn discard_entries(&mut self, entries: Vec<(B::Value, B::Value)>, error: JsonDecodeError) -> JsonDecodeError {
        for (key, value) in entries {
            self.builder.discard(key);
            self.builder.discard(value);
        }
        error
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

/// A refinement failure for a field whose JSON `raw` already decoded.
/// Refinements constrain `Int` and `String` fields (through `Tainted`),
/// and the finding is the value as JSON, so `"ab"` keeps its quotes.
fn refinement_violation(path: &str, field: &IrField, raw: &J) -> Option<JsonDecodeError> {
    let refinement = field.refinement.as_ref()?;
    let mut ty = &field.ty;
    while let Type::Tainted(inner) = ty {
        ty = inner;
    }
    let admitted = match (ty, raw) {
        (Type::Int, J::Number(n)) => n.as_i64().map_or(true, |i| refinement.admits_int(i)),
        (Type::String, J::String(s)) => refinement.admits_text(s),
        _ => true,
    };
    (!admitted).then(|| JsonDecodeError::new(path, format!("a value {}", refinement.describe()), raw.to_string()))
}

fn mismatch(path: &str, expected: &str, found: &J) -> JsonDecodeError {
    JsonDecodeError::new(path, expected, json_kind(found))
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
