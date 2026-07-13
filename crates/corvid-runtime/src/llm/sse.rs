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
    let bytes = resp.bytes_stream();
    futures::stream::unfold(
        (Box::pin(bytes), String::new(), std::collections::VecDeque::<String>::new(), false),
        move |(mut bytes, mut buffer, mut pending, mut done)| async move {
            loop {
                if let Some(line) = pending.pop_front() {
                    return Some((Ok(line), (bytes, buffer, pending, done)));
                }
                if done {
                    return None;
                }
                match bytes.next().await {
                    Some(Ok(chunk)) => {
                        match std::str::from_utf8(&chunk) {
                            Ok(text) => buffer.push_str(text),
                            Err(_) => {
                                // Provider chunks can split UTF-8;
                                // fall back to lossy for the tail.
                                buffer.push_str(&String::from_utf8_lossy(&chunk));
                            }
                        }
                        while let Some(pos) = buffer.find('\n') {
                            let line: String =
                                buffer.drain(..=pos).collect::<String>();
                            let line = line.trim_end_matches(['\n', '\r']).to_string();
                            if !line.is_empty() {
                                pending.push_back(line);
                            }
                        }
                    }
                    Some(Err(e)) => {
                        done = true;
                        return Some((
                            Err(RuntimeError::AdapterFailed {
                                adapter: adapter.into(),
                                message: format!("stream read failed: {e}"),
                            }),
                            (bytes, buffer, pending, done),
                        ));
                    }
                    None => {
                        done = true;
                        let tail = buffer.trim().to_string();
                        buffer.clear();
                        if !tail.is_empty() {
                            pending.push_back(tail);
                        }
                    }
                }
            }
        },
    )
}

/// Strip an SSE `data: ` prefix, if present.
pub(super) fn sse_data(line: &str) -> Option<&str> {
    line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:"))
}
