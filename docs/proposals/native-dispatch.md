# Proposal: native dispatch of runtime-owned calls

Status: **decided 2026-11-25, in progress.** The user chose to build this as B05 slice 7, by the laws in `AGENTS.md`. Owner: backend and AI development, macOS session. It changes no grammar.

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

**The circuit breaker moves into the runtime for connectors, for both tiers.** The interpreter counts breaker failures in its tool arm, which native code does not run. Rather than copy that count into generated code, the runtime counts a connector's failures inside `call_tool`, where every tier passes. The interpreter stops counting connector calls itself. The behavior is unchanged: the same failures count, a success resets the count, and an open breaker is the `on failure` variant or a stop.

**Mock mode stays on the interpreter.** A connector's `mock:` is a Corvid expression the interpreter evaluates. `corvid run --mode mock` picks the interpreter for a program with connectors, and says so. Real and replay modes run natively.

## Slices

1. **N1. Connector calls in native code.** The export, connector operations whose parameters and result have a native public JSON form, the runtime-owned breaker, `corvid run` choosing native code for them in real and replay modes, and parity tests. The parity tests run the same program on both tiers and compare the result and the recorded `ToolCall`, `ToolResult` and `HttpExchange` events.
2. **N2. The standard library's runtime tools in native code:** `uuid_v4`, `uuid_v7`, `time_now`, the `random_*` tools, and `secret_read` through the same export.
3. **N3. Native `Secret`.** An opaque, runtime-owned handle that no string, print or JSON path can read, so `secret`, `random_token`, `hash_password` and an HMAC keyed by a `Secret` compile natively. This is B04's open item.
4. **N4. `Response<T>` in native code**, as a native record built from `RESPONSE_FIELDS`.

## Not in scope

- `Stream<T>` connector operations in native code. Native code has no streams yet (`StreamLoweringNotImplemented`), for any source.
- File bodies (`Upload`) in native code. An `Upload` only exists in a `corvid serve` route, which runs on the interpreter.
- Mock mode in native code (see above).
- Moving user tools onto the export. Their direct-call path works and is a separate decision.

## Across the tiers

- **Backend:** the same connector behaves the same in a native binary, a served app and the interpreter. Parity tests hold that, so a fix in the runtime reaches every tier.
- **Frontend:** nothing changes. The contract and the routes are the same.
- **AI:** a recorded native run replays on the interpreter and the other way round. The trace is the same, so an assistant reading what a run did sees the same record from either tier.
