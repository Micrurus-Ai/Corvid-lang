//! Recording of outbound HTTP exchanges in the trace.
//!
//! The transport reports every hop of every attempt as an
//! [`HttpExchangeObservation`]; this turns each one into a
//! `TraceEvent::HttpExchange`, so a trace shows every host a call
//! reached, every retry, and how each ended. The observation carries
//! nothing credential-shaped, and the tracer's configured redaction
//! still applies to the URL.

use crate::http::HttpExchangeObservation;
use crate::tracing::now_ms;
use corvid_trace_schema::TraceEvent;

use super::Runtime;

impl Runtime {
    /// Record one exchange made on behalf of `operation`, a connector
    /// operation (with its `connector`) or a standard library tool.
    pub(crate) fn record_http_exchange(
        &self,
        connector: Option<&str>,
        operation: &str,
        observation: HttpExchangeObservation,
    ) {
        if !self.tracer.is_enabled() {
            return;
        }
        self.tracer.emit(TraceEvent::HttpExchange {
            ts_ms: now_ms(),
            run_id: self.tracer.run_id().to_string(),
            connector: connector.map(str::to_string),
            operation: operation.to_string(),
            attempt: observation.attempt,
            hop: observation.hop,
            method: observation.method,
            url: observation.url,
            request_headers: observation.request_headers,
            request_body_sha256: observation.request_body_sha256,
            request_body_bytes: observation.request_body_bytes,
            status: observation.status,
            response_headers: observation.response_headers,
            response_body_bytes: observation.response_body_bytes,
            elapsed_ms: observation.elapsed_ms,
            outcome: observation.outcome.as_str().to_string(),
            detail: observation.detail,
        });
    }
}
