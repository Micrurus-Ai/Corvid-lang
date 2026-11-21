//! Shared line-streaming helper for provider SSE / JSONL responses
//! (slice 46d).
//!
//! Buffers the HTTP byte stream and yields complete lines. SSE
//! consumers filter `data: ` prefixes; Ollama's JSONL consumer
//! parses each line directly.

use crate::errors::RuntimeError;
use futures::stream::Stream;
use futures::StreamExt;

/// Yield complete text lines from a streaming HTTP response.
/// Carriage returns are stripped; empty lines are skipped (SSE
/// event separators carry no data we consume).
pub(super) fn response_lines(
    resp: reqwest::Response,
    adapter: &'static str,
) -> impl Stream<Item = Result<String, RuntimeError>> + Send + 'static {
    async_stream::try_stream! {
        let mut bytes = resp.bytes_stream();
        let mut pending = Vec::new();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|e| RuntimeError::AdapterFailed {
                adapter: adapter.into(), message: format!("stream read failed: {e}"),
            })?;
            for part in chunk.split_inclusive(|byte| *byte == b'\n') {
                pending.extend_from_slice(part);
                if part.ends_with(b"\n") {
                    let line = decode_line(std::mem::take(&mut pending), adapter)?;
                    if !line.is_empty() { yield line; }
                }
            }
        }
        if !pending.is_empty() {
            yield decode_line(pending, adapter)?;
        }
    }
}

fn decode_line(mut bytes: Vec<u8>, adapter: &str) -> Result<String, RuntimeError> {
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    String::from_utf8(bytes).map_err(|_| RuntimeError::AdapterFailed {
        adapter: adapter.into(),
        message: "invalid UTF-8 in provider stream".into(),
    })
}

/// Strip an SSE `data: ` prefix, if present.
pub(super) fn sse_data(line: &str) -> Option<&str> {
    line.strip_prefix("data: ")
        .or_else(|| line.strip_prefix("data:"))
}
