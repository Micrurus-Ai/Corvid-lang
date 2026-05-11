# Pre-phase chat — `corvid-runtime` split for 33J7b

**Status:** OPEN. Not yet started. This doc captures the proposal
+ open questions so the chat can start at a concrete agenda rather
than re-deriving the problem.

**Filed:** 2026-05-12, after CTO confirmation of Path B (full cloud
IDE for v1.0, ~3 month launch slip).

**Dependents:** 33J7c (`corvid-vm` wasm port), 33J7d (`run_agent`
bridge), 33J7e (BYO API key). Doing this split wrong costs more
later than the split saves now. Get the boundary right first.

---

## The problem

`crates/corvid-runtime` today is the project's largest crate and
its biggest grab-bag. Its dep graph (`cargo tree -p corvid-runtime
| wc -l` → 843) pulls:

- `tokio` (async runtime)
- `hyper` + `reqwest` + `hyper-rustls` (HTTP clients)
- `tokio-postgres` + `postgres` + `rusqlite` (DB drivers)
- `opentelemetry` + `opentelemetry-otlp` + `opentelemetry_sdk` (OTel SDK)
- `libloading` (dynamic library loading)
- Connector OAuth, OAuth token storage, signing, etc.

None of these compile to `wasm32-unknown-unknown`. The cloud IDE
needs the runtime to compile to WASM so `run_agent(...)` can
execute agent code in the browser. Refactor required.

This isn't novel work — it's the file-responsibility rule
(`CLAUDE.md`) applied at crate scale. The runtime grew through
Phases 21 (replay), 27 (eval), 28 (HITL), 29 (memory), 30 (Python
FFI), 31 (LLM adapters), 32 (stdlib), 36 (backend), 37
(persistence), 38 (jobs), 39 (auth), 40 (observability), 41
(connectors). Each phase added more native-IO surface to one
crate. Now the boundary is overdue.

## The proposed split

Two new crates replace `corvid-runtime`:

### `corvid-runtime-core` — wasm-clean

Owns the **deterministic, IO-free, replay-relevant** runtime
state machine. Compiles to `wasm32-unknown-unknown`.

In scope:

- Agent / prompt / tool dispatch — the interpreter machinery for
  walking IR and producing effects.
- Effect-row composition and budget tracking — pure math on
  effect dimensions.
- Approval-token state — what's been `approve`d, in what scope,
  with what arguments.
- Grounded provenance state — which values flowed from which
  sources, with citation metadata.
- Replay state machine — recording shape, key derivation, replay
  scheduler. The recorder writes to an injected `RecorderSink`
  trait; the host crate provides the concrete sink (file,
  network, stdout).
- Trace event schema + emission to an injected `TraceEmitter`
  trait.
- Suspend/resume primitive: when execution hits a JS-resolvable
  boundary, yield a structured `HostRequest` and accept a
  `HostResponse` on resume. This is what 33J7d wires through
  `wasm-bindgen-futures` in the browser, and what
  `corvid-runtime-host` resolves synchronously / async on
  native.

Dep set restricted to:

- `corvid-ast`, `corvid-ir`, `corvid-types`, `corvid-resolve`,
  `corvid-guarantees`, `corvid-trace-schema`, `corvid-vm`
  (once 33J7c lands).
- `serde` + `serde_json` for trace event serialization.
- Pure-Rust algorithmic deps (e.g. `ahash`, `indexmap`).
- **No tokio, no hyper, no postgres, no rusqlite, no
  opentelemetry, no libloading, no reqwest.**

### `corvid-runtime-host` — native-only

Owns the **native-side capabilities** the agent runtime calls
through traits implemented by the host. Stays exactly as
`corvid-runtime` is today minus what moved into core.

In scope:

- Tokio runtime (single-threaded or multi-thread, configurable).
- `std.db` SQLite + Postgres connections + transactions + audit
  log writes. Real driver-backed.
- `std.http` real HTTP client (reqwest) for connector + arbitrary
  outbound calls.
- LLM provider adapters (OpenAI, Anthropic, Google, Bedrock,
  Cohere, Ollama) — implements `LlmProvider` trait declared in
  core.
- `std.auth` JWT verifier + JWKS fetcher + OAuth flows.
- `std.observability` OTel SDK exporter + OTLP/HTTP wire.
- `std.connectors` real-mode connector implementations (Gmail,
  Slack, MS365, Calendar, Tasks, Files).
- `std.jobs` durable runner + multi-worker pool + DST-aware cron.
- Signed receipt persistence + DSSE bundle emission.
- Filesystem-backed `RecorderSink` and `TraceEmitter` impls.
- `libloading` for dynamic plugin loading (if still used).

Dep set: everything the current `corvid-runtime` has that
doesn't move into core. Tokio, hyper, postgres, etc. all stay.

### How agents run

```
┌────────────────────────────────────────────────────┐
│  corvid-runtime-core (wasm-clean)                  │
│                                                    │
│  ┌─────────────────────────────────────────────┐   │
│  │ AgentExecutor                               │   │
│  │   walks IR, composes effects, tracks        │   │
│  │   approvals, runs prompts                   │   │
│  └─────────────────────────────────────────────┘   │
│           ↓ at a JS-resolvable boundary             │
│  ┌─────────────────────────────────────────────┐   │
│  │ Yield HostRequest (enum):                   │   │
│  │   - LlmCall { provider, model, messages }   │   │
│  │   - HostCall { ns, method, args }           │   │
│  │   - DbQuery { sql, params }                 │   │
│  │   - FsRead { path } / FsWrite { path, body} │   │
│  │   - HttpRequest { method, url, headers }    │   │
│  └─────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────┘
         ↑                                ↓
         │ resume with HostResponse       │ host implements
         │                                │
┌────────────────────────────┐  ┌────────────────────────────┐
│ Browser host (corvid-      │  │ Native host (corvid-       │
│ browser via wasm-bindgen-  │  │ runtime-host)              │
│ futures + JS Promise)      │  │                            │
│                            │  │ Uses tokio + reqwest +     │
│ Yields HostRequest as a    │  │ postgres + OTel SDK to     │
│ structured JS object; JS   │  │ fulfill each HostRequest   │
│ resolves; the future       │  │ on the native side.        │
│ resolves with HostResponse │  │                            │
└────────────────────────────┘  └────────────────────────────┘
```

The core knows nothing about how a `HostRequest` is resolved.
The host (browser or native) owns capability resolution. This is
the same shape Wasmtime uses with its `Linker` trait,
ServiceWorker uses with `FetchEvent`, and POSIX uses with
syscalls.

## Open questions for the chat

The proposal above is a starting point, not a decision. These
are the specific questions to resolve before code:

### Q1. Receipts and signing — core or host?

A signed run produces a DSSE-attested receipt. The receipt
contents (audit log entries, prompt records, tool calls, trace
metadata) are recorded by the core's `RecorderSink`. The
*signing* (ed25519 over the canonical bytes) is a host
responsibility — keys live on the host, not in the
deterministic core.

**Proposal:** core emits canonical receipt bytes; host signs
them. WASM playground does not sign receipts (no key material in
the browser); native CLI signs as today.

**Alternative:** keep all receipt machinery in host. Core just
emits trace events; host assembles the receipt structure.

Pick one. Affects how 33J7d surfaces "I ran an agent" results
to the browser.

### Q2. Replay record + playback — core or host?

Replay is the project's flagship invention. It must work in
both contexts:

- **Native:** replay a trace stored on disk.
- **Browser:** replay a trace stored in IndexedDB.

The replay state machine (which step are we on, what's the next
expected event, etc.) is deterministic and IO-free — that
belongs in core. The *persistence* of the trace (file path,
IndexedDB key, etc.) is IO — that's host.

**Proposal:** core owns `ReplayPlayer`/`ReplayRecorder` state
machines reading/writing through `RecorderSink`/`ReplaySource`
traits. Host implements the traits for native filesystem; the
browser implements them for IndexedDB.

### Q3. `corvid-vm` — split or just port?

`corvid-vm` is the interpreter. Its dep graph has tokio +
postgres + reqwest etc. — but those might be *transitive* through
`corvid-runtime`, not direct.

**Audit needed before chat:** `cargo tree -p corvid-vm
--no-default-features` + `--depth 1` to see if corvid-vm's
direct deps are clean or if the runtime entanglement is intrinsic
to the VM crate itself.

If clean → 33J7c is a port, not a split. If entangled → 33J7c
is its own split following the same pattern as 33J7b.

### Q4. Stdlib modules — where do `std.db`, `std.http`, `std.jobs`, `std.auth`, `std.observability`, `std.connectors` live?

Each stdlib module has two halves:
- The **type surface** (envelopes, effect rows, contract
  declarations) — load-bearing for typecheck, must be core-
  visible.
- The **impl** (DB driver, HTTP client, OAuth flow, OTel SDK
  exporter) — load-bearing for runtime, must be host-only.

**Proposal:** stdlib `.cor` files (which declare the type
surface) stay in `std/` at the workspace root, unchanged.
Implementation crates (currently inside `corvid-runtime`) move
to per-module crates `corvid-stdlib-db`, `corvid-stdlib-http`,
etc., each depending on `corvid-runtime-host`. Sounds heavy but
gives each module its own dep-graph audit boundary.

**Alternative:** keep all impls in one `corvid-runtime-host`
crate. Simpler dep tree; harder to audit per-module
wasm-incompatibility.

### Q5. Connector mock/replay/real boundary — where?

Phase 41 ships every connector in three modes. Mock and replay
are deterministic and IO-free (mock returns canned responses;
replay reads from a recorded trace). Real mode is the host-only
network-touching mode.

**Proposal:** mock + replay connector machinery moves to core.
Real-mode adapters stay in host. The browser playground gets
mock + replay connectors for free; real-mode connectors return
a "sandboxed in browser" diagnostic via the suspend/resume
bridge.

### Q6. Public re-exports — how do we keep `corvid::runtime::Foo` working?

Today external Corvid users see one crate `corvid-runtime` with
all the types. Splitting into core + host could break every
`use corvid_runtime::*` in user code if we're not careful.

**Proposal:** `corvid-runtime-host` re-exports everything from
`corvid-runtime-core` so `corvid_runtime::Foo` keeps working.
The split is invisible to native users. WASM users depend on
`corvid-runtime-core` directly via `corvid-browser`.

## Risks

**R1. Wrong boundary.** Drawing the core/host line at the wrong
place forces a re-split later. Mitigate by writing the proposal
above and stress-testing it against each Phase 21–41 feature
before code starts.

**R2. Hidden tokio.** `tokio::sync::Mutex`, `tokio::time::sleep`,
etc. can appear in code that "looks" pure but isn't. Build
`corvid-runtime-core` as a `wasm32-unknown-unknown` target from
slice 1 — it will refuse to compile if tokio creeps in.

**R3. Replay determinism.** The core's deterministic state machine
depends on the host providing responses in a stable order. The
suspend/resume bridge must serialize requests; parallel `await`
on multiple host calls is a determinism-breaking pattern. Probably
need an `AgentExecutor` invariant that pending host requests are
sequential.

**R4. Wire format coupling.** The `HostRequest` / `HostResponse`
enum is a wire format between core and host. Versioning rules
need to be specified (same shape as the `CheckResult` `version:
"v1"` field). Schema changes need coordinated rollout.

**R5. Tokio runtime startup in tests.** Many existing
`#[tokio::test]` tests assume a tokio runtime is implicit. After
the split, `corvid-runtime-core` tests run synchronously without
tokio; `corvid-runtime-host` tests still need it. Will require
test annotation cleanup.

## Acceptance criteria for closing this pre-phase chat

The chat closes when:

- [ ] Q1 (receipts) decided.
- [ ] Q2 (replay) decided.
- [ ] Q3 (corvid-vm split-or-port) decided based on the audit.
- [ ] Q4 (stdlib modules) decided.
- [ ] Q5 (connector modes) decided.
- [ ] Q6 (re-exports) decided.
- [ ] The decisions are recorded in this doc, dated, and signed
      by the CTO.
- [ ] Risks R1–R5 each have a mitigation that lands in the
      slice plan.

After the chat closes, 33J7b can start as a real code slice
with a stable design. Cost of the chat is ~half a session,
probably split across two: one to read this doc + ask sharper
questions, one to land the decisions.

## Where this fits

The chat happens in a separate session. This doc is the input.
Output is a "decisions" section appended at the bottom of this
file, listing each question + its resolution + a date.

The chat does not produce code. After it closes, slice 33J7b
opens as a multi-week code slice with its own commit cadence.

---

## Decisions (2026-05-12 — CTO delegated to Rust lead)

All six questions resolved. Recording here. CTO confirmation:
"choose the best ones for corvid please" + "even if it takes
20,000 years we need it working no shortcuts" + Path B locked.
The Rust lead exercised the delegation.

### D1 (resolves Q1, receipts) — Core emits canonical bytes; host signs

**Decision.** `corvid-runtime-core` produces canonical receipt
bytes (audit log entries, prompt records, tool calls, trace
metadata in a byte-stable order). `corvid-runtime-host` owns
DSSE signing and key-material access. The browser playground
can **verify** receipts (read DSSE signature, check ed25519
public key against an embedded or fetched key list) but never
**signs** them — no key material in the browser ever.

**Why best for Corvid.** The receipt contents are a public
contract; they must be byte-identical regardless of where the
signing happens. Keeping the canonical-bytes derivation in core
guarantees that. Keeping the signing in host respects the
trust boundary — keys belong to the host, full stop. The
browser-verify path is a small bonus: a user can paste a
signed trace into the playground and verify "yes this run is
authentic" without needing to install Corvid. That deepens the
"see how it works" experience without compromising the key-
material posture.

### D2 (resolves Q2, replay) — State machine in core; persistence behind traits

**Decision.** `corvid-runtime-core` owns the replay state
machine (`ReplayPlayer` / `ReplayRecorder`) — the deterministic
"we're on step N, the next expected event is X, here's the
replay-key derivation" logic. Persistence is exposed as two
traits in core (`ReplaySource` / `RecorderSink`). The host
crate provides the filesystem-backed implementations for
native; the browser implements them for IndexedDB.

**Why best for Corvid.** Replay is the project's flagship
invention. If the state machine lived separately in browser
vs. native, the moat would split — a trace recorded on native
might replay differently in browser. State machine in core is
the only honest call. Persistence is genuinely
context-dependent, so the trait abstraction is the right
boundary. This is the same pattern the project used for
`TraceEmitter` in Phase 21.

### D3 (resolves Q3, corvid-vm) — Split (not just port); same shape as runtime-split

**Decision.** `corvid-vm` is NOT a pure port. Audit found
**direct** wasm-blocking deps in `crates/corvid-vm/Cargo.toml`:
`corvid-runtime` (full grab-bag), direct `tokio`, plus
`async-recursion` and `async-trait`. The fix mirrors the
runtime split:

- `corvid-vm-core` (wasm-clean) — the IR-walking interpreter
  loop as a synchronous state machine. Yields `HostRequest` at
  every prompt / tool / external-host-call boundary; resumes
  on `HostResponse`. No tokio. No async fn in the hot path.
- `corvid-vm-host` (native-only) — native-side async dispatch
  via tokio, wrapping `corvid-vm-core` and providing the
  request-resolution loop. Same shape as `corvid-runtime-host`.

This is a structural refactor of corvid-vm too, not just a
`cfg(target_arch)` gate. The async-fn-in-VM pattern unwinds to
sync state machines. Estimate revised upward from "~2 weeks
port" to **~3 weeks split** for 33J7c.

**Why best for Corvid.** The suspend/resume primitive is the
right shape for browser execution (`wasm-bindgen-futures`
coroutines map to it 1:1) AND it cleans up the VM's existing
async-trait soup, which has been a recurring source of
async-recursion grief on the native side. The refactor pays
double dividends. Skipping the split and trying to compile
async-trait + tokio + corvid-runtime to wasm32 would either
not work or require so much `cfg` gating that the code
diverges between targets — and diverged code is what the no-
shortcut rule guards against.

### D4 (resolves Q4, stdlib impls) — Single `corvid-runtime-host` with per-module feature flags

**Decision.** All stdlib impls (`std.db`, `std.http`,
`std.jobs`, `std.auth`, `std.observability`, `std.connectors`)
live in `corvid-runtime-host` initially, gated by feature
flags (`db`, `http`, `jobs`, `auth`, `observability`,
`connectors`) so users can disable a module's native deps if
they don't use it. Per-module crate extraction is a follow-up
that lands if any specific module grows past the file-
responsibility threshold (~3,000 lines per CLAUDE.md's rubric)
or starts needing its own dep audit boundary.

**Why best for Corvid.** Two competing pressures here. (1) The
file-responsibility rule prefers tight crates; per-module
extraction is the principled answer. (2) Per-module
proliferation has its own costs (6 new crates × 6 Cargo.toml +
6 lib.rs + 6 test surfaces = ~30 new files to maintain). The
compromise is feature flags: same isolation for users (turn off
a module, lose its deps) without the crate-proliferation tax.
If a module's footprint or dep audit pressure grows, extract
it then — under user-driven evidence rather than upfront
guesswork. This is the principle-aligned-with-pragmatism move
Phase 35V's pattern 5 ("cross-component coupling discovered at
verification time") taught: extract when the boundary
discovers itself, not before.

### D5 (resolves Q5, connector modes) — Mock + replay in core, real in host

**Decision.** Mock and replay connector machinery lives in
`corvid-runtime-core` — they're deterministic and IO-free
(mock returns canned responses; replay reads from a recorded
trace). Real-mode connector adapters live in
`corvid-runtime-host`. The browser playground gets mock +
replay automatically; real-mode connectors return a
"sandboxed in browser — install locally to use real
connectors" diagnostic via the suspend/resume bridge that
33J7d ships.

**Why best for Corvid.** Same shape as D2 (replay): the
deterministic surface lives in core, the IO-touching surface
lives in host. A real bonus: the Phase 41L connector contract
drift test (mock ≡ replay ≡ real shared typed surface) becomes
a core-only test (mock ≡ replay) PLUS a host integration test
(real ≡ replay). Two tighter tests with clearer failure modes.
The cross-component coupling between mock/replay/real that
Phase 41 originally pinned now has a structural enforcement
mechanism — they're in the same crate, sharing types directly.

### D6 (resolves Q6, public re-exports) — Host re-exports core; `corvid_runtime::Foo` stays working

**Decision.** `corvid-runtime-host` re-exports every public
type from `corvid-runtime-core` so existing `use
corvid_runtime::*` paths keep resolving for native users. The
split is invisible to native CLI / REPL / test consumers.
Browser code depends on `corvid-runtime-core` directly via
`corvid-browser`.

**Why best for Corvid.** No-shortcut answer for SemVer
stability. Breaking every existing CLI/REPL/example/test user
on a behind-the-scenes refactor would violate the project's
stability contract (`docs/security/stability-contract.md`).
Re-exports cost ~one line per public type and produce zero
divergence between what native users see today and what they
see post-split. The pattern is well-trodden: Tokio uses it
extensively (`tokio::sync::*` re-exports from internal
crates), Serde uses it (`serde::*` re-exports from
`serde_core` for some types), no surprises.

## Risk mitigations (recorded)

Each of the five identified risks gets a mitigation that lands
in the 33J7b slice plan:

- **R1 Wrong boundary.** Stress-test the proposed split against
  every Phase 21–41 feature in a checklist before writing code.
  Drafts as a `33J7b-pre-code-stress-test.md` doc; each Phase
  20–41 feature is listed with "where does it land — core or
  host? what does the test of this need?" If any feature
  resists clean placement, revisit the boundary.
- **R2 Hidden tokio.** Build `corvid-runtime-core` as a
  `wasm32-unknown-unknown` target from slice 1 of 33J7b's
  decomposition. Failed builds are the early-warning system. CI
  enforces the wasm32 build on every push to the core crate.
- **R3 Replay determinism.** The `AgentExecutor` in core
  enforces an invariant: pending `HostRequest`s are sequential.
  Parallel-await on multiple host calls is disallowed and
  caught at compile time via the suspend/resume API shape
  (the executor yields one request, then awaits one response,
  then continues — never yields multiple in flight).
- **R4 Wire format coupling.** `HostRequest` and `HostResponse`
  carry a `version: "v1"` field at the root, matching the
  `CheckResult` schema. Schema changes follow the same protocol
  documented in `crates/corvid-browser/README.md`'s "Schema-
  change protocol" section.
- **R5 Tokio in tests.** `corvid-runtime-core` tests are pure
  `#[test]` (synchronous). `corvid-runtime-host` tests stay
  `#[tokio::test]` where they need the runtime. The 33J7b
  refactor includes a per-test-fn audit to move each test to
  the right crate.

## Closing the chat

All six questions decided. Pre-phase chat closed.
33J7b is now an open code slice with stable design. Estimated
duration: ~3-4 weeks (was ~3-4 weeks before chat; unchanged
since the design held up). Estimated 33J7c duration revised
from ~2 weeks port to **~3 weeks split** (Q3 finding).

Updated 33J7 path total: ~13-15 weeks Rust (was ~12-14).

---

## Reference

- The current `corvid-runtime` crate:
  <https://github.com/Micrurus-Ai/Corvid-lang/tree/main/crates/corvid-runtime>
- The Phase 35V Track 1 pattern of orthogonal sentinels: relevant
  for verifying the split — we'll want forward (every public type
  resolves) + inverse-broad (no tokio symbol resolves in core) +
  inverse-narrow (every native-capability boundary goes through
  HostRequest).
- The probe that surfaced this slice:
  `cargo tree -p corvid-runtime | grep -iE 'tokio|hyper|postgres|reqwest|opentelemetry|libloading' | wc -l` →
  100+ matches.
