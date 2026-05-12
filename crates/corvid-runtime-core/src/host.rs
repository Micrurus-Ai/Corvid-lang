//! Suspend/resume host bridge: the wire boundary between
//! deterministic core execution and native-only capabilities.
//!
//! Core execution is wasm-clean. When the agent reaches a point that
//! needs a real LLM call, a DB query, a filesystem touch, or any other
//! native capability, it yields a [`HostRequest`] and awaits a
//! [`HostResponse`]. The host crate (native or browser) implements
//! [`HostBridge`] and resolves each request against its own runtime.
//!
//! `corvid-runtime-host` resolves through tokio + reqwest + postgres
//! + the OTel SDK on native. `corvid-browser` resolves through
//! `wasm-bindgen-futures` and JS Promises in the browser. Same wire
//! format, two hosts, one core.
//!
//! ## Versioning (R4 mitigation)
//!
//! Both `HostRequest` and `HostResponse` carry a `version: "v1"` field
//! at the root. Additive changes to a variant payload don't bump the
//! version; older deserializers ignore unknown fields. Non-additive
//! changes (renamed / removed fields, changed semantics) bump
//! [`SchemaVersion`] and require coordinated rollout, same protocol as
//! `crates/corvid-browser/README.md`'s "Schema-change protocol"
//! section.
//!
//! ## Determinism (R3 mitigation)
//!
//! [`HostBridge::resolve`] is single-shot: one request in, one
//! response out. Parallel-await on multiple host calls is impossible
//! through this trait by construction — the executor yields one
//! request, awaits one response, then continues. Parallelism, when
//! needed, lives above the trait (e.g. multi-worker pools each
//! holding their own bridge instance) so each replay-deterministic
//! execution still sees a totally-ordered request sequence.

use serde::{Deserialize, Serialize};

/// Wire format version. Bump when a non-additive change ships to
/// either [`HostRequest`] or [`HostResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SchemaVersion {
    #[serde(rename = "v1")]
    V1,
}

/// A request the deterministic core yields to the host. Add variants
/// as new native capabilities move through the bridge in later
/// 33J7b-3+ slices; additions are non-breaking on the deserializer
/// side as long as new variants ship with `HostResponse` siblings
/// before they are produced by any core code path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostRequest {
    pub version: SchemaVersion,
    #[serde(flatten)]
    pub kind: HostRequestKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum HostRequestKind {
    LlmCall {
        provider: String,
        model: String,
        messages: Vec<LlmMessage>,
    },
    HostCall {
        ns: String,
        method: String,
        args: serde_json::Value,
    },
    DbQuery {
        sql: String,
        params: Vec<serde_json::Value>,
    },
    FsRead {
        path: String,
    },
    FsWrite {
        path: String,
        body: Vec<u8>,
    },
    HttpRequest {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    },
    OtelEmit {
        event: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

/// A response the host returns to the core. Variants parallel
/// [`HostRequestKind`] by name (`LlmCall` -> `LlmResult`,
/// `DbQuery` -> `DbRows`, ...) plus a single [`HostResponseKind::Error`]
/// for any host-side failure regardless of request type. Cores must
/// route errors uniformly so a fault in one capability cannot become a
/// silent success in another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostResponse {
    pub version: SchemaVersion,
    #[serde(flatten)]
    pub kind: HostResponseKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum HostResponseKind {
    LlmResult {
        content: String,
        usage: TokenUsage,
    },
    HostResult {
        value: serde_json::Value,
    },
    DbRows {
        rows: Vec<Vec<serde_json::Value>>,
        affected: u64,
    },
    FsBytes {
        content: Vec<u8>,
    },
    FsAck,
    HttpReply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    OtelAck,
    Error {
        message: String,
        // Renamed from `kind` to avoid colliding with the
        // `#[serde(tag = "kind")]` discriminator on the outer enum.
        // The wire format reads `{ "kind": "Error", "category": "refused" }`.
        category: HostErrorKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostErrorKind {
    /// The host refuses the requested capability outright (e.g. the
    /// browser host receiving a `DbQuery` it cannot resolve, or any
    /// host receiving a wire format it does not understand).
    Refused,
    /// The capability is supported but the underlying provider failed
    /// (network down, DB locked, LLM provider 5xx).
    ProviderFailure,
    /// The host timed out waiting on the underlying capability.
    Timeout,
    /// The request failed input validation at the host boundary.
    InvalidRequest,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// The single trait both hosts implement to resolve a [`HostRequest`].
///
/// `corvid-runtime-host` implements this with tokio + reqwest +
/// postgres + the OTel SDK + filesystem IO. `corvid-browser`
/// implements this with `wasm-bindgen-futures` over JS Promises that
/// the playground's JS side fulfills (BYO API keys for LLM calls,
/// IndexedDB for the playground's mock filesystem, etc).
///
/// The trait is single-method on purpose. The R3 determinism
/// invariant — pending host requests are sequential — is encoded by
/// the shape: one request in, one response out, no batch variant. If
/// a future capability genuinely needs parallel resolution (e.g. a
/// fan-out RAG search), it ships as a single richer request whose
/// payload carries the parallel work, not as multiple concurrent
/// `resolve` calls.
///
/// `async fn` in a public trait is allowed because both hosts pin
/// their own `Send`/`Sync` bounds at the impl site: native hosts add
/// `Send + Sync` for tokio multi-thread runtimes; the browser host
/// runs single-threaded so it does not.
#[allow(async_fn_in_trait)]
pub trait HostBridge {
    async fn resolve(&self, req: HostRequest) -> HostResponse;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip_request(req: HostRequest) {
        let json_str = serde_json::to_string(&req).expect("serialize HostRequest");
        let parsed: HostRequest =
            serde_json::from_str(&json_str).expect("deserialize HostRequest");
        assert_eq!(req, parsed);
    }

    fn round_trip_response(resp: HostResponse) {
        let json_str = serde_json::to_string(&resp).expect("serialize HostResponse");
        let parsed: HostResponse =
            serde_json::from_str(&json_str).expect("deserialize HostResponse");
        assert_eq!(resp, parsed);
    }

    #[test]
    fn host_request_carries_version_v1_at_root() {
        let req = HostRequest {
            version: SchemaVersion::V1,
            kind: HostRequestKind::FsRead {
                path: "src/main.cor".to_string(),
            },
        };
        let value: serde_json::Value =
            serde_json::to_value(&req).expect("serialize to value");
        assert_eq!(value["version"], json!("v1"));
        assert_eq!(value["kind"], json!("FsRead"));
        assert_eq!(value["path"], json!("src/main.cor"));
    }

    #[test]
    fn host_response_carries_version_v1_at_root() {
        let resp = HostResponse {
            version: SchemaVersion::V1,
            kind: HostResponseKind::FsAck,
        };
        let value: serde_json::Value =
            serde_json::to_value(&resp).expect("serialize to value");
        assert_eq!(value["version"], json!("v1"));
        assert_eq!(value["kind"], json!("FsAck"));
    }

    #[test]
    fn host_request_round_trips_llm_call() {
        round_trip_request(HostRequest {
            version: SchemaVersion::V1,
            kind: HostRequestKind::LlmCall {
                provider: "anthropic".to_string(),
                model: "claude-opus-4-7".to_string(),
                messages: vec![
                    LlmMessage {
                        role: "system".to_string(),
                        content: "You are a careful agent.".to_string(),
                    },
                    LlmMessage {
                        role: "user".to_string(),
                        content: "Refund order 42.".to_string(),
                    },
                ],
            },
        });
    }

    #[test]
    fn host_request_round_trips_host_call() {
        round_trip_request(HostRequest {
            version: SchemaVersion::V1,
            kind: HostRequestKind::HostCall {
                ns: "std.io".to_string(),
                method: "read_text".to_string(),
                args: json!({ "path": "policy.cor" }),
            },
        });
    }

    #[test]
    fn host_request_round_trips_db_query() {
        round_trip_request(HostRequest {
            version: SchemaVersion::V1,
            kind: HostRequestKind::DbQuery {
                sql: "SELECT id FROM customers WHERE id = $1".to_string(),
                params: vec![json!(42)],
            },
        });
    }

    #[test]
    fn host_request_round_trips_fs_write() {
        round_trip_request(HostRequest {
            version: SchemaVersion::V1,
            kind: HostRequestKind::FsWrite {
                path: "out.json".to_string(),
                body: vec![0x7b, 0x7d],
            },
        });
    }

    #[test]
    fn host_request_round_trips_http_request() {
        round_trip_request(HostRequest {
            version: SchemaVersion::V1,
            kind: HostRequestKind::HttpRequest {
                method: "POST".to_string(),
                url: "https://api.anthropic.com/v1/messages".to_string(),
                headers: vec![
                    ("content-type".to_string(), "application/json".to_string()),
                    ("x-api-key".to_string(), "redacted".to_string()),
                ],
                body: Some(b"{\"model\":\"claude\"}".to_vec()),
            },
        });
    }

    #[test]
    fn host_request_round_trips_otel_emit() {
        round_trip_request(HostRequest {
            version: SchemaVersion::V1,
            kind: HostRequestKind::OtelEmit {
                event: json!({ "name": "agent.run", "trace_id": "abc" }),
            },
        });
    }

    #[test]
    fn host_response_round_trips_llm_result() {
        round_trip_response(HostResponse {
            version: SchemaVersion::V1,
            kind: HostResponseKind::LlmResult {
                content: "Refund processed.".to_string(),
                usage: TokenUsage {
                    prompt_tokens: 42,
                    completion_tokens: 7,
                },
            },
        });
    }

    #[test]
    fn host_response_round_trips_db_rows() {
        round_trip_response(HostResponse {
            version: SchemaVersion::V1,
            kind: HostResponseKind::DbRows {
                rows: vec![vec![json!(42), json!("alice")]],
                affected: 1,
            },
        });
    }

    #[test]
    fn host_response_round_trips_http_reply() {
        round_trip_response(HostResponse {
            version: SchemaVersion::V1,
            kind: HostResponseKind::HttpReply {
                status: 200,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: b"{\"ok\":true}".to_vec(),
            },
        });
    }

    #[test]
    fn host_response_round_trips_error_variants() {
        for kind in [
            HostErrorKind::Refused,
            HostErrorKind::ProviderFailure,
            HostErrorKind::Timeout,
            HostErrorKind::InvalidRequest,
        ] {
            round_trip_response(HostResponse {
                version: SchemaVersion::V1,
                kind: HostResponseKind::Error {
                    message: "the host refused".to_string(),
                    category: kind,
                },
            });
        }
    }

    #[test]
    fn unknown_version_is_rejected() {
        // R4: deserializing a payload with an unknown version must
        // fail closed rather than be silently re-tagged as v1.
        let payload = json!({
            "version": "v999",
            "kind": "FsAck",
        });
        let parsed: Result<HostResponse, _> = serde_json::from_value(payload);
        assert!(parsed.is_err(), "v999 must not deserialize as v1");
    }
}
