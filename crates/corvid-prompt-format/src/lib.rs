//! JSON Schema generation for Corvid prompt return types.
//!
//! `schema_for(&Type, &types_by_id) -> serde_json::Value` builds the
//! JSON Schema fragment that LLM adapters embed in structured-output
//! requests (Anthropic via `tool_use`, OpenAI via
//! `response_format: json_schema`, future providers via their own
//! dialects). The schema is the language's contract with the model:
//! it tells the model what shape Corvid expects back.
//!
//! The schema describes the public JSON form, the one `T.to_json()`
//! writes and HTTP bodies use (`corvid_public_json`): an `Option` is
//! its value or `null`, a sum type is
//! `{"tag": "variant", "type": …, "variant": …, "fields": [...]}`. The
//! shape of each construct comes from `corvid_public_json::schema`,
//! which the published OpenAPI document shares, so what a model is
//! shown and what a frontend is shown cannot drift. Every type is
//! written inline (no `$ref`s), which providers' structured-output APIs
//! accept best.
//!
//! This logic used to live in `corvid-vm` because the interpreter
//! happened to need it first. It moved into its own crate so the
//! native code generator can reuse it without depending on the
//! interpreter, and so future providers' schema dialects have an
//! obvious place to live.
//!
//! The schema generator depends on `corvid-types` (the `Type` enum),
//! `corvid-ir` (struct field metadata), and `corvid-resolve`
//! (`DefId`). It does not depend on `corvid-runtime` — the runtime
//! crate stays type-agnostic by design.

use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_public_json::schema;
use corvid_types::Type;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Build a JSON Schema (Draft 2020-12 compatible subset) for `ty`.
///
/// `types_by_id` is consulted for struct types so the schema includes
/// nested object definitions inline (no `$ref`s — keeps things simple
/// and matches what providers' structured-output APIs accept best).
pub fn schema_for(ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> Value {
    schema_for_inner(ty, types_by_id, &mut Vec::new())
}

/// JSON Schema for the compiler-known `JsonError` record. Prompt
/// schemas and the published OpenAPI document both use this one.
pub fn json_error_schema() -> Value {
    let fields = corvid_types::json_error_fields();
    let properties: serde_json::Map<String, Value> =
        fields.iter().map(|(name, _)| (name.to_string(), json!({ "type": "string" }))).collect();
    let required: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn schema_for_inner(
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    visiting: &mut Vec<DefId>,
) -> Value {
    match ty {
        Type::Int => json!({ "type": "integer" }),
        Type::Float => json!({ "type": "number" }),
        Type::String => json!({ "type": "string" }),
        Type::Bool => json!({ "type": "boolean" }),
        Type::Nothing => json!({ "type": "null" }),
        Type::Map(key, value) => {
            if matches!(**key, Type::String) {
                schema::text_keyed_map(schema_for_inner(value, types_by_id, visiting))
            } else {
                schema::pair_map(
                    schema_for_inner(key, types_by_id, visiting),
                    schema_for_inner(value, types_by_id, visiting),
                )
            }
        }
        Type::List(elem) => schema::list(schema_for_inner(elem, types_by_id, visiting)),
        Type::Stream(inner) => schema_for_inner(inner, types_by_id, visiting),
        Type::Option(inner) => schema::nullable(schema_for_inner(inner, types_by_id, visiting)),
        Type::Result(ok, err) => schema::result(
            schema_for_inner(ok, types_by_id, visiting),
            schema_for_inner(err, types_by_id, visiting),
        ),
        Type::Weak(inner, _) => json!({
            "type": "object",
            "properties": {
                "tag": { "const": "weak" },
                "value": schema_for_inner(inner, types_by_id, visiting),
            },
            "required": ["tag", "value"],
            "additionalProperties": false,
        }),
        Type::Grounded(inner) => schema_for_inner(inner, types_by_id, visiting),
        Type::Tainted(inner) => schema_for_inner(inner, types_by_id, visiting),
        Type::Partial(inner) => partial_schema(inner, types_by_id, visiting),
        Type::ResumeToken(inner) => resume_token_schema(inner, types_by_id, visiting),
        Type::Struct(def_id) => {
            // Cycle guard: if we're already building this struct's schema
            // higher up the stack, emit an empty object placeholder. The
            // type system doesn't actually permit recursive types in
            // v0.5, so this is defensive only.
            if visiting.contains(def_id) {
                return json!({ "type": "object" });
            }
            let Some(ir_type) = types_by_id.get(def_id).copied() else {
                return json!({ "type": "object" });
            };
            visiting.push(*def_id);
            let schema = if !ir_type.variants.is_empty() {
                schema::sum(
                    ir_type
                        .variants
                        .iter()
                        .map(|variant| {
                            let payload = variant
                                .fields
                                .iter()
                                .map(|field| schema_for_inner(&field.ty, types_by_id, visiting))
                                .collect();
                            schema::variant(&ir_type.name, &variant.name, payload)
                        })
                        .collect(),
                )
            } else {
                schema::record(
                    ir_type
                        .fields
                        .iter()
                        .map(|field| schema::RecordField {
                            name: field.name.clone(),
                            schema: schema_for_inner(&field.ty, types_by_id, visiting),
                            optional: matches!(field.ty, Type::Option(_)),
                        })
                        .collect(),
                )
            };
            visiting.pop();
            schema
        }
        // A conflict has the sum-type shape, one alternative per case it
        // can be.
        Type::Conflict(row, cases) => schema::sum(
            cases
                .cases()
                .map(|case| {
                    let payload = case.field().map(|_| schema_for_inner(row, types_by_id, visiting)).into_iter().collect();
                    schema::variant("Conflict", case.name(), payload)
                })
                .collect(),
        ),
        Type::ImportedStruct(_) | Type::RouteParams(_) => json!({ "type": "object" }),
        // `Function` and `Unknown` shouldn't appear as prompt return
        // types in well-typed programs. Emit a permissive schema so the
        // adapter doesn't fail catastrophically; the type checker is the
        // real backstop.
        Type::Function { .. } => json!({}),
        // `TraceId` also shouldn't appear as a prompt return type;
        // if it does, fall back to string (traces are path-backed).
        Type::TraceId => json!({ "type": "string" }),
        // Phase 33S3a — `DbHandle` is structurally not a prompt
        // return type: it carries an Arc<rusqlite::Connection> the
        // model could never produce. Emit an empty permissive
        // schema so a hypothetical malformed prompt declaration
        // doesn't crash the format layer; the typechecker is the
        // real backstop and will surface the misuse upstream.
        Type::DbHandle => json!({}),
        // Phase 33R5b-a — `JsonValue` could conceivably be a
        // prompt return type (an LLM that emits structured JSON)
        // but the schema layer doesn't constrain its shape — by
        // design JsonValue is the dynamic-JSON path. Emit a
        // permissive schema; downstream prompt validation can
        // tighten if the prompt declares a typed shape.
        Type::JsonValue => json!({}),
        // Phase 33R5b-a — `JsonBuilder` is process-internal and
        // shouldn't appear as a prompt return type; emit a
        // permissive schema and let the typechecker enforce.
        Type::JsonBuilder => json!({}),
        Type::JsonError => json_error_schema(),
        // B03 — a `RegexMatch` points into one text the program
        // matched; a model has no such text to point into, so it is
        // not a prompt return type. Permissive, like `DbHandle`.
        Type::RegexMatch => json!({}),
        // A model never produces or receives a secret: the checker
        // refuses a `Secret` in a prompt's parameters and return type.
        // Should one reach a schema anyway, nothing validates against it.
        Type::Secret => json!({ "not": {} }),
        // B06 — a model reads and writes these as their exact text, with
        // the schema every public projection shares, so an amount never
        // passes through a float on its way to or from a model.
        // `Bytes` likewise crosses as its base64url text.
        Type::Decimal
        | Type::Duration
        | Type::Instant
        | Type::TimeZone
        | Type::ZonedTime
        | Type::Currency
        | Type::Locale
        | Type::Bytes => {
            corvid_public_json::text_scalar::json_schema(ty).expect("a text-scalar type has a schema")
        }
        // Slice 51f — `Upload<Format>` and `Page<Item>` are
        // HTTP-boundary types, not prompt return types: an upload is
        // received bytes and a page is assembled by the server, so
        // neither is something a model produces. Emit permissive
        // schemas (an upload as a string handle, a page as an array of
        // its element schema); the typechecker is the real backstop.
        Type::Upload(_) => json!({ "type": "string" }),
        // The checker keeps a connector `Response` out of every prompt, so
        // no schema ever describes one; an empty schema keeps this total.
        Type::Response(_) => json!({}),
        Type::Page(inner) => schema::list(schema_for_inner(inner, types_by_id, visiting)),
        Type::Unknown => json!({}),
    }
}

fn resume_token_schema(
    inner: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    visiting: &mut Vec<DefId>,
) -> Value {
    json!({
        "type": "object",
        "properties": {
            "tag": { "const": "resume_token" },
            "prompt": { "type": "string" },
            "args": { "type": "array", "items": {} },
            "delivered": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "value": schema_for_inner(inner, types_by_id, visiting),
                        "cost": { "type": "number" },
                        "confidence": { "type": "number" },
                        "tokens": { "type": "integer" },
                    },
                    "required": ["value"],
                    "additionalProperties": false,
                }
            },
            "provider_session": { "type": ["string", "null"] },
        },
        "required": ["tag", "prompt", "delivered"],
        "additionalProperties": false,
    })
}

fn partial_schema(
    inner: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    visiting: &mut Vec<DefId>,
) -> Value {
    let Type::Struct(def_id) = inner else {
        return json!({ "type": "object" });
    };
    let Some(ir_type) = types_by_id.get(def_id).copied() else {
        return json!({ "type": "object" });
    };

    let mut properties = serde_json::Map::new();
    for field in &ir_type.fields {
        let field_schema = schema_for_inner(&field.ty, types_by_id, visiting);
        properties.insert(
            field.name.clone(),
            json!({
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "tag": { "const": "complete" },
                            "value": field_schema,
                        },
                        "required": ["tag", "value"],
                        "additionalProperties": false,
                    },
                    {
                        "type": "object",
                        "properties": {
                            "tag": { "const": "streaming" },
                        },
                        "required": ["tag"],
                        "additionalProperties": false,
                    }
                ]
            }),
        );
    }

    json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::Span;
    use corvid_ir::{IrField, IrType as IrT};

    fn empty() -> HashMap<DefId, &'static IrT> {
        HashMap::new()
    }

    #[test]
    fn primitives_match_json_schema_types() {
        let by_id = empty();
        assert_eq!(schema_for(&Type::Int, &by_id), json!({"type": "integer"}));
        assert_eq!(schema_for(&Type::Float, &by_id), json!({"type": "number"}));
        assert_eq!(schema_for(&Type::String, &by_id), json!({"type": "string"}));
        assert_eq!(schema_for(&Type::Bool, &by_id), json!({"type": "boolean"}));
        assert_eq!(schema_for(&Type::Nothing, &by_id), json!({"type": "null"}));
    }

    #[test]
    fn list_emits_array_with_items() {
        let by_id = empty();
        let s = schema_for(&Type::List(Box::new(Type::String)), &by_id);
        assert_eq!(s, json!({"type": "array", "items": {"type": "string"}}));
    }

    #[test]
    fn struct_emits_object_with_required_fields() {
        let id = DefId(11);
        let ir_type: IrT = IrT {
            variants: Vec::new(),
            id,
            name: "Decision".into(),
            fields: vec![
                IrField {
                    name: "should_refund".into(),
                    ty: Type::Bool,
                    refinement: None,
                    span: Span::new(0, 0),
                },
                IrField {
                    name: "reason".into(),
                    ty: Type::String,
                    refinement: None,
                    span: Span::new(0, 0),
                },
            ],
            span: Span::new(0, 0),
        };
        // Leak via Box::leak so the &'static reference works in this
        // narrow test scope without lifetime gymnastics.
        let leaked: &'static IrT = Box::leak(Box::new(ir_type));
        let mut by_id: HashMap<DefId, &IrT> = HashMap::new();
        by_id.insert(id, leaked);
        let s = schema_for(&Type::Struct(id), &by_id);
        let obj = s.as_object().unwrap();
        assert_eq!(obj["type"], "object");
        assert_eq!(obj["additionalProperties"], false);
        assert_eq!(
            obj["properties"]["should_refund"],
            json!({"type": "boolean"})
        );
        assert_eq!(obj["properties"]["reason"], json!({"type": "string"}));
        let required = obj["required"].as_array().unwrap();
        assert!(required.contains(&json!("should_refund")));
        assert!(required.contains(&json!("reason")));
    }

    #[test]
    fn nested_struct_inlines_subschema() {
        let inner_id = DefId(20);
        let outer_id = DefId(21);
        let inner: &'static IrT = Box::leak(Box::new(IrT {
            variants: Vec::new(),
            id: inner_id,
            name: "Order".into(),
            fields: vec![IrField {
                name: "id".into(),
                ty: Type::String,
                refinement: None,
                span: Span::new(0, 0),
            }],
            span: Span::new(0, 0),
        }));
        let outer: &'static IrT = Box::leak(Box::new(IrT {
            variants: Vec::new(),
            id: outer_id,
            name: "Wrap".into(),
            fields: vec![IrField {
                name: "order".into(),
                ty: Type::Struct(inner_id),
                refinement: None,
                span: Span::new(0, 0),
            }],
            span: Span::new(0, 0),
        }));
        let mut by_id: HashMap<DefId, &IrT> = HashMap::new();
        by_id.insert(inner_id, inner);
        by_id.insert(outer_id, outer);
        let s = schema_for(&Type::Struct(outer_id), &by_id);
        let order_schema = &s["properties"]["order"];
        assert_eq!(order_schema["type"], "object");
        assert_eq!(order_schema["properties"]["id"], json!({"type": "string"}));
    }

    /// The schema describes the public form the reply is read in: an
    /// `Option` is the value or `null` and may be left out of a record, a
    /// `Result` is `{"tag": "ok", "ok": …}`, and a sum type names its
    /// variants with `tag`, `type` and positional `fields`.
    #[test]
    fn schemas_describe_the_public_json_form() {
        let by_id = empty();
        assert_eq!(
            schema_for(&Type::Option(Box::new(Type::Int)), &by_id),
            json!({"type": ["integer", "null"]})
        );
        let result = schema_for(&Type::Result(Box::new(Type::Int), Box::new(Type::String)), &by_id);
        assert_eq!(result["oneOf"][0]["properties"]["ok"], json!({"type": "integer"}));
        assert_eq!(result["oneOf"][0]["required"], json!(["ok"]));
        assert_eq!(result["oneOf"][1]["properties"]["err"], json!({"type": "string"}));

        let record_id = DefId(41);
        let sum_id = DefId(42);
        let sum: &'static IrT = Box::leak(Box::new(IrT {
            id: sum_id,
            name: "Severity".into(),
            fields: Vec::new(),
            variants: vec![
                corvid_ir::IrEnumVariant { name: "Low".into(), fields: Vec::new(), span: Span::new(0, 0) },
                corvid_ir::IrEnumVariant {
                    name: "High".into(),
                    fields: vec![IrField { name: "reason".into(), ty: Type::String, refinement: None, span: Span::new(0, 0) }],
                    span: Span::new(0, 0),
                },
            ],
            span: Span::new(0, 0),
        }));
        let record: &'static IrT = Box::leak(Box::new(IrT {
            id: record_id,
            name: "Verdict".into(),
            variants: Vec::new(),
            fields: vec![
                IrField { name: "summary".into(), ty: Type::String, refinement: None, span: Span::new(0, 0) },
                IrField {
                    name: "assignee".into(),
                    ty: Type::Option(Box::new(Type::String)),
                    refinement: None,
                    span: Span::new(0, 0),
                },
                IrField { name: "severity".into(), ty: Type::Struct(sum_id), refinement: None, span: Span::new(0, 0) },
            ],
            span: Span::new(0, 0),
        }));
        let mut by_id: HashMap<DefId, &IrT> = HashMap::new();
        by_id.insert(record_id, record);
        by_id.insert(sum_id, sum);
        let s = schema_for(&Type::Struct(record_id), &by_id);
        assert_eq!(s["required"], json!(["summary", "severity"]));
        assert_eq!(s["properties"]["assignee"], json!({"type": ["string", "null"]}));
        let high = &s["properties"]["severity"]["oneOf"][1];
        assert_eq!(high["properties"]["tag"], json!({"const": "variant"}));
        assert_eq!(high["properties"]["type"], json!({"const": "Severity"}));
        assert_eq!(high["properties"]["variant"], json!({"const": "High"}));
        assert_eq!(high["properties"]["fields"]["prefixItems"], json!([{"type": "string"}]));
        assert_eq!(high["required"], json!(["variant", "fields"]));
        assert_eq!(s["properties"]["severity"]["oneOf"][0]["required"], json!(["variant"]));
    }

    #[test]
    fn partial_struct_schema_marks_each_field_complete_or_streaming() {
        let id = DefId(30);
        let ir_type: &'static IrT = Box::leak(Box::new(IrT {
            variants: Vec::new(),
            id,
            name: "Plan".into(),
            fields: vec![
                IrField {
                    name: "title".into(),
                    ty: Type::String,
                    refinement: None,
                    span: Span::new(0, 0),
                },
                IrField {
                    name: "ready".into(),
                    ty: Type::Bool,
                    refinement: None,
                    span: Span::new(0, 0),
                },
            ],
            span: Span::new(0, 0),
        }));
        let mut by_id: HashMap<DefId, &IrT> = HashMap::new();
        by_id.insert(id, ir_type);

        let s = schema_for(&Type::Partial(Box::new(Type::Struct(id))), &by_id);
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"], false);
        assert!(s.get("required").is_none());

        let title_states = s["properties"]["title"]["oneOf"].as_array().unwrap();
        assert_eq!(title_states[0]["properties"]["tag"], json!({"const": "complete"}));
        assert_eq!(title_states[0]["properties"]["value"], json!({"type": "string"}));
        assert_eq!(title_states[1]["properties"]["tag"], json!({"const": "streaming"}));

        let ready_states = s["properties"]["ready"]["oneOf"].as_array().unwrap();
        assert_eq!(ready_states[0]["properties"]["value"], json!({"type": "boolean"}));
    }

    #[test]
    fn imported_struct_falls_back_to_permissive_object() {
        // Imported structs don't have inline IR field metadata
        // available at schema-build time. Emit a permissive object
        // schema so structured-output validators don't fail; the
        // type checker is the real backstop for cross-module shape
        // mismatches.
        let imported = corvid_types::types::ImportedStructType {
            module_path: "other_module.cor".into(),
            def_id: DefId(99),
            name: "External".into(),
        };
        let by_id = empty();
        let s = schema_for(&Type::ImportedStruct(imported), &by_id);
        assert_eq!(s, json!({ "type": "object" }));
    }
}
