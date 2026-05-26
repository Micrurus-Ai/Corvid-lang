# Phase 38 Replay-Quarantine For Durable Jobs

This is the design brief for the audit-correction track
`35V2-P38-C-replay-quarantine`, pulled forward from the
`35V2-P38-C-deferred` filing under the 2026-05-26 pre-phase chat.

The goal is to make `@replayable` durable jobs replayable in the same
sense agent traces already are: a recorded job trace must drive a
second execution that reproduces the deterministic shape of the run
(events, ordering, outcomes) without any real side effect leaving the
process. Promotes `jobs.replayable_side_effects` from `OutOfScope` to
`RuntimeChecked`.

## Goals

- Every `@replayable` job persists a typed JSONL trace at completion.
- `corvid jobs replay <job-id>` drives the queue runtime in replay
  mode using that trace.
- During job-replay mode, no real LLM call, HTTP request, store write,
  or file IO can leave the process. A replayed job that deviates from
  the recorded trace must fail with a typed quarantine error, not a
  silent live side effect.
- The promotion to `RuntimeChecked` carries ≥1 positive and ≥1
  adversarial test reference per quarantine surface (LLM, HTTP, store,
  IO).

## Non-Goals

- Distributed multi-host replay or cross-service trace correlation.
- Exactly-once side effects across external providers.
- Replay-time recompilation or trace-mutating debuggers.
- Live-LLM differential replay for jobs. The differential mode that
  exists at the agent layer (`run_replay_from_source` with
  `Differential`) is not extended to jobs in this slice; jobs replay
  in `Plain` mode only.
- A second job runner. Replay is a diagnostic path, not a hidden
  retry. Production execution still flows through the normal queue
  runtime.

## Integration Surface

### Job→Runtime Executor Bridge (C-1)

Today `crates/corvid-cli/src/commands/jobs.rs::cmd_jobs_run` constructs
a `WorkerPool` with a no-op executor closure. The first slice extends
the surface so production job execution uses a real `Runtime`:

- New trait `JobRuntimeExecutor` in `corvid-runtime` with one method:
  `execute(&self, runtime: &Runtime, job: &QueueJob) -> JobOutcome`.
- Default impl resolves the job's agent name from the kind field and
  dispatches through the same VM entry point a CLI `corvid run` uses,
  with the input fingerprint deserialized as the agent args.
- `cmd_jobs_run` threads a `RuntimeBuilder`-constructed `Runtime` into
  the `WorkerPool`. The pool keeps its existing lease/concurrency
  semantics; only the executor body changes.
- Unit test: one persisted job executes through the real Runtime
  stack and returns a typed output that round-trips through the
  checkpoint schema.

### Per-Job Trace Emission (C-2)

`QueueJob.replay_key` becomes the path to the job's JSONL trace.
Naming: `target/trace/jobs/<job-id>.jsonl`. The schema reuses
`corvid-trace-schema` events without extension:

- `SchemaHeader` (writer tier = `Job`, source_path = agent source).
- `RunStarted` (agent name, args fingerprint as recorded JSON array).
- Interleaved `ToolCall` / `ToolResult` / `LlmCall` / `LlmResult` /
  `ApprovalRequest` / `ApprovalResponse` / `ApprovalDecision` /
  `SeedRead` / `ClockRead`.
- `RunCompleted` (ok, output fingerprint, terminal status).

A job that is not `@replayable` does not emit a trace. The emission is
gated on the agent attribute being present at compile time, surfaced
through the existing `AgentAttribute::Replayable` AST path.

The trace file is written on every terminal transition (`succeeded`,
`dead_lettered`, `canceled`) so a crashed job leaves a partial trace
the operator can inspect.

### `replay_job` Entry Point (C-3)

New CLI: `corvid jobs replay <job-id>`. Dispatches to:

```rust
replay_job(
    queue: &DurableQueueRuntime,
    job_id: &JobId,
) -> Result<ReplayOutcome, ReplayError>;
```

The function:

1. Loads `QueueJob` by id; refuses if `replay_key` is empty.
2. Loads the trace at `replay_key`.
3. Builds a `Runtime` in `RuntimeMode::Replay(ReplaySource::JobTrace { job_id, trace_path })`.
4. Installs the four quarantine wrappers (see below).
5. Drives the executor through the recorded events.
6. Returns a `ReplayOutcome` with deterministic event-sequence equality
   against the trace, or a `QuarantineViolation` naming the surface
   and the unrecorded side-effect attempt.

The `ReplaySource` enum lives in
`crates/corvid-runtime/src/runtime/mod.rs`. C-3 adds the `JobTrace`
variant; the existing `AgentTrace` variant (or equivalent) is left
alone. Reuses existing `is_replay_mode()` and
`replay_uses_live_llm()` predicates.

## The Four Quarantine Surfaces

The pattern is identical for every surface: when the `Runtime` is
constructed in job-replay mode, each side-effect-bearing component is
wrapped by a quarantined adapter that:

- Reads from the recorded trace if the call matches a recorded event.
- Returns a typed `QuarantineViolation` if the call does not match.
- Never escapes the process for an unrecorded call.

### C-4 — LLM Quarantine

Wraps each adapter registered in `LlmRegistry`. The wrapper compares
the incoming `LlmCallSpec` against the next expected `LlmCall` event
in the trace; on match returns the recorded `LlmResult`; on mismatch
returns `QuarantineViolation::LlmCallUnrecorded`. The existing
`MockAdapter` and `EnvVarMockAdapter` are not removed; quarantine sits
above them and applies to every adapter equally.

### C-5 — HTTP / Store / IO Quarantine

- **HTTP (`HttpClient`):** every connector request is matched against
  the trace's `ToolCall` events whose effect tag is connector-typed.
  Unmatched requests fail with `QuarantineViolation::HttpUnrecorded`.
- **Store (`StoreManager`):** application writes routed through tool
  calls fail with `QuarantineViolation::StoreWriteUnrecorded` if not
  matched. Note: the queue's own checkpoint writes are not quarantined
  — replay still needs to record its own progress.
- **IO (`IoRuntime`):** file writes, env mutations, and subprocess
  spawn fail with `QuarantineViolation::IoUnrecorded`. File reads are
  permitted only against the same content recorded in the trace
  (content hash check); reads of unrecorded files fail with the same
  violation type.

### Open question for C-5

The store and IO surfaces have a coarse-grained distinction between
"queue-internal" (which must work during replay) and
"application-tool" (which must be quarantined). The first
implementation will use the call-site (`Runtime` accessor used) to
distinguish: anything reached via `Runtime::stores()` or
`Runtime::io()` is application; anything reached via the queue's own
`DurableQueueRuntime` handle is queue-internal. If recon during C-5
reveals this distinction is leaky, the slice will introduce an
explicit `QuarantineContext` token instead, before any test is
written that depends on the wrong layering.

## Adversarial Test Posture

Each of the four quarantine surfaces gets at least one adversarial
test in `crates/corvid-runtime/tests/replay_quarantine_corpus.rs`:

- **LLM:** record a trace where the agent makes one `LlmCall`. Rewrite
  the trace's `LlmCall` event to a different prompt. Replay must fail
  with `QuarantineViolation::LlmCallUnrecorded`. Assert the mock
  network counter remained zero.
- **HTTP:** record a trace with one connector tool call. Mutate the
  trace's `ToolCall` payload to require a different endpoint. Replay
  must fail with `QuarantineViolation::HttpUnrecorded`.
- **Store:** record a trace with one application DB write. Mutate the
  trace so the write is absent. Replay must fail with
  `QuarantineViolation::StoreWriteUnrecorded` when the agent tries to
  write.
- **IO:** record a trace with one `std.files.write` call. Mutate the
  trace so the write is absent. Replay must fail with
  `QuarantineViolation::IoUnrecorded`.

Positive cases mirror the same shape: the unmodified trace replays
cleanly with all counters at zero (no real network / DB / file IO
escapes the process).

## Trust Boundary

The quarantine wrappers belong to the runtime TCB. They are
configured by `RuntimeBuilder` when `RuntimeMode::Replay(_)` is set
and cannot be disabled by Corvid source code. A `@replayable` agent
that tries to bypass the wrapper (for example by reaching into the
host via FFI) is out-of-scope for v1.0; the trust boundary is the
same one `docs/security/model.md` already names.

## Spec & Registry References

- Promotes `jobs.replayable_side_effects` in
  `crates/corvid-guarantees/src/registry.rs` from `OutOfScope` to
  `RuntimeChecked` with positive + adversarial test refs into
  `crates/corvid-runtime/tests/replay_quarantine_corpus.rs`.
- `validate_signed_claim_coverage` walks `AgentAttribute::Replayable`
  declarations against this guarantee id; a signed cdylib cannot ship
  a `@replayable` agent without this guarantee in its descriptor.
- `docs/reference/core-semantics.md` regenerated to carry the
  promoted row.
- `docs/internals/effect-spec/14-replay.md` referenced for the
  underlying replay semantics; this design extends those semantics
  to the job layer.

## Closing Discipline

Each sub-slice C-1 through C-6 lands as its own commit with its own
validation gate (`cargo check --workspace` + targeted lib tests +
`cargo run -q -p corvid-cli -- verify --corpus tests/corpus`). No
batching. dev-log entry per sub-slice; learnings entry once at C-6
capturing the cross-cutting lesson (the audit's "~2-4 days when it
lands" estimate was wrong by an order of magnitude; recon-before-tick
is the standing rule).
