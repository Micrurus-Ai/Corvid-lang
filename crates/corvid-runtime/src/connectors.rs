//! Real-mode connector HTTP dispatch (slice 52g-3c-4).
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
use crate::http::HttpRequest;
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
    pub base_url: String,
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
    /// Credentials — secret NAMES only.
    pub auth: Option<ConnectorAuthSpec>,
    /// `on status <code> -> Variant` mappings (slice 52g-3c-5): a mapped
    /// HTTP status becomes the named (nullary) error variant.
    pub error_map: Vec<(u16, String)>,
    /// Whether the operation's return type is `Result<Success, Error>`
    /// (slice 52g-3c-5). When true a 2xx body is wrapped as `Ok(..)` and
    /// a mapped status becomes `Err(Variant)`; when false the body
    /// decodes directly and a non-2xx is a transport failure.
    pub returns_result: bool,
    /// Retry attempts (slice 52g-3c reliability). `None` = no retry.
    pub retry: Option<u64>,
    /// Client-side rate limit `(limit, window_secs)` (slice 52g-3c-5).
    /// `None` = unlimited. Enforced per connector before a real request
    /// is sent; an exceeded limit fails the call rather than flooding
    /// the provider.
    pub rate_limit: Option<(u64, u64)>,
    /// Consecutive-failure threshold from the connector's
    /// `circuit_breaker: N` (slice 52h-3). Governs the verified-protocol
    /// poll loop: a transient poll failure is tolerated and retried on
    /// the next tick, but N consecutive failures trip the breaker and
    /// fail the protocol rather than polling a broken provider forever.
    pub circuit_breaker: Option<u64>,
}

/// A recoverable failure while building a connector request — surfaced
/// as a `RuntimeError` so the operation call fails cleanly. A missing
/// credential names only the SECRET NAME, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorRequestError {
    MissingSecret { operation: String, secret: String },
    MissingArgument { operation: String, param: String },
    BadArgument { operation: String, param: String },
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
                    auth: connector.auth.as_ref().map(auth_from_ir),
                    error_map: op
                        .error_map
                        .iter()
                        .map(|m| (m.status, m.variant.clone()))
                        .collect(),
                    returns_result: matches!(tool.return_ty, corvid_types::Type::Result(_, _)),
                    retry: connector.retry,
                    rate_limit: connector.rate_limit.map(|r| (r.limit, r.window_secs)),
                    circuit_breaker: connector.circuit_breaker,
                },
            );
        }
    }
    specs
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
    let filled_path = fill_path(&spec.path, &by_name, &spec.operation)?;
    let url = format!("{}{}", spec.base_url.trim_end_matches('/'), filled_path);

    // Body: the named parameter, JSON- or form-encoded.
    let (method_default_body, mut request) = match &spec.body {
        Some(body) => {
            let value = by_name
                .get(body.param.as_str())
                .copied()
                .ok_or_else(|| ConnectorRequestError::MissingArgument {
                    operation: spec.operation.clone(),
                    param: body.param.clone(),
                })?;
            match body.encoding {
                ConnectorBodyEncoding::Json => {
                    let json = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
                    (true, HttpRequest::post_json(url.clone(), json))
                }
                ConnectorBodyEncoding::Form => {
                    let form = encode_form(value);
                    let req = HttpRequest {
                        method: spec.method.clone(),
                        url: url.clone(),
                        headers: vec![crate::http::HttpHeader {
                            name: "content-type".to_string(),
                            value: "application/x-www-form-urlencoded".to_string(),
                        }],
                        body: Some(form),
                        timeout_ms: 30_000,
                        retry: retry_policy(spec),
                        effect_tag: Some("connector.request".to_string()),
                    };
                    (true, req)
                }
            }
        }
        None => (false, HttpRequest::get(url.clone())),
    };

    // `post_json`/`get` set a default method; override with the
    // operation's declared verb so `PUT`/`PATCH`/`DELETE` bodies work.
    request.method = spec.method.clone();
    request.effect_tag = Some("connector.request".to_string());
    if !method_default_body {
        request.retry = retry_policy(spec);
    }

    // Auth header from the resolved credential — the ONLY place the
    // secret value appears.
    if let Some(auth) = &spec.auth {
        let header = build_auth_header(auth, &spec.operation, resolve_secret)?;
        request = request.header(header.0, header.1);
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
/// poll request (slice 52h-2) so placeholder binding has ONE
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
            connector: "github".to_string(),
            operation: "get_repo".to_string(),
            base_url: "https://api.github.com".to_string(),
            method: "GET".to_string(),
            path: "/repos/{owner}/{repo}".to_string(),
            param_names: vec!["owner".to_string(), "repo".to_string()],
            body: None,
            auth: Some(ConnectorAuthSpec::Bearer {
                secret: "GITHUB_TOKEN".to_string(),
            }),
            error_map: vec![],
            returns_result: false,
            retry: None,
            rate_limit: None,
            circuit_breaker: None,
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
