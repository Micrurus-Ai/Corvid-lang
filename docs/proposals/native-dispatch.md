# Proposal: native dispatch of runtime-owned calls

Status: **decided 2026-11-25; N1 and N2 built 2026-11-25, N3–N5 to come.** The user chose to build this as B05 slice 7, by the laws in `AGENTS.md`. Owner: backend and AI development, macOS session. It changes no grammar.

## Problem

A program compiles to native code only if every call it makes can run there. Today none of the calls the runtime owns can:

- connector operations (B05);
- the standard library's tools that the runtime implements: `secret`, `secret_read`, `random_token`, `hash_password`, `uuid_v4`, `uuid_v7`, `time_now`, the `random_*` tools.

So a program that calls an API, reads a secret or makes a UUID always runs on the interpreter. It cannot be built into a fast single binary, and `corvid build` refuses it.

These calls are not plain functions. The runtime gives each one its guarantees:
- a trace record of the call and its result;
- replay from that record without side effects;
- redaction of secrets, and of uploaded files by digest;
- the HTTP policy (SSRF block, `[http] allow`), declared timeouts and redirects, `HttpExchange` records, `on failure`, and circuit breakers.

A native call that skipped the runtime would silently lose all of them. That is why native code refuses these programs today.

## What exists today

- **A runtime instance for native code.** Compiled code reaches it through `ffi_bridge::bridge().corvid_runtime()`, and blocks on async work through the bridge's tokio handle. Native prompt calls (`prompt_exports.rs`) already go through it.
- **Native tool calls with their own tracing.** A user tool linked with `--with-tools-lib` is called directly. The generated code emits `corvid_trace_tool_call` and `corvid_trace_tool_result_*` itself, and replays through `corvid_replay_tool_call_*`. That path re-implements part of what `Runtime::call_tool` does, without redaction, file digests, connector policy, `on failure` or breakers.
- **Native public JSON.** `corvid_json_encode_typed` and `corvid_json_decode_typed` encode and decode any native value through the shared public mapping, with a plan the compiler embeds per type. `T.to_json()` and `T.from_json()` use them.
- **The connector result is already public JSON.** The runtime answers a `Result` operation with `{"tag":"ok","ok":<body>}`, a mapped status or `on failure` with `{"tag":"err","err":{"tag":"variant","variant":V,"fields":[...]}}`, and a plain operation with its body. The public decoder reads all three as the operation's declared type.

## Decision

**Native code reaches every runtime-owned call through one export, which calls `Runtime::call_tool`.** It does not emit its own trace events for these calls, and it does not re-implement any of the runtime's guarantees. The interpreter and native code then run the same function for the same call: the same recording, replay, redaction, policy, `HttpExchange` records and failure mapping. A guarantee added later reaches both tiers at once.

`corvid_runtime_call(name, args, arg_count, plan, plan_len, typeinfos, payload) -> tag`:
- The generated code encodes each argument with `corvid_json_encode_typed`, the mapping the interpreter's connector calls use (slice 3a), and passes the texts.
- The runtime runs `Runtime::call_tool(name, args)` on the bridge's tokio handle.
- It decodes the answer into the declared return type with the same public decoder `corvid_json_decode_typed` uses, ignoring unknown fields.
- For a connector operation with `on failure`, a body that does not decode becomes the declared variant with the reason. That is the rule the interpreter applies in `connector_json::failure_value`, and both call the runtime's `connectors::failure_result`.
- Any other failure stops the program with the interpreter's message, as it does there.

**A connector with `circuit_breaker` stays on the interpreter for now.** The interpreter counts breaker failures per run, in its tool arm, which native code does not run. Moving the count into the runtime would change its scope without the source saying so: under `corvid serve` one runtime serves every request, so an opened breaker would stay open for all later requests with no rule for closing it. Copying the count into generated code would be a second implementation. A breaker that spans a server needs a declared reset policy (how long it stays open, how it probes), which is a consequential choice the source must make, so it is its own decision, recorded on the roadmap. Until then, `corvid run` keeps a program whose connector declares `circuit_breaker` on the interpreter and names the reason.

**A native binary is configured as the interpreter's runtime is.** A compiled binary builds its runtime from the environment when it starts (`ffi_bridge::tokio_handle`), so three things must reach it:
- *The connector specs.* The compiler serializes the specs from `connectors::connector_calls_from_ir`, the projection the interpreter's runtime is given, into the binary. The generated entry installs them before the program runs, so there is still one source.
- *The mode.* The binary reads `CORVID_CONNECTOR_MODE`. `corvid run --mode` sets it, and a binary whose program declares connectors refuses to start without it, as the interpreter does, rather than picking one.
- *The HTTP policy and the startup check.* `corvid run` loads `[http] allow` and runs the connector startup check before launching the binary, as for the interpreter, and passes the allowlist as `CORVID_HTTP_ALLOW`. A binary run on its own reads the same variable and runs the same runtime checks at startup.

**Approval is unchanged.** Native `approve` already calls the runtime's approval gate (`corvid_approve_sync`), so a `dangerous` operation needs its approval in native code as in the interpreter. The comment in `native_ability.rs` that calls it a no-op is out of date and is corrected in N1.

**An operation with a provider protocol stays on the interpreter.** The interpreter runs it through `Runtime::call_protocol_operation`, the durable lifecycle, not `call_tool`; the export would send a one-shot request instead. Found while building N1.

**The `connector.invocation` record moves into the runtime.** The interpreter wrote it after each connector call, in its tool arm, so a native run would have lacked it. `Runtime::record_connector_invocation` now builds it for both tiers; the spec carries the operation's effects so native code names them too.

**Mock mode stays on the interpreter.** A connector's `mock:` is a Corvid expression the interpreter evaluates. `corvid run --mode mock` picks the interpreter for a program with connectors, and says so. Real and replay modes run natively.

## Slices

1. **N1. Connector calls in native code.** The export, connector operations whose parameters and result have a native public JSON form, `corvid run` choosing native code for them in real and replay modes (not mock mode, not a connector with `circuit_breaker`), and parity tests. The parity tests run the same program on both tiers and compare the result and the recorded `ToolCall`, `ToolResult` and `HttpExchange` events.
2. **N2. The standard library's runtime tools in native code:** `uuid_v4`, `uuid_v7`, `time_now`, the `random_*` tools, and `secret_read` through the same export.
3. **N3. Native `Secret`.** An opaque, runtime-owned handle that no string, print or JSON path can read, so `secret`, `random_token`, `hash_password` and an HMAC keyed by a `Secret` compile natively. This is B04's open item.
4. **N4. `Response<T>` in native code**, as a native record built from `RESPONSE_FIELDS`.
5. **N5. Replay on the other tier.** See "Across the tiers".

## N2: the standard library's runtime tools

**Which tools.** The runtime implements these by exact name inside `dispatch_tool`: the time tools (`time_now`, `time_now_utc`, `time_monotonic_ms`, `time_parse_iso`, `time_format_iso`), `random_int`, `random_float`, `uuid_v4`, `uuid_v7` and `secret_read`. One runtime function, `is_native_runtime_tool`, names them, built from the same gates `dispatch_tool` uses. The tier picker and the code generator both call it, so the list cannot drift between the runtime, the picker and native code.

**The same export.** A call to one of them lowers to `corvid_runtime_call`, as a connector call does. `call_tool` records it, replay substitutes the recorded value (a clock read or a draw never re-runs on replay), and `secret_read` keeps its own rules: the trace holds `<redacted>`, and replay re-reads the environment.

**Why the public JSON mapping is safe here.** The interpreter's generic tool arm decodes a tool's answer with its internal JSON form, not the public one connectors use. The two differ only for `Option`, and none of these tools has an `Option` in its signature: they take and return `Int`, `Float`, `String`, `Instant`, flat records and `Result<_, String>`. A parity test calls each tool on both tiers and compares the values, or for clock reads and draws, the recorded events and the values replay gives back. A tool added to the list later must pass the same test.

**Left out, and why.**
- `secret`, `random_token` and `hash_password` return or take a `Secret`: N3.
- The io, path, cache, retrieval, database, JSON-handle and MCP tools return handles or need policies native code does not set up yet. Each is a later decision, not part of N2.
- A user tool that happens to share a name with one of these is already dispatched by name to the runtime's implementation on the interpreter, so native code does the same; nothing changes for it.

**Across the tiers.** Backend: a program that makes IDs, reads the clock or draws random numbers now builds into a native binary, and those values replay exactly. Frontend: nothing changes. AI: the recorded run is the same from either tier, so an assistant reviewing it sees the same clock reads and IDs.

## Not in scope

- `Stream<T>` connector operations in native code. Native code has no streams yet (`StreamLoweringNotImplemented`), for any source.
- File bodies (`Upload`) in native code. An `Upload` only exists in a `corvid serve` route, which runs on the interpreter.
- Mock mode in native code (see above).
- Moving user tools onto the export. Their direct-call path works and is a separate decision.

## Across the tiers

- **Backend:** the same connector behaves the same in a native binary, a served app and the interpreter. Parity tests hold that, so a fix in the runtime reaches every tier.
- **Frontend:** nothing changes. The contract and the routes are the same.
- **AI:** a connector run records the same events on either tier, so an assistant reading what a run did sees the same record whichever tier ran it. Replaying a recording on the *other* tier is refused today: replay checks the trace's writer and refuses a trace from the other tier (`CrossTierReplayUnsupported`), for every kind of event. N1 proves connector events match; lifting the refusal needs the same proof for prompt calls, user tools, approvals and seeds, so it is slice N5 rather than part of N1, and this section claimed more than N1 delivers until that was found.

## As built (N1)

- `corvid_runtime_call` in `ffi_bridge/typed_json.rs`; codegen in `lowering/connector_call.rs`, chosen in the `Tool` call arm for a connector operation.
- The specs come from `connectors::connector_calls_json`, the serialized form of `connector_calls_from_ir`; the generated `main` passes them to `corvid_register_connectors` before the runtime starts, and both runtime builders install them.
- `native_ability` names why an operation stays on the interpreter: a `circuit_breaker`, a stream, a provider protocol, a `Response`, a file or multipart body, or a type with no native JSON form. `corvid run --mode mock` keeps a program with connectors on the interpreter.
- Tests: `crates/corvid-driver/tests/native_connectors.rs` (same value and the same recorded run on both tiers, each tier replaying its recording without the provider, the refusals) and `crates/corvid-cli/tests/native_connectors_run.rs` (`corvid run --target native` sends the request under the project's `[http] allow`, and refuses a host outside it).

## As built (N2)

- `corvid_runtime::runtime::is_native_runtime_tool`, built from `dispatch_tool`'s own gates. Codegen adds those tools to `runtime_call_ops`; `native_ability` accepts a call to one whose types have a native JSON form and names it otherwise (`RuntimeToolNotNative`).
- Found while building it: importing `std/secrets` for `secret_read` brings in `secret`, whose signature holds a `Secret`, and the native check refused every program that declared a `Secret` anywhere. A tool's signature now counts only when the tool is called; records, agents and prompts still count wherever they are declared.
- Found by the replay test: serde_json read a recorded `Float` back with its fast, inexact parser, so a replayed `random_float` could differ in the last digit and replay reported a divergence. The workspace enables `float_roundtrip` (separate commit, with a trace-schema test).
- Tests: `crates/corvid-driver/tests/native_runtime_tools.rs` calls every tool the list names (and fails if one is added without a call), compares fixed answers and the recorded calls on both tiers, checks `secret_read`'s recorded result is redacted, and replays each tier's recording exactly.
