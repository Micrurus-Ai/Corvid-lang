//! The JSON Schema shape of each construct in the public mapping.
//!
//! Two generators describe the public form: the published OpenAPI
//! document, which names declared types as `$ref` components, and the
//! schema a prompt shows a model, which writes every type inline. They
//! differ only in how a child type is written, so the shape of each
//! construct (a nullable value, a `Result`, a sum type's variant, a
//! record, a list, a map) lives here once and both call it with the
//! children they built. A change to the public mapping changes both.
//!
//! Each function matches what [`crate::decode`] accepts: an `Option`
//! field may be absent, a `Result`'s and a variant's `tag` is optional
//! but checked when present, and a variant's `fields` is an array of
//! exactly its payload.

use serde_json::{json, Map, Value};

/// Allow `null` alongside `schema`: added to a simple `type`, otherwise
/// the schema and `null` as alternatives.
pub fn nullable(schema: Value) -> Value {
    match schema.get("type") {
        Some(Value::String(kind)) => {
            let mut schema = schema.clone();
            schema["type"] = json!([kind, "null"]);
            schema
        }
        _ => json!({ "anyOf": [schema, { "type": "null" }] }),
    }
}

/// `Result<T, E>`: `{"tag": "ok", "ok": T}` or `{"tag": "err", "err": E}`.
pub fn result(ok: Value, err: Value) -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": { "tag": { "const": "ok" }, "ok": ok },
                "required": ["ok"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": { "tag": { "const": "err" }, "err": err },
                "required": ["err"],
                "additionalProperties": false
            }
        ]
    })
}

/// One variant of the sum type `type_name`, carrying `payload` in order.
pub fn variant(type_name: &str, variant_name: &str, payload: Vec<Value>) -> Value {
    let count = payload.len();
    let mut fields = json!({ "type": "array", "minItems": count, "maxItems": count });
    let mut required = vec!["variant"];
    if count > 0 {
        fields["prefixItems"] = Value::Array(payload);
        required.push("fields");
    }
    json!({
        "type": "object",
        "required": required,
        "properties": {
            "tag": { "const": "variant" },
            "type": { "const": type_name },
            "variant": { "const": variant_name },
            "fields": fields
        }
    })
}

/// A sum type: exactly one of its variants.
pub fn sum(variants: Vec<Value>) -> Value {
    json!({ "oneOf": variants })
}

/// One field of a record: its name, its schema, and whether it may be
/// left out (an `Option` field).
pub struct RecordField {
    pub name: String,
    pub schema: Value,
    pub optional: bool,
}

/// A record: an object with exactly its fields, each required unless it
/// is an `Option`.
pub fn record(fields: Vec<RecordField>) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in fields {
        if !field.optional {
            required.push(Value::String(field.name.clone()));
        }
        properties.insert(field.name, field.schema);
    }
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), Value::Array(required));
    }
    schema.insert("additionalProperties".into(), json!(false));
    Value::Object(schema)
}

/// `List<T>`.
pub fn list(item: Value) -> Value {
    json!({ "type": "array", "items": item })
}

/// `Map<String, V>`: an object of values.
pub fn text_keyed_map(value: Value) -> Value {
    json!({ "type": "object", "additionalProperties": value })
}

/// `Map<K, V>` with any other key: an array of `[key, value]` pairs.
pub fn pair_map(key: Value, value: Value) -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "array",
            "prefixItems": [key, value],
            "minItems": 2,
            "maxItems": 2
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_simple_type_becomes_nullable_in_place_and_a_composite_as_an_alternative() {
        assert_eq!(nullable(json!({"type": "integer"})), json!({"type": ["integer", "null"]}));
        let reference = json!({"$ref": "#/components/schemas/Card"});
        assert_eq!(nullable(reference.clone()), json!({"anyOf": [reference, {"type": "null"}]}));
    }

    #[test]
    fn a_variant_without_payload_does_not_require_fields() {
        let none = variant("Status", "Open", vec![]);
        assert_eq!(none["required"], json!(["variant"]));
        assert_eq!(none["properties"]["fields"], json!({"type": "array", "minItems": 0, "maxItems": 0}));
        let some = variant("Status", "Closed", vec![json!({"type": "string"})]);
        assert_eq!(some["required"], json!(["variant", "fields"]));
        assert_eq!(some["properties"]["fields"]["prefixItems"], json!([{"type": "string"}]));
    }

    #[test]
    fn an_option_field_is_not_required() {
        let schema = record(vec![
            RecordField { name: "title".into(), schema: json!({"type": "string"}), optional: false },
            RecordField { name: "note".into(), schema: json!({"type": ["string", "null"]}), optional: true },
        ]);
        assert_eq!(schema["required"], json!(["title"]));
        assert_eq!(schema["additionalProperties"], json!(false));
    }
}
