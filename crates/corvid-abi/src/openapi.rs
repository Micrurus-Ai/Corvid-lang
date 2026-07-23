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

use crate::app_contract::{
    ApplicationContract, ContractField, ContractType, ContractUpload,
};
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
        // A cursor-paginated route takes an optional forward `cursor`.
        if let Some(pagination) = &route.pagination {
            if let Some(cursor) = &pagination.cursor_param {
                parameters.push(json!({
                    "name": cursor,
                    "in": "query",
                    "required": false,
                    "schema": { "type": "string" },
                    "description": "Opaque forward cursor from a prior page's next_cursor."
                }));
            }
        }
        if !parameters.is_empty() {
            operation.insert("parameters".into(), Value::Array(parameters));
        }

        if let Some(body) = &route.body_type {
            // A direct upload or a body carrying an upload field is
            // multipart/form-data; otherwise it is JSON. A direct
            // upload uses the route's explicit policy as its schema.
            let (media_type, schema) = if let Some(upload) = &route.upload {
                ("multipart/form-data", schema_for_upload(upload))
            } else if body_type_has_upload(body, contract) {
                ("multipart/form-data", schema_for_type_name(body))
            } else {
                ("application/json", schema_for_type_name(body))
            };
            operation.insert(
                "requestBody".into(),
                json!({
                    "required": true,
                    "content": {
                        media_type: { "schema": schema }
                    }
                }),
            );
        }

        let mut responses = Map::new();
        responses.insert(
            "200".into(),
            json!({
                "description": "Success",
                "content": {
                    "application/json": { "schema": schema_for_type_name(&route.response_type) }
                }
            }),
        );
        // A `Result<T, E>` route whose `E` is an error enum carrying
        // `@status(code)` variants projects one response per status the
        // enum can produce, so a standard client generates typed error
        // branches instead of a single opaque failure (slice 51e).
        for (status, schema) in error_status_responses(&route.response_type, contract) {
            responses.entry(status).or_insert(schema);
        }
        operation.insert("responses".into(), Value::Object(responses));

        // A route requiring an approval-bearing effect OR an auth
        // policy advertises security so tools prompt for auth. An auth
        // policy's roles/permissions ride as the session's scopes.
        let mut scopes: Vec<String> = route.requires.clone();
        if let Some(policy) = &route.policy {
            for role in &policy.roles {
                scopes.push(format!("role:{role}"));
            }
            for perm in &policy.permissions {
                scopes.push(format!("permission:{perm}"));
            }
            if policy.authenticated && scopes.is_empty() {
                // Authenticated with no finer scope still needs a session.
                scopes.push("authenticated".to_string());
            }
        }
        if !scopes.is_empty() {
            operation.insert("security".into(), json!([{ "corvidSession": scopes }]));
        }

        let method = route.method.to_lowercase();
        let entry = paths
            .entry(route.path.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(map) = entry {
            map.insert(method, Value::Object(operation));
        }
    }

    // Auto-exposed auth routes (slice 51h): a standard OpenAPI client
    // sees the sign-in / callback / logout / session surface derived
    // from the `identity` block.
    for identity in &contract.identities {
        for auth in &identity.auth_routes {
            let summary = match auth.purpose.as_str() {
                "login" => "Begin sign-in (Authorization Code + PKCE); redirects to the provider.",
                "callback" => "OAuth callback: verifies state/nonce and the ID-token signature, then issues a session.",
                "logout" => "Revoke the session and clear the session cookie.",
                "session" => "Return the current authenticated actor, or 401 if unauthenticated.",
                "link_start" => "Start linking this provider to the signed-in account (requires an active session); returns a pending link to confirm.",
                "link_confirm" => "Confirm ownership and complete the account link after authenticating the new provider. Never merges by email silently.",
                _ => "Auth route.",
            };
            let mut op = Map::new();
            op.insert("summary".into(), json!(summary));
            op.insert("tags".into(), json!(["auth"]));
            op.insert(
                "x-corvid-auth-safeguards".into(),
                json!(identity.safeguards),
            );
            if let Some(provider) = &auth.provider {
                op.insert("x-corvid-provider".into(), json!(provider));
            }
            let responses = match auth.purpose.as_str() {
                "login" | "callback" => json!({ "302": { "description": "Redirect" } }),
                "session" => json!({
                    "200": { "description": "The authenticated actor" },
                    "401": { "description": "Not authenticated" }
                }),
                "link_start" => json!({
                    "200": { "description": "A pending link to confirm" },
                    "401": { "description": "Not authenticated" }
                }),
                "link_confirm" => json!({
                    "200": { "description": "The account link was confirmed" },
                    "401": { "description": "Not authenticated" },
                    "409": { "description": "Ownership could not be proven; no merge performed" }
                }),
                _ => json!({ "204": { "description": "No Content" } }),
            };
            // Linking requires an active session.
            if matches!(auth.purpose.as_str(), "link_start" | "link_confirm") {
                op.insert(
                    "security".into(),
                    json!([{ "corvidSession": ["authenticated"] }]),
                );
            }
            op.insert("responses".into(), responses);
            let entry = paths
                .entry(auth.path.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(map) = entry {
                map.insert(auth.method.to_lowercase(), Value::Object(op));
            }
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
        // A sum type projects to an enum of its variant names; a
        // frontend reads the richer per-variant status/ui/payload from
        // the application contract (51e).
        let names: Vec<&str> = ty.variants.iter().map(|v| v.name.as_str()).collect();
        return json!({ "type": "string", "enum": names });
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
    // An `Upload<Format>` field is a binary string constrained to the
    // accepted MIME and max size — what a multipart form part carries.
    if let Some(upload) = &field.upload {
        return schema_for_upload(upload);
    }
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

fn schema_for_upload(upload: &ContractUpload) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), json!("string"));
    map.insert("format".into(), json!("binary"));
    if let Some(first) = upload.accepted_mime.first() {
        map.insert("contentMediaType".into(), json!(first));
    }
    if upload.accepted_mime.len() > 1 {
        map.insert("x-corvid-accepted-mime".into(), json!(upload.accepted_mime));
    }
    if let Some(max) = upload.max_bytes {
        map.insert("maxLength".into(), json!(max));
    }
    if let Some(days) = upload.retention_days {
        map.insert("x-corvid-retention-days".into(), json!(days));
    }
    Value::Object(map)
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
            // Upload<Format> is a binary string at the JSON boundary;
            // the accepted-MIME / size constraints attach where the
            // field is projected (schema_for_field).
            "Upload" => json!({ "type": "string", "format": "binary" }),
            // Page<Item> is the cursor-pagination envelope.
            "Page" => json!({
                "type": "object",
                "properties": {
                    "items": { "type": "array", "items": schema_for_type_name(inner) },
                    "next_cursor": { "type": "string", "nullable": true },
                    "has_more": { "type": "boolean" }
                },
                "required": ["items", "has_more"]
            }),
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

/// Whether a route body type (or the type it directly references)
/// contains an `Upload<Format>` field — which makes the request
/// `multipart/form-data`. A bare `Upload<...>` body also counts.
fn body_type_has_upload(body: &str, contract: &ApplicationContract) -> bool {
    if body.starts_with("Upload<") {
        return true;
    }
    contract
        .types
        .iter()
        .find(|t| t.name == body)
        .is_some_and(|t| t.fields.iter().any(|f| f.upload.is_some()))
}

/// OpenAPI error responses for a route whose response is
/// `Result<T, E>` with `E` an error enum. Each `@status(code)` variant
/// yields one `(status, response)` pair; variants sharing a code are
/// grouped, and the response schema references the error enum so the
/// client can narrow on the variant tag. Returns nothing when the
/// response is not a `Result` or `E` has no status-bearing variant.
fn error_status_responses(
    response_type: &str,
    contract: &ApplicationContract,
) -> Vec<(String, Value)> {
    let Some(("Result", inner)) = split_generic(response_type) else {
        return Vec::new();
    };
    let Some((_ok, err)) = inner.split_once(',') else {
        return Vec::new();
    };
    let err = err.trim();
    let Some(err_ty) = contract.types.iter().find(|t| t.name == err) else {
        return Vec::new();
    };

    let mut by_status: std::collections::BTreeMap<u64, Vec<&str>> = std::collections::BTreeMap::new();
    for v in &err_ty.variants {
        if let Some(code) = v.status {
            by_status.entry(code).or_default().push(v.name.as_str());
        }
    }
    by_status
        .into_iter()
        .map(|(code, variants)| {
            let description = format!("Error: {}", variants.join(", "));
            (
                code.to_string(),
                json!({
                    "description": description,
                    "content": {
                        "application/json": { "schema": schema_for_type_name(err) }
                    }
                }),
            )
        })
        .collect()
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
                        upload: None,
                    },
                    ContractField {
                        name: "explanation".into(),
                        type_name: "String".into(),
                        minimum: None,
                        maximum: None,
                        min_length: Some(20),
                        max_length: Some(500),
                        ui: Default::default(),
                        upload: None,
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
                upload: None,
                response_type: "Answer".into(),
                requires: vec!["issue_refund".into()],
                pagination: None,
                policy: None,
            }],
            agents: vec![ContractCallable {
                name: "chat".into(),
                inputs: vec![ContractParam { name: "m".into(), type_name: "String".into() }],
                output_type: "Stream<String>".into(),
                capabilities: Capabilities::default(),
            }],
            prompts: vec![],
            identities: vec![],
            connectors: vec![],
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
    fn result_route_projects_per_status_error_responses() {
        use crate::app_contract::ContractVariant;
        let mut c = sample();
        c.types.push(ContractType {
            name: "RefundError".into(),
            fields: vec![],
            variants: vec![
                ContractVariant {
                    name: "PaymentNotFound".into(),
                    fields: vec![],
                    status: Some(404),
                    ui: Default::default(),
                },
                ContractVariant {
                    name: "RefundWindowExpired".into(),
                    fields: vec![],
                    status: Some(410),
                    ui: Default::default(),
                },
                ContractVariant {
                    name: "AlreadyGone".into(),
                    fields: vec![],
                    status: Some(410),
                    ui: Default::default(),
                },
            ],
        });
        c.routes[0].response_type = "Result<Answer, RefundError>".into();
        let openapi = emit_openapi(&c);
        let responses = &openapi["paths"]["/refund"]["post"]["responses"];
        assert!(responses.get("200").is_some());
        assert_eq!(
            responses["404"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/RefundError"
        );
        // Variants sharing a status collapse to one response listing both.
        let desc = responses["410"]["description"].as_str().unwrap();
        assert!(desc.contains("RefundWindowExpired") && desc.contains("AlreadyGone"), "{desc}");
    }

    #[test]
    fn upload_field_projects_binary_and_multipart_body() {
        use crate::app_contract::ContractUpload;
        let mut c = sample();
        c.types.push(ContractType {
            name: "DocSubmission".into(),
            fields: vec![ContractField {
                name: "file".into(),
                type_name: "Upload<Pdf>".into(),
                minimum: None,
                maximum: None,
                min_length: None,
                max_length: None,
                ui: Default::default(),
                upload: Some(ContractUpload {
                    format: "Pdf".into(),
                    accepted_mime: vec!["application/pdf".into()],
                    max_bytes: Some(10 * 1024 * 1024),
                    retention_days: Some(7),
                }),
            }],
            variants: vec![],
        });
        c.routes.push(ContractRoute {
            method: "POST".into(),
            path: "/docs".into(),
            path_params: vec![],
            query_type: None,
            body_type: Some("DocSubmission".into()),
            upload: None,
            response_type: "Answer".into(),
            requires: vec![],
            pagination: None,
            policy: None,
        });
        let openapi = emit_openapi(&c);
        // Body carrying an upload is multipart/form-data.
        let body = &openapi["paths"]["/docs"]["post"]["requestBody"]["content"];
        assert!(body.get("multipart/form-data").is_some(), "{body:?}");
        // The upload field is a size-constrained binary string.
        let file = &openapi["components"]["schemas"]["DocSubmission"]["properties"]["file"];
        assert_eq!(file["type"], "string");
        assert_eq!(file["format"], "binary");
        assert_eq!(file["contentMediaType"], "application/pdf");
        assert_eq!(file["maxLength"], 10 * 1024 * 1024);
    }

    #[test]
    fn direct_upload_route_projects_its_exact_boundary_policy() {
        let mut c = sample();
        c.routes.push(ContractRoute {
            method: "POST".into(),
            path: "/imports".into(),
            path_params: vec![],
            query_type: None,
            body_type: Some("Upload<Csv>".into()),
            upload: Some(ContractUpload {
                format: "Csv".into(),
                accepted_mime: vec!["text/csv".into()],
                max_bytes: Some(32),
                retention_days: None,
            }),
            response_type: "Answer".into(),
            requires: vec![],
            pagination: None,
            policy: None,
        });
        let schema =
            &emit_openapi(&c)["paths"]["/imports"]["post"]["requestBody"]["content"]
                ["multipart/form-data"]["schema"];
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["format"], "binary");
        assert_eq!(schema["contentMediaType"], "text/csv");
        assert_eq!(schema["maxLength"], 32);
    }

    #[test]
    fn page_route_projects_envelope_and_cursor_param() {
        use crate::app_contract::{Pagination, PaginationStyle};
        let mut c = sample();
        c.types.push(ContractType {
            name: "Item".into(),
            fields: vec![ContractField {
                name: "id".into(),
                type_name: "String".into(),
                minimum: None,
                maximum: None,
                min_length: None,
                max_length: None,
                ui: Default::default(),
                upload: None,
            }],
            variants: vec![],
        });
        c.routes.push(ContractRoute {
            method: "GET".into(),
            path: "/items".into(),
            path_params: vec![],
            query_type: None,
            body_type: None,
            upload: None,
            response_type: "Page<Item>".into(),
            requires: vec![],
            pagination: Some(Pagination {
                style: PaginationStyle::Cursor,
                item_type: "Item".into(),
                cursor_param: Some("cursor".into()),
            }),
            policy: None,
        });
        let openapi = emit_openapi(&c);
        let op = &openapi["paths"]["/items"]["get"];
        // Optional cursor query parameter.
        let params = op["parameters"].as_array().unwrap();
        let cursor = params.iter().find(|p| p["name"] == "cursor").unwrap();
        assert_eq!(cursor["in"], "query");
        assert_eq!(cursor["required"], false);
        // Response is the page envelope: items array + next_cursor.
        let schema = &op["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["items"]["type"], "array");
        assert_eq!(
            schema["properties"]["items"]["items"]["$ref"],
            "#/components/schemas/Item"
        );
        assert_eq!(schema["properties"]["next_cursor"]["nullable"], true);
    }

    #[test]
    fn identity_auth_routes_and_policy_project_to_openapi() {
        use crate::app_contract::{
            ContractAuthRoute, ContractIdentity, ContractRoutePolicy, ContractSession,
        };
        let mut c = sample();
        c.identities.push(ContractIdentity {
            name: "app_users".into(),
            providers: vec![],
            session: ContractSession {
                lifetime_secs: None,
                cookie_secure: true,
                cookie_http_only: true,
                same_site: "lax".into(),
                rotate_on_privilege_change: true,
            },
            auth_routes: vec![
                ContractAuthRoute {
                    method: "GET".into(),
                    path: "/auth/google/login".into(),
                    purpose: "login".into(),
                    provider: Some("google".into()),
                },
                ContractAuthRoute {
                    method: "GET".into(),
                    path: "/auth/session".into(),
                    purpose: "session".into(),
                    provider: None,
                },
            ],
            safeguards: vec!["authorization_code_with_pkce".into()],
            linking: crate::app_contract::ContractLinking {
                confirmation_required: true,
                email_match: "never".into(),
                verified_domains: vec![],
            },
        });
        // A route with an auth policy.
        c.routes.push(ContractRoute {
            method: "GET".into(),
            path: "/admin".into(),
            path_params: vec![],
            query_type: None,
            body_type: None,
            upload: None,
            response_type: "Answer".into(),
            requires: vec![],
            pagination: None,
            policy: Some(ContractRoutePolicy {
                authenticated: true,
                roles: vec!["admin".into()],
                permissions: vec![],
            }),
        });
        let openapi = emit_openapi(&c);
        // Auth login path is present and carries the safeguards.
        let login = &openapi["paths"]["/auth/google/login"]["get"];
        assert_eq!(login["tags"][0], "auth");
        assert_eq!(login["x-corvid-auth-safeguards"][0], "authorization_code_with_pkce");
        assert!(openapi["paths"]["/auth/session"]["get"].is_object());
        // The policy route advertises session security scoped to the role.
        let admin = &openapi["paths"]["/admin"]["get"];
        let scopes = &admin["security"][0]["corvidSession"];
        assert_eq!(scopes[0], "role:admin");
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
