# Corvid Standard Library

Phase 32 starts the standard library as ordinary Corvid source under `std/`.
The modules are intentionally small and effect-explicit so they can be imported,
audited, packaged, and eventually shipped through the same content-addressed
package path as user code.

## `std.ai`

`std/ai.cor` contains reusable AI application data envelopes and pure helpers:

- `AiMessage` plus `system_message`, `user_message`, and `assistant_message`
- `AiSession` plus `start_session` and `next_turn`
- `ToolResultEnvelope` plus `tool_ok` and `tool_error`
- `ModelRoute` plus `route_to`
- `StructuredValidation` plus `validation_ok` and `validation_error`
- `Confidence` plus `confidence`
- `TraceEventSummary` plus `trace_event`

These primitives are deliberately plain Corvid types and agents. A program can
import the module today with a local path:

```corvid
import "./std/ai" use AiMessage, user_message

agent main() -> String:
    msg = user_message("hello")
    return msg.content
```

Later Phase 32 slices will add package-style `std.ai` resolution and extend the
same module with routing, prompt rendering, structured-output validation, and
trace helpers that carry effects, replay, cost, and provenance metadata.

## `std.http`

`std/http.cor` defines the executing HTTP-client surface added in Phase 33S2
plus the request/response envelopes the surface returns.

Executing tools:

- `http_get(url) -> HttpResponseEnvelope uses http_egress_get` — executing tool
- `http_post_json(url, body) -> HttpResponseEnvelope uses http_egress_post` — executing tool

Envelope-builder agents (pure; construct a request without executing it):

- `HttpHeader`, `HttpRequestEnvelope`, `HttpResponseEnvelope`
- `http_request_get`, `http_request_post_json`, `http_with_retry`,
  `http_with_timeout`, `http_ok`

The executing tools enforce three RuntimeChecked guarantees:
`io_source.http_ssrf_structural_block` (always-on private / loopback /
link-local refusal; structural floor underneath the allowlist),
`io_source.http_allowlist_enforcement` (URL host must be in the project's
`[http] allow` list; missing config fails closed), and
`io_source.http_quarantine_on_replay` (POST + GET both gated by the replay-
substitution path; never reaches the live network during replay). Programs
that call these tools from a `@deterministic` agent are rejected at
typecheck (the existing decl-replayability rule treats all tool calls as
non-deterministic).

See [`http.md`](./http.md) for the full reference, including the corvid.toml
`[http] allow` allowlist, the `CORVID_HTTP_ALLOW` env override, the SSRF
block ranges, and the envelope schemas.

The native runtime also exposes a matching `HttpClient`/`HttpRequest` API. Its
calls emit `std.http.request`, `std.http.response`, and `std.http.error` trace
events with method, URL, timeout, retry, status, attempt, latency, and payload
size metadata.

## `std.io`

`std/io.cor` defines path and file-system envelopes plus the executing
file-I/O tool surface added in Phase 33S1:

- `PathInfo`
- `FileReadEnvelope`
- `FileWriteEnvelope`
- `DirectoryEntryEnvelope`
- `io_read_text(path) -> FileReadEnvelope uses io_read` — executing tool
- `io_write_text(path, content) -> FileWriteEnvelope uses io_write` — executing tool
- `io_list_dir(path) -> List<DirectoryEntryEnvelope> uses io_list` — executing tool

The executing tools enforce three RuntimeChecked guarantees:
`io_source.fs_path_confinement` (paths stay inside the configured `[io] root`),
`io_source.fs_write_quarantine_on_replay`, and
`io_source.fs_read_quarantine_on_replay`. Programs that call these tools from
a `@deterministic` agent are rejected at typecheck (the existing decl-
replayability rule treats all tool calls as non-deterministic).

See [`io.md`](./io.md) for the full reference, including the corvid.toml
`[io] root` security model, the `CORVID_IO_ROOT` env override, and the
envelope schemas.

## `std.db`

`std/db.cor` defines the executing SQLite surface added in Phase 33S3 plus
typed parameter constructors and envelope types. The Postgres path remains
envelope-only — declare your Postgres tool in user code and reach the
`corvid-runtime::PostgresDbRuntime` from a tool wrapper.

Executing tools:

- `db_open(path) -> DbHandle uses db_egress_open` — executing tool
- `db_query(handle, sql, params) -> List<DbResult> uses db_egress_read` — executing tool
- `db_execute(handle, sql, params) -> DbResult uses db_egress_write` — executing tool

Typed parameter constructors (the typechecker's `List<DbParam>` signature
forces every value through these — there is no string-interpolation path):

- `db_param_int(value)`, `db_param_float(value)`, `db_param_text(value)`,
  `db_param_bool(value)`, `db_param_null()`

`DbHandle` is an opaque, refcounted primitive type produced ONLY by
`db_open`. The opacity is structural: there is no path in user code that
fabricates a `DbHandle`. See the [reference](./db.md) for the security
argument.

The executing tools enforce three RuntimeChecked guarantees:
`io_source.sqlite_parameter_binding_only` (all SQL parameters bound via
`rusqlite::params_from_iter`; no interpolation),
`io_source.sqlite_write_quarantine_on_replay` (`db_execute` refuses
during Substitute-mode replay), and `io_source.sqlite_read_passthrough_on_replay`
(`db_query` not blocked; trace-substitution upper gate lands in a follow-up
slice). `db_open` reuses the existing `io_source.fs_path_confinement`
guarantee — there is no separate `[db]` allowlist, the SQLite path
boundary is the same `[io] root` the file-I/O surface enforces.

Programs that call these tools from a `@deterministic` agent are rejected
at typecheck (the existing decl-replayability rule treats all tool calls
as non-deterministic).

See [`db.md`](./db.md) for the full reference, including the `[io] root`
reuse, the typed `DbParam` value-binding shapes, the worked typed-user-store
example, and the v1.0 post-scope.

## `std.json`

`std/json.cor` defines the executing JSON surface added in Phase 33R5b. The
umbrella ships TWO complementary shapes: the opaque-handle path (for
dynamic JSON) and the typed-decoder convention (for typed APIs). Together
they make "no Python required for JSON" structurally true at the language
level.

Executing tools (opaque path):

- `json_parse(text) -> Result<JsonValue, String> uses json_egress_read`
- `json_get_int / _float / _string / _bool / _object / _array(value, field) -> Result<T, String>` — typed accessors
- `json_object_new() -> JsonBuilder` + `json_object_set_int / _float / _string / _bool(builder, key, value) -> JsonBuilder` — fluent builder
- `json_object_finish(builder) -> String` — snapshot semantics; builder remains usable

Typed-decoder convention: the user declares
`tool decode_<X>_from_json(text: String) -> Result<X, String> uses <effect>`
where `<X>` is any Corvid type the runtime can convert from JSON. The
interpreter pattern-matches the tool name + return type and routes through
`serde_json::from_str` + `json_to_value` against the declared target type.
**No per-type runtime handler exists** — the dispatch is generic over the
declared signature.

`JsonValue` and `JsonBuilder` are opaque, refcounted primitive types
produced ONLY by the executing JSON tools. The codegen-cl backend emits a
structured "interpreter-only in 33R5b; cdylib bridging lands in a
follow-up slice" diagnostic (the C-ABI `corvid_json_*` exports already
exist in `corvid-runtime::ffi_bridge::json_exports`, so the cdylib wire-up
is plumbing).

The executing tools enforce two RuntimeChecked guarantees:
`json.parse_safety_no_panic` (malformed input returns `Result::Err`, never
panics) and `json.field_type_safety_at_access_boundary` (typed-accessor
mismatches return `Result::Err`, never coerce or panic). Programs that
call these tools from a `@deterministic` agent are rejected at typecheck
(the existing decl-replayability rule treats all tool calls as
non-deterministic).

See [`json.md`](./json.md) for the full reference, including the two
shapes (opaque + typed-decoder), the worked typed-user-store HTTP →
JSON → SQLite pipeline (no Python glue), and the v1.0 post-scope.

## `std.secrets`

`std/secrets.cor` defines `SecretReadEnvelope` plus constructors for present and
missing reads. The runtime exposes environment-backed secret reads that return
the value to the caller but only emit redacted trace metadata:

- secret name
- whether the secret was present
- whether the value was redacted

Trace events never include the secret value.

## `std.observe`

`std/observe.cor` defines typed observability envelopes for metrics, cost
counters, latency histograms, routing decisions, approval summaries, and runtime
observation summaries.

The runtime exposes an observation snapshot API that aggregates normalized LLM
usage and provider health. Emitting the snapshot records a `std.observe.summary`
trace event with call counts, token totals, cost totals, local-call counts, and
degraded-provider counts.

## `std.cache`

`std/cache.cor` defines typed cache-key and cache-entry envelopes for prompt,
model, and tool-result caching. The runtime exposes deterministic cache-key
construction over namespace, subject, model, arguments, effect key, provenance
key, and version metadata. Cache-key creation emits `std.cache.key` trace events
so cache decisions are replay-auditable without storing cached payloads in the
metadata event.

## `std.queue`

`std/queue.cor` defines typed background-job envelopes with task, status, retry,
budget, effect-summary, and replay-key metadata. The runtime exposes an
in-process queue foundation for enqueue and cancel operations. Each operation
emits `std.queue.*` trace events so long-running AI work can be audited and later
backed by a durable store without changing the job contract.

## `std.jobs`

`std/jobs.cor` defines durable job input, output, retry-policy, dead-letter, and
lifecycle-state envelopes for persisted backend work. Job metadata carries
redacted input/output fingerprints, queue name, job kind, status, attempts,
budget, approval requirement, idempotency key, effect metadata, and replay key so
AI work can be audited before and after execution.

## `std.agent`

`std/agent.cor` defines pure typed envelopes for common AI application patterns:
classification, extraction, ranking, adjudication, planning, tool-use records,
approval labels, critique/review, and grounded answer metadata. These are
ordinary Corvid values, so applications can compose them with effects, replay,
approval, provenance, and cache keys without introducing framework glue.

## `std.rag`

`std/rag.cor` defines typed document, chunk, and embedder envelopes. The runtime
adds document construction, markdown loading, deterministic chunking with
per-chunk provenance keys, OpenAI/Ollama embedder configuration envelopes, and a
SQLite-backed chunk index. Chunks carry provenance metadata so retrieval results
can compose with grounding, cache keys, replay, and audit trails.

## `std.effects`

`std/effects.cor` defines common effect metadata envelopes: effect tags, budget
summaries, provenance keys, approval labels, cache keys, and replay keys. The
module gives every `std.*` surface a shared vocabulary for carrying Corvid's
effect, approval, budget, replay, cache, and provenance semantics through normal
application values.
