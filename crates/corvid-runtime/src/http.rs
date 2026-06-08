use crate::errors::RuntimeError;
use sha2::{Digest, Sha256};
use serde::Serialize;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRetryPolicy {
    pub max_retries: u32,
    pub retry_on_5xx: bool,
}

impl Default for HttpRetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 0,
            retry_on_5xx: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<HttpHeader>,
    pub body: Option<String>,
    pub timeout_ms: u64,
    pub retry: HttpRetryPolicy,
    pub effect_tag: Option<String>,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: "GET".to_string(),
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout_ms: 30_000,
            retry: HttpRetryPolicy::default(),
            effect_tag: None,
        }
    }

    pub fn post_json(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            method: "POST".to_string(),
            url: url.into(),
            headers: vec![HttpHeader {
                name: "content-type".to_string(),
                value: "application/json".to_string(),
            }],
            body: Some(body.into()),
            timeout_ms: 30_000,
            retry: HttpRetryPolicy::default(),
            effect_tag: None,
        }
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn retry(mut self, retry: HttpRetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push(HttpHeader {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    pub fn effect_tag(mut self, effect_tag: impl Into<String>) -> Self {
        self.effect_tag = Some(effect_tag.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<HttpHeader>,
    pub body: String,
    pub attempts: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedHttpExchange {
    pub request_fingerprint: String,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub attempts: u32,
    pub effect_tag: Option<String>,
    pub response_body: String,
}

#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    /// Slice `35V2-P38-C-5`: set to `true` by
    /// `RuntimeBuilder::build` when entering Substitute-mode replay.
    /// `send` short-circuits with `RuntimeError::QuarantineViolation`
    /// so a replayed run cannot reach the network even if some caller
    /// bypasses the runtime's trace-substitution path. Recorded HTTP
    /// exchanges are not yet substituted at the runtime layer (filed
    /// for post-v1.0 — connector calls today go through the
    /// connector-runtime), so any HTTP send during replay is a
    /// quarantine violation by construction.
    quarantined: bool,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .expect("reqwest client builds with default config"),
            quarantined: false,
        }
    }
}

impl HttpClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Flip into replay-quarantine mode. Subsequent `send` calls fail
    /// closed with `RuntimeError::QuarantineViolation { surface:
    /// "http", .. }` instead of reaching the network. Called by
    /// `RuntimeBuilder::build` when entering Substitute-mode replay.
    pub fn quarantine(&mut self) {
        self.quarantined = true;
    }

    /// True when this client refuses live HTTP calls. Test helper.
    pub fn is_quarantined(&self) -> bool {
        self.quarantined
    }

    pub async fn send(&self, request: &HttpRequest) -> Result<HttpResponse, RuntimeError> {
        if self.quarantined {
            return Err(RuntimeError::QuarantineViolation {
                surface: "http".to_string(),
                detail: format!(
                    "blocked an unrecorded {} call to `{}` during replay-mode quarantine. \
                     A replayed run must not reach the network; if the connector layer needs \
                     this request, the trace did not record an equivalent exchange.",
                    request.method, request.url
                ),
            });
        }
        let started = Instant::now();
        let attempts_allowed = request.retry.max_retries.saturating_add(1);
        let mut attempt = 0;
        let mut last_error = None;
        while attempt < attempts_allowed {
            attempt += 1;
            match self.send_once(request).await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let headers = response_headers(response.headers());
                    let body = response.text().await.map_err(|err| RuntimeError::ToolFailed {
                        tool: "std.http".to_string(),
                        message: format!("failed to read HTTP response body: {err}"),
                    })?;
                    let should_retry =
                        request.retry.retry_on_5xx && status >= 500 && attempt < attempts_allowed;
                    if should_retry {
                        last_error = Some(format!("HTTP {status}"));
                        continue;
                    }
                    return Ok(HttpResponse {
                        status,
                        headers,
                        body,
                        attempts: attempt,
                        elapsed_ms: elapsed_ms(started),
                    });
                }
                Err(err) if attempt < attempts_allowed => {
                    last_error = Some(err.to_string());
                }
                Err(err) => return Err(err),
            }
        }
        Err(RuntimeError::ToolFailed {
            tool: "std.http".to_string(),
            message: last_error.unwrap_or_else(|| "HTTP request failed".to_string()),
        })
    }

    pub async fn send_recorded(
        &self,
        request: &HttpRequest,
    ) -> Result<(HttpResponse, RecordedHttpExchange), RuntimeError> {
        let response = self.send(request).await?;
        let record = record_exchange(request, &response);
        Ok((response, record))
    }

    async fn send_once(
        &self,
        request: &HttpRequest,
    ) -> Result<reqwest::Response, RuntimeError> {
        let method = request.method.parse::<reqwest::Method>().map_err(|err| {
            RuntimeError::ToolFailed {
                tool: "std.http".to_string(),
                message: format!("invalid HTTP method `{}`: {err}", request.method),
            }
        })?;
        let mut builder = self
            .client
            .request(method, &request.url)
            .timeout(Duration::from_millis(request.timeout_ms.max(1)));
        for header in &request.headers {
            builder = builder.header(&header.name, &header.value);
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }
        builder.send().await.map_err(|err| RuntimeError::ToolFailed {
            tool: "std.http".to_string(),
            message: err.to_string(),
        })
    }
}

fn response_headers(headers: &reqwest::header::HeaderMap) -> Vec<HttpHeader> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|value| HttpHeader {
                name: name.as_str().to_string(),
                value: value.to_string(),
            })
        })
        .collect()
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub fn record_exchange(request: &HttpRequest, response: &HttpResponse) -> RecordedHttpExchange {
    RecordedHttpExchange {
        request_fingerprint: request_fingerprint(request),
        method: request.method.clone(),
        url: request.url.clone(),
        status: response.status,
        attempts: response.attempts,
        effect_tag: request.effect_tag.clone(),
        response_body: response.body.clone(),
    }
}

pub fn request_fingerprint(request: &HttpRequest) -> String {
    let mut headers = request.headers.clone();
    headers.sort_by(|left, right| left.name.cmp(&right.name).then(left.value.cmp(&right.value)));
    let canonical = serde_json::json!({
        "method": request.method,
        "url": request.url,
        "headers": headers,
        "body": request.body,
        "timeout_ms": request.timeout_ms,
        "retry": {
            "max_retries": request.retry.max_retries,
            "retry_on_5xx": request.retry.retry_on_5xx,
        },
        "effect_tag": request.effect_tag,
    });
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string().as_bytes());
    encode_hex(&hasher.finalize())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

// ============================================================================
// Phase 33S2a — HttpEgressPolicy.
//
// The executing `http_get` / `http_post_json` stdlib tools
// (declared in `std/http.cor`) carry their security boundary in
// this policy struct. The `Runtime` holds one `HttpEgressPolicy`;
// the `Runtime::call_tool` interception path for `http_*` tool
// names runs each call's `url` argument through `check` before
// the actual `HttpClient::send` runs.
//
// The policy enforces TWO independent properties:
//
//   1. SSRF block — **always on, never configurable**. Any URL
//      whose host resolves to a private RFC1918 range
//      (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16), loopback
//      (127.0.0.0/8 + ::1), or link-local (169.254.0.0/16 +
//      fe80::/10) is refused regardless of allowlist contents.
//      This is a STRUCTURAL property — there is no way for a
//      program to opt out without modifying this code.
//
//   2. Allowlist — `[http] allow = [...]` in corvid.toml (or
//      the `CORVID_HTTP_ALLOW` env override). REQUIRED: when
//      the list is empty, every executing HTTP call fails
//      closed with the missing-config diagnostic.
//
// Composition: a URL passes only when BOTH the SSRF check
// passes AND its host appears verbatim (case-insensitive) in
// the allowlist. Either failure mode emits a structured
// diagnostic that names the URL, the failing check, and the
// configured allowlist (when relevant) so the operator can
// see why the call was refused.
// ============================================================================

/// Policy carrying the configured `[http] allow` list for the
/// executing HTTP-client surface plus the always-on SSRF block.
/// Construct via `HttpEgressPolicy::new` once per `Runtime` and
/// store on the runtime; the `http_*` tool dispatch interception
/// calls `check` per request.
#[derive(Debug, Clone, Default)]
pub struct HttpEgressPolicy {
    /// Lower-cased host strings allowed for egress. `None` and
    /// `Some(empty)` both mean unconfigured — every `check`
    /// call fails closed. `Some(non-empty)` is the active
    /// allowlist; case-insensitive host comparison.
    allow: Option<Vec<String>>,
}

impl HttpEgressPolicy {
    /// Build a policy from the parsed `[http] allow` value. An
    /// empty input list is treated the same as `None` (the
    /// fail-closed default). Each entry is lower-cased so host
    /// comparison in `check` is case-insensitive.
    pub fn new(allow_list: Option<&[String]>) -> Self {
        let allow = allow_list.and_then(|list| {
            if list.is_empty() {
                None
            } else {
                Some(list.iter().map(|s| s.to_ascii_lowercase()).collect())
            }
        });
        Self { allow }
    }

    /// Empty policy — no `[http] allow` configured. Every
    /// `check` call returns the missing-config error.
    pub fn unset() -> Self {
        Self { allow: None }
    }

    /// True when the policy has a non-empty allowlist.
    pub fn is_configured(&self) -> bool {
        self.allow.as_ref().is_some_and(|v| !v.is_empty())
    }

    /// Return the configured allowlist for inspection (used by
    /// `corvid doctor`-style introspection). Empty when
    /// unconfigured.
    pub fn allow_list(&self) -> &[String] {
        self.allow.as_deref().unwrap_or(&[])
    }

    /// Check a request URL against (1) the always-on SSRF
    /// block and (2) the configured allowlist. Returns the host
    /// string on success (for tracing) or a structured
    /// `RuntimeError` naming the violation.
    pub fn check(&self, url: &str) -> Result<String, RuntimeError> {
        let host = parse_host_lowercase(url).ok_or_else(|| RuntimeError::ToolFailed {
            tool: "http".to_string(),
            message: format!(
                "URL `{url}` could not be parsed for SSRF + allowlist check; \
                 the executing HTTP surface requires an absolute http(s):// URL"
            ),
        })?;

        // SSRF check first — always on, regardless of
        // allowlist contents. Blocks loopback + RFC1918 +
        // link-local. Names that ALSO appear in the allowlist
        // still get refused (the SSRF block is the floor).
        if is_ssrf_blocked_host(&host) {
            return Err(RuntimeError::ToolFailed {
                tool: "http".to_string(),
                message: format!(
                    "URL `{url}` is refused by Corvid's structural SSRF block: \
                     host `{host}` resolves to a private / loopback / link-local \
                     address range, which is never reachable through the \
                     executing HTTP surface regardless of `[http] allow` \
                     contents. SSRF protection is a structural property of \
                     the language, not a configurable setting."
                ),
            });
        }

        // Allowlist check second. Empty allowlist = fail closed
        // with the missing-config diagnostic.
        let Some(allow) = &self.allow else {
            return Err(RuntimeError::ToolFailed {
                tool: "http".to_string(),
                message: format!(
                    "URL `{url}` cannot be reached: no `[http] allow` list is \
                     configured in this project's corvid.toml. Add the host to \
                     `[http]\\nallow = [\"...\"]` (or set CORVID_HTTP_ALLOW) to \
                     scope executing HTTP egress. The executing HTTP surface \
                     fails closed without an explicit allowlist — this is the \
                     33S0 security model."
                ),
            });
        };
        if !allow.iter().any(|h| h == &host) {
            return Err(RuntimeError::ToolFailed {
                tool: "http".to_string(),
                message: format!(
                    "URL `{url}` is refused: host `{host}` is not in this \
                     project's `[http] allow` allowlist ({}). Add it to the \
                     list or change the request URL.",
                    if allow.is_empty() {
                        "<empty>".to_string()
                    } else {
                        format!("[{}]", allow.join(", "))
                    }
                ),
            });
        }
        Ok(host)
    }
}

/// Extract the host part of a URL and lower-case it. Returns
/// `None` for inputs that don't carry an http(s) scheme or
/// have no host segment. Used by `HttpEgressPolicy::check` to
/// drive both the SSRF check and the allowlist comparison.
fn parse_host_lowercase(url: &str) -> Option<String> {
    let lower = url.trim();
    let after_scheme = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return None;
    };
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Authority may carry a port suffix (`host:port`) and
    // `user:pass@host`; strip both.
    let after_userinfo = authority.rsplit('@').next().unwrap_or(authority);
    // IPv6 literal authority uses `[::1]:port` form — the
    // brackets disambiguate the host from the port. Detect
    // and unwrap the bracketed segment before splitting on
    // `:`, otherwise the IPv6 colons cut the host short.
    let host_only = if let Some(rest) = after_userinfo.strip_prefix('[') {
        let close = rest.find(']')?;
        rest[..close].to_ascii_lowercase()
    } else {
        after_userinfo
            .split(':')
            .next()
            .unwrap_or(after_userinfo)
            .to_ascii_lowercase()
    };
    if host_only.is_empty() {
        None
    } else {
        Some(host_only)
    }
}

/// Classify a host string as one Corvid will refuse on SSRF
/// grounds. Covers:
///   * Literal IPv4 in private / loopback / link-local ranges
///     (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 127.0.0.0/8,
///     169.254.0.0/16).
///   * Literal IPv6 loopback `::1` and link-local `fe80::/10`.
///   * Hostname `localhost` (DNS-shaped loopback alias).
///
/// Hosts that don't fit any of these patterns pass through and
/// land in the allowlist check next.
fn is_ssrf_blocked_host(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    // IPv4 literal?
    if let Ok(ipv4) = host.parse::<std::net::Ipv4Addr>() {
        let octets = ipv4.octets();
        // 10.0.0.0/8
        if octets[0] == 10 {
            return true;
        }
        // 172.16.0.0/12
        if octets[0] == 172 && (16..=31).contains(&octets[1]) {
            return true;
        }
        // 192.168.0.0/16
        if octets[0] == 192 && octets[1] == 168 {
            return true;
        }
        // 127.0.0.0/8 — loopback
        if octets[0] == 127 {
            return true;
        }
        // 169.254.0.0/16 — link-local
        if octets[0] == 169 && octets[1] == 254 {
            return true;
        }
        // 0.0.0.0/8 — "any" / unspecified (treated as loopback
        // for egress purposes; binding to 0.0.0.0 means
        // every-local-interface, which includes loopback).
        if octets[0] == 0 {
            return true;
        }
        return false;
    }
    // IPv6 literal? Handle the bracketed form already stripped
    // by parse_host_lowercase — host arrives without `[]`.
    if let Ok(ipv6) = host.parse::<std::net::Ipv6Addr>() {
        if ipv6.is_loopback() {
            return true;
        }
        // Link-local fe80::/10.
        if ipv6.segments()[0] & 0xffc0 == 0xfe80 {
            return true;
        }
        // Unique-local fc00::/7.
        if ipv6.segments()[0] & 0xfe00 == 0xfc00 {
            return true;
        }
        // Unspecified ::.
        if ipv6.is_unspecified() {
            return true;
        }
        return false;
    }
    // Plain hostname (not a literal IP) — let it through to
    // the allowlist check. Production-grade SSRF would also
    // resolve the hostname via DNS and re-check the resolved
    // IPs; that adds a DNS round-trip on every call AND is
    // racy (the DNS answer can change between check and
    // connect). v1.0 ships with literal-IP coverage; richer
    // DNS-resolution-time SSRF is a follow-up slice.
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Slice 35V2-P38-C-5: a quarantined `HttpClient` short-circuits
    /// `send` with `RuntimeError::QuarantineViolation { surface:
    /// "http", .. }` before reaching the network. No mock server is
    /// needed — the test asserts the request never gets out.
    #[tokio::test]
    async fn quarantined_http_client_refuses_send_with_typed_violation() {
        let mut client = HttpClient::new();
        client.quarantine();
        assert!(client.is_quarantined());
        let req = HttpRequest::get("https://example.invalid/should-not-reach");
        let err = client.send(&req).await.expect_err("quarantine must error");
        match err {
            RuntimeError::QuarantineViolation { surface, detail } => {
                assert_eq!(surface, "http");
                assert!(
                    detail.contains("example.invalid/should-not-reach"),
                    "detail should name the URL: {detail}"
                );
                assert!(
                    detail.contains("GET"),
                    "detail should name the method: {detail}"
                );
            }
            other => panic!("expected http QuarantineViolation, got {other:?}"),
        }
    }

    /// Slice 35V2-P38-C-5: a default (non-quarantined) `HttpClient`
    /// continues to function — the flag defaults to false and the
    /// existing tests below all rely on it.
    #[tokio::test]
    async fn default_http_client_is_not_quarantined() {
        let client = HttpClient::new();
        assert!(!client.is_quarantined());
    }

    #[tokio::test]
    async fn http_client_gets_text_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hello"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
            .mount(&server)
            .await;

        let response = HttpClient::new()
            .send(&HttpRequest::get(format!("{}/hello", server.uri())))
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, "hello");
        assert_eq!(response.attempts, 1);
    }

    #[tokio::test]
    async fn http_client_retries_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let response = HttpClient::new()
            .send(
                &HttpRequest::get(format!("{}/flaky", server.uri())).retry(HttpRetryPolicy {
                    max_retries: 1,
                    retry_on_5xx: true,
                }),
            )
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, "ok");
        assert_eq!(response.attempts, 2);
    }

    #[tokio::test]
    async fn http_client_records_exchange_with_effect_tag() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/recorded"))
            .respond_with(ResponseTemplate::new(200).set_body_string("captured"))
            .mount(&server)
            .await;

        let request = HttpRequest::get(format!("{}/recorded", server.uri())).effect_tag("network:http");
        let (response, record) = HttpClient::new().send_recorded(&request).await.unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(record.status, 200);
        assert_eq!(record.effect_tag.as_deref(), Some("network:http"));
        assert_eq!(record.response_body, "captured");
        assert_eq!(record.request_fingerprint.len(), 64);
    }

    // ------------ Slice 33S2a: HttpEgressPolicy plumbing tests ------------

    /// 33S2a — Unconfigured policy fails closed: every `check`
    /// returns the missing-config diagnostic naming `[http] allow`
    /// and the 33S0 security model.
    #[test]
    fn http_egress_policy_unconfigured_fails_closed_on_check() {
        let policy = HttpEgressPolicy::unset();
        assert!(!policy.is_configured());
        assert!(policy.allow_list().is_empty());
        let err = policy
            .check("https://api.example.com/v1/health")
            .expect_err("unconfigured policy must fail closed");
        match err {
            RuntimeError::ToolFailed { tool, message } => {
                assert_eq!(tool, "http");
                assert!(
                    message.contains("[http] allow"),
                    "diagnostic must name [http] allow; got {message}"
                );
                assert!(
                    message.contains("33S0"),
                    "diagnostic must reference the 33S0 security model; got {message}"
                );
            }
            other => panic!("expected ToolFailed, got {other:?}"),
        }
    }

    /// 33S2a — Empty allowlist is the same as None: still fails
    /// closed. Operators who declare `[http] allow = []`
    /// explicitly get the same diagnostic.
    #[test]
    fn http_egress_policy_empty_allow_list_is_unconfigured() {
        let policy = HttpEgressPolicy::new(Some(&[]));
        assert!(!policy.is_configured());
        let err = policy
            .check("https://api.example.com/v1/health")
            .expect_err("empty allowlist must fail closed");
        assert!(matches!(err, RuntimeError::ToolFailed { .. }));
    }

    /// 33S2a — Configured allowlist permits matching hosts.
    /// Host comparison is case-insensitive (the policy lower-
    /// cases all entries at construction).
    #[test]
    fn http_egress_policy_allowlist_permits_matching_host() {
        let allow = vec!["api.example.com".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        assert!(policy.is_configured());
        let host = policy
            .check("https://api.example.com/v1/health")
            .expect("allowed host must pass");
        assert_eq!(host, "api.example.com");

        // Case-insensitive: caller URL has uppercase host.
        let host = policy
            .check("https://API.Example.COM/v1/health")
            .expect("case-insensitive match");
        assert_eq!(host, "api.example.com");
    }

    /// 33S2a — Configured allowlist refuses non-matching hosts
    /// with a diagnostic that names the offending host AND the
    /// full allowlist so the operator can see both.
    #[test]
    fn http_egress_policy_allowlist_refuses_unlisted_host_with_clear_diagnostic() {
        let allow = vec!["api.example.com".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        let err = policy
            .check("https://untrusted.example.org/data")
            .expect_err("unlisted host must be refused");
        let msg = format!("{err}");
        assert!(
            msg.contains("untrusted.example.org"),
            "diagnostic must name the refused host; got {msg}"
        );
        assert!(
            msg.contains("api.example.com"),
            "diagnostic must list the allowlist; got {msg}"
        );
    }

    /// 33S2a — SSRF block fires for all IPv4 private + loopback
    /// + link-local ranges, regardless of allowlist contents.
    /// The diagnostic names the violation as "structural" and
    /// "not a configurable setting" — load-bearing language for
    /// the security model.
    #[test]
    fn http_egress_policy_ssrf_block_refuses_all_private_loopback_ipv4_ranges() {
        let allow = vec![
            "10.0.0.1".to_string(),
            "127.0.0.1".to_string(),
            "192.168.1.1".to_string(),
            "169.254.169.254".to_string(),
            "0.0.0.0".to_string(),
        ];
        let policy = HttpEgressPolicy::new(Some(&allow));

        for url in [
            "http://10.0.0.1/admin",
            "http://10.255.255.255/admin",
            "http://172.16.0.1/admin",
            "http://172.31.255.255/admin",
            "http://192.168.1.1/admin",
            "http://127.0.0.1:8080/secret",
            "http://169.254.169.254/aws/instance-meta",
            "http://0.0.0.0/admin",
        ] {
            let err = policy
                .check(url)
                .expect_err(&format!("SSRF block must refuse {url}"));
            let msg = format!("{err}");
            assert!(
                msg.contains("structural SSRF block"),
                "URL {url}: diagnostic must name structural SSRF block; got {msg}"
            );
            assert!(
                msg.contains("not a configurable setting"),
                "URL {url}: diagnostic must say SSRF is not configurable; got {msg}"
            );
        }
    }

    /// 33S2a — SSRF block also fires for IPv6 loopback (`::1`),
    /// link-local (`fe80::/10`), unique-local (`fc00::/7`), and
    /// unspecified (`::`).
    #[test]
    fn http_egress_policy_ssrf_block_refuses_ipv6_loopback_and_link_local() {
        let policy = HttpEgressPolicy::new(Some(&["[::1]".to_string()]));
        // The parser strips the `[]` brackets — both forms hit
        // the SSRF check.
        for url in [
            "http://[::1]/admin",
            "http://[fe80::1]/admin",
            "http://[fc00::1]/admin",
            "http://[::]/admin",
        ] {
            let err = policy
                .check(url)
                .expect_err(&format!("IPv6 SSRF must refuse {url}"));
            assert!(
                format!("{err}").contains("structural SSRF block"),
                "URL {url}: expected SSRF block, got {err}"
            );
        }
    }

    /// 33S2a — `localhost` (DNS-shaped loopback alias) is also
    /// SSRF-blocked, NOT just literal IPs. A reviewer who tries
    /// `http://localhost:8080` gets refused without an explicit
    /// IP literal.
    #[test]
    fn http_egress_policy_ssrf_block_refuses_localhost_dns_alias() {
        let allow = vec!["localhost".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        let err = policy
            .check("http://localhost:8080/data")
            .expect_err("localhost must be refused even if explicitly allowed");
        assert!(format!("{err}").contains("structural SSRF block"));
    }

    /// 33S2a — Non-http(s) URLs fail with a structured error
    /// that says the request URL must be `http(s)://...`. Guards
    /// against `file://`, `ftp://`, `gopher://`, etc.
    #[test]
    fn http_egress_policy_rejects_non_http_schemes() {
        let allow = vec!["api.example.com".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        for url in [
            "file:///etc/passwd",
            "ftp://api.example.com/",
            "gopher://api.example.com/0/",
            "javascript:alert(1)",
            "",
        ] {
            let err = policy
                .check(url)
                .expect_err(&format!("non-http URL {url:?} must be rejected"));
            assert!(format!("{err}").contains("absolute http(s):// URL"));
        }
    }

    /// 33S2a — URL with `user:pass@host` userinfo strips
    /// correctly: the policy compares the bare host, not the
    /// userinfo-prefixed authority. Prevents authority-confusion
    /// SSRF where a request like
    /// `http://api.example.com@evil.com/x` would otherwise be
    /// mis-categorized.
    #[test]
    fn http_egress_policy_strips_userinfo_and_port_before_host_comparison() {
        let allow = vec!["api.example.com".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        // Authority is `evil.com` — the userinfo-prefixed form
        // must NOT be treated as `api.example.com`.
        let err = policy
            .check("http://api.example.com@evil.com/")
            .expect_err("authority-confusion URL must be refused");
        assert!(
            format!("{err}").contains("evil.com"),
            "diagnostic must name the actual authority host"
        );

        // Port suffix is stripped: `api.example.com:8443` still
        // matches the allowlist entry `api.example.com`.
        let host = policy
            .check("https://api.example.com:8443/data")
            .expect("port suffix must not block allowlist match");
        assert_eq!(host, "api.example.com");
    }
}
