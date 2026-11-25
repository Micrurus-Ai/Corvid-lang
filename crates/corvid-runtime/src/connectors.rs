//! Real-mode connector HTTP dispatch.
//!
//! A connector `operation` declared in source lowers to a callable tool;
//! in the deployment-selected `real` mode a call to it becomes an HTTP
//! request against the connector's `base_url`. This module holds the
//! runtime-side dispatch spec (derived from the IR at build time) and
//! the PURE request builder that turns a spec + the call's arguments +
//! a resolved secret into an [`HttpRequest`].
//!
//! The security-sensitive step lives here and is unit-tested in
//! isolation: a credential is resolved from the secret store at the last
//! moment, placed into a request header, and never returned to the
//! caller, never stored in the spec, and never written into the
//! recorded trace (the trace records the ToolCall arguments and the
//! response body — neither carries the credential, which rides only in
//! the header).

use crate::errors::RuntimeError;
use crate::http::{HttpRequest, RedirectPolicy};
use base64::Engine;
use std::collections::BTreeMap;

/// How an operation encodes its request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorBodyEncoding {
    Json,
    Form,
}

/// The request body binding of an operation: which parameter supplies
/// the body, and how it is encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorBodySpec {
    pub param: String,
    pub encoding: ConnectorBodyEncoding,
}

/// Where an operation header's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorHeaderValue {
    /// The named `String` parameter's value on this call.
    Param(String),
    /// The same text on every call.
    Literal(String),
}

/// How a connector authenticates — every credential is the NAME of a
/// secret, resolved to a value only at dispatch and never held here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorAuthSpec {
    Bearer {
        secret: String,
    },
    Header {
        name: String,
        secret: String,
    },
    Basic {
        username_secret: String,
        password_secret: String,
    },
}

/// One connector operation's real-mode HTTP dispatch spec, derived from
/// the IR at runtime-build time and keyed by the operation's tool name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorHttpSpec {
    pub connector: String,
    pub operation: String,
    /// Absolute `http(s)://` base URL. The operation path is appended.
    /// `None` for a `hosts: allowed_hosts` connector: the URL is the
    /// `url_param` argument, checked against the project's HTTP policy
    /// like every request.
    pub base_url: Option<String>,
    /// The parameter holding the whole URL, when `base_url` is `None`.
    pub url_param: Option<String>,
    /// `response: text` — the body is passed on as text, not parsed as JSON.
    pub response_text: bool,
    /// `on failure -> Variant`: the error variant every failure becomes,
    /// and whether it carries the reason (one `String` field).
    pub on_failure: Option<(String, bool)>,
    /// HTTP method, upper-case (`GET`, `POST`, …).
    pub method: String,
    /// Path template appended to `base_url`; `{name}` placeholders bind
    /// from the operation's parameters by name.
    pub path: String,
    /// The operation's parameter names, positionally matching the call's
    /// arguments. Used to bind `{name}` placeholders and the body param.
    pub param_names: Vec<String>,
    /// The request body binding, if any.
    pub body: Option<ConnectorBodySpec>,
    /// The record parameter whose fields become the query, if any.
    pub query: Option<String>,
    /// The operation's `header` lines, in source order.
    pub headers: Vec<(String, ConnectorHeaderValue)>,
    /// Credentials — secret NAMES only.
    pub auth: Option<ConnectorAuthSpec>,
    /// `on status <code> -> Variant` mappings: a mapped
    /// HTTP status becomes the named (nullary) error variant.
    pub error_map: Vec<(u16, String)>,
    /// Whether the operation's return type is `Result<Success, Error>`
    ///. When true a 2xx body is wrapped as `Ok(..)` and
    /// a mapped status becomes `Err(Variant)`; when false the body
    /// decodes directly and a non-2xx is a transport failure.
    pub returns_result: bool,
    /// Whether the operation returns `Response<Body>` (alone or as a
    /// `Result`'s success): the answer then carries the status and the
    /// response headers as well as the body.
    pub returns_response: bool,
    /// Retry attempts (reliability). `None` = no retry.
    pub retry: Option<u64>,
    /// Client-side rate limit `(limit, window_secs)`.
    /// `None` = unlimited. Enforced per connector before a real request
    /// is sent; an exceeded limit fails the call rather than flooding
    /// the provider.
    pub rate_limit: Option<(u64, u64)>,
    /// Consecutive-failure threshold from the connector's
    /// `circuit_breaker: N`. Governs the verified-protocol
    /// poll loop: a transient poll failure is tolerated and retried on
    /// the next tick, but N consecutive failures trip the breaker and
    /// fail the protocol rather than polling a broken provider forever.
    pub circuit_breaker: Option<u64>,
    /// The operation's declared timeout in seconds (its own `timeout:`,
    /// or the connector's). Always present for a checked program; a spec
    /// without one is refused at send time rather than given a default.
    pub timeout_secs: Option<u64>,
    /// The connector's declared `redirects:` policy, present for every
    /// checked program as `timeout_secs` is.
    pub redirects: Option<RedirectPolicy>,
    /// Extra headers attached to THIS request, on top of the connector's
    /// declared auth. The verified-protocol dispatcher uses this to carry
    /// the intent key on a submit (and only on a submit) via the header
    /// the protocol declared, so the request builder stays the single
    /// place a connector request is assembled.
    pub extra_headers: Vec<(String, String)>,
}

/// A recoverable failure while building a connector request — surfaced
/// as a `RuntimeError` so the operation call fails cleanly. A missing
/// credential names only the SECRET NAME, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorRequestError {
    MissingSecret { operation: String, secret: String },
    MissingArgument { operation: String, param: String },
    BadArgument { operation: String, param: String },
    /// The spec carries no declared `timeout:` or `redirects:`. Only IR
    /// from unchecked source can get here; the request is refused rather
    /// than sent under a policy nobody chose.
    UndeclaredPolicy { operation: String, item: &'static str },
}

impl ConnectorRequestError {
    pub fn into_runtime_error(self) -> RuntimeError {
        let (operation, message) = match self {
            Self::MissingSecret { operation, secret } => (
                operation,
                format!("credential `{secret}` did not resolve for real-mode dispatch"),
            ),
            Self::MissingArgument { operation, param } => {
                (operation, format!("missing argument for `{param}`"))
            }
            Self::BadArgument { operation, param } => (
                operation,
                format!("argument for `{param}` is not a usable path/body value"),
            ),
            Self::UndeclaredPolicy { operation, item } => (
                operation,
                format!(
                    "the connector declares no `{item}`, so the request is not sent; \
                     the checker requires `{item}` on every connector"
                ),
            ),
        };
        RuntimeError::ToolFailed {
            tool: operation,
            message,
        }
    }
}

/// Derive the real-mode dispatch specs for every connector operation in
/// a lowered file, keyed by the operation's tool name. This is the
/// single projection from IR to dispatch spec — the driver installs the
/// result on the runtime via `RuntimeBuilder::connector_calls`. Param
/// names come from the operation's lowered `IrTool` (looked up by
/// `tool_id`), which is where the parameter list lives.
pub fn connector_calls_from_ir(
    ir: &corvid_ir::IrFile,
) -> std::collections::HashMap<String, ConnectorHttpSpec> {
    use corvid_resolve::DefId;
    let tools_by_id: std::collections::HashMap<DefId, &corvid_ir::IrTool> =
        ir.tools.iter().map(|t| (t.id, t)).collect();
    let mut specs = std::collections::HashMap::new();
    for connector in &ir.connectors {
        for op in &connector.operations {
            let Some(tool) = tools_by_id.get(&op.tool_id) else {
                continue;
            };
            let param_names = tool.params.iter().map(|p| p.name.clone()).collect();
            specs.insert(
                op.name.clone(),
                ConnectorHttpSpec {
                    extra_headers: Vec::new(),
                    connector: connector.name.clone(),
                    operation: op.name.clone(),
                    base_url: connector.base_url.clone(),
                    method: op.method.as_str().to_string(),
                    path: op.path.clone(),
                    param_names,
                    body: op.body.as_ref().map(|b| ConnectorBodySpec {
                        param: b.param_name.clone(),
                        encoding: match b.encoding {
                            corvid_ast::BodyEncoding::Json => ConnectorBodyEncoding::Json,
                            corvid_ast::BodyEncoding::Form => ConnectorBodyEncoding::Form,
                        },
                    }),
                    query: op.query.clone(),
                    url_param: op.url_param.clone(),
                    response_text: op.response_text,
                    on_failure: op.on_failure.as_ref().map(|variant| {
                        let takes_reason = failure_variant_takes_reason(ir, &tool.return_ty, variant);
                        (variant.clone(), takes_reason)
                    }),
                    headers: op
                        .headers
                        .iter()
                        .map(|h| {
                            let value = match &h.value {
                                corvid_ir::IrOperationHeaderValue::Param(p) => {
                                    ConnectorHeaderValue::Param(p.clone())
                                }
                                corvid_ir::IrOperationHeaderValue::Literal(text) => {
                                    ConnectorHeaderValue::Literal(text.clone())
                                }
                            };
                            (h.name.clone(), value)
                        })
                        .collect(),
                    auth: connector.auth.as_ref().map(auth_from_ir),
                    error_map: op
                        .error_map
                        .iter()
                        .map(|m| (m.status, m.variant.clone()))
                        .collect(),
                    returns_result: matches!(tool.return_ty, corvid_types::Type::Result(_, _)),
                    returns_response: match &tool.return_ty {
                        corvid_types::Type::Response(_) => true,
                        corvid_types::Type::Result(ok, _) => matches!(**ok, corvid_types::Type::Response(_)),
                        _ => false,
                    },
                    retry: connector.retry,
                    rate_limit: connector.rate_limit.map(|r| (r.limit, r.window_secs)),
                    circuit_breaker: connector.circuit_breaker,
                    timeout_secs: op.timeout_secs,
                    redirects: connector.redirects,
                },
            );
        }
    }
    specs
}

/// Whether the `on failure` variant of an operation's error type has a
/// field for the reason (the checker allows none or one `String`).
fn failure_variant_takes_reason(ir: &corvid_ir::IrFile, return_ty: &corvid_types::Type, variant: &str) -> bool {
    let corvid_types::Type::Result(_, err) = return_ty else { return false };
    let corvid_types::Type::Struct(id) = err.as_ref() else { return false };
    ir.types
        .iter()
        .find(|t| t.id == *id)
        .and_then(|t| t.variants.iter().find(|v| v.name == variant))
        .is_some_and(|v| !v.fields.is_empty())
}

/// The recorded result an `on failure` operation answers with when its
/// call fails: `Err(Variant)` or `Err(Variant(reason))`, in the envelope
/// mapped statuses use, so replay reproduces it exactly.
pub fn failure_result(variant: &str, takes_reason: bool, reason: &str) -> serde_json::Value {
    let fields = if takes_reason { vec![serde_json::json!(reason)] } else { Vec::new() };
    serde_json::json!({
        "tag": "err",
        "err": { "tag": "variant", "variant": variant, "fields": fields },
    })
}

fn auth_from_ir(auth: &corvid_ir::IrConnectorAuth) -> ConnectorAuthSpec {
    match auth {
        corvid_ir::IrConnectorAuth::Bearer { secret } => ConnectorAuthSpec::Bearer {
            secret: secret.clone(),
        },
        corvid_ir::IrConnectorAuth::Header { name, secret } => ConnectorAuthSpec::Header {
            name: name.clone(),
            secret: secret.clone(),
        },
        corvid_ir::IrConnectorAuth::Basic {
            username_secret,
            password_secret,
        } => ConnectorAuthSpec::Basic {
            username_secret: username_secret.clone(),
            password_secret: password_secret.clone(),
        },
    }
}

/// Build the `HttpRequest` for a real-mode connector call. Pure: given
/// the spec, the positional call arguments (JSON), and a secret
/// resolver, it fills path placeholders, encodes the body, and attaches
/// the auth header from the resolved credential. The resolver returns
/// `None` for an unresolved secret (→ `MissingSecret`); the credential
/// value flows ONLY into the returned request's header.
pub fn build_connector_request(
    spec: &ConnectorHttpSpec,
    args: &[serde_json::Value],
    resolve_secret: &dyn Fn(&str) -> Option<String>,
) -> Result<HttpRequest, ConnectorRequestError> {
    // Map parameter name -> argument value by position.
    let by_name: BTreeMap<&str, &serde_json::Value> = spec
        .param_names
        .iter()
        .map(|n| n.as_str())
        .zip(args.iter())
        .collect();

    // Fill `{placeholder}` path segments from the arguments.
    let mut url = match (&spec.base_url, &spec.url_param) {
        (Some(base_url), _) => {
            let filled_path = fill_path(&spec.path, &by_name, &spec.operation)?;
            format!("{}{}", base_url.trim_end_matches('/'), filled_path)
        }
        // A `hosts: allowed_hosts` operation: the argument is the URL. The
        // dispatcher runs the SSRF block and `[http] allow` on it before
        // anything is sent.
        (None, Some(param)) => by_name
            .get(param.as_str())
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| ConnectorRequestError::BadArgument {
                operation: spec.operation.clone(),
                param: param.clone(),
            })?,
        (None, None) => {
            return Err(ConnectorRequestError::BadArgument {
                operation: spec.operation.clone(),
                param: "url".to_string(),
            })
        }
    };
    if let Some(query_param) = &spec.query {
        let value = by_name.get(query_param.as_str()).copied().ok_or_else(|| {
            ConnectorRequestError::MissingArgument {
                operation: spec.operation.clone(),
                param: query_param.clone(),
            }
        })?;
        let query = encode_query(value).ok_or_else(|| ConnectorRequestError::BadArgument {
            operation: spec.operation.clone(),
            param: query_param.clone(),
        })?;
        if !query.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&query);
        }
    }

    let timeout_secs = spec.timeout_secs.ok_or_else(|| ConnectorRequestError::UndeclaredPolicy {
        operation: spec.operation.clone(),
        item: "timeout:",
    })?;
    let redirects = spec.redirects.ok_or_else(|| ConnectorRequestError::UndeclaredPolicy {
        operation: spec.operation.clone(),
        item: "redirects:",
    })?;
    let mut request = HttpRequest::new(
        spec.method.clone(),
        url,
        std::time::Duration::from_secs(timeout_secs),
        redirects,
    )
    .retry(retry_policy(spec))
    .effect_tag("connector.request");

    // Body: the named parameter, JSON- or form-encoded.
    if let Some(body) = &spec.body {
        let value = by_name
            .get(body.param.as_str())
            .copied()
            .ok_or_else(|| ConnectorRequestError::MissingArgument {
                operation: spec.operation.clone(),
                param: body.param.clone(),
            })?;
        request = match body.encoding {
            ConnectorBodyEncoding::Json => {
                let json = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
                request.json_body(json)
            }
            ConnectorBodyEncoding::Form => {
                request.body("application/x-www-form-urlencoded", encode_form(value))
            }
        };
    }

    // Auth header from the resolved credential — the ONLY place the
    // secret value appears. It is marked as the credential, so it is
    // never sent to another origin and never enters a fingerprint.
    if let Some(auth) = &spec.auth {
        let header = build_auth_header(auth, &spec.operation, resolve_secret)?;
        request = request.credential_header(header.0, header.1);
    }

    // The operation's `header` lines. The checker keeps them off every
    // header `auth:`, the body and the transport own.
    for (name, value) in &spec.headers {
        let text = match value {
            ConnectorHeaderValue::Literal(text) => text.clone(),
            ConnectorHeaderValue::Param(param) => by_name
                .get(param.as_str())
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| ConnectorRequestError::BadArgument {
                    operation: spec.operation.clone(),
                    param: param.clone(),
                })?,
        };
        if text.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
            return Err(ConnectorRequestError::BadArgument {
                operation: spec.operation.clone(),
                param: name.clone(),
            });
        }
        request = request.header(name.clone(), text);
    }

    // Request-specific headers (the protocol's idempotency key on a
    // submit). Attached last, but they can never displace the credential:
    // the auth header name is fixed by the declared auth scheme and these
    // come from a declared idempotency transport.
    for (name, value) in &spec.extra_headers {
        request = request.header(name.clone(), value.clone());
    }

    Ok(request)
}

fn retry_policy(spec: &ConnectorHttpSpec) -> crate::http::HttpRetryPolicy {
    crate::http::HttpRetryPolicy {
        max_retries: spec.retry.unwrap_or(0) as u32,
        retry_on_5xx: true,
    }
}

/// Fill `{placeholder}` segments in a path from a name→value map.
/// Shared by the operation request builder and the provider-protocol
/// poll request so placeholder binding has ONE
/// implementation.
pub(crate) fn fill_path(
    path: &str,
    by_name: &BTreeMap<&str, &serde_json::Value>,
    operation: &str,
) -> Result<String, ConnectorRequestError> {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find('}')
            .ok_or_else(|| ConnectorRequestError::BadArgument {
                operation: operation.to_string(),
                param: "path".to_string(),
            })?;
        let name = &after[..close];
        let value = by_name
            .get(name)
            .copied()
            .ok_or_else(|| ConnectorRequestError::MissingArgument {
                operation: operation.to_string(),
                param: name.to_string(),
            })?;
        out.push_str(&scalar_to_path_segment(value).ok_or_else(|| {
            ConnectorRequestError::BadArgument {
                operation: operation.to_string(),
                param: name.to_string(),
            }
        })?);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn scalar_to_path_segment(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(urlencode(s)),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The query string for a record argument in its public JSON form: one
/// `name=value` pair per field, in field-name order. An absent `Option`
/// (`null`) is left out and a `List` repeats the name, the forms most
/// APIs read. `None` when the value is not a flat record, which the
/// checker already refuses.
fn encode_query(value: &serde_json::Value) -> Option<String> {
    let serde_json::Value::Object(fields) = value else {
        return None;
    };
    let mut pairs = Vec::new();
    for (name, field) in fields {
        let items: Vec<&serde_json::Value> = match field {
            serde_json::Value::Null => continue,
            serde_json::Value::Array(items) => items.iter().collect(),
            single => vec![single],
        };
        for item in items {
            let text = match item {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => return None,
            };
            pairs.push(format!("{}={}", urlencode(name), urlencode(&text)));
        }
    }
    Some(pairs.join("&"))
}

fn encode_form(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let vs = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{}={}", urlencode(k), urlencode(&vs))
            })
            .collect::<Vec<_>>()
            .join("&"),
        serde_json::Value::String(s) => urlencode(s),
        other => urlencode(&other.to_string()),
    }
}

/// Minimal application/x-www-form-urlencoded percent-encoding for the
/// characters that must not appear unescaped in a path segment or form
/// value. Deterministic (no allocation surprises) so replay stays stable.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn build_auth_header(
    auth: &ConnectorAuthSpec,
    operation: &str,
    resolve_secret: &dyn Fn(&str) -> Option<String>,
) -> Result<(String, String), ConnectorRequestError> {
    let resolve = |name: &str| -> Result<String, ConnectorRequestError> {
        resolve_secret(name).ok_or_else(|| ConnectorRequestError::MissingSecret {
            operation: operation.to_string(),
            secret: name.to_string(),
        })
    };
    match auth {
        ConnectorAuthSpec::Bearer { secret } => {
            let token = resolve(secret)?;
            Ok(("authorization".to_string(), format!("Bearer {token}")))
        }
        ConnectorAuthSpec::Header { name, secret } => {
            let value = resolve(secret)?;
            Ok((name.clone(), value))
        }
        ConnectorAuthSpec::Basic {
            username_secret,
            password_secret,
        } => {
            let user = resolve(username_secret)?;
            let pass = resolve(password_secret)?;
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
            Ok(("authorization".to_string(), format!("Basic {encoded}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec() -> ConnectorHttpSpec {
        ConnectorHttpSpec {
            extra_headers: Vec::new(),
            connector: "github".to_string(),
            operation: "get_repo".to_string(),
            base_url: Some("https://api.github.com".to_string()),
            url_param: None,
            response_text: false,
            on_failure: None,
            method: "GET".to_string(),
            path: "/repos/{owner}/{repo}".to_string(),
            param_names: vec!["owner".to_string(), "repo".to_string()],
            body: None,
            query: None,
            headers: Vec::new(),
            auth: Some(ConnectorAuthSpec::Bearer {
                secret: "GITHUB_TOKEN".to_string(),
            }),
            error_map: vec![],
            returns_result: false,
            returns_response: false,
            retry: None,
            rate_limit: None,
            circuit_breaker: None,
            timeout_secs: Some(10),
            redirects: Some(crate::http::RedirectPolicy::Never),
        }
    }

    #[test]
    fn fills_path_placeholders_and_appends_to_base_url() {
        let req = build_connector_request(
            &spec(),
            &[json!("micrurus"), json!("corvid")],
            &|name| (name == "GITHUB_TOKEN").then(|| "tok-123".to_string()),
        )
        .expect("build");
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, "https://api.github.com/repos/micrurus/corvid");
    }

    #[test]
    fn bearer_secret_flows_only_into_the_authorization_header() {
        let req = build_connector_request(&spec(), &[json!("o"), json!("r")], &|_| {
            Some("super-secret".to_string())
        })
        .expect("build");
        let auth = req
            .headers
            .iter()
            .find(|h| h.name == "authorization")
            .expect("auth header");
        assert_eq!(auth.value, "Bearer super-secret");
        // The credential appears nowhere else — not in the URL, not in a
        // body. (The recorded trace carries only args + response body,
        // never the request headers.)
        assert!(!req.url.contains("super-secret"));
        assert!(req.body.is_none());
    }

    #[test]
    fn the_request_carries_the_declared_timeout_and_redirect_policy() {
        let mut declared = spec();
        declared.timeout_secs = Some(45);
        declared.redirects = Some(RedirectPolicy::SameHost);
        let req = build_connector_request(&declared, &[json!("o"), json!("r")], &|_| {
            Some("tok".to_string())
        })
        .expect("build");
        assert_eq!(req.timeout_ms, 45_000);
        assert_eq!(req.redirects, RedirectPolicy::SameHost);
    }

    /// A custom auth header is marked as the credential, so a redirect
    /// to another origin drops it the way it drops `Authorization`.
    #[test]
    fn a_custom_auth_header_is_marked_as_the_credential() {
        let mut keyed = spec();
        keyed.auth = Some(ConnectorAuthSpec::Header {
            name: "X-Api-Key".to_string(),
            secret: "KEY".to_string(),
        });
        let req = build_connector_request(&keyed, &[json!("o"), json!("r")], &|_| {
            Some("key-value".to_string())
        })
        .expect("build");
        assert_eq!(req.credential_headers, vec!["x-api-key".to_string()]);
    }

    /// IR from unchecked source could lack a declared policy; the request
    /// is refused rather than sent under a timeout nobody chose.
    #[test]
    fn a_spec_without_a_declared_policy_is_refused() {
        let mut undeclared = spec();
        undeclared.timeout_secs = None;
        let err = build_connector_request(&undeclared, &[json!("o"), json!("r")], &|_| {
            Some("tok".to_string())
        })
        .expect_err("no timeout declared");
        assert!(err.into_runtime_error().to_string().contains("declares no `timeout:`"));

        let mut undeclared = spec();
        undeclared.redirects = None;
        let err = build_connector_request(&undeclared, &[json!("o"), json!("r")], &|_| {
            Some("tok".to_string())
        })
        .expect_err("no redirect policy declared");
        assert!(err.into_runtime_error().to_string().contains("declares no `redirects:`"));
    }

    /// A query record becomes `name=value` pairs: `None` is left out, a
    /// list repeats the name, and values are percent-encoded.
    #[test]
    fn a_query_record_becomes_the_query_string() {
        let mut listed = spec();
        listed.path = "/repos/{owner}/{repo}/issues".to_string();
        listed.param_names.push("filter".to_string());
        listed.query = Some("filter".to_string());
        let req = build_connector_request(
            &listed,
            &[
                json!("o"),
                json!("r"),
                json!({ "labels": ["bug", "good first"], "milestone": null, "state": "open" }),
            ],
            &|_| Some("tok".to_string()),
        )
        .expect("build");
        assert_eq!(
            req.url,
            "https://api.github.com/repos/o/r/issues?labels=bug&labels=good%20first&state=open"
        );
    }

    #[test]
    fn header_lines_carry_a_parameter_or_a_fixed_value() {
        let mut headed = spec();
        headed.param_names.push("request_id".to_string());
        headed.headers = vec![
            ("X-Request-Id".to_string(), ConnectorHeaderValue::Param("request_id".to_string())),
            ("Accept".to_string(), ConnectorHeaderValue::Literal("application/json".to_string())),
        ];
        let req = build_connector_request(&headed, &[json!("o"), json!("r"), json!("req-7")], &|_| {
            Some("tok".to_string())
        })
        .expect("build");
        let value = |name: &str| req.headers.iter().find(|h| h.name == name).map(|h| h.value.as_str());
        assert_eq!(value("X-Request-Id"), Some("req-7"));
        assert_eq!(value("Accept"), Some("application/json"));

        // A value that would split the header block is refused.
        let err = build_connector_request(&headed, &[json!("o"), json!("r"), json!("a\r\nX-Evil: 1")], &|_| {
            Some("tok".to_string())
        })
        .expect_err("a line break in a header value must be refused");
        assert!(matches!(err, ConnectorRequestError::BadArgument { .. }));
    }

    #[test]
    fn an_unresolved_secret_is_a_named_error_without_a_value() {
        let err = build_connector_request(&spec(), &[json!("o"), json!("r")], &|_| None)
            .expect_err("missing secret");
        assert_eq!(
            err,
            ConnectorRequestError::MissingSecret {
                operation: "get_repo".to_string(),
                secret: "GITHUB_TOKEN".to_string()
            }
        );
    }

    #[test]
    fn json_body_uses_the_named_parameter() {
        let mut s = spec();
        s.operation = "create_issue".to_string();
        s.method = "POST".to_string();
        s.path = "/repos/{owner}/issues".to_string();
        s.param_names = vec!["owner".to_string(), "req".to_string()];
        s.body = Some(ConnectorBodySpec {
            param: "req".to_string(),
            encoding: ConnectorBodyEncoding::Json,
        });
        let req = build_connector_request(
            &s,
            &[json!("micrurus"), json!({"title": "bug"})],
            &|_| Some("t".to_string()),
        )
        .expect("build");
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "https://api.github.com/repos/micrurus/issues");
        assert_eq!(req.body.as_deref(), Some(r#"{"title":"bug"}"#));
    }

    #[test]
    fn header_and_basic_auth_build_the_right_headers() {
        let mut s = spec();
        s.auth = Some(ConnectorAuthSpec::Header {
            name: "x-api-key".to_string(),
            secret: "KEY".to_string(),
        });
        let req =
            build_connector_request(&s, &[json!("o"), json!("r")], &|_| Some("k".to_string()))
                .expect("build");
        assert_eq!(
            req.headers.iter().find(|h| h.name == "x-api-key").unwrap().value,
            "k"
        );

        let mut b = spec();
        b.auth = Some(ConnectorAuthSpec::Basic {
            username_secret: "U".to_string(),
            password_secret: "P".to_string(),
        });
        let req = build_connector_request(&b, &[json!("o"), json!("r")], &|name| {
            Some(if name == "U" { "user" } else { "pass" }.to_string())
        })
        .expect("build");
        let auth = req.headers.iter().find(|h| h.name == "authorization").unwrap();
        // base64("user:pass") = dXNlcjpwYXNz
        assert_eq!(auth.value, "Basic dXNlcjpwYXNz");
    }
}
