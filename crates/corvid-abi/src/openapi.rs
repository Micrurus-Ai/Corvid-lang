//! OpenAPI 3.1 projection of the Application Contract (slice 51b).
//!
//! Any standard OpenAPI tool — client generators, Swagger UI, Postman
//! — consumes this without knowing anything about Corvid. It is a
//! pure transform of [`crate::app_contract::ApplicationContract`]:
//! routes become paths, public types become `components/schemas`
//! (JSON Schema, with field refinements as value constraints), and
//! the two artifacts reference the same schemas by name.
//!
//! The AI-native capabilities that OpenAPI cannot express (streaming
//! events, approvals, grounding, confidence) live in the companion
//! `corvid-ai.json` (slice 51c); this file stays a clean, standard
//! OpenAPI document so ordinary tooling never trips on Corvid-specific
//! shapes.

use crate::app_contract::{ApplicationContract, ContractField, ContractType};
use serde_json::{json, Map, Value};

/// Project the application contract to an OpenAPI 3.1 document.
pub fn emit_openapi(contract: &ApplicationContract) -> Value {
    let mut schemas = Map::new();
    for ty in &contract.types {
        schemas.insert(ty.name.clone(), schema_for_type(ty));
    }

    let mut paths = Map::new();
    for route in &contract.routes {
        let mut operation = Map::new();

        // Path + query parameters.
        let mut parameters = Vec::new();
        for p in &route.path_params {
            parameters.push(json!({
                "name": p.name,
                "in": "path",
                "required": true,
                "schema": schema_for_type_name(&p.type_name),
            }));
        }
        if let Some(query) = &route.query_type {
            // A query struct's fields become individual query params
            // via a schema reference; OpenAPI tools expand it.
            parameters.push(json!({
                "name": "query",
                "in": "query",
                "required": true,
                "schema": schema_for_type_name(query),
            }));
        }
        if !parameters.is_empty() {
            operation.insert("parameters".into(), Value::Array(parameters));
        }

        if let Some(body) = &route.body_type {
            operation.insert(
                "requestBody".into(),
                json!({
                    "required": true,
                    "content": {
                        "application/json": { "schema": schema_for_type_name(body) }
                    }
                }),
            );
        }

        operation.insert(
            "responses".into(),
            json!({
                "200": {
                    "description": "Success",
                    "content": {
                        "application/json": { "schema": schema_for_type_name(&route.response_type) }
                    }
                }
            }),
        );

        // A route requiring an approval-bearing effect advertises
        // security so tools prompt for auth.
        if !route.requires.is_empty() {
            operation.insert(
                "security".into(),
                json!([{ "corvidSession": route.requires }]),
            );
        }

        let method = route.method.to_lowercase();
        let entry = paths
            .entry(route.path.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(map) = entry {
            map.insert(method, Value::Object(operation));
        }
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Corvid application",
            "version": contract.compiler_version,
            "x-corvid-contract-version": contract.contract_version,
        },
        "paths": Value::Object(paths),
        "components": {
            "schemas": Value::Object(schemas),
            "securitySchemes": {
                "corvidSession": {
                    "type": "apiKey",
                    "in": "cookie",
                    "name": "corvid_session"
                }
            }
        }
    })
}

fn schema_for_type(ty: &ContractType) -> Value {
    if !ty.variants.is_empty() {
        // A sum type projects to an enum of its variant names (v1;
        // payload-carrying variants get oneOf in a later slice).
        return json!({ "type": "string", "enum": ty.variants });
    }
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in &ty.fields {
        properties.insert(field.name.clone(), schema_for_field(field));
        // Corvid fields are non-optional unless the type is Option<T>;
        // scalars/named are always required.
        if !field.type_name.starts_with("Option<") {
            required.push(Value::String(field.name.clone()));
        }
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

fn schema_for_field(field: &ContractField) -> Value {
    let mut schema = match scalar_schema(&field.type_name) {
        Some(s) => s,
        None => return schema_for_type_name(&field.type_name),
    };
    if let Value::Object(map) = &mut schema {
        if let Some(min) = field.minimum {
            map.insert("minimum".into(), json!(min));
        }
        if let Some(max) = field.maximum {
            map.insert("maximum".into(), json!(max));
        }
        if let Some(min) = field.min_length {
            map.insert("minLength".into(), json!(min));
        }
        if let Some(max) = field.max_length {
            map.insert("maxLength".into(), json!(max));
        }
    }
    schema
}

/// A JSON Schema fragment for a scalar type name, or `None` for a
/// non-scalar (named type / generic).
fn scalar_schema(name: &str) -> Option<Value> {
    Some(match name {
        "Int" => json!({ "type": "integer" }),
        "Float" => json!({ "type": "number" }),
        "String" | "TraceId" => json!({ "type": "string" }),
        "Bool" => json!({ "type": "boolean" }),
        "Nothing" => json!({ "type": "null" }),
        _ => return None,
    })
}

/// JSON Schema for an arbitrary type name: scalars inline, named
/// types as `$ref`, and the common generic wrappers unwrapped to the
/// shape a JSON client actually receives.
fn schema_for_type_name(name: &str) -> Value {
    if let Some(scalar) = scalar_schema(name) {
        return scalar;
    }
    if let Some((head, inner)) = split_generic(name) {
        return match head {
            // Grounded<T> is delivered as { value: T, sources: [...] }.
            "Grounded" => json!({
                "type": "object",
                "properties": {
                    "value": schema_for_type_name(inner),
                    "sources": { "type": "array", "items": { "type": "object" } }
                },
                "required": ["value"]
            }),
            // Stream<T> is an SSE stream of T; the JSON body schema is
            // the element type (the streaming nature is documented in
            // corvid-ai.json).
            "Stream" | "Tainted" => schema_for_type_name(inner),
            "Option" => {
                let mut s = schema_for_type_name(inner);
                if let Value::Object(map) = &mut s {
                    map.insert("nullable".into(), json!(true));
                }
                s
            }
            "List" => json!({ "type": "array", "items": schema_for_type_name(inner) }),
            "Result" => {
                // Result<T, E> → oneOf ok/err tagged shape.
                let (ok, err) = inner.split_once(',').unwrap_or((inner, "String"));
                json!({
                    "oneOf": [
                        { "type": "object", "properties": { "ok": schema_for_type_name(ok.trim()) }, "required": ["ok"] },
                        { "type": "object", "properties": { "err": schema_for_type_name(err.trim()) }, "required": ["err"] }
                    ]
                })
            }
            _ => json!({ "$ref": format!("#/components/schemas/{name}") }),
        };
    }
    // A bare named type references its component schema.
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

/// Split `Head<Inner>` into `("Head", "Inner")`; `None` if not
/// generic. `Inner` may itself contain commas (e.g. `Result<A, B>`).
fn split_generic(name: &str) -> Option<(&str, &str)> {
    let open = name.find('<')?;
    if !name.ends_with('>') {
        return None;
    }
    Some((&name[..open], &name[open + 1..name.len() - 1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_contract::{
        Capabilities, ContractCallable, ContractField, ContractParam, ContractRoute, ContractType,
        CONTRACT_VERSION,
    };

    fn sample() -> ApplicationContract {
        ApplicationContract {
            contract_version: CONTRACT_VERSION,
            compiler_version: "1.0.0".into(),
            generated_at: "now".into(),
            source_path: "app.cor".into(),
            types: vec![ContractType {
                name: "RefundRequest".into(),
                fields: vec![
                    ContractField {
                        name: "amount".into(),
                        type_name: "Float".into(),
                        minimum: None,
                        maximum: None,
                        min_length: None,
                        max_length: None,
                        ui: Default::default(),
                    },
                    ContractField {
                        name: "explanation".into(),
                        type_name: "String".into(),
                        minimum: None,
                        maximum: None,
                        min_length: Some(20),
                        max_length: Some(500),
                        ui: Default::default(),
                    },
                ],
                variants: vec![],
            }],
            routes: vec![ContractRoute {
                method: "POST".into(),
                path: "/refund".into(),
                path_params: vec![],
                query_type: None,
                body_type: Some("RefundRequest".into()),
                response_type: "Answer".into(),
                requires: vec!["issue_refund".into()],
            }],
            agents: vec![ContractCallable {
                name: "chat".into(),
                inputs: vec![ContractParam { name: "m".into(), type_name: "String".into() }],
                output_type: "Stream<String>".into(),
                capabilities: Capabilities::default(),
            }],
            prompts: vec![],
        }
    }

    #[test]
    fn projects_routes_types_and_constraints() {
        let openapi = emit_openapi(&sample());
        assert_eq!(openapi["openapi"], "3.1.0");
        // Route → path.
        let op = &openapi["paths"]["/refund"]["post"];
        assert_eq!(
            op["requestBody"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/RefundRequest"
        );
        assert!(op.get("security").is_some(), "approval-bearing route advertises security");
        // Refinement → JSON Schema constraint.
        let explanation =
            &openapi["components"]["schemas"]["RefundRequest"]["properties"]["explanation"];
        assert_eq!(explanation["minLength"], 20);
        assert_eq!(explanation["maxLength"], 500);
    }

    #[test]
    fn is_valid_json_and_declares_security_scheme() {
        let openapi = emit_openapi(&sample());
        assert_eq!(
            openapi["components"]["securitySchemes"]["corvidSession"]["type"],
            "apiKey"
        );
        // Round-trips as JSON.
        let s = serde_json::to_string(&openapi).unwrap();
        let _: Value = serde_json::from_str(&s).unwrap();
    }
}
