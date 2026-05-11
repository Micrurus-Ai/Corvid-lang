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

### Feature checklist (fill in)

| Phase | Feature | Lands in | Capabilities | Tests | Notes |
|------|---------|----------|--------------|-------|-------|
| 21 — Replay | (record / playback / quarantine / determinism) | | | | |
| 22 — C ABI / library mode | (cdylib emit, ABI descriptor, signed claim) | | | | |
| 24 — LSP / IDE | (diagnostics, hover, completion) | | | | |
| 25 — Package manager | (resolve, publish, verify-lock, import-summary) | | | | |
| 26 — Testing primitives | (`test`, `fixture`, `mock`, `assert_snapshot`) | | | | |
| 27 — Eval | (eval runner, swap-model, ratio archives) | | | | |
| 28 — HITL | (await_approval, approvals queue, operator UI hooks) | | | | |
| 29 — Memory primitives | (session, memory stores) | | | | |
| 30 — Python FFI | (PyO3 bridge, sandbox config) | | | | |
| 31 — Multi-provider LLM | (model substrate, provider routing, adapters) | | | | |
| 32 — Stdlib | (std.io, std.json, std.ai, std.http envelopes) | | | | |
| 36 — Backend | (server render, route dispatch, middleware, handler isolation) | | | | |
| 37 — Persistence | (std.db, migrations, drift, encrypted tokens, audit log) | | | | |
| 38 — Jobs | (durable runner, schedules, DST cron, idempotency, approval-wait) | | | | |
| 39 — Auth / approvals | (JWT verifier, JWKS, OAuth, approval CLI, replay quarantine) | | | | |
| 40 — Observability | (OTel SDK, lineage graph, redaction, runbooks) | | | | |
| 41 — Connectors | (Gmail/Slack/MS365/Calendar/Tasks/Files, mock/replay/real, threat corpus, DSSE bundle) | | | | |

Each row gets filled in by reading the phase's entry in
[`ROADMAP.md`](../../ROADMAP.md) and the matching closed-phase
doc under [`docs/phases/`](../phases/) if one exists. The
phase doc tells you the canonical feature list; you decide
which bucket each feature lands in per D1-D6.

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
