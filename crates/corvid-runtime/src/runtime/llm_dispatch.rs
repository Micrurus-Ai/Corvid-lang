//! LLM and tool dispatch methods on `Runtime`, plus the
//! approval-gate state machine and the human-in-the-loop ask /
//! choose helpers. These are the central call paths the
//! interpreter takes for every LLM, tool, approval, and human
//! interaction — they own the trace-event bracketing and the
//! replay-source consultation that those paths require.

use sha2::{Digest, Sha256};

use crate::approvals::{ApprovalDecision, ApprovalRequest, ApprovalToken};
use crate::errors::RuntimeError;
use crate::human::{HumanChoiceRequest, HumanInputRequest};
use crate::llm::{LlmRequest, LlmRequestRef, LlmResponse};
use crate::prompt_cache::PromptCache;
use crate::tracing::now_ms;
use crate::usage::{normalized_total_tokens, LlmUsageRecord};
use corvid_trace_schema::TraceEvent;

use super::{Runtime, APPROVAL_TOKEN_SCOPE_ONE_TIME, APPROVAL_TOKEN_TTL_MS};

impl Runtime {
    // ---- dispatch helpers ----

    /// Call a tool by name. Emits trace events bracketing the call.
    ///
    /// Slice 33S1a (with 33S1-fix-naming 2026-06-08): tool names
    /// starting with `io_` are intercepted and routed to
    /// `dispatch_stdlib_io_tool` so the executing file-I/O stdlib
    /// tools (declared in `std/io.cor` as `io_read_text`,
    /// `io_write_text`, `io_list_dir`) can reach the `IoRuntime` +
    /// the `[io] root` policy that the standard `tools.call`
    /// handler-closure path can't see. The original 33S1a wired
    /// the interception against an `io.` dotted prefix, but the IR
    /// lowers `import "./std/io" use io_read_text; io_read_text(p)`
    /// to a tool call with bare `callee_name = "io_read_text"` —
    /// no module prefix. Underscore matches the bare IR name.
    /// Replay-mode reads still substitute from the recorded trace
    /// (the `replay_source` branch runs first); writes pass through
    /// the dispatch which then hits the `IoRuntime::quarantine_writes`
    /// guard if write-quarantine is on.
    pub async fn call_tool(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, RuntimeError> {
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: name.to_string(),
                args: args.clone(),
            });
        }
        let result = if let Some(replay) = self.replay_source()? {
            replay.replay_tool_call(name, &args)?
        } else if is_stdlib_io_tool(name) {
            self.dispatch_stdlib_io_tool(name, args.clone()).await?
        } else if is_stdlib_http_tool(name) {
            self.dispatch_stdlib_http_tool(name, args.clone()).await?
        } else if is_stdlib_time_tool(name) {
            dispatch_stdlib_time_tool(name, &args)?
        } else if is_stdlib_random_tool(name) {
            dispatch_stdlib_random_tool(name, &args)?
        } else {
            self.tools.call(name, args.clone()).await?
        };
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: name.to_string(),
                result: result.clone(),
            });
        }
        Ok(result)
    }

    /// Slice 33S1a: dispatch handler for the three executing
    /// stdlib file-I/O tools (`io_read_text` / `io_write_text` /
    /// `io_list_dir`). Receives the full tool name; the caller
    /// (`call_tool`) gated entry via `is_stdlib_io_tool` so an
    /// unknown name here is a programmer error rather than a
    /// fall-through. Each branch:
    ///
    ///   1. Extracts + validates args as JSON values.
    ///   2. Resolves the caller's path through `self.io_policy`
    ///      (fails closed when `[io] root` is unconfigured;
    ///      rejects traversal + absolute escapes).
    ///   3. Calls the existing `IoRuntime` method.
    ///   4. Marshals the typed result back to a JSON value
    ///      matching the envelope schema declared in `std/io.cor`
    ///      (FileReadEnvelope / FileWriteEnvelope /
    ///      [DirectoryEntryEnvelope]).
    async fn dispatch_stdlib_io_tool(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, RuntimeError> {
        match name {
            "io_read_text" => {
                let path_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "io_read_text".to_string(),
                        message: "expected one String argument (path)".to_string(),
                    })?;
                let resolved = self.io_policy.resolve(path_arg)?;
                let read = self.io.read_text(&resolved).await?;
                Ok(serde_json::json!({
                    "path_value": read.path.display().to_string(),
                    "contents": read.contents,
                    "bytes": read.bytes as i64,
                    "effect_meta": stdlib_io_effect_envelope(&read.effect),
                }))
            }
            "io_write_text" => {
                let path_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "io_write_text".to_string(),
                        message: "expected (path: String, content: String) — path missing"
                            .to_string(),
                    })?;
                let content_arg = args
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "io_write_text".to_string(),
                        message: "expected (path: String, content: String) — content missing"
                            .to_string(),
                    })?;
                let resolved = self.io_policy.resolve(path_arg)?;
                let write = self.io.write_text(&resolved, content_arg).await?;
                Ok(serde_json::json!({
                    "path_value": write.path.display().to_string(),
                    "bytes": write.bytes as i64,
                    "effect_meta": stdlib_io_effect_envelope(&write.effect),
                }))
            }
            "io_list_dir" => {
                let path_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "io_list_dir".to_string(),
                        message: "expected one String argument (path)".to_string(),
                    })?;
                let resolved = self.io_policy.resolve(path_arg)?;
                let entries = self.io.list_dir(&resolved).await?;
                let json_entries: Vec<serde_json::Value> = entries
                    .into_iter()
                    .map(|entry| {
                        serde_json::json!({
                            "path_value": entry.path.display().to_string(),
                            "name": entry.name,
                            "is_dir": entry.is_dir,
                            "effect_meta": stdlib_io_effect_envelope(&entry.effect),
                        })
                    })
                    .collect();
                Ok(serde_json::Value::Array(json_entries))
            }
            other => Err(RuntimeError::UnknownTool(other.to_string())),
        }
    }

    /// Slice 33S2a: dispatch handler for the two executing
    /// stdlib HTTP tools (`http_get` / `http_post_json`).
    /// Receives the full tool name; entry is gated by
    /// `is_stdlib_http_tool`. Each branch:
    ///
    ///   1. Extracts + validates args as JSON values.
    ///   2. Checks the URL through `self.http_policy` (always-on
    ///      SSRF block + required `[http] allow` allowlist; fails
    ///      closed when allowlist is unconfigured).
    ///   3. Calls `HttpClient::send` with the constructed
    ///      `HttpRequest`.
    ///   4. Marshals the typed `HttpResponse` back to a JSON
    ///      value matching the `HttpResponseEnvelope` schema
    ///      declared in `std/http.cor`.
    async fn dispatch_stdlib_http_tool(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, RuntimeError> {
        use crate::http::HttpRequest;
        match name {
            "http_get" => {
                let url_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "http_get".to_string(),
                        message: "expected one String argument (url)".to_string(),
                    })?;
                self.http_policy.check(url_arg)?;
                let request = HttpRequest::get(url_arg.to_string())
                    .effect_tag("std.http.request");
                let response = self.http.send(&request).await?;
                Ok(serde_json::json!({
                    "status": response.status as i64,
                    "body": response.body,
                    "attempts": 1_i64,
                    "elapsed_ms": 0_i64,
                    "effect_meta": stdlib_http_effect_envelope(),
                }))
            }
            "http_post_json" => {
                let url_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "http_post_json".to_string(),
                        message: "expected (url: String, body: String) — url missing"
                            .to_string(),
                    })?;
                let body_arg = args
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "http_post_json".to_string(),
                        message: "expected (url: String, body: String) — body missing"
                            .to_string(),
                    })?;
                self.http_policy.check(url_arg)?;
                let request = HttpRequest::post_json(url_arg.to_string(), body_arg.to_string())
                    .effect_tag("std.http.request");
                let response = self.http.send(&request).await?;
                Ok(serde_json::json!({
                    "status": response.status as i64,
                    "body": response.body,
                    "attempts": 1_i64,
                    "elapsed_ms": 0_i64,
                    "effect_meta": stdlib_http_effect_envelope(),
                }))
            }
            other => Err(RuntimeError::UnknownTool(other.to_string())),
        }
    }

    // ========================================================================
    // Slice 33S3b — typed-Value dispatch for the executing SQLite
    // surface.
    //
    // Unlike the io / http stdlib tools (which round-trip through
    // `serde_json::Value`), the SQLite tools `db_open` / `db_query` /
    // `db_execute` need a typed-Value dispatch path because
    // `Value::DbHandle(Arc<DbHandleInner>)` cannot be marshalled
    // through JSON — the inner `Arc` carries pointer identity into
    // the runtime's `DbHandleRegistry` slotmap, and a JSON value
    // cannot carry that pointer. The opacity gate in
    // `corvid_vm::conv::json_to_value` rejects any attempt to
    // reconstruct a handle from JSON.
    //
    // The three methods below are the typed entry points the
    // interpreter calls (via the special-case branch in
    // `interp.rs`'s tool-call site). `db_open_tool` returns an
    // `Arc<DbHandleInner>` directly; `db_query_tool` /
    // `db_execute_tool` take an `Arc<DbHandleInner>` reference
    // (the same Arc the caller's `Value::DbHandle` wraps) and
    // return JSON envelopes the interpreter then marshals into
    // typed `DbResult` / `List<DbResult>` values.
    //
    // Path confinement: `db_open_tool` resolves the path through
    // `self.io_policy` so `db_open(path)` is structurally as
    // narrow as the `io_*` tools — a program with `[io] root =
    // "./data"` cannot open `/etc/passwd` as a database, even
    // though sqlite would have refused it as malformed (the
    // STRUCTURAL refusal happens BEFORE rusqlite ever sees the
    // path). The documented special case `":memory:"` bypasses
    // policy resolution because there is no filesystem path to
    // confine.
    //
    // Trace emission: all three methods emit `ToolCall` /
    // `ToolResult` events so traces capture db operations
    // alongside the io / http surfaces. The JSON shape for the
    // ToolCall payload uses the opaque sentinel
    // (`{"tag": "db_handle_opaque_sentinel", ...}`) for handles —
    // the SAME shape `value_to_json` emits — so trace renderers
    // can show "a DbHandle was used here." The opacity gate
    // still holds because the only path that mints a handle is
    // `db_open_tool` itself, which the interpreter recognises by
    // name.
    // ========================================================================

    /// Slice 33S3b — typed-Value dispatch for `db_open`. Resolves
    /// the supplied path through the IoToolPolicy (with the
    /// `":memory:"` special case), opens a SQLite connection,
    /// registers it under a fresh handle id, and returns an
    /// `Arc<DbHandleInner>` the interpreter wraps in
    /// `Value::DbHandle`. Production callers go through the
    /// interpreter's `Type::DbHandle`-aware branch in
    /// `crates/corvid-vm/src/interp.rs`; tests can call this
    /// directly to mint a handle.
    pub async fn db_open_tool(
        &self,
        path: String,
    ) -> Result<std::sync::Arc<crate::db::DbHandleInner>, RuntimeError> {
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: "db_open".to_string(),
                args: vec![serde_json::Value::String(path.clone())],
            });
        }
        let resolved_path = if path == ":memory:" {
            ":memory:".to_string()
        } else {
            self.io_policy.resolve(&path)?.display().to_string()
        };
        let handle = self.db_registry.open(&resolved_path)?;
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: "db_open".to_string(),
                result: serde_json::json!({
                    "tag": "db_handle_opaque_sentinel",
                    "handle_id": handle.handle_id,
                    "path": handle.path,
                }),
            });
        }
        Ok(handle)
    }

    /// Slice 33S3b — typed-Value dispatch for `db_query`. Takes
    /// the `Arc<DbHandleInner>` directly (the interpreter
    /// extracts it from the caller's `Value::DbHandle`), runs
    /// the parameterised SELECT through the registry, marshals
    /// rows into the JSON envelope the interpreter then converts
    /// into a `List<DbResult>` via the standard `json_to_value`
    /// path.
    pub async fn db_query_tool(
        &self,
        handle: &std::sync::Arc<crate::db::DbHandleInner>,
        sql: String,
        params: Vec<crate::db::DbValue>,
    ) -> Result<serde_json::Value, RuntimeError> {
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: "db_query".to_string(),
                args: vec![
                    serde_json::json!({
                        "tag": "db_handle_opaque_sentinel",
                        "handle_id": handle.handle_id,
                        "path": handle.path,
                    }),
                    serde_json::Value::String(sql.clone()),
                    serde_json::Value::Array(
                        params.iter().map(db_value_to_json_param).collect(),
                    ),
                ],
            });
        }
        let rows = self.db_registry.query(handle.handle_id, &sql, &params)?;
        let payload = db_query_rows_to_envelope(&rows);
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: "db_query".to_string(),
                result: payload.clone(),
            });
        }
        Ok(payload)
    }

    /// Slice 33S3b — typed-Value dispatch for `db_execute`. Same
    /// shape as `db_query_tool` but routes through
    /// `DbHandleRegistry::execute`, which refuses with
    /// `QuarantineViolation { surface: "db", .. }` during
    /// Substitute-mode replay (the registry's `write-quarantine`
    /// flag is flipped by `RuntimeBuilder::build`).
    pub async fn db_execute_tool(
        &self,
        handle: &std::sync::Arc<crate::db::DbHandleInner>,
        sql: String,
        params: Vec<crate::db::DbValue>,
    ) -> Result<serde_json::Value, RuntimeError> {
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: "db_execute".to_string(),
                args: vec![
                    serde_json::json!({
                        "tag": "db_handle_opaque_sentinel",
                        "handle_id": handle.handle_id,
                        "path": handle.path,
                    }),
                    serde_json::Value::String(sql.clone()),
                    serde_json::Value::Array(
                        params.iter().map(db_value_to_json_param).collect(),
                    ),
                ],
            });
        }
        let result = self.db_registry.execute(handle.handle_id, &sql, &params)?;
        let payload = serde_json::json!({
            "rows_affected": result.rows_affected as i64,
            "row_count": 0_i64,
            "replay_key": "",
            "effect_meta": stdlib_db_effect_envelope(),
        });
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: "db_execute".to_string(),
                result: payload.clone(),
            });
        }
        Ok(payload)
    }

    // ========================================================================
    // Slice 33R5b-a — typed-Value dispatch for the executing JSON
    // surface.
    //
    // The opaque path: `json_parse` returns a Value::JsonValue
    // (the interpreter wraps the Arc<serde_json::Value> we hand
    // back); `json_get_*` take the Arc and return JSON envelopes
    // matching the `Result<T, String>` shape declared in
    // `std/json.cor`; `json_object_new` returns a
    // Value::JsonBuilder; `json_object_set_*` mutate the builder
    // and return the SAME Arc; `json_object_finish` serialises.
    //
    // Unlike the io / http / db surfaces, JSON has no security
    // boundary beyond serde validation — no policy parameter,
    // no allowlist, no quarantine flag. The structural property
    // the dispatch carries is `json.parse_safety_no_panic`:
    // malformed input returns `Result::Err(message)` through the
    // standard Corvid Result envelope.
    // ========================================================================

    /// Slice 33R5b-a — `json_parse(text) -> Result<JsonValue, String>`.
    /// Returns a typed Value directly (the interpreter wraps the
    /// `Arc<serde_json::Value>` we return in `Value::JsonValue`
    /// for the Ok branch; the Err branch flows through
    /// `Value::ResultErr`).
    pub async fn json_parse_tool(
        &self,
        text: String,
    ) -> Result<Result<std::sync::Arc<serde_json::Value>, String>, RuntimeError> {
        Ok(crate::json::parse(&text))
    }

    /// Slice 33R5b-a — `json_get_int(value, field) -> Result<Int, String>`.
    /// Takes the `Arc<serde_json::Value>` directly (the interpreter
    /// extracts it from `Value::JsonValue`) and returns the typed
    /// Result.
    pub async fn json_get_int_tool(
        &self,
        value: &std::sync::Arc<serde_json::Value>,
        field: String,
    ) -> Result<Result<i64, String>, RuntimeError> {
        Ok(crate::json::get_int(value, &field))
    }

    /// Slice 33R5b-a — `json_get_float`. Same shape as `json_get_int`.
    pub async fn json_get_float_tool(
        &self,
        value: &std::sync::Arc<serde_json::Value>,
        field: String,
    ) -> Result<Result<f64, String>, RuntimeError> {
        Ok(crate::json::get_float(value, &field))
    }

    /// Slice 33R5b-a — `json_get_string`.
    pub async fn json_get_string_tool(
        &self,
        value: &std::sync::Arc<serde_json::Value>,
        field: String,
    ) -> Result<Result<String, String>, RuntimeError> {
        Ok(crate::json::get_string(value, &field))
    }

    /// Slice 33R5b-a — `json_get_bool`.
    pub async fn json_get_bool_tool(
        &self,
        value: &std::sync::Arc<serde_json::Value>,
        field: String,
    ) -> Result<Result<bool, String>, RuntimeError> {
        Ok(crate::json::get_bool(value, &field))
    }

    /// Slice 33R5b-a — `json_get_object`. Returns a fresh Arc
    /// over the cloned subtree so the caller can pass it back
    /// into other typed accessors.
    pub async fn json_get_object_tool(
        &self,
        value: &std::sync::Arc<serde_json::Value>,
        field: String,
    ) -> Result<Result<std::sync::Arc<serde_json::Value>, String>, RuntimeError> {
        Ok(crate::json::get_object(value, &field))
    }

    /// Slice 33R5b-a — `json_get_array`. Returns `Vec<Arc<JsonValue>>`
    /// the interpreter wraps as `List<JsonValue>`.
    pub async fn json_get_array_tool(
        &self,
        value: &std::sync::Arc<serde_json::Value>,
        field: String,
    ) -> Result<Result<Vec<std::sync::Arc<serde_json::Value>>, String>, RuntimeError> {
        Ok(crate::json::get_array(value, &field))
    }

    /// Slice 33R5b-a — `json_object_new() -> JsonBuilder`.
    /// Returns the `Arc<Mutex<Map>>` directly (the interpreter
    /// wraps in Value::JsonBuilder).
    pub async fn json_object_new_tool(
        &self,
    ) -> Result<
        std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        RuntimeError,
    > {
        Ok(crate::json::object_new())
    }

    /// Slice 33R5b-a — `json_object_set_int(builder, key, value) -> JsonBuilder`.
    /// Mutates and returns the same builder Arc.
    pub async fn json_object_set_int_tool(
        &self,
        builder: std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        key: String,
        value: i64,
    ) -> Result<
        std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        RuntimeError,
    > {
        crate::json::object_set_int(builder, &key, value)
    }

    /// Slice 33R5b-a — `json_object_set_float`.
    pub async fn json_object_set_float_tool(
        &self,
        builder: std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        key: String,
        value: f64,
    ) -> Result<
        std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        RuntimeError,
    > {
        crate::json::object_set_float(builder, &key, value)
    }

    /// Slice 33R5b-a — `json_object_set_string`.
    pub async fn json_object_set_string_tool(
        &self,
        builder: std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        key: String,
        value: String,
    ) -> Result<
        std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        RuntimeError,
    > {
        crate::json::object_set_string(builder, &key, &value)
    }

    /// Slice 33R5b-a — `json_object_set_bool`.
    pub async fn json_object_set_bool_tool(
        &self,
        builder: std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        key: String,
        value: bool,
    ) -> Result<
        std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
        RuntimeError,
    > {
        crate::json::object_set_bool(builder, &key, value)
    }

    /// Slice 33R5b-a — `json_object_finish(builder) -> String`.
    /// Snapshots the builder's current state and serialises to a
    /// String. The builder remains usable for further set+finish
    /// cycles.
    pub async fn json_object_finish_tool(
        &self,
        builder: &std::sync::Arc<
            std::sync::Mutex<serde_json::Map<String, serde_json::Value>>,
        >,
    ) -> Result<String, RuntimeError> {
        crate::json::object_finish(builder)
    }

    /// Call an LLM. Falls back to `default_model` if `req.model` is empty.
    pub async fn call_llm(&self, mut req: LlmRequest) -> Result<LlmResponse, RuntimeError> {
        if req.model.is_empty() {
            req.model = self.default_model.clone();
        }
        self.call_llm_ref(req.as_ref()).await
    }

    /// Call an LLM through the prompt-response cache when the source
    /// prompt declared `cacheable: true`. Replay mode bypasses the live
    /// cache and consumes the recorded `LlmCall` / `LlmResult` pair instead.
    pub async fn call_llm_cacheable(
        &self,
        mut req: LlmRequest,
        cacheable: bool,
    ) -> Result<LlmResponse, RuntimeError> {
        if req.model.is_empty() {
            req.model = self.default_model.clone();
        }
        self.call_llm_ref_impl(req.as_ref(), None, cacheable).await
    }

    pub async fn call_llm_ref_with_trace_rendered(
        &self,
        req: LlmRequestRef<'_>,
        trace_rendered: Option<&str>,
    ) -> Result<LlmResponse, RuntimeError> {
        self.call_llm_ref_impl(req, trace_rendered, false).await
    }

    /// Borrowed LLM-call path for native bridges that already hold prompt and
    /// rendered text as borrowed strings and only need owned clones when
    /// tracing or provider JSON construction requires them.
    pub async fn call_llm_ref(&self, req: LlmRequestRef<'_>) -> Result<LlmResponse, RuntimeError> {
        self.call_llm_ref_impl(req, None, false).await
    }

    async fn call_llm_ref_impl(
        &self,
        req: LlmRequestRef<'_>,
        trace_rendered_override: Option<&str>,
        cacheable: bool,
    ) -> Result<LlmResponse, RuntimeError> {
        let req = if req.model.is_empty() {
            req.with_model(&self.default_model)
        } else {
            req
        };
        let trace_rendered = trace_rendered_override.unwrap_or(req.rendered);
        let replay = self.replay_source()?;
        let live_model_override = replay
            .and_then(|source| source.live_model_override())
            .map(str::to_owned);
        let trace_model = live_model_override.as_deref().unwrap_or(req.model);
        let recorded_model_version = self.model_version(req.model);
        let trace_model_version = self.model_version(trace_model);
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::LlmCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                prompt: req.prompt.to_string(),
                model: if trace_model.is_empty() {
                    None
                } else {
                    Some(trace_model.to_string())
                },
                model_version: trace_model_version.clone(),
                rendered: Some(trace_rendered.to_string()),
                args: req.args.to_vec(),
                sampling: req.sampling.to_trace_json(),
            });
        }
        let cache_fingerprint = if cacheable && replay.is_none() {
            Some(PromptCache::fingerprint(req))
        } else {
            None
        };
        if let Some(fingerprint) = cache_fingerprint.as_deref() {
            if let Some(cached) = self.prompt_cache.get(fingerprint) {
                if self.tracer.is_enabled() {
                    self.tracer.emit(TraceEvent::PromptCache {
                        ts_ms: now_ms(),
                        run_id: self.tracer.run_id().to_string(),
                        prompt: req.prompt.to_string(),
                        model: if trace_model.is_empty() {
                            None
                        } else {
                            Some(trace_model.to_string())
                        },
                        model_version: trace_model_version.clone(),
                        fingerprint: fingerprint.to_string(),
                        hit: true,
                    });
                    self.tracer.emit(TraceEvent::LlmResult {
                        ts_ms: now_ms(),
                        run_id: self.tracer.run_id().to_string(),
                        prompt: req.prompt.to_string(),
                        model: if trace_model.is_empty() {
                            None
                        } else {
                            Some(trace_model.to_string())
                        },
                        model_version: trace_model_version.clone(),
                        result: cached.value.clone(),
                    });
                }
                return Ok(PromptCache::cached_response(cached));
            }
        }
        let mut actual_model = live_model_override
            .as_deref()
            .unwrap_or(req.model)
            .to_string();
        let mut actual_adapter = if replay.is_some() {
            self.llms.adapter_name_for_model(&actual_model)
        } else {
            None
        };
        let mut result_trace_model = trace_model.to_string();
        let mut result_trace_model_version = trace_model_version.clone();
        let resp = if let Some(replay) = replay {
            let live_req = if let Some(model) = live_model_override.as_deref() {
                req.with_model(model)
            } else {
                req
            };
            replay
                .replay_llm_call(
                    req.prompt,
                    if req.model.is_empty() {
                        None
                    } else {
                        Some(req.model)
                    },
                    recorded_model_version.as_deref(),
                    trace_rendered,
                    req.args,
                    live_req,
                    &self.llms,
                )
                .await?
        } else {
            match self.llms.call_with_adapter_name(&req).await {
                Ok(outcome) => {
                    actual_adapter = Some(outcome.adapter);
                    outcome.response
                }
                Err(primary_err) => {
                    let primary_error = primary_err.to_string();
                    self.emit_host_event(
                        "llm.provider_degraded",
                        serde_json::json!({
                            "prompt": req.prompt,
                            "model": req.model,
                            "provider": self.model_catalog.get(req.model).and_then(|model| model.provider.clone()),
                            "error": primary_error,
                        }),
                    );
                    let mut last_err = primary_err;
                    let fallbacks = self.model_catalog.compatible_fallbacks_for(
                        req.model,
                        estimate_tokens(trace_rendered),
                        0,
                    );
                    let mut fallback_response = None;
                    for fallback in fallbacks {
                        let fallback_req = req.with_model(&fallback.model);
                        match self.llms.call_with_adapter_name(&fallback_req).await {
                            Ok(outcome) => {
                                self.emit_host_event(
                                    "llm.provider_failover",
                                    serde_json::json!({
                                        "prompt": req.prompt,
                                        "from_model": req.model,
                                        "from_provider": self.model_catalog.get(req.model).and_then(|model| model.provider.clone()),
                                        "to_model": fallback.model.clone(),
                                        "to_provider": fallback.provider.clone(),
                                        "adapter": outcome.adapter,
                                    }),
                                );
                                actual_model = fallback.model;
                                actual_adapter = Some(outcome.adapter);
                                result_trace_model = actual_model.clone();
                                result_trace_model_version = self.model_version(&actual_model);
                                fallback_response = Some(outcome.response);
                                break;
                            }
                            Err(err) => {
                                self.emit_host_event(
                                    "llm.provider_degraded",
                                    serde_json::json!({
                                        "prompt": req.prompt,
                                        "model": fallback.model.clone(),
                                        "provider": fallback.provider.clone(),
                                        "error": err.to_string(),
                                    }),
                                );
                                last_err = err;
                            }
                        }
                    }
                    fallback_response.ok_or(last_err)?
                }
            }
        };
        if let Some(fingerprint) = cache_fingerprint.as_deref() {
            self.prompt_cache
                .insert(fingerprint.to_string(), resp.clone());
            if self.tracer.is_enabled() {
                self.tracer.emit(TraceEvent::PromptCache {
                    ts_ms: now_ms(),
                    run_id: self.tracer.run_id().to_string(),
                    prompt: req.prompt.to_string(),
                    model: if trace_model.is_empty() {
                        None
                    } else {
                        Some(trace_model.to_string())
                    },
                    model_version: trace_model_version.clone(),
                    fingerprint: fingerprint.to_string(),
                    hit: false,
                });
            }
        }
        let cost_usd = if actual_model.is_empty() {
            0.0
        } else {
            self.model_catalog
                .describe_named_model(
                    &actual_model,
                    resp.usage.prompt_tokens as u64,
                    resp.usage.completion_tokens as u64,
                )
                .cost_estimate
        };
        let model_metadata = self.model_catalog.get(&actual_model);
        let provider = model_metadata.and_then(|model| model.provider.clone());
        let privacy_tier = model_metadata.and_then(|model| model.privacy_tier.clone());
        let total_tokens = normalized_total_tokens(resp.usage);
        let usage_record = LlmUsageRecord {
            ts_ms: now_ms(),
            prompt: req.prompt.to_string(),
            model: actual_model.clone(),
            provider: provider.clone(),
            adapter: actual_adapter.clone(),
            privacy_tier: privacy_tier.clone(),
            prompt_tokens: resp.usage.prompt_tokens as u64,
            completion_tokens: resp.usage.completion_tokens as u64,
            total_tokens,
            cost_usd,
            local: provider.as_deref() == Some("ollama") || privacy_tier.as_deref() == Some("local"),
        };
        self.usage_ledger.record(usage_record.clone());
        self.emit_host_event(
            "llm.usage",
            serde_json::json!({
                "prompt": usage_record.prompt,
                "model": usage_record.model,
                "provider": usage_record.provider,
                "adapter": usage_record.adapter,
                "privacy_tier": usage_record.privacy_tier,
                "prompt_tokens": usage_record.prompt_tokens,
                "completion_tokens": usage_record.completion_tokens,
                "total_tokens": usage_record.total_tokens,
                "cost_usd": usage_record.cost_usd,
                "currency": "USD",
                "unit": "token",
                "local": usage_record.local,
            }),
        );
        crate::observation_handles::record_llm_usage(resp.usage, cost_usd);
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::LlmResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                prompt: req.prompt.to_string(),
                model: if result_trace_model.is_empty() {
                    None
                } else {
                    Some(result_trace_model)
                },
                model_version: result_trace_model_version,
                result: resp.value.clone(),
            });
        }
        Ok(resp)
    }

    /// Ask the approver about an action. Returns `ApprovalDenied` if
    /// denied; the interpreter surfaces this as `InterpError::Runtime`.
    pub async fn approval_gate(
        &self,
        label: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        let trace_enabled = self.tracer.is_enabled();
        let label_owned = label.to_string();
        if trace_enabled {
            self.tracer.emit(TraceEvent::ApprovalRequest {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                label: label_owned.clone(),
                args: args.clone(),
            });
        }
        let req = ApprovalRequest {
            label: label_owned.clone(),
            args,
        };
        let (approved, detail) = if let Some(replay) = self.replay_source()? {
            let outcome = replay.replay_approval(&label_owned, &req.args)?;
            let detail =
                outcome
                    .decision
                    .map(|decision| crate::approver_bridge::ApprovalDecisionInfo {
                        accepted: decision.accepted,
                        decider: decision.decider,
                        rationale: decision.rationale,
                    });
            (outcome.approved, detail)
        } else {
            let approved = self.approver.approve(&req).await? == ApprovalDecision::Approve;
            let detail = Some(crate::catalog_c_api::take_last_approval_detail().unwrap_or(
                crate::approver_bridge::ApprovalDecisionInfo {
                    accepted: approved,
                    decider: "runtime-approver".to_string(),
                    rationale: None,
                },
            ));
            (approved, detail)
        };
        if trace_enabled {
            if let Some(detail) = detail {
                self.tracer.emit(TraceEvent::ApprovalDecision {
                    ts_ms: now_ms(),
                    run_id: self.tracer.run_id().to_string(),
                    site: label_owned.clone(),
                    args: req.args.clone(),
                    accepted: detail.accepted,
                    decider: detail.decider,
                    rationale: detail.rationale,
                });
            }
        }
        if trace_enabled {
            self.tracer.emit(TraceEvent::ApprovalResponse {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                label: label_owned.clone(),
                approved,
            });
        }
        if approved {
            if trace_enabled {
                let issued_at_ms = now_ms();
                let expires_at_ms = issued_at_ms.saturating_add(APPROVAL_TOKEN_TTL_MS);
                let run_id = self.tracer.run_id().to_string();
                self.tracer.emit(TraceEvent::ApprovalTokenIssued {
                    ts_ms: issued_at_ms,
                    run_id: run_id.clone(),
                    token_id: approval_token_id(
                        &run_id,
                        &label_owned,
                        &req.args,
                        APPROVAL_TOKEN_SCOPE_ONE_TIME,
                        issued_at_ms,
                        expires_at_ms,
                    ),
                    label: label_owned.clone(),
                    args: req.args.clone(),
                    scope: APPROVAL_TOKEN_SCOPE_ONE_TIME.to_string(),
                    issued_at_ms,
                    expires_at_ms,
                });
            }
            Ok(())
        } else {
            Err(RuntimeError::ApprovalDenied {
                action: label_owned,
            })
        }
    }

    pub fn validate_approval_token_scope(
        &self,
        token: &mut ApprovalToken,
        label: &str,
        args: &[serde_json::Value],
        session_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let now = now_ms();
        match token.validate(label, args, now, session_id) {
            Ok(()) => Ok(()),
            Err(reason) => {
                if self.tracer.is_enabled() {
                    self.tracer.emit(TraceEvent::ApprovalScopeViolation {
                        ts_ms: now,
                        run_id: self.tracer.run_id().to_string(),
                        token_id: token.token_id.clone(),
                        label: label.to_string(),
                        reason: reason.clone(),
                    });
                }
                Err(RuntimeError::ApprovalFailed(format!(
                    "approval token scope violation: {reason}"
                )))
            }
        }
    }

    pub async fn ask_human(
        &self,
        prompt: &str,
        expected_type: impl Into<String>,
    ) -> Result<serde_json::Value, RuntimeError> {
        let req = HumanInputRequest {
            prompt: prompt.to_string(),
            expected_type: expected_type.into(),
        };
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::HumanInputRequest {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                prompt: req.prompt.clone(),
                expected_type: req.expected_type.clone(),
            });
        }
        let value = self.human.ask(&req).await?;
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::HumanInputResponse {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                prompt: req.prompt,
                value: value.clone(),
            });
        }
        Ok(value)
    }

    pub async fn choose_human(
        &self,
        options: Vec<serde_json::Value>,
    ) -> Result<usize, RuntimeError> {
        let req = HumanChoiceRequest { options };
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::HumanChoiceRequest {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                options: req.options.clone(),
            });
        }
        let selected_index = self.human.choose(&req).await?;
        let selected_value = req.options.get(selected_index).cloned().ok_or_else(|| {
            RuntimeError::Other(format!("human choice index {selected_index} out of range"))
        })?;
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::HumanChoiceResponse {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                selected_index,
                selected_value,
            });
        }
        Ok(selected_index)
    }
}

fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4).max(1)
}

fn approval_token_id(
    run_id: &str,
    label: &str,
    args: &[serde_json::Value],
    scope: &str,
    issued_at_ms: u64,
    expires_at_ms: u64,
) -> String {
    let args_json = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
    let mut hasher = Sha256::new();
    hasher.update(run_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(label.as_bytes());
    hasher.update(b"\0");
    hasher.update(args_json.as_bytes());
    hasher.update(b"\0");
    hasher.update(scope.as_bytes());
    hasher.update(b"\0");
    hasher.update(issued_at_ms.to_le_bytes());
    hasher.update(expires_at_ms.to_le_bytes());
    format!("apr_{}", hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Slice 33S1a: marshal `IoRuntime::FileSystemEffect` into a JSON
/// object matching the `EffectEnvelope` type declared in
/// `std/effects.cor`. Field order matches the type's declaration
/// order so the IR-side struct construction picks up each field
/// positionally even though we emit them as a named JSON object.
///
/// EffectEnvelope fields: effect_name, provenance_key,
/// approval_label, cache_key, replay_key.
/// Slice 45m: exact-match gate for the stdlib time tools. Same
/// exact-names-only rationale as `is_stdlib_io_tool`.
fn is_stdlib_time_tool(name: &str) -> bool {
    matches!(
        name,
        "time_now_utc" | "time_monotonic_ms" | "time_parse_iso" | "time_format_iso"
    )
}

/// Slice 45m: exact-match gate for the stdlib randomness tools.
fn is_stdlib_random_tool(name: &str) -> bool {
    matches!(name, "random_float" | "random_int")
}

fn stdlib_time_effect_envelope(replay_key: &str) -> serde_json::Value {
    serde_json::json!({
        "effect_name": "std.time.now",
        "provenance_key": "",
        "approval_label": "",
        "cache_key": "",
        "replay_key": replay_key,
    })
}

/// Slice 45m — the executing stdlib time tools. The clock reads
/// (`time_now_utc`, `time_monotonic_ms`) are nondeterministic and
/// rely on `call_tool`'s tracing + replay substitution (the replay
/// branch runs BEFORE dispatch, so a replayed program never
/// touches the real clock). The conversions are pure functions of
/// their arguments.
fn dispatch_stdlib_time_tool(
    name: &str,
    args: &[serde_json::Value],
) -> Result<serde_json::Value, RuntimeError> {
    use chrono::{TimeZone, Utc};
    match name {
        "time_now_utc" => {
            let now = Utc::now();
            let epoch_ms = now.timestamp_millis();
            Ok(serde_json::json!({
                "epoch_ms": epoch_ms,
                "iso": now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "effect_meta": stdlib_time_effect_envelope("std.time.now"),
            }))
        }
        "time_monotonic_ms" => {
            // Monotonic origin = first read in this process. Only
            // DIFFERENCES between reads are meaningful.
            use std::sync::OnceLock;
            use std::time::Instant;
            static ORIGIN: OnceLock<Instant> = OnceLock::new();
            let origin = *ORIGIN.get_or_init(Instant::now);
            Ok(serde_json::json!(origin.elapsed().as_millis() as i64))
        }
        "time_parse_iso" => {
            let text = args.first().and_then(|v| v.as_str()).ok_or_else(|| {
                RuntimeError::ToolFailed {
                    tool: "time_parse_iso".to_string(),
                    message: "expected one String argument (ISO-8601 text)".to_string(),
                }
            })?;
            match chrono::DateTime::parse_from_rfc3339(text.trim()) {
                Ok(dt) => Ok(serde_json::json!({
                    "tag": "ok",
                    "ok": dt.timestamp_millis(),
                })),
                Err(e) => Ok(serde_json::json!({
                    "tag": "err",
                    "err": format!("not ISO-8601: `{text}` ({e})"),
                })),
            }
        }
        "time_format_iso" => {
            let epoch_ms = args.first().and_then(|v| v.as_i64()).ok_or_else(|| {
                RuntimeError::ToolFailed {
                    tool: "time_format_iso".to_string(),
                    message: "expected one Int argument (epoch milliseconds)".to_string(),
                }
            })?;
            let dt = Utc.timestamp_millis_opt(epoch_ms).single().ok_or_else(|| {
                RuntimeError::ToolFailed {
                    tool: "time_format_iso".to_string(),
                    message: format!("epoch_ms out of representable range: {epoch_ms}"),
                }
            })?;
            Ok(serde_json::json!(
                dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            ))
        }
        other => Err(RuntimeError::ToolFailed {
            tool: other.to_string(),
            message: "unknown stdlib time tool (gate/dispatch mismatch)".to_string(),
        }),
    }
}

/// Slice 45m — the executing stdlib randomness tools. OS-entropy
/// draws (rand_core/getrandom); traced + replay-substituted like
/// every tool, so replays reproduce the recorded draws.
fn dispatch_stdlib_random_tool(
    name: &str,
    args: &[serde_json::Value],
) -> Result<serde_json::Value, RuntimeError> {
    fn entropy_u64(tool: &str) -> Result<u64, RuntimeError> {
        use rand_core::{OsRng, RngCore};
        let mut rng = OsRng;
        let mut buf = [0u8; 8];
        rng.try_fill_bytes(&mut buf)
            .map_err(|e| RuntimeError::ToolFailed {
                tool: tool.to_string(),
                message: format!("OS entropy source failed: {e}"),
            })?;
        Ok(u64::from_le_bytes(buf))
    }
    match name {
        "random_float" => {
            // 53 uniform bits -> [0.0, 1.0), the standard f64 recipe.
            let bits = entropy_u64("random_float")? >> 11;
            Ok(serde_json::json!(bits as f64 / (1u64 << 53) as f64))
        }
        "random_int" => {
            let min = args.first().and_then(|v| v.as_i64()).ok_or_else(|| {
                RuntimeError::ToolFailed {
                    tool: "random_int".to_string(),
                    message: "expected (min: Int, max: Int) — min missing".to_string(),
                }
            })?;
            let max = args.get(1).and_then(|v| v.as_i64()).ok_or_else(|| {
                RuntimeError::ToolFailed {
                    tool: "random_int".to_string(),
                    message: "expected (min: Int, max: Int) — max missing".to_string(),
                }
            })?;
            if min > max {
                return Err(RuntimeError::ToolFailed {
                    tool: "random_int".to_string(),
                    message: format!("min ({min}) must be <= max ({max})"),
                });
            }
            // Rejection sampling over the inclusive span — no
            // modulo bias.
            let span = (max as i128 - min as i128 + 1) as u128;
            let zone = u128::from(u64::MAX) + 1;
            let limit = zone - (zone % span);
            let draw = loop {
                let x = u128::from(entropy_u64("random_int")?);
                if x < limit {
                    break x % span;
                }
            };
            Ok(serde_json::json!((min as i128 + draw as i128) as i64))
        }
        other => Err(RuntimeError::ToolFailed {
            tool: other.to_string(),
            message: "unknown stdlib random tool (gate/dispatch mismatch)".to_string(),
        }),
    }
}

fn stdlib_io_effect_envelope(
    effect: &crate::io::FileSystemEffect,
) -> serde_json::Value {
    serde_json::json!({
        "effect_name": effect.effect_tag,
        "provenance_key": "",
        "approval_label": effect.approval_label,
        "cache_key": "",
        "replay_key": effect.replay_key,
    })
}

/// Slice 33S1a (refactored 33S2a): exact-match gate for the
/// stdlib file-I/O tools. Returns true only for the three
/// declared tool names; a user-defined tool that happens to
/// start with `io_` (e.g. `io_foobar`) reaches the normal
/// `tools.call` path. Prevents the dispatch interception from
/// stealing user tool names.
fn is_stdlib_io_tool(name: &str) -> bool {
    matches!(name, "io_read_text" | "io_write_text" | "io_list_dir")
}

/// Slice 33S2a: exact-match gate for the stdlib HTTP tools.
/// Same rationale as `is_stdlib_io_tool` — exact names only,
/// no prefix sweep.
fn is_stdlib_http_tool(name: &str) -> bool {
    matches!(name, "http_get" | "http_post_json")
}

/// Slice 33S2a: marshal an `EffectEnvelope` for the executing
/// HTTP tools. The runtime side doesn't carry a per-call
/// effect-tag structure for HTTP (the request/response cycle
/// is the unit), so we emit a fixed `std.http.request` tag
/// matching what `std/http.cor` programs already expect.
fn stdlib_http_effect_envelope() -> serde_json::Value {
    serde_json::json!({
        "effect_name": "std.http.request",
        "provenance_key": "",
        "approval_label": "",
        "cache_key": "",
        "replay_key": "std.http.request",
    })
}

/// Slice 33S3b — marshal an `EffectEnvelope` for the executing
/// SQLite tools. Mirrors `stdlib_http_effect_envelope` — the
/// runtime side doesn't carry a per-call effect-tag structure
/// for SQLite (each statement is a unit), so we emit a fixed
/// `std.db.execute` tag matching what `std/db.cor` programs
/// already expect from the envelope types.
fn stdlib_db_effect_envelope() -> serde_json::Value {
    serde_json::json!({
        "effect_name": "std.db.execute",
        "provenance_key": "",
        "approval_label": "",
        "cache_key": "",
        "replay_key": "std.db.execute",
    })
}

/// Slice 33S3b — render a `DbValue` for trace emission. The
/// runtime stores parameters as the typed `DbValue` enum
/// (`Null`, `Integer`, `Float`, `Text`, `Bool`) — `params_from_iter`
/// binds them, never interpolates, so a literal `"'; DROP TABLE
/// users; --"` survives as `Text("...")` data. The trace payload
/// reflects the same typed shape so an audit trail shows exactly
/// what was bound.
fn db_value_to_json_param(value: &crate::db::DbValue) -> serde_json::Value {
    use crate::db::DbValue;
    match value {
        DbValue::Null => serde_json::Value::Null,
        DbValue::Integer(n) => serde_json::Value::from(*n),
        DbValue::Float(f) => serde_json::Value::from(*f),
        DbValue::Text(s) => serde_json::Value::String(s.clone()),
        DbValue::Bool(b) => serde_json::Value::Bool(*b),
    }
}

/// Slice 33S3b — marshal `DbQueryRows` into the JSON envelope
/// the interpreter then converts to a `List<DbResult>` via the
/// standard `json_to_value` path. The envelope carries one
/// `DbResult`-shaped object per row with `rows_affected: 0`
/// (irrelevant for SELECTs) and the row's cells folded into the
/// effect_meta as a debug aid. The richer "rows as a
/// `List<Map<String, Cell>>`" shape is post-v1.0 — for 33S3b
/// the minimum-viable shape is row-as-`DbResult` so the existing
/// std/db.cor envelopes work without changes.
fn db_query_rows_to_envelope(rows: &crate::db::DbQueryRows) -> serde_json::Value {
    let cells: Vec<serde_json::Value> = rows
        .rows
        .iter()
        .map(|row| {
            let cells: serde_json::Map<String, serde_json::Value> = row
                .iter()
                .map(|(name, cell)| (name.clone(), db_value_to_json_param(&cell.value)))
                .collect();
            serde_json::json!({
                "rows_affected": 0_i64,
                "row_count": rows.row_count as i64,
                "replay_key": "",
                "effect_meta": serde_json::json!({
                    "effect_name": "std.db.query",
                    "provenance_key": "",
                    "approval_label": "",
                    "cache_key": "",
                    "replay_key": serde_json::Value::Object(cells),
                }),
            })
        })
        .collect();
    serde_json::Value::Array(cells)
}
