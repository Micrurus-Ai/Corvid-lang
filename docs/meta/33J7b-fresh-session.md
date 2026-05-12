# Fresh-session prompt — open slice 33J7b (runtime-core/host split)

This is a self-contained brief for a fresh Claude session (or
another Rust developer) opening slice 33J7b. The pre-phase chat
closed 2026-05-12 with six load-bearing decisions; this session
turns those decisions into committed code.

**Do not modify the decisions.** They are the contract this
session executes. If the work surfaces evidence that a decision
was wrong, stop and open a follow-up chat in a separate
session — do not unilaterally re-open the design.

---

## You are

The Rust lead on Corvid (`Micrurus-Ai/Corvid-lang`). You know
the workspace conventions documented in
[`CLAUDE.md`](../../CLAUDE.md): file-responsibility discipline,
no-shortcut rule, pre-phase-chat-mandatory, commit-at-slice-
boundaries, push-every-slice-done, update-learnings-per-slice.
You have full repo access. The remote pushes to
`Corvid-lang/Corvid-lang.git` (a mirror); the canonical home is
<https://github.com/Micrurus-Ai/Corvid-lang>.

## Where you are in the roadmap

Phase 33 launch track. CTO confirmed Path B (full cloud IDE for
v1.0, no shortcuts, ~3 month launch slip accepted). Sub-slices:

- **33J7a** ✅ shipped — `corvid-browser::check_project(files,
  entry)` multi-file typecheck for the playground. See
  `crates/corvid-browser/src/multi_file.rs`.
- **33J7b** ← **THIS SESSION** — split `corvid-runtime` into
  `corvid-runtime-core` (wasm-clean) + `corvid-runtime-host`
  (native-only). Estimated 3-4 weeks.
- **33J7c** ⏸ blocked on 33J7b — split `corvid-vm` similarly.
  ~3 weeks. Audit found direct `tokio` / `async-trait` /
  `async-recursion` in vm's `Cargo.toml`; it is NOT a pure port.
- **33J7d** ⏸ — `run_agent` suspend/resume bridge.
- **33J7e** ⏸ — BYO API key + external security review.

## Required reading before code

In order:

1. **The pre-phase chat decisions:**
   [`docs/meta/runtime-split-design.md`](runtime-split-design.md)
   — read the "Decisions" section (D1-D6) and the "Risk
   mitigations" section. These are the contract. The body of the
   doc above the Decisions section is the proposal that produced
   them; useful context but not load-bearing now.

2. **The crate that consumes the split:**
   [`crates/corvid-browser/`](../../crates/corvid-browser/) —
   especially `src/lib.rs` and `src/multi_file.rs`. Slice 33J7a
   shipped here; 33J7c/d will extend it. Keep its dep set
   tight; do not add new dep arrows from corvid-browser to
   anything except corvid-runtime-core post-split.

3. **The wasm-block audit precedent:**
   [`learnings.md`](../../learnings.md) sections "Phase 33J7-prereq
   — `corvid-browser` crate (probe-first slice)" and "Phase 33J7b
   — direct-deps audit catches what transitive-deps misses".
   These document the audit shape you'll repeat.

4. **The thing being split:**
   [`crates/corvid-runtime/`](../../crates/corvid-runtime/) —
   skim the `src/lib.rs` module declaration list to know the
   surface area you're moving.

## Step 1 — R1 stress-test (must come before any code)

The R1 mitigation in the design doc: stress-test the boundary
proposal against every Phase 21–41 feature before writing code.
**This is the single most important step in 33J7b.** Wrong
boundary = re-split later, which costs more than the original
split saves.

Process: walk the Phase 21–41 feature surface. For each feature,
write one row in the table below (which currently sits empty
in this doc — fill it in as you work). Each row answers:

- **Phase / slice** the feature shipped in.
- **What it is** (one-line description).
- **Lands in** (`core` / `host`).
- **Capabilities it touches** (filesystem, network, DB, async-
  runtime, key-material, OS-process, cargo-build, etc.).
- **Test surface** (which existing tests cover it; will they need
  to move with the code?).
- **Notes / red flags** — anything that looks like it might not
  cleanly land in the proposed bucket.

If any feature resists clean placement (e.g. "this needs the
filesystem AND the deterministic state machine at the same call
site"), STOP and open a separate session: that's evidence the
D1-D6 boundary needs revision. Do not patch around it; the
boundary integrity is what 33J7b's value depends on.

### Feature checklist (filled 2026-05-11, slice 33J7b-0)

Conventions: `core` = `corvid-runtime-core` (wasm-clean, IO-free).
`host` = `corvid-runtime-host` (native-only, tokio/reqwest/etc).
`mixed` = trait-split — surface lives in core, impl lives in host
(the D2 / D5 pattern). `n/a (separate crate)` = the phase's code
already lives outside `corvid-runtime` today and the split does
not move it. 🚩 = boundary-application flag; see flagged-rows
section below.

| Phase | Feature | Lands in | Capabilities | Tests | Notes |
|------|---------|----------|--------------|-------|-------|
| 21 — Replay | Record / playback / counterfactual mutation / differential / determinism guard | **mixed** (D2) | State machine = pure; persistence = filesystem | `crates/corvid-runtime/src/replay/*` (mod, cursor, mutation, differential, substitute, diverge, result_factory, event_classify, mutation_session, approval_outcome); `record/writer.rs`; `replay_dispatch.rs` | D2 boundary. `ReplayPlayer` / `ReplayRecorder` state, key derivation, divergence reports, counterfactual mutation, schema header all move to core. `RecorderSink` + `ReplaySource` traits declared in core; filesystem writer impl + JSONL on-disk format stay in host. **R3 caveat**: the executor invariant (sequential `HostRequest`s) is what makes replay deterministic across the bridge — gets enforced in the `AgentExecutor` API shape in slice 33J7b-2. |
| 22 — C ABI / library mode | cdylib emit, ABI descriptor, runtime-queryable catalog, approval bridge, grounded return, budget observe, replay-across-FFI, host bindings, ownership check | **mixed** | cdylib codegen = out of split (lives in `corvid-codegen-cl`); ABI descriptor + catalog descriptor types = pure data (core); FFI bridge raw-pointer dispatch (`unsafe` + tokio) = host; C-runtime staticlib = host build product; attestation store (signed claims) = host (key material per D1) | `abi.rs`, `catalog/*`, `catalog_c_api/*`, `ffi_bridge/*`, `attestation_store.rs` | Three sub-pieces, three buckets. `abi::ToolMetadata`, `CorvidString`, `CorvidGroundedHandle`, `CorvidObservationHandle`, `catalog::descriptor`, `catalog::filter` are core. `catalog::invoke::call_agent`, `catalog_c_api::*`, `ffi_bridge::*`, `attestation_store` are host. The `c_runtime` `OUT_DIR` staticlib path is a host build artefact. Replay-across-FFI (22-H) inherits D2 — works in core, persistence in host. |
| 23 — WASM target | (already shipped; the consumer of this split) | **core** (consumed by `corvid-browser`) | wasm-clean entry path is the whole point of the split | `crates/corvid-codegen-wasm/*`, `crates/corvid-browser/*` | Listed for completeness even though the brief's checklist starts at 21. The wasm codegen + browser crate consume `corvid-runtime-core`; that's what 33J7b is for. |
| 24 — LSP / IDE | Diagnostics, hover, completion, navigation, rename | **n/a (separate crate)** | `crates/corvid-lsp` depends on `corvid-driver` typecheck + `corvid-ast`/`corvid-types`/`corvid-resolve`, not on `corvid-runtime` | `corvid-lsp` tests | Out of runtime-split scope. LSP serves a typecheck-only surface today; if it ever needs runtime semantics (effect rendering, budget burn-down) the split's re-export contract (D6) keeps `corvid_runtime::Foo` working — zero LSP changes required. |
| 25 — Package manager | `corvid add`, lockfile, signed publish, registry HTTP, semver, policy gates, `verify-lock`, import summary | **n/a (separate crate)** | Lives in `corvid-cli` + `corvid-driver`; not in `corvid-runtime` | `corvid-driver` package tests | Package-import math (SHA-256 hash verification, ed25519 signature verification, semver resolution) is pure and wasm-compatible already. Registry HTTP fetch is host-only by nature but lives in the driver, not the runtime. No runtime-split impact. |
| 26 — Testing primitives | `test`, `fixture`, `mock`, `assert_snapshot`, trace fixtures | **mixed** | `test_from_traces` analyzer (TraceHarnessRequest, Divergence, FlakeRank, ModelSwapOutcome, PromoteDecision) = deterministic (core); snapshot fs IO + `corvid test --update-snapshots` runner = host (lives in `corvid-cli` + `corvid-driver`); mock tool dispatch = core (no IO) | `test_from_traces.rs`; runner tests in driver | `test_from_traces.rs` is one of the clean wins — it's already an analyzer over JSONL with no IO. Moves to core. The snapshot file-IO path lives in CLI/driver layer outside corvid-runtime. |
| 27 — Eval | `corvid eval`, swap-model, ratio archives, prompt-diff, regression detection, budget enforcement | **mixed** | Eval analyzer = core (reuses Phase 21 replay infra per D2); HTML/JSON/terminal report writes = host (in driver/CLI); regression detection over stored summaries = pure (could be core if it were in runtime, but it's in driver) | driver eval tests | Reuses the replay D2 boundary cleanly. No new boundary concerns. |
| 28 — HITL | `await_approval`, approvals queue, operator UI hooks, audit | **mixed** | Approval state machine (`approval_authorization::authorize_approval_transition`, `approval_policy::validate_*`, `approvals::token`, `Approver` trait, `ProgrammaticApprover`) = core (deterministic); `StdinApprover` = host (stdin); approval queue SQLite persistence (`approval_queue::sqlite`) = host; approval UI payload builder (`approval_ui_payload`, `check_approval_ui_contract`) = core (pure data shapes); `ApprovalQueueRuntime` trait = core | `approvals/*`, `approval_authorization.rs`, `approval_policy.rs`, `approval_queue/*`, `approval_ui.rs`, `approver_bridge/*`, `review_queue.rs`, `auth/approvals.rs` | Same trait-split pattern as D2/D5: state machine in core, persistence + IO impls in host. `approver_bridge::compile/state/simulate` is largely deterministic (core); `approver_bridge` runtime hook into actual stdin/UI = host. |
| 29 — Memory primitives | `session`, memory stores | **mixed** | `StoreBackend` trait, `StoreManager`, `StoreKind`, `StorePolicySet`, `StoreRecord`, `InMemoryStoreBackend` = core; `SqliteStoreBackend` = host; `store::policy_parse` = core (pure parsing) | `store.rs`, `store/*`, `runtime/tests/store.rs` | Trait-split mirrors approval queue and replay. In-memory backend in core means the playground gets working session/memory primitives "for free" — bonus reach for D5's pattern beyond connectors. |
| 30 — Python FFI | PyO3 bridge, sandbox profile | **host** (D4 feature flag) | PyO3 links libpython at build time; can never be wasm-clean | `runtime/tests/python.rs` | Already gated behind `feature = "python"` in current runtime. Becomes one of the D4 per-module flags on `corvid-runtime-host`. No additional split work — already isolated correctly. |
| 31 — Multi-provider LLM | Model substrate, provider routing, adapters (Anthropic, OpenAI, Gemini, Ollama, OpenAI-compatible) | **mixed** | `LlmAdapter` trait, `LlmRegistry`, `LlmRequest`, `LlmResponse`, `ProviderHealth`, `TokenUsage`, `ModelCatalog`, `ModelSelection`, `RegisteredModel`, `MockAdapter`, `EnvVarMockAdapter`, `LlmUsageLedger` = core; `AnthropicAdapter`, `GeminiAdapter`, `OllamaAdapter`, `OpenAiAdapter`, `OpenAiCompatibleAdapter` (all reqwest-backed) = host | `llm::*`, `models.rs`, `usage.rs`, `runtime/tests/llm.rs` | Cleanest split in the table. Mirrors D5 exactly: trait + mock + registry + ledger in core; real adapters in host. Provider routing logic (`ModelSelection`) is pure → core. |
| 32 — Stdlib | std.io, std.json, std.ai, std.http envelopes | **mixed per-module (D4)** | `IoRuntime` (filesystem reads/writes/streams) = host; `HttpClient` (reqwest) + `HttpRetryPolicy` = host; `record_exchange`, `request_fingerprint` (deterministic) = core; `HttpRequest`/`HttpResponse`/`HttpHeader`/`RecordedHttpExchange` types = core; std.json/std.ai envelope types = core; `cache::CacheRuntime` + `build_cache_key` = core (math); `secrets::SecretRuntime` = host (filesystem + key material); `env::load_dotenv*` = host (filesystem); `redact::RedactionSet` = core | `io.rs`, `http.rs`, `cache.rs`, `secrets.rs`, `env.rs`, `redact.rs`, `runtime/tests/io_http.rs`, `runtime/tests/secrets_cache.rs` | D4 maps to per-stdlib-module feature flags. Type envelopes (declared in stdlib `.cor` files) stay where they are — unchanged. Each impl module's host code goes behind its feature flag in host. |
| 33 — Launch track | Docs site, playground, BYO API key, security review | **n/a (separate concern)** | Out of runtime split | n/a | The phase that's running this slice. Not stress-test material. |
| 34 — (not in checklist) | — | n/a | — | — | — |
| 35 — Defensible core | Stability contract, claim audit, audit CLI | **n/a (separate concern)** | Lives in docs + audit tooling, not runtime | n/a | Not affected by split. |
| 36 — Backend | Server render, route dispatch, middleware, handler isolation | **n/a (code-emit only)** | `corvid-driver::build::server_render` emits a Rust source string with axum that ships as part of the user's compiled binary; the runtime crate doesn't link axum | `corvid-driver/src/build/server_render.rs` tests | Out of runtime-split scope. The generated server is a separate Rust project the user builds with their own `cargo`. Important: this means the runtime split does NOT need to handle backend-server lifecycle. |
| 37 — Persistence | std.db (Postgres + SQLite), migrations, drift, encrypted tokens, audit log | **mixed** (host-heavy) | `DbValue`, `DbCell`, `DbQueryRows`, `DbExecuteResult`, `DbDecodeError`, `decode_i64`, `decode_string` (envelope + decode math) = core; `PostgresDbRuntime`, `SqliteDbRuntime` (drivers) = host; audit log canonical-bytes derivation per D1 = core (sign-less); migration runner + `migrate down` = host; encrypted-token storage = host (key material) | `db.rs`, runtime tests | D1 pattern (canonical bytes in core, signing/persistence in host) applies cleanly. Encrypted-token surface lives in host with key material; the storage envelope type can be in core. |
| 38 — Jobs | Durable runner, schedules, DST-aware cron, idempotency, approval-wait, worker pool | **mixed** | `QueueJob`, `QueueJobStatus`, `QueueRuntime` trait, idempotency-key derivation, schedule arithmetic = core (deterministic); `DurableQueueRuntime` (SQLite-backed) = host; `worker_pool` (multi-worker tokio loop) = host; approval-wait integrates with Phase 28's split | `queue/*`, `worker_pool.rs`, `queue/tests/*` | Schedule + cron math is pure (core). Lease management (`queue/leases.rs`) is mostly deterministic but touches DB rows — split: lease state machine in core, SQLite lease writes in host. **R3 (replay determinism) most-stressed here**: jobs naturally have parallel `await` patterns (worker pool); core's `AgentExecutor` invariant must hold even when N workers are running, which means each worker is its own sequential executor — confirm this is the slice 33J7b-2 contract before slice 33J7b-3 starts moving queue code. |
| 39 — Auth / approvals | JWT verifier, JWKS fetcher, OAuth callback, approval CLI, replay quarantine, session auth | **mixed** | `validate_jwt_verification_contract`, `JwtVerificationContract`, `JwtContractDiagnostic` (contract validation logic) = core; JWT signature verification (`hash_*`, `verify_*`, ed25519/HMAC math) = core; JWKS HTTP fetcher = host (reqwest); OAuth state derivation (`hash_oauth_state`, `OAuthStateCreate`, `OAuthStateRecord`) = core; OAuth callback IO (`OAuthCallbackResolution` resolver, redirect flows) = host; `SessionAuthRuntime` (DB-backed) = host; `ApiKeyCreate`, `ApiKeyRecord`, `ApiKeyResolution`, `authorize_trace_permission`, `AuthorizationDecision`, `PermissionRequirement`, `AuthActor`, `AuthAuditEvent`, `AuthTraceContext` = core | `auth/*`, `jwt_verify.rs` | JWT signature verification (HMAC + ed25519 + JWS) is pure crypto → core. JWKS HTTP fetch → host. The cleanest pattern in 39 is that almost all the *contract validation* code is core (it's just data transforms), while only the network/DB IO is host. |
| 40 — Observability | OTel SDK, lineage graph, redaction, runbooks, drift, eval promotion | **mixed** | `otel_schema::*`, `lineage_to_otel_span`, `required_otel_metrics`, `OtelMetricMapping`, `OtelSpanMapping`, `OTEL_SCHEMA_VERSION` = core; `lineage::*` (span_id, validate, events) = core; `lineage_drift::*` (drift report computation) = core; `lineage_eval::*` (fixture promotion) = core; `lineage_redact::*` (redaction policy + apply) = core; `lineage_render::render_lineage_tree` = core (text render); `lineage_incidents::group_lineage_incidents` = core; `otel_export::build_otel_export_batch`, `OtelExportBatch`, `OtelExporterConfig` = core (batch building is pure); `OtelHttpExporter`, `OtelExportError`, `OtelExportReport` (the actual HTTP exporter) = host; `otel_sdk_export` (OTel SDK + OTLP/HTTP wire) = host; `observe::*` summaries (provider/route/approval/latency observation) = core; `calibration::*`, `ensemble::*` = core | `otel_*`, `lineage*`, `observe.rs`, `calibration.rs`, `ensemble.rs` | Phase 40 is overwhelmingly core. The SDK exporter and HTTP wire are the only host surface. Even `build_otel_export_batch` is a pure data transform — it just emits an `OtelExportBatch` value the host serializes. This phase justifies the split's value cleanly: lineage analysis works in the browser playground for free. |
| 41 — Connectors 🚩 | Gmail / Slack / MS365 / Calendar / Tasks / Files connectors in three modes (mock / replay / real), threat corpus, DSSE bundle, OAuth refresh, rate-limit, webhook verify, redaction | **🚩 boundary-application gap — D5 needs extending to `corvid-connector-runtime` crate** | All connector code lives in **separate crate** `crates/corvid-connector-runtime/` (NOT in `corvid-runtime`), with `reqwest` as a **direct** Cargo.toml dep — wasm-blocking on the whole crate. Files mix mock+replay+real together: `auth.rs`, `calendar.rs`, `files.rs`, `github_real.rs`, `gmail.rs`, `gmail_real.rs`, `manifest.rs`, `ms365.rs`, `oauth2_refresh.rs`, `rate_limit.rs`, `real_client.rs`, `runtime.rs`, `slack.rs`, `slack_real.rs`, `tasks.rs`, `test_kit.rs`, `trace.rs`, `webhook_verify.rs` | corvid-connector-runtime tests | **Flag.** D5 says "mock + replay connector machinery moves to core" — but the connector crate is not inside `corvid-runtime` today, so the runtime split doesn't reach it. To honour D5's intent, slice 33J7b needs to either: (a) extend its scope to also split `corvid-connector-runtime` into `-core` (manifest, mock, replay, contract types, rate-limit math, webhook signature verification math) + `-host` (real_client, *_real.rs, oauth2_refresh, reqwest path); or (b) declare the connector split a follow-up sub-slice 33J7b-Connectors filed before slice 33J7b-7 closes; or (c) explicitly note in the slice-7 closing audit that D5 is honoured only for the connector type-surface + replay/mock infrastructure that ride through `corvid-runtime`'s `replay::*` machinery, while the connector crate itself stays native-only until a separate `33J7b-Connectors` slice. Per the user's "flag and keep going" policy: continuing with the table, not amending D1–D6. The remediation choice gets made before slice 33J7b-3 starts (when connector-adjacent code starts moving). |

### Flagged rows summary

One row flagged (Phase 41, Connectors). The flag is a **boundary-application gap**, not evidence that D5 is wrong:

- D5 itself ("mock + replay machinery in core, real in host") is the right call.
- The gap is that D5 was written assuming connector code lived inside `corvid-runtime`. In reality it lives in the separate crate `corvid-connector-runtime` (with direct `reqwest`), and the runtime split as currently scoped doesn't reach into that crate.
- Three remediation paths exist (see the Phase 41 row's Notes). The choice gets made before slice 33J7b-3 (when connector-adjacent moves would have to happen).
- This is the value-of-the-stress-test in action: caught at slice 0, not at slice 3 when half the runtime is already moved.

Per the user's slice-0 commit policy (flag-and-keep-going, no checkpoint), this commit closes slice 33J7b-0. The Connectors-crate remediation is a precondition for slice 33J7b-4 (the slice that actually moves connector code per D5); the call gets made before 33J7b-4 opens, not now.

When the table is complete, the stress-test is closed. Commit
this doc with the filled table; that commit is **the slice-0
deliverable** of 33J7b ("boundary verified").

## Step 2 — Code slices (after stress-test closes)

Commit cadence: one commit per logical step. Per CLAUDE.md's
"commit at slice boundaries" rule, do not batch. Validation
gate runs between every commit. Push every commit.

### 33J7b-1 — Scaffold `corvid-runtime-core` empty

- Create `crates/corvid-runtime-core/` with `Cargo.toml` +
  `src/lib.rs`. `crate-type = ["rlib"]`, no `cdylib` (this is
  consumed as a library, not a JS-callable cdylib).
- Dep set restricted to: `corvid-ast`, `corvid-ir`,
  `corvid-resolve`, `corvid-types`, `corvid-guarantees`,
  `corvid-trace-schema`, `corvid-prompt-format`, `serde`,
  `serde_json`. **No tokio, no postgres, no reqwest, no
  opentelemetry, no libloading, no hyper.**
- Add to workspace members.
- `src/lib.rs` is empty + a single docstring.
- **Acceptance**: `cargo build -p corvid-runtime-core --target
  wasm32-unknown-unknown --release` succeeds.

Commit message: `feat(33J7b-1): scaffold corvid-runtime-core
(wasm-clean empty crate)`.

### 33J7b-2 — Define `HostRequest` / `HostResponse` enum + suspend/resume primitive

- Add `core/src/host.rs` with the wire-format enum + a
  `HostBridge` trait. Both serde-derived. `version: "v1"` field
  at the root of `HostRequest` and `HostResponse` per R4
  mitigation.
- `HostRequest` variants (initial; expand as features move):
  `LlmCall`, `HostCall`, `DbQuery`, `FsRead`, `FsWrite`,
  `HttpRequest`, `OtelEmit`.
- `HostBridge` trait exposes one method: `async fn resolve(req:
  HostRequest) -> HostResponse`. The trait is what
  `corvid-runtime-host` and `corvid-browser` both implement.
- Unit tests for serialization round-trip.

Commit message: `feat(33J7b-2): HostRequest/HostResponse +
HostBridge trait`.

### 33J7b-3 — Move deterministic state into core

- Move (with `git mv` where structurally clean):
  - The `Effect` / `EffectRow` / approval-token / grounded-
    provenance state machinery from `corvid-runtime` to
    `corvid-runtime-core`.
  - The replay state machine (`ReplayPlayer` / `ReplayRecorder`)
    per D2. Persistence behind `ReplaySource` / `RecorderSink`
    traits (defined in core; implemented in host).
  - The canonical receipt-bytes derivation per D1 (sign-less).
- These moves should NOT touch any tokio / reqwest / postgres
  code — those stay in `corvid-runtime` for now.
- Existing `use corvid_runtime::*` paths in CLI/REPL/tests
  continue to work because `corvid-runtime` re-exports
  everything that just moved (set up the re-export in this
  commit).
- **Acceptance**: full workspace tests pass + `cargo build -p
  corvid-runtime-core --target wasm32-unknown-unknown` stays
  green.

Commit message: `refactor(33J7b-3): move deterministic state to
corvid-runtime-core`.

### 33J7b-4 — Move mock + replay connector machinery to core (per D5)

- Per D5, mock-mode and replay-mode connector adapters live in
  core. Real-mode stays in `corvid-runtime`.
- Phase 41L's drift test (`mock ≡ replay ≡ real` shared typed
  surface) splits into:
  - `mock ≡ replay` — core-only test, runs in core's test suite.
  - `real ≡ replay` — host-only integration test, stays in
    `corvid-runtime-host` once that crate exists.
- Document the test split in `crates/corvid-runtime-core/
  tests/connector_mock_replay.rs` and the host-side mirror.
- **Acceptance**: connector contract drift tests pass on both
  sides.

Commit message: `refactor(33J7b-4): move mock+replay connector
adapters to core (D5)`.

### 33J7b-5 — Rename `corvid-runtime` → `corvid-runtime-host`

- `git mv crates/corvid-runtime → crates/corvid-runtime-host`.
- Update workspace members.
- `corvid-runtime-host` re-exports everything from
  `corvid-runtime-core` per D6 so `corvid_runtime::Foo` from
  user code keeps resolving.
- This is the load-bearing rename — every consumer's
  `Cargo.toml` updates from `corvid-runtime = ...` to
  `corvid-runtime-host = ...` (most consumers won't need to,
  since they go through the re-export).
- **Acceptance**: full workspace builds + tests pass; `cargo
  test --workspace` is green.

Commit message: `refactor(33J7b-5): rename corvid-runtime to
corvid-runtime-host; preserve re-export contract`.

### 33J7b-6 — Add per-module feature flags to corvid-runtime-host (per D4)

- Add features in `corvid-runtime-host/Cargo.toml`: `db`,
  `http`, `jobs`, `auth`, `observability`, `connectors`.
  `default = ["db", "http", "jobs", "auth", "observability",
  "connectors"]` so nothing breaks for existing users.
- Wire each stdlib impl module behind its corresponding
  `#[cfg(feature = "...")]`.
- **Acceptance**: `cargo build -p corvid-runtime-host
  --no-default-features` succeeds (proves the gating works);
  `cargo build -p corvid-runtime-host` with defaults succeeds
  (proves nothing broke).

Commit message: `feat(33J7b-6): per-module feature flags on
corvid-runtime-host (D4)`.

### 33J7b-7 — Closing audit + ROADMAP tick + learnings

- Tick `33J7b-runtime-split` in
  [`ROADMAP.md`](../../ROADMAP.md).
- Append closing-audit section to
  [`docs/meta/runtime-split-design.md`](runtime-split-design.md):
  outcome per decision (D1-D6 verified shipped vs. needed
  revision), what changed in the proposal, what stayed.
- Append slice closeout to
  [`learnings.md`](../../learnings.md) with whichever cross-
  slice patterns surfaced during the refactor.
- Validate corpus baseline + full workspace tests.

Commit message: `docs(33J7b-7): close runtime-split slice; tick
33J7b in ROADMAP`.

## Validation gate (run between every commit)

1. `cargo check --workspace --tests` clean.
2. `cargo test -p corvid-guarantees --lib` 22/22 pass (registry
   sentinels — the no-shortcut canary).
3. `cargo test -p corvid-browser --tests` 14/14 pass (the
   playground side; nothing changed but ensure no regression).
4. `cargo build -p corvid-runtime-core --target
   wasm32-unknown-unknown --release` succeeds (the load-bearing
   property — R2 mitigation).
5. After 33J7b-5 onward: `cargo test --workspace` passes (the
   full integration surface).
6. `cargo run -q -p corvid-cli -- verify --corpus tests/corpus`
   — same baseline (the pre-existing `combined_all.cor`
   `Grounded<String> + String` baseline from Phase 35V T1-H
   stays the only failure; any new failure means a regression).

## Acceptance criteria for closing 33J7b

- [ ] All 7 sub-commits (33J7b-1 through 33J7b-7) on `main`.
- [ ] `corvid-runtime-core` compiles to
      `wasm32-unknown-unknown` on every push (CI step added,
      same shape as the existing `browser-typechecker-wasm`
      job).
- [ ] `corvid_runtime::Foo` paths still resolve for existing
      native users — re-export contract holds (verified by
      running existing CLI/REPL/test surfaces unchanged).
- [ ] Phase 41L's `mock ≡ replay ≡ real` drift test split into
      core-only (`mock ≡ replay`) + host integration (`real ≡
      replay`); both green.
- [ ] `cargo test --workspace` green; `cargo run -q -p
      corvid-cli -- verify --corpus tests/corpus` baseline
      unchanged.
- [ ] Closing-audit section appended to
      `docs/meta/runtime-split-design.md` recording any
      deviation from D1-D6 and why.
- [ ] `33J7b-runtime-split` ticked in `ROADMAP.md`.
- [ ] `learnings.md` updated with any new cross-slice pattern.

## Constraints (the no-shortcut rules for this slice)

- **Do not skip the stress-test.** R1 mitigation. The
  prediction is that 1-2 features will surface placement
  ambiguity. Discover those in the table, not in slice 33J7b-3.
- **Do not amend D1-D6.** If a stress-test row resists clean
  placement, stop and open a new chat. Don't compromise the
  boundary to avoid a session boundary.
- **Build wasm32 from slice 1.** Failed builds catch tokio
  creep early. R2 mitigation.
- **Keep `corvid_runtime::*` re-exports working.** D6. Existing
  CLI / REPL / test users see no API change.
- **No new wasm-blocking deps in core.** The whole point of
  the split is that core stays clean. CI enforces this in
  33J7b-7.
- **Push after every commit.** Per the project's "push every
  slice done" rule; do not let work stack up locally.

## What's out of scope for this session

- **33J7c (vm split).** Separate slice after 33J7b. Same shape
  (split core + host) but applied to `corvid-vm`. Pre-phase
  chat on the vm split happens in another session.
- **33J7d (run_agent bridge).** Depends on 33J7c.
- **33J7e (BYO API key + security review).** Depends on 33J7d.
- **Performance work.** Do not optimize during the refactor.
  Move code, get tests green, ship. Performance is a separate
  slice if it becomes a problem.
- **Documentation updates beyond what's named here.** The
  `docs/` tree's user-facing material (book, guides, reference)
  doesn't change as a function of the internal split — it
  documents the language, not the crate structure.

## How to start

1. Read this entire brief.
2. Read [`docs/meta/runtime-split-design.md`](runtime-split-design.md)
   — Decisions + Risk mitigations sections specifically.
3. Read [`learnings.md`](../../learnings.md) sections on
   33J7-prereq and 33J7b.
4. Open the Phase 21–41 feature checklist in this doc. Walk
   the rows. Fill them in. Commit when done.
5. If no feature surfaces a boundary problem, open 33J7b-1
   and proceed through to 33J7b-7.
6. If any feature resists placement, stop. Open a new session
   with the row's evidence. Do not amend D1-D6 unilaterally.

Estimated session duration: 3-4 weeks of focused work. The
stress-test alone is 1-2 days (walking ~17 phases × 2-5
features each). The actual refactor is 2-3 weeks split across
the 7 sub-commits.

Good luck. The boundary is designed; your job is to verify it
and execute. Ship one honest split, not seven aspirational ones.

— Rust lead
