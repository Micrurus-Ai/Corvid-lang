//! Real-mode connector HTTP dispatch on `Runtime`.
//!
//! Reached from the connector branch of [`Runtime::call_tool`] only in
//! `real` mode (a replay source short-circuits earlier, so replay never
//! performs a real request). Resolves the credential from the secret
//! store at the last moment, sends the request through the egress-gated
//! HTTP client, and hands the response body back for the VM to decode.
//!
//! The credential value appears ONLY in the outgoing request header —
//! never in the ToolCall arguments, the ToolResult body, or an error.

use super::Runtime;
use crate::connectors::{build_connector_request, ConnectorHttpSpec};
use crate::errors::RuntimeError;

/// What the provider said ABOUT the exchange, alongside the decoded
/// payload. The governed scheduler needs the provider's own
/// backoff request; ordinary connector calls ignore it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectorResponseMeta {
    pub status: u16,
    /// `Retry-After`, in milliseconds, when the provider asked us to wait.
    /// Only the delta-seconds form is honoured; an HTTP-date form is
    /// ignored rather than guessed at.
    pub retry_after_ms: Option<u64>,
}

impl Runtime {
    pub(super) async fn dispatch_connector_http(
        &self,
        spec: &ConnectorHttpSpec,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, RuntimeError> {
        self.dispatch_connector_http_detailed(spec, args)
            .await
            .map(|(payload, _meta)| payload)
    }

    /// As [`Self::dispatch_connector_http`], but also reporting what the
    /// provider said about the exchange (status + `Retry-After`). The
    /// verified-protocol poll loop uses this so a provider can push back
    /// on our cadence.
    pub(super) async fn dispatch_connector_http_detailed(
        &self,
        spec: &ConnectorHttpSpec,
        args: &[serde_json::Value],
    ) -> Result<(serde_json::Value, ConnectorResponseMeta), RuntimeError> {
        // Resolve credentials from the secret store at the last moment.
        // The resolver hands the value ONLY to the request builder, which
        // places it in a header; it is never returned or recorded.
        let resolve = |name: &str| -> Option<String> {
            self.secrets.read_env(name).ok().and_then(|read| read.value)
        };
        let mut request =
            build_connector_request(spec, args, &resolve).map_err(|e| e.into_runtime_error())?;

        // Outbound egress gate: the always-on SSRF block plus the
        // configured `[http] allow` list. A connector never reaches an
        // unpermitted host, even though the base URL is fixed in source.
        self.http_policy.check(&request.url)?;

        // Retries are driven HERE, above the rate limiter, not inside the
        // HTTP client beneath it.
        //
        // A `rate_limit` is a promise about how many requests the provider
        // receives. With the retry policy handed to the client, one
        // admitted logical call could emit `1 + retry` NETWORK requests —
        // so a connector declaring `retry: 3` and `rate_limit: 10 per 60s`
        // could legally send 40. The limiter has to admit each attempt, or
        // it is not a rate limit on the thing the provider actually sees.
        let attempts = spec.retry.unwrap_or(0).saturating_add(1);
        request.retry = crate::http::HttpRetryPolicy {
            max_retries: 0,
            retry_on_5xx: false,
        };

        let mut last_err = None;
        let mut response = None;
        for attempt in 0..attempts {
            // Every attempt is admitted separately. An exceeded limit
            // fails BEFORE the request is sent, so neither a runaway loop
            // nor a retry storm can flood the provider.
            if let Some((limit, window_secs)) = spec.rate_limit {
                self.check_connector_rate_limit(&spec.connector, limit, window_secs)?;
            }
            match self.http.send(&request).await {
                Ok(got) => {
                    // Retry only what the previous policy retried: a 5xx.
                    // A 4xx is the provider's answer, not a hiccup, and
                    // re-sending it burns the declared budget for nothing.
                    if got.status >= 500 && attempt + 1 < attempts {
                        response = Some(got);
                        continue;
                    }
                    response = Some(got);
                    break;
                }
                Err(err) => {
                    last_err = Some(err);
                    if attempt + 1 >= attempts {
                        break;
                    }
                }
            }
        }
        let response = match response {
            Some(response) => response,
            None => return Err(last_err.expect("a failed attempt records its error")),
        };
        let meta = ConnectorResponseMeta {
            status: response.status,
            retry_after_ms: retry_after_ms(&response),
        };

        // Typed status -> error mapping. A status named
        // by an `on status <code> -> Variant` mapping becomes the typed
        // (nullary) error variant: the JSON envelope decodes to
        // `Err(Variant)` against the operation's `Result<_, Error>`
        // return type. Checked FIRST so a mapped status wins over the
        // 2xx/non-2xx split.
        if let Some((_, variant)) = spec
            .error_map
            .iter()
            .find(|(code, _)| *code == response.status)
        {
            return Ok((
                serde_json::json!({
                    "tag": "err",
                    "err": { "tag": "variant", "variant": variant, "fields": [] },
                }),
                meta,
            ));
        }

        // A 2xx body is decoded to the operation's success type. When the
        // operation returns `Result<Success, Error>` it is wrapped as
        // `Ok(..)`; otherwise the body decodes directly.
        if (200..300).contains(&response.status) {
            let body: serde_json::Value =
                serde_json::from_str(&response.body).unwrap_or(serde_json::Value::Null);
            if spec.returns_result {
                Ok((serde_json::json!({ "tag": "ok", "ok": body }), meta))
            } else {
                Ok((body, meta))
            }
        } else {
            // An unmapped non-2xx status is a transport-level failure.
            Err(RuntimeError::ToolFailed {
                tool: spec.operation.clone(),
                message: format!("provider returned HTTP {}", response.status),
            })
        }
    }

    /// Fixed-window rate limiter keyed by connector. Increments the
    /// window count; when it exceeds `limit` within `window_secs`, the
    /// call fails with a rate-limit error before any request is sent.
    /// Uses wall-clock time — only real-mode dispatch reaches here, and
    /// real mode is never replayed, so this introduces no replay
    /// nondeterminism.
    fn check_connector_rate_limit(
        &self,
        connector: &str,
        limit: u64,
        window_secs: u64,
    ) -> Result<(), RuntimeError> {
        let now_ms = crate::tracing::now_ms();
        let window_ms = window_secs.saturating_mul(1000).max(1);
        let mut state = self
            .connector_rate_state
            .lock()
            .expect("connector rate state poisoned");
        let entry = state.entry(connector.to_string()).or_insert((now_ms, 0));
        if now_ms.saturating_sub(entry.0) >= window_ms {
            // Window elapsed — reset.
            *entry = (now_ms, 0);
        }
        if entry.1 >= limit {
            return Err(RuntimeError::ToolFailed {
                tool: connector.to_string(),
                message: format!(
                    "connector rate limit exceeded ({limit} per {window_secs}s) — refusing to \
                     flood the provider"
                ),
            });
        }
        entry.1 += 1;
        Ok(())
    }
}

/// Read `Retry-After`. Only the delta-seconds form is
/// honoured — an HTTP-date needs a trusted clock comparison, and guessing
/// at it would be worse than ignoring it, so an unparseable value simply
/// leaves the declared cadence in charge.
fn retry_after_ms(response: &crate::http::HttpResponse) -> Option<u64> {
    response
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("retry-after"))
        .and_then(|h| h.value.trim().parse::<u64>().ok())
        .map(|secs| secs.saturating_mul(1000))
}
