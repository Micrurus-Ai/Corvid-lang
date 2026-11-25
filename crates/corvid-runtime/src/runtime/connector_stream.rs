//! Streamed connector calls (B05 slice 5).
//!
//! An operation that returns `Stream<T>` opens its response once and
//! yields one JSON event at a time as it arrives, framed by its declared
//! `streams:` format. The declared timeout bounds opening the response
//! and then the wait for each event, not the whole stream.
//!
//! Recording follows streamed prompts: the `ToolCall` is recorded when
//! the stream opens, and one `ToolResult` holding every event delivered
//! when it ends: `{"events": [...], "end": "complete" | "error" |
//! "cancelled", "error"?}`. A stream the program drops is recorded by the
//! guard's `Drop` as `cancelled`, with the events delivered so far.
//! Dropping the stream also drops the HTTP response, which closes the
//! upstream connection. Replay yields the recorded events, then the
//! recorded error if there was one, and never opens a connection.

use futures::StreamExt;
use std::pin::Pin;

use crate::connectors::build_connector_request;
use crate::errors::RuntimeError;
use crate::http_events::{EventFramer, FramedEvent};
use crate::tracing::{now_ms, Tracer};
use corvid_trace_schema::TraceEvent;

use super::Runtime;

/// The events of a streamed connector call, each a JSON value.
pub type ConnectorEventStream =
    Pin<Box<dyn futures::Stream<Item = Result<serde_json::Value, RuntimeError>> + Send>>;

impl Runtime {
    /// Open the streamed connector operation `name`. See the module docs.
    pub async fn call_connector_stream(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<ConnectorEventStream, RuntimeError> {
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: name.to_string(),
                args: args.clone(),
            });
        }
        if let Some(replay) = self.replay_source()? {
            let recorded = replay.replay_tool_call(name, &args)?;
            return Ok(replayed_stream(name, recorded));
        }
        let spec = self
            .connector_calls
            .get(name)
            .ok_or_else(|| RuntimeError::UnknownTool(name.to_string()))?;
        let mut recording = StreamRecording::new(self.tracer.clone(), name);
        let opened = async {
            let resolve = |secret: &str| self.secrets.read_env(secret).ok().and_then(|read| read.value);
            let request = build_connector_request(spec, &args, &resolve).map_err(|e| e.into_runtime_error())?;
            self.http_policy.check(&request.url)?;
            if let Some((limit, window_secs)) = spec.rate_limit {
                self.check_connector_rate_limit(&spec.connector, limit, window_secs)?;
            }
            let record = |observation| self.record_http_exchange(Some(&spec.connector), &spec.operation, observation);
            let opened = self.http.open_stream(&request, &record).await?;
            if !(200..300).contains(&opened.status) {
                return Err(RuntimeError::ToolFailed {
                    tool: spec.operation.clone(),
                    message: format!("provider returned HTTP {}", opened.status),
                });
            }
            Ok(opened)
        }
        .await;
        let opened = match opened {
            Ok(opened) => opened,
            Err(error) => {
                recording.finish("error", Some(error.to_string()));
                return Err(error);
            }
        };
        let format = spec.streams.ok_or_else(|| RuntimeError::ToolFailed {
            tool: spec.operation.clone(),
            message: "the operation declares no `streams:` format".to_string(),
        })?;
        let framer = EventFramer::new(format, spec.stream_until.clone());
        let operation = spec.operation.clone();
        let timeout = opened.timeout;
        let stream = async_stream::stream! {
            let mut recording = recording;
            let mut framer = framer;
            let mut body = opened.response.bytes_stream();
            let mut ready: std::collections::VecDeque<FramedEvent> = Default::default();
            let mut ended = false;
            loop {
                if let Some(event) = ready.pop_front() {
                    match event {
                        FramedEvent::End => {
                            recording.finish("complete", None);
                            break;
                        }
                        FramedEvent::Data(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(value) => {
                                recording.events.push(value.clone());
                                yield Ok(value);
                            }
                            Err(err) => {
                                let error = stream_error(&operation, format!("an event that is not JSON ({err}): {text}"));
                                recording.finish("error", Some(error.to_string()));
                                yield Err(error);
                                break;
                            }
                        },
                    }
                    continue;
                }
                if ended {
                    recording.finish("complete", None);
                    break;
                }
                let framed = match tokio::time::timeout(timeout, body.next()).await {
                    Err(_) => Err(format!("no event within its declared timeout of {}s", timeout.as_secs())),
                    Ok(None) => {
                        ended = true;
                        framer.finish()
                    }
                    Ok(Some(Err(err))) => Err(format!("the stream broke: {err}")),
                    Ok(Some(Ok(bytes))) => framer.feed(&bytes),
                };
                match framed {
                    Ok(events) => ready.extend(events),
                    Err(message) => {
                        let error = stream_error(&operation, message);
                        recording.finish("error", Some(error.to_string()));
                        yield Err(error);
                        break;
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }
}

fn stream_error(operation: &str, message: String) -> RuntimeError {
    RuntimeError::ToolFailed { tool: operation.to_string(), message }
}

/// A recorded stream, replayed: its events, then its error if it had one.
fn replayed_stream(name: &str, recorded: serde_json::Value) -> ConnectorEventStream {
    let events: Vec<serde_json::Value> = recorded
        .get("events")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let error = (recorded.get("end").and_then(|e| e.as_str()) == Some("error")).then(|| RuntimeError::ToolFailed {
        tool: name.to_string(),
        message: recorded.get("error").and_then(|e| e.as_str()).unwrap_or("the recorded stream failed").to_string(),
    });
    let items = events.into_iter().map(Ok).chain(error.map(Err));
    Box::pin(futures::stream::iter(items.collect::<Vec<_>>()))
}

/// Records the stream's `ToolResult` exactly once: when it ends, fails,
/// or is dropped before either (`cancelled`).
struct StreamRecording {
    tracer: Tracer,
    tool: String,
    events: Vec<serde_json::Value>,
    finished: bool,
}

impl StreamRecording {
    fn new(tracer: Tracer, tool: &str) -> Self {
        Self { tracer, tool: tool.to_string(), events: Vec::new(), finished: false }
    }

    fn finish(&mut self, end: &str, error: Option<String>) {
        if self.finished {
            return;
        }
        self.finished = true;
        if !self.tracer.is_enabled() {
            return;
        }
        let mut result = serde_json::json!({ "events": std::mem::take(&mut self.events), "end": end });
        if let Some(error) = error {
            result["error"] = serde_json::json!(error);
        }
        self.tracer.emit(TraceEvent::ToolResult {
            ts_ms: now_ms(),
            run_id: self.tracer.run_id().to_string(),
            tool: self.tool.clone(),
            result,
        });
    }
}

impl Drop for StreamRecording {
    fn drop(&mut self) {
        self.finish("cancelled", None);
    }
}
