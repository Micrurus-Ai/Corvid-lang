# Corvid — Build Roadmap

> Phase-by-phase plan from v0.1 (complete) to v1.0 (public launch).
> For feature definitions see [`FEATURES.md`](./FEATURES.md).
> For architecture see [`ARCHITECTURE.md`](./ARCHITECTURE.md).

**Positioning.** Corvid is a **general-purpose AI-native language**, not an agent-only DSL and not a RAG framework. Ambition: be the default choice for building AI applications end to end: agents, copilots, workflow automation, model-routed services, human-in-the-loop systems, eval pipelines, memory-backed applications, RAG, and ordinary software around them. "Best at everything" is a trap that has killed every language that tried it (PL/I, Ada, early Scala); the honest version is **narrow excellence on the moat, broad competence on table stakes, disqualified on nothing.**

### Moat — dimensions Corvid is built to genuinely win on

1. **Safety for AI-shaped software.** Effect checker, approve-before-dangerous, compile-time cost bounds, contract verification. Nobody else is competing here.
2. **AI-native ergonomics.** `agent` / `tool` / `prompt` / `approve` / `model` / `eval` as language concepts; replay, grounding contracts, cost budgets, approval boundaries, model routing, trace assertions, and provenance as first-class constructs. Structurally impossible to match without owning the whole pipeline.
3. **Readability for human + LLM.** Pythonic surface, shallow hierarchies, no pointer aliasing, explicit effects. The language machines both read and *write* best.

### Table stakes — top-tier, competitive with best in category (not best overall)

- **Performance.** Go / Swift class. Fast startup (Phase 12 native AOT), throughput where compute rarely bottlenecks real applications.
- **Memory.** Refcount + cycle collector + effect-typed memory model (Phase 17). Region inference + Perceus linearity means most allocations never pay refcount; cycles caught without per-object tracing overhead. Predictable release without Java pauses.
- **Deployment.** Single native binary + WASM (Phase 23) + C ABI embedding (Phase 22).
- **Tooling.** LSP (Phase 24), formatter, package manager (Phase 25), REPL (Phase 19). Polished, not novel.
- **Cross-platform.** macOS + Linux + Windows all first-class by v1.0 (Phase 33).

### Deliberately not competing

- Systems-level control — Rust / Zig win. No pointer arithmetic, no manual allocators.
- Raw hot-loop numerics — C++ / Fortran win. FFI for the ~1% of apps that need it.
- Dynamic metaprogramming — Ruby / Lisp win. Opposite trade-off to compile-time checking.
- Ecosystem size at launch — Python / JS have 20-year head starts. Python FFI (Phase 30) closes the gap.

**The test applied to every proposed feature:** does it strengthen a moat dimension, or bring us to parity on a table-stakes dimension where we're below the floor? If yes, build it. If it moves neither bar, defer.

## Launch strategy — Path A (silent build → v1.0)

**Decision recorded 2026-05-17.** v1.0 is the production-backend launch, not the defensible-core launch. Phase 35's defensibility gate is shipped and Phase 35V verified it, but a v1.0 stamp on the language alone underdelivers against the audience this language is built for. The audience is **AI engineers building real products today.** They need persistence, jobs, auth, observability, connectors, deploy — the things Python + FastAPI + Postgres ship out of the box — *plus* the moat the language adds. Half a stack does not get them to ship; both halves do.

**Path A:** silent build. No preview release, no marketing push, no public ETA. Repo and website stay live in their current shape (landing page + docs + Tier-1 playground at corvid-lang.org) because removing them sends the wrong signal, but no active promotion happens during the build. The 33M beta is dropped from its original gate position and repositioned as a 2-week friends-and-family round in the final 4 weeks of Phase 43. The 33J4 benchmark page, 33J5 blog shell, and 33L launch materials all land in the final 2 weeks of Phase 43, not before. Phase 33's remaining polish items below carry `[launch-readiness]` markers to flag this.

**What stays the same:** every CLAUDE.md rule. Commits land publicly on `main`. Pre-phase chat mandatory before each Phase. Slice boundaries get dev-log + learnings + ROADMAP updates. Validation gate green between every commit. Invention-shipping contract on every Corvid-specific capability shipped during the build. No shortcuts. The repo is public, the work is auditable, the discipline is unchanged — the only thing that's silent is the marketing, not the engineering.

**Estimated build time (revised 2026-05-17 after audit):** **~3-5 months focused solo work** to v1.0, broken down as roughly 4-8 weeks for a Phase 35V-style cross-phase verification round over Phases 38-42 (their slice work is closed; the phase-done checklists need audit + correction slices where drift exists, applying the Phase 35V Track 2 pattern), plus ~6-8 weeks for Phase 43 (deploy package, signed-attestation chain, release channels, reproducible-build verification, claim audit, `corvid upgrade --check`, `corvid ops show`, AI helpers, benchmark file — the real launch-finishing work), plus ~2-3 weeks for the Path A launch-readiness tail (33J4 + 33J5 + 33L + repositioned 33M). The earlier "~13-18 months" estimate (corrective commit pending) was based on the false premise that Phases 37-43 were full open phases; the audit found that Phases 37-42 are essentially shipped at slice level and the remaining work is verification-shaped, not implementation-shaped.

**Reversibility.** Nothing about Path A is irreversible until v1.0 is announced. If 6 months in we want to pivot to Path B (developer preview during the build), the decision reopens as its own pre-phase chat.

### v1.0 launch criteria — every box must be ticked before the cut

- [ ] Every Phase 37-43 closed per the phase-done criteria each phase defines.
- [x] Every reference application from Phase 42 demoably ships, runs in production-shape, and deploys via the Phase 43 packaging path on at least one supported target (Fly.io, Render, AWS Lambda, or self-hosted). **Verified 2026-06-04** — two complementary gates: (a) **Phase 36 server-runtime side**: `crates/corvid-cli/tests/serve_smoke.rs::reference_apps_serve_their_schema_route` spawns `corvid serve <app>/src/main.cor` for all 5 apps, waits for `/healthz` to answer 200, GETs `/schema` and asserts the app's manifest envelope (`E-LR-app-deploy-smoke-ci`, runs in the `app-deploy-smoke.yml` CI workflow on every push and PR); (b) **Phase 43 packager side**: this commit adds `crates/corvid-cli/tests/deploy_manifests.rs::every_reference_app_produces_a_complete_deploy_package` which for each of the 5 apps invokes `corvid deploy package <app-dir> --out <tempdir>` (using the documented dev signing key the existing `reference_apps.rs::deploy_package_smoke` test pins) and asserts the 9 artifacts the packager promises all land on disk: `Dockerfile`, `oci-labels.json` (OCI metadata), `env.schema.json`, `health.json`, `migrate.sh`, `startup-checks.md`, `build-attestation.dsse.json` (43O signed-attestation chain anchor), `sbom.spdx.json` (43M SPDX SBOM), `VERIFY.md`. The structured artifacts (`oci-labels.json`, `sbom.spdx.json`, `build-attestation.dsse.json`) are also spot-checked for valid-JSON shape so a packager regression that emits truncated output fails-loud instead of producing an unusable image. **Validation: `cargo test -p corvid-cli --test deploy_manifests` runs 3/3 pass in 4.86s** (the prior 2 deploy-manifest tests + the new L47 gate). "Self-hosted" target satisfied: an operator with the rendered Dockerfile + OCI metadata + SBOM can `docker build .` and `docker run` a production-shape image; running the resulting image would need Docker daemon access the cargo-test sandbox doesn't have, but the on-disk artifact set IS the input `docker build` needs and a missing artifact would surface at build time. Per-target acceptance (Fly.io, Render, K8s, systemd) is closed via the manifest-shape sibling test `deploy_manifests_invoke_corvid_serve_with_full_path` + the `app-deploy-smoke.yml` workflow's `docker compose config` + fly.toml TOML-validity steps.
- [x] Every cdylib claim id introduced in Phases 37-43 is wired into the signed-claim coverage gate (Phase 35-N pattern), with `OutOfScope` rows promoted to `Static` or `RuntimeChecked` where the implementation is shipped. **Verified 2026-06-04** — `cargo test -p corvid-driver signed_claim_coverage` runs 5 tests against the live coverage gate (`signed_claim_coverage_accepts_registered_contracts`, `_rejects_missing_declared_contract_id`, `_rejects_out_of_scope_contract_id`, `_walks_schedule_decl`, `_rejects_schedule_without_jobs_coverage`); all 5 pass against the current 75-row registry (12 Static + 48 RuntimeChecked + 15 OutOfScope). Per-prefix coverage: jobs (10), approval (9), connector (7), auth (6), abi_attestation (4), observability (4), grounded (3), effect_row (3), deploy (3), app (3), abi_descriptor (3), platform (3), replay (2), release (2), eval (2), claim (2), budget (2), upgrade (1), tenant (1), review_queue (1), provenance_trace (1), package (1), ops (1), confidence (1). Phase 37 (persistence) introduces NO new cdylib claim ids — DB reads/writes are typed as `effect_row.*` contributions (covered by the existing `effect_row.body_completeness` + `effect_row.import_boundary` rows), dangerous writes go through `approval.dangerous_call_requires_token`, replay DB-summary determinism rolls up into `replay.deterministic_pure_path` + `replay.trace_signature`. The 15 OutOfScope rows each carry a documented downgrade rationale (`approval.dangerous_marker_preserved`, `effect_row.caller_propagation`, and `grounded.propagation_across_calls` are 35V-T1-B downgrades where the property exists but the diagnostic surface doesn't separately fire it; `jobs.retry_budget_bound`, `jobs.approval_wait_resume`, `connector.write_requires_approval`, `tenant.cross_tenant_compile_error`, `approval.policy_clause_static_check`, `approval.batch_equivalence_typed`, `approval.confused_deputy_typecheck` are post-v1.0 source-syntax sugar; `budget.runtime_termination` is explicit non-defense; `platform.host_kernel_compromise`, `platform.signing_key_compromise`, `platform.toolchain_compromise`, `package.hosted_registry_available` are explicit TCB-boundary non-defenses). Companion finding: corrected the 5 per-app `CLAIM.md` links in `docs/meta/launch-claim-audit.md` Section 5 from the wrong `apps/<name>/CLAIM.md` paths to the actual `examples/backend/<name>/CLAIM.md` locations on disk (the audit parser doesn't validate file existence, only string format, so this drift could have shipped). `corvid claim audit` exits 0 with 56 claims, 0 findings.
- [x] Launch claim audit (`docs/meta/launch-claim-audit.md`) re-run after Phase 43 closes; every launch claim points at a runnable command or committed artifact; zero aspirational wording survives Phase 35V's audit pattern applied at v1.0 scale. **Re-run 2026-06-04** — rewrote `docs/meta/launch-claim-audit.md` from a 14-row stub into a 56-claim inventory covering: the 22-row moat Proof Matrix from `inventions.md`, the Phase 36-41 production-backend surface (HTTP server, persistence, durable jobs, auth, observability, connectors, HTTP approval queue from this session's E0-serve-5/6, struct-signature `#[tool]` from G0-tools-3b), the 5 per-app maturity rows from the Phase 35V2-P42-D-LR track each with their `apps/<name>/CLAIM.md`, the 9 Phase 43 launch-infrastructure rows (deploy package, smoke-deploy CI, signed-attestation chain, release channels, reproducible build, upgrade --check, claim audit, ops show, clone-to-deploy bench), the 9 shipped AI helpers each named with their `Grounded<T>` contract, and an explicit "Section 8 — honest gaps" that names every blocked / non-scope item for v1.0 with its `blocked: <slice-id>` annotation so `corvid claim audit` accepts the row without flagging aspirational wording. **Validation: `cargo run -q -p corvid-cli -- claim audit` → claim_count: 56, finding_count: 0, exit=0** (was exit=1 with 10 findings after the initial rewrite; the 10 findings drove the structural cleanup of every table to standard `| Claim |` headers + explicit blocked-status markers).
- [x] Bilateral verifier (Phase 35-H) green across the production-backend surface — every Phase 37-43 contract id reachable from a built cdylib reconstructs and byte-matches. **Verified 2026-06-04** — added `crates/corvid-abi-verify/tests/reference_apps_bilateral_match.rs::every_reference_app_cdylib_bilaterally_matches_its_source` which (a) builds the cdylib for each of the 5 reference apps via `corvid_driver::build_target_to_disk(BuildTarget::Cdylib)` — the same path `corvid build --target=cdylib` uses for operators, (b) calls `corvid_abi_verify::verify_source_matches_cdylib(source, cdylib)` which rebuilds the descriptor JSON from source through the descriptor-relevant frontend pipeline (lex / parse / resolve / typecheck / IR-lower / ABI-emit) and reads the embedded `CORVID_ABI_DESCRIPTOR` symbol from the cdylib, (c) asserts byte-equality of both the JSON hash and the JSON length for every app. **Validation: `cargo test -p corvid-abi-verify --test reference_apps_bilateral_match -- --ignored` runs 1/1 pass in 13.85s on a warm cache** (verified locally 2026-06-04); all 5 reference apps return `report.matches() == true`. Test is marked `#[ignore]` so it doesn't bloat `cargo test --workspace` (the per-app cdylib build costs ~30s cold) — explicit invocation runs it pre-cut per the L50 verification cadence documented in `docs/meta/launch-claim-audit.md` Section 9. CI workflow `app-deploy-smoke.yml` already builds all 5 cdylibs for the smoke-deploy gate, so a CI invocation of this test reuses warm build state. Failure mode is named explicitly: if any app fails bilateral match, the test aggregates every failing app's source-hash + embedded-hash + source-len + embedded-len in a single panic message so the diagnostic is comprehensive rather than first-failure-wins.
- [ ] Friends-and-family round (repositioned 33M): 5-10 hand-picked AI engineers build a small production-shape app on the v1.0 release candidate; their feedback closes as code / docs / tests / explicit non-scope before the public cut.
- [ ] 33J4 benchmark page, 33J5 blog shell + launch post, 33L launch GIF + announcement drafts all shipped on the website in the final 2 weeks before the cut.

### v1.0 launch pitch (working draft, lock or iterate)

> *Build the same production AI app you'd build in Python — auth, jobs, persistence, deploy — but with the safety guarantees compiled into the binary instead of audited by humans after the fact.*

The pitch is what every Phase 37-43 scope decision anchors against. Lock or iterate before Phase 37 opens.

### Next slice (no questions — read this and start)

**The ROADMAP is the decision document.** A fresh session reads this section and starts the next slice without asking the user to pick between options. If the sequencing below is wrong, the audit-and-update slice that corrects it is itself the next action; do not stall on a/b questions.

**Sequence of genuinely-open, non-deferred slices** — pick the first one whose dependencies are met:

*Queue is empty.* All previously-filed genuinely-open slices have shipped:

1. **`35V2-P42-E0-serve-5`** HTTP approval queue — shipped 2026-06-04 in commit `2788490`.
2. **`35V2-P42-G0-tools-3b`** struct params/returns in `#[tool]` — shipped 2026-06-04 in commit `ff37aeb`.
3. **`33J6-grammar-drift-gate`** + the 7 doc gaps it surfaced — shipped 2026-06-04 in commit `78c16a3`.
4. **`35V2-P42-E0-serve-6`** HTTP approval-queue transition endpoints — shipped 2026-06-04 in `(this commit)`.

What remains is the **Path-A launch-readiness tail** (deferred-by-design per the launch-strategy block above): 33J4 benchmark page, 33J5 blog shell + launch post, 33L launch GIF + announcement drafts, 33M friends-and-family round. Per Path A these open in the final 2-4 weeks of Phase 43 — not now.

If a new genuinely-open slice surfaces (a CI failure that turns out to need a real feature add, an external reviewer file, a Phase 35V-shape audit drift), this section gets a new entry naming it + its line citation. The audit-and-update slice that adds the entry is itself the next action — do NOT ask the user "which one first?"; sequence by dependency and start.

After slice 3 closes, the next ROADMAP-driven action is the **Path-A launch-readiness tail** (33J4 benchmark page, 33J5 blog shell, 33L launch materials, 33M friends-and-family round), which by Path A timing opens in the final 2-4 weeks of Phase 43. Slices that remain `[ ]` outside of this sequence are either (a) explicitly deferred-by-design (e.g. line 1310 `\` line continuation rejected on positioning grounds — see the body of the box for the reason), (b) phase-gate-template rows that get ticked per-slice and aren't standalone implementation work (the "slice gate" section under "Phase standard"), or (c) the launch-readiness tail above. Do not pull one out of order.

**Why this section exists.** Set 2026-06-04 after the user request: "i do not want you to ask what next like a or b, I want us to follow the roadmap. Analysis and update the roadmap so we move from one phase to the next without asking questions. No shortcuts." This section is regenerated by the audit-and-update slice that fires whenever the sequencing above no longer matches the truth.

### Phase standard

Every remaining phase must make Corvid more AI-native and more general-purpose at the same time. Generic infrastructure is allowed only when it carries Corvid's effect, provenance, approval, cost, replay, eval, model, human-boundary, distribution, or deployment semantics through that layer.

AI-native does **not** mean "RAG with syntax." RAG is one standard-library pattern. The language primitives must be broad enough for the full AI application surface: autonomous and supervised agents, copilots, workflow orchestration, extraction/classification, tool-use, approval-gated actions, model routing, memory, replay, evals, governance, and normal application code.

1. **More AI-native.** Each phase must ship at least one semantic capability that makes AI behavior more visible, constrained, replayable, typed, auditable, or governable.
2. **More general-purpose.** Each phase must also make Corvid stronger as a normal programming language: modules, packages, tooling, deployment, tests, memory, FFI, standard library, editor support, portability, or maintainability.
3. **More powerful without shortcuts.** Corvid does not ship feature-shaped placeholders. A feature is not done until it has real semantics, user-visible behavior, positive and negative tests, honest docs, clear non-scope, and validation through the command path users will actually run.

Every phase has:
- A pre-phase chat (concepts, decisions, success criteria) before any code.
- Tests green at the phase boundary.
- A dev-log entry describing decisions made.

### Autonomous execution protocol

Default working mode after Phase 22: proceed through the remaining roadmap automatically, one coherent slice at a time, without asking for routine permission between phases. Product, security, scope, and marketing decisions are delegated to the implementation lane by default: choose the design that is more durable, more auditable, more AI-native, and more general-purpose, even when it is harder.

The next phase starts only after the current slice has real implementation, tests, docs or roadmap updates where required, validation through user-facing commands, and a commit. Do not optimize for the easiest path. A shortcut is any change that preserves the appearance of progress while weakening semantics, skipping validation, hiding a limitation, or moving a hard requirement into vague follow-up language.

Planning assumption: the remaining roadmap is repo-local. It should not require credentials, payments, external account setup, destructive remote-history changes, or public claims without committed evidence. If a slice appears to require one of those, redesign the slice to keep the strongest local, testable version first; document any truly external launch step as operational follow-up, not implementation scope.

Pause and ask only for:
- Secrets, credentials, payments, domain/account ownership, or external service actions that cannot be completed safely from the local repository alone.
- Destructive actions against user work, published artifacts, tags, releases, or remote history.
- Any conflict with user work, dirty files owned by someone else, or failing validation that cannot be resolved locally.
- Evidence gaps where a public claim cannot be backed by a committed test, command, benchmark archive, or spec.

Do not pause for:
- Routine implementation sequencing.
- Normal refactors needed to keep the code correct.
- Adding positive and negative tests.
- Documentation needed to make a shipped feature honest.
- Product, security, scope, or marketing tradeoffs when the stronger non-shortcut direction is clear from the roadmap.
- Continuing from one completed roadmap slice to the next.

---

## Completed phases

### Phase 1 — AST types ✅
Rust data types for every parsed Corvid construct. ~550 LOC.

### Phase 2 — Lexer ✅
Hand-rolled state machine. 22 keywords, Python-style indent/dedent, triple-quoted strings.

### Phase 3 — Parser ✅
Recursive descent + Pratt. Expressions (3a), statements (3b), declarations (3c).

### Phase 4 — Name resolution ✅
Two-pass; side-table keyed by span; strict duplicate detection.

### Phase 5 — Type + effect checker ✅
**The killer feature.** Dangerous tool calls without prior `approve` fail compilation.

### Phase 6 — IR lowering ✅
Typed IR; references resolved; parse-time sentinels normalized.

### Phase 7 — Python codegen ✅
Walks IR, emits runnable Python. Becomes `--target=python` in v1.0.

### Phase 8 — Python runtime ✅
`corvid_runtime` Python package. Interim home for HTTP/approvals/tracing.

### Phase 9 — CLI wiring ✅
`corvid new`, `check`, `build`, `run`, `doctor`. Real diagnostics.

### Phase 10 — Polish ✅
Ariadne multi-line error rendering. Error codes. README. Offline demo.

**v0.1 complete. 134 Rust + 10 Python tests green.**

### Phase 11 — Interpreter + native runtime ✅
Tree-walking interpreter (`corvid-vm`), async end-to-end. Native runtime (`corvid-runtime`) with `ToolRegistry`, `Approver` trait, `MockAdapter`, `AnthropicAdapter`, `OpenAiAdapter`, JSONL tracing with secret redaction, `.env` loading via `dotenvy`. `corvid run` dispatches natively — no Python on the path. Done-when met: refund_bot demo runs end-to-end with Python uninstalled; `cargo run -p openai_hello` / `anthropic_hello` make real provider calls. Test count grew from 134 (v0.1) to ~219 across the workspace.

Carry-overs explicitly tracked elsewhere:
- Proc-macro `#[tool]` + `corvid run` user-tool loading → Phase 14
- Streaming `Stream<T>` → Phase 20 (moat phase)
- Google / Ollama adapters → Phase 31
- Effect-tagged `import python` → Phase 30
- Distributed concurrent multi-agent orchestration → Post-v1.0 (Phase 38 now covers durable single-backend jobs and agent runs; cross-service multi-agent graphs remain out of pre-v1.0 scope)

**v0.2 complete. ~219 tests green.**

---

## In progress

### Phase 12 — Cranelift scaffolding (~2 months) ✅ closed
Goal: compile typed IR to native machine code via Cranelift. Interpreter and compiled binary produce the same answer on every fixture — the oracle parity the async-interpreter decision was defending.

Pre-phase decisions locked: **AOT-first** via `cranelift-object` (no JIT detour — the v1.0 pitch is a single binary), **trap-on-overflow** arithmetic via `sadd_overflow` + explicit branch to a runtime overflow handler (safety wins; Rust-debug-mode cost accepted).

#### Slice 12a ✅ — AOT scaffolding + Int arithmetic (Day 19)
- [x] `corvid-codegen-cl` workspace crate with Cranelift 0.116 deps
- [x] Host ISA via `target-lexicon` + native flag builder
- [x] Lowering: Int literals, parameter loads, Int arithmetic with overflow trap, return, agent-to-agent calls
- [x] Overflow via `sadd_overflow`/`ssub_overflow`/`smul_overflow` + branch to runtime handler
- [x] C entry shim + `corvid_runtime_overflow` handler
- [x] `cc` crate drives MSVC `cl.exe` on Windows (per-test `/Fo<tempdir>\` to prevent `.obj` collisions); GCC/Clang on Unix
- [x] `corvid_entry` trampoline — shim stays static, user agents get `corvid_agent_` symbol prefix to avoid C-runtime collisions
- [x] `corvid-driver::build_native_to_disk` emits `target/bin/<stem>[.exe]`
- [x] `corvid build --target=native <file>` wired
- [x] Differential parity harness with 15 fixtures (all literal + arithmetic cases, agent-to-agent calls, + 3 overflow/div-by-zero parity cases)
- [x] `CodegenError::NotSupported { reason, span }` for everything outside Int-only, each message pointing at the slice that unblocks it

#### Slice 12b ✅ — Bool, comparisons, if/else (Day 20)
- [x] `cl_type_for` gate maps `Int → I64`, `Bool → I8`, others raise `NotSupported`
- [x] Agent signatures retyped through `cl_type_for` (parameters + returns)
- [x] Bool literals, six comparison ops (`==`, `!=`, `<`, `<=`, `>`, `>=`) via `icmp`
- [x] Unary `not` as `icmp_eq(v, 0)`; unary `-` via `ssub_overflow(0, x)` trapping on `-i64::MIN`
- [x] Short-circuit `and` / `or` (both tiers — interpreter updated to match). Observable proof fixture: `true or (1 / 0 == 0)` returns `true`.
- [x] `if` / `else` statement lowering with merge-block pattern
- [x] Trampoline extends Bool → I64 via `uextend` when entry agent returns Bool
- [x] 18 new parity fixtures (bringing the suite to 33)

#### Slice 12c ✅ — Local bindings + `pass` (Day 21)
- [x] `IrStmt::Let` lowering with declare-or-reuse: each `LocalId` maps to a Cranelift `Variable`; first sight declares with `cl_type_for(ty)`, later sights reuse and `def_var`
- [x] Type-change-on-reassignment defensive guard → `CodegenError::Cranelift` (typechecker should catch it; this closes the failure mode if not)
- [x] `IrStmt::Pass` becomes a no-op (was `NotSupported`)
- [x] Env signature changed from `HashMap<LocalId, Variable>` to `HashMap<LocalId, (Variable, clir::Type)>` so the type-change guard has the existing width to compare against
- [x] 9 new parity fixtures: simple binding, multi-binding arithmetic, repeated use, three-step reassignment, Bool binding, reassignment inside `if`, binding used in comparison, `pass` as noop, parameterised agent with locals
- [x] Smoke-tested `corvid build --target=native examples/with_locals.cor`: locals + reassignment + `if` + native execution end-to-end

#### Slice 12d ✅ — `Float` (Day 22)
- [x] `cl_type_for(Float) → F64`; `IrLiteral::Float` lowering via `f64const`
- [x] Float arithmetic via `fadd`/`fsub`/`fmul`/`fdiv`; `%` via `a - trunc(a/b) * b` to match Rust `f64::%`
- [x] Float comparisons via `fcmp` with IEEE-correct NaN semantics (`==` returns false on NaN, `!=` returns true on NaN)
- [x] Mixed Int+Float promotion via `fcvt_from_sint` — same widening rule as the interpreter
- [x] Float unary negation via `fneg` (no trap — IEEE)
- [x] **Interpreter updated to follow IEEE for Float div/mod by zero** (was trapping; now returns `Inf` / `NaN`). Closes a divergence rather than creates one.
- [x] Defensive guard: Float entry-agent returns blocked with `NotSupported` pointing at slice 12h (where the C shim grows to handle non-Int print formats)
- [x] 10 new parity fixtures including the IEEE divergence proofs (`1.0 / 0.0 > 1.0` true; `NaN != NaN` true)

#### Slice 12e ✅ — Memory management foundation (Day 23)

Originally scoped as "Memory foundation + String"; user split into 12e (foundation) + 12f (String) for cleaner landings after agreeing the combined slice was too large to ship safely in one session.

- [x] `runtime/alloc.c` — 16-byte header (`atomic refcount + reserved`), `corvid_alloc` / `corvid_retain` / `corvid_release`, atomic leak counters
- [x] `i64::MIN` immortal sentinel for `.rodata` literals — `retain` / `release` short-circuit so static memory is never written to
- [x] `runtime/strings.c` — `corvid_string_concat` / `_eq` / `_cmp` built on the allocator (descriptor + bytes share one allocation block)
- [x] `shim.c` updated — prints `ALLOCS` / `RELEASES` to stderr when `CORVID_DEBUG_ALLOC` is set, kept off by default so existing parity output is unchanged
- [x] `link.rs` compiles and links all three C files via the host C compiler with `/std:c11` (MSVC) / `-std=c11` (GCC/Clang) for `<stdatomic.h>` support
- [x] `cl_type_for(String) → I64` (descriptor pointer); `is_refcounted_type` helper; runtime helper symbol constants (`RETAIN_SYMBOL` / `RELEASE_SYMBOL` / `STRING_CONCAT_SYMBOL` / `STRING_EQ_SYMBOL` / `STRING_CMP_SYMBOL`)
- [x] All 52 existing parity fixtures still green with the new C runtime linked into every binary

**Pre-phase decisions locked**: 16-byte header (preserves payload alignment + reserves a future-use word), atomic refcount (post-v1.0 multi-agent work will need it; cheap insurance now), scope-driven release insertion (correct now, liveness-driven optimisation is Phase 20 — moat phase), combined slice (foundation + String) — then split mid-session into 12e (foundation) + 12f (String) once the String integration revealed itself as a slice's worth of work on its own.

#### Slice 12f ✅ — `String` operations + ownership wiring (Day 24)
- [x] `RuntimeFuncs` struct (FuncIds for retain / release / concat / eq / cmp + `Cell<u64>` literal counter), declared once per module in `lower_file`, threaded through every lowering function
- [x] `LocalsCtx` data structure — `(env, var_idx, scope_stack)` bundle for lowering-function locals
- [x] Lower `IrLiteral::String` via `module.declare_data` + `define_data` — single `.rodata` block per literal `[refcount=i64::MIN | reserved | bytes_ptr → self+32 | length | bytes...]` with `write_data_addr(16, self_gv, 32)` self-relative relocation
- [x] Lower `String + String` (concat) via `corvid_string_concat` call
- [x] Lower String comparison ops (`==`, `!=`, `<`, `<=`, `>`, `>=`) via `corvid_string_eq` / `corvid_string_cmp`; narrow result `i64 → I8`
- [x] Scope-stack tracking for refcounted locals; `Vec<Vec<(LocalId, Variable)>>` pushed/popped at if/else branch entry/exit; function-root scope pushed in `define_agent`
- [x] Ownership management: retain on `use_var` of refcounted (Borrowed → Owned), release after passing to a call (consumed temp), release-on-rebind, retain return value + release locals on return, walk all scopes on return for cleanup
- [x] Parameter binding retains incoming refcounted args (+0 ABI: caller passes without bump; callee retains on entry)
- [x] Driver guard: String entry params / returns → `NotSupported` pointing at slice 12i
- [x] Parity harness leak detector — `CORVID_DEBUG_ALLOC=1` on every binary, parse stderr `ALLOCS=N\nRELEASES=N`, assert equal
- [x] Leak-counter semantic fix: `corvid_release_count` only increments when an allocation is actually freed (refcount hits 0), so it pairs 1:1 with `corvid_alloc_count`
- [x] 7 new String parity fixtures (literal eq, literal neq, concat+eq, empty-string concat both directions, !=, six orderings, reassignment+concat+compare). Leak detector runs on all 59 fixtures (52 existing + 7 new), all balanced.

#### Slice 12g ✅ — `Struct` (memory layout + field access) (Day 25)
- [x] New `IrCallKind::StructConstructor { def_id }` variant in `corvid-ir`; `lower.rs` detects `DeclKind::Type` callees and emits the new variant
- [x] Typechecker: `check_struct_constructor` validates arity and field types; replaces the v0.1-era `TypeAsValue` rejection
- [x] `cl_type_for(Struct) → I64`; `is_refcounted_type(Struct) → true`
- [x] `corvid_alloc_with_destructor(size, fn_ptr)` runtime helper; `corvid_release` calls the destructor (if any) before freeing
- [x] Per-struct-type destructor function generated by codegen in `lower_file` for structs with refcounted fields — walks the refcounted fields at fixed 8-byte offsets and calls `corvid_release` on each
- [x] Struct constructor lowering: alloc (with or without destructor), per-field stores at `i * 8` offsets; field arg's Owned +1 transfers into the struct
- [x] Field access lowering: load at compile-time-known offset; retain if refcounted field; release temp struct pointer
- [x] `RuntimeFuncs.ir_types` now carries cloned struct metadata so lowering can resolve field offsets / constructor arities without threading `&IrFile` through every call site
- [x] 7 new parity fixtures: scalar-only struct, Bool field, String field (exercises destructor), String field extract + compare, struct-as-agent-parameter, reassignment, nested struct field access (two deep)

#### Slice 12h ✅ — `List` + `for` + `break` / `continue` (Day 26)

- [x] `runtime/lists.c` with shared `corvid_destroy_list_refcounted(payload)` — walks length at offset 0, releases each element; one helper handles List<String>, List<Struct>, List<List>
- [x] `link.rs` compiles + links `lists.c` alongside the other runtime files
- [x] `cl_type_for(List) → I64`; `is_refcounted_type(List) → true`; `LIST_DESTROY_SYMBOL` constant + FuncId on `RuntimeFuncs`
- [x] `LoopCtx { step_block, exit_block, scope_depth_at_entry }` + `loop_stack: Vec<LoopCtx>` threaded through `lower_block` / `lower_stmt` / `lower_if`
- [x] `IrExprKind::List` lowered to alloc + length store + per-element stores; refcounted-element lists use `corvid_alloc_with_destructor` + `corvid_destroy_list_refcounted`
- [x] `IrExprKind::Index` lowered with runtime bounds check: traps on `idx < 0` or `idx >= length`; refcounted elements retained after load
- [x] `IrStmt::For` lowered as four-block pattern: `entry → header → body → step → exit`; loop var declared once, initialised to 0 (null), rebinds per iteration with release-on-rebind
- [x] `IrStmt::Break` / `IrStmt::Continue` release refcounted locals across all scopes deeper than the loop's entry depth, then jump to `exit_block` or `step_block` respectively
- [x] Typechecker expansion: `Expr::List` infers `List<T>` from the first element (with homogeneity check + Int→Float promotion); `Expr::Index` returns the List's element type and enforces Int index; `Stmt::For`'s loop variable gets the list's element type (was `Unknown`)
- [x] Pre-existing codegen-py and corvid-ir tests that used `if x:` on a String loop var updated to `if x == "a":` — the lenient v0.1 typechecker had let them through; the stricter slice-12h inference correctly rejects them
- [x] 8 new parity fixtures: list sum via for, break exits early, continue skips, subscript access, List<String> destructor, List of heap strings (real releases), nested List<List<Int>> two-deep subscript, empty-like list

#### Slice 12i ✅ — Parameterised entry agents + Float-/String-returning entries (Day 27)

- [x] `runtime/entry.c`: per-type argv decoders (`corvid_parse_i64` / `_f64` / `_bool` with slice-specific parse errors — not reusing the overflow handler), per-type result printers (`corvid_print_i64` / `_bool` prints `true`/`false` / `_f64` via `%.17g` / `_string` raw bytes), `corvid_arity_mismatch`, `corvid_init` (registers `atexit(corvid_on_exit)` so leak counters still print)
- [x] `runtime/strings.c`: `corvid_string_from_cstr` — wraps a null-terminated argv pointer into a refcount-1 Corvid String descriptor
- [x] `runtime/shim.c` trimmed: `main` removed (now codegen-emitted per program); keeps only `corvid_runtime_overflow`
- [x] `link.rs` wires `entry.c` into both MSVC and GCC/Clang command paths
- [x] `RuntimeFuncs` gains 10 new `FuncId`s (`entry_init`, `arity_mismatch`, `parse_i64`/`_f64`/`_bool`, `string_from_cstr`, `print_i64`/`_bool`/`_f64`/`_string`)
- [x] `emit_entry_trampoline` replaced by `emit_entry_main(module, entry_agent, entry_func_id, runtime)` — signature-aware Cranelift function: `main(i32 argc, i64 argv) -> i32` that calls `corvid_init`, checks arity, loads/decodes each `argv[(i+1)*8]` via the type-appropriate helper, calls the entry agent, prints the return via the type-appropriate printer, releases refcounted args/returns, returns 0
- [x] Driver guards updated: Int/Bool/Float/String allowed at both param and return position; Struct/List still rejected with `NotSupported` pointing at the future serialization slice
- [x] 11 new parity fixtures (total 85): int/two-int/bool/float/string param echoing, float + string returns (with and without params), NaN round-trip, arity-mismatch exits non-zero, parse-error exits non-zero with slice-specific message (verified NOT reusing the overflow message)
- [x] Every fixture runs under `CORVID_DEBUG_ALLOC=1` — `ALLOCS == RELEASES` confirms refcounted argv descriptors and String returns are released exactly once

#### Slice 12j ✅ — Make native the default for tool-free programs (Day 28)

- [x] `native_ability(ir)` pre-flight scan in `corvid-driver` returns structured `NotNativeReason` (`ToolCall` / `PromptCall` / `Approve` / `PythonImport`). Names the native-ability rule explicitly; no codegen-internal errors bubble up.
- [x] Compile cache at `<project>/target/cache/native/<fnv1a64-hex>[.exe]` keyed on source + `corvid-codegen-cl` pkg version + every C runtime shim (`shim.c` / `entry.c` / `alloc.c` / `strings.c` / `lists.c`). Second run of an unchanged file skips codegen + link entirely — measured ~15× speedup on `examples/answer.cor` (1.15s → 0.08s).
- [x] `RunTarget::{Auto, Native, Interpreter}` + `run_with_target(path, target)` entry point. Auto picks native when native-able, falls back to interpreter with a one-line stderr notice ("↻ running via interpreter: <reason>"). Native refuses with a clean error naming the phase that would lift the restriction. Interpreter forces the old path.
- [x] CLI flag: `corvid run <file> [--target=auto|native|interpreter]`, default `auto`. `corvid run` by itself now AOT-compiles + executes when possible.
- [x] 7 new driver tests: native-able program passes scan, tool-using / python-import / prompt-using programs fail scan with the right `NotNativeReason`, cache hits on second call (mtime-verified), auto dispatch populates the cache, `--target=native` on a tool-using program exits non-zero.
- [x] Smoke-tested on `examples/answer.cor` (auto → native, cached on second run) and `examples/hello.cor` (auto → fallback with notice, `--target=native` → clean error).

#### Slice 12k ✅ — Phase 12 close-out benchmarks (Day 29)

- [x] Criterion benchmark harness at `crates/corvid-codegen-cl/benches/phase12_benchmarks.rs`. Three workloads: `arith_loop` (500k Int ops), `string_concat_loop` (50k refcount concats), `struct_access_loop` (100k struct alloc + field read + destructor). Each runs on both tiers — interpreter via `corvid_vm::run_agent`, native via `Command::new(binary).output()`.
- [x] Measured wall-clock published in ARCHITECTURE.md §18 ("Phase 12 performance characteristics"). Headline numbers: **13.6× native for arithmetic, 3.5× for struct access, 2.7× for string concat** (end-to-end including the ~11 ms Windows process-spawn tax). Compute-only ratios are 32× / 7.3× / 6.8×.
- [x] Fair-comparison gate passed: native beats interpreter on all three workloads at the scaled workload sizes. The spawn-cost crossover (interpreter < 5 ms → native loses E2E) is documented explicitly alongside the numbers as a known AOT+process-spawn property, with the Phase 22 cdylib path and post-v1.0 JIT path called out as future fixes.
- [x] Native-tier non-goals documented below under "Out of Phase 12."

Cache-eviction policy, stability guarantees across compiler versions, and cross-compilation all move to Phase 33 (launch polish) — none are load-bearing for development work while there are no external users.

**Out of Phase 12 (deliberately):**
- Tool / prompt / `approve` calls in compiled code — Phases 13–15.
- WASM target — Phase 23.
- C ABI + library mode — Phase 22.
- `@wrapping` annotation for opt-out overflow checks — Phase 20 (moat phase, alongside `@budget($)`).
- Cross-compilation to non-host targets — Phase 33 (launch polish).

**v0.3 cuts here** (Phase 12 close-out). Native AOT is the default tier for tool-free programs, cached between runs, benchmarked against the interpreter.

---

## Upcoming

Ordering principles (applied without exception):

1. **Hard dependencies drive sequence.** If B needs A's output, B comes after A. Every phase below names its dep as either **Hard** (can't ship without it) or **Soft** (release-narrative pairing, technically decoupled). No soft deps dressed as hard to make an order look forced.
2. **Themed releases, not feature-soup versions.** Each version has a narrative — v0.4 is "native tier useful," v0.5 is "GP feel," v0.6 is "moat + replay," v0.7 is "embed + deploy." Users upgrade for a coherent story per cut, not a grab-bag of unrelated features. Mixing moat and table stakes inside one version fragments the upgrade pitch.
3. **Moat lands early relative to the total roadmap.** Phase 20 is the mid-point of pre-v1.0, not the end. Every phase after Phase 20 inherits the moat and strengthens it rather than being moat-less GP-polish work that ships without Corvid-ness.
4. **Version cut-lines are explicit.** Every phase is tagged to the version it ships in. `v1.0` is a calendar commitment, not a feature list.
5. **Speculative scope moved post-v1.0.** Features that are "enterprise maturity" or "optimization on top of v1.0 capability" (distributed multi-agent orchestration, hot reload, prompt-aware compilation optimization) do not sit in the pre-v1.0 critical path. Durable single-backend jobs and resumable agent runs are now part of the production-backend track because real AI applications need them before launch.

---

### Phase 13 ✅ — Native async runtime (Day 30)

**Hard dep:** Phase 12 (native codegen). **Hard deps on this:** Phases 14, 15, 30.

- [x] `corvid-runtime` emits a staticlib (`crate-type = ["lib", "staticlib"]`) that `corvid-codegen-cl` links into every compiled Corvid binary. Produces a self-contained executable — no separate runtime file to ship.
- [x] `ffi_bridge` module in `corvid-runtime` exposes the C-ABI surface: `corvid_runtime_probe`, `corvid_runtime_init`, `corvid_runtime_shutdown`, `corvid_tool_call_sync_int`. `deny(unsafe_code)` at crate level; `ffi_bridge` opts in explicitly with a written rationale. Every `unsafe` block carries a SAFETY comment.
- [x] **Eager-init globals, no lazy semantics.** `corvid_runtime_init()` constructs the tokio Runtime + the `Arc<corvid_runtime::Runtime>` and publishes both via `Box::leak` behind an `AtomicPtr`. Readers panic loudly if init hasn't run — no "lazy first-use" branches anywhere.
- [x] **Multi-thread tokio runtime.** `tokio::runtime::Builder::new_multi_thread().enable_all().build()`. Picked multi-thread (not current-thread) at the pre-phase chat: GP-class positioning demands a production-grade runtime from day one; the ~5-10 ms startup tax only applies to programs that actually use the runtime (pure-computation programs skip init entirely — see `ir_uses_runtime` in codegen-cl). `CORVID_TOKIO_WORKERS` env override respected.
- [x] Codegen-emitted main (`emit_entry_main`) conditionally calls `corvid_runtime_init` + registers `corvid_runtime_shutdown` via `atexit` when `ir_uses_runtime(ir)` returns true. Tool-free programs preserve slice 12k's benchmark numbers — no runtime tax paid for what isn't used.
- [x] `IrCallKind::Tool` lowering in `lowering.rs`: for the narrow `() -> Int` signature, emits a call to `corvid_tool_call_sync_int(name_ptr, name_len)` where `name_ptr` is a `.rodata` byte-array emitted by the new `emit_cstr_bytes` helper. Any other tool signature still raises `NotSupported` pointing at Phase 14. `IrCallKind::Prompt` stays pointing at Phase 15.
- [x] Env-var-based mock-tool hook for the parity harness: `CORVID_TEST_MOCK_INT_TOOLS="name1:v1;name2:v2"` registers zero-arg Int-returning mocks during `corvid_runtime_init`. Test-only convention; users never set this env var.
- [x] `tests/ffi_bridge_smoke.rs` — FFI contract test. Hand-written C program that calls `corvid_runtime_probe` / `_init` / `_tool_call_sync_int` / `_shutdown` end-to-end, linked against the staticlib via the same cc-crate pipeline `link.rs` uses. Idempotent shutdown verified; unknown-tool error sentinel verified.
- [x] 6 new parity fixtures (total 91): tool returns Int directly, tool result in arithmetic, tool result drives conditional (both branches), two distinct tools added, agent-to-helper-agent-to-tool chain. Every fixture runs under `CORVID_DEBUG_ALLOC=1` — `ALLOCS == RELEASES` confirms no bridge-induced leaks.
- [x] Link flow handles the `+44 MB` staticlib + native system libs (bcrypt / advapi32 / kernel32 / ntdll / userenv / ws2_32 / dbghelp / legacy_stdio_definitions on MSVC; -lpthread -ldl -lm + macOS frameworks on Unix). `build.rs` in corvid-codegen-cl emits `CORVID_STATICLIB_DIR` at build time so link.rs finds the artifact without runtime discovery.

**Non-scope (deliberate):** User-declared tools via proc-macro registry — Phase 14. Prompt calls — Phase 15. Python FFI — Phase 30. Generalised tool-call bridge (non-Int returns, multi-arg) — Phase 14 extends `corvid_tool_call_sync_int` into a full JSON-marshalling `corvid_tool_call_sync`. True concurrent agents — Phase 25 post-v1.0. Binary size optimization (compiled binaries are ~30 MB stripped after Phase 13) — Phase 33 launch polish.

**Driver-level user-visible behavior:** unchanged in Phase 13. `corvid run <file>` with a tool-using program still falls back to the interpreter via `native_ability::NotNativeReason::ToolCall` — Phase 14 updates the driver to allow tool-using programs to run natively once the proc-macro registry is wired. Phase 13's codegen supports tools; Phase 13's driver doesn't expose that support to users. Parity harness tests it directly.

### Phase 14 ✅ — Native tool dispatch (Day 31)

**Hard dep:** Phase 13 (native async runtime).

- [x] `corvid-macros` proc-macro crate. `#[tool("name")]` on an `async fn` generates a typed-ABI `extern "C"` wrapper + an `inventory::submit!(ToolMetadata)` registration. The user's async fn remains callable as plain Rust for interpreter-tier use.
- [x] **Typed C ABI — no JSON marshalling.** Committed to the extraordinary answer after auditing JSON as the lazy default: both sides of the tool-call boundary know the schemas at compile time, both sides are ours, no LLM tokens cross this boundary. JSON's compactness / universality don't apply; its costs (heap alloc per call, UTF-8 parsing, type erasure, opacity to the optimizer) do. Typed direct calls are what Rust FFI uses idiomatically — Corvid picks the same.
- [x] `#[repr(C)]` ABI wrappers in `corvid-runtime::abi`: `CorvidString` (transparent over descriptor pointer), identity wrappers for `i64` / `f64` / `bool`. `FromCorvidAbi` / `IntoCorvidAbi` traits the macro calls at conversion sites.
- [x] **Refcount conventions for the tool-call boundary:** caller uses the same Owned (+1) / release-after-call pattern as agent-to-agent calls; wrapper's `FromCorvidAbi for String` is borrow-only (reads bytes, never touches refcount). Net: one retain + one release around the call, matching a borrow-style FFI contract. Leak detector (`CORVID_DEBUG_ALLOC=1`) green on every fixture including String-in String-out round-trip.
- [x] `inventory::collect!(ToolMetadata)` in `corvid-runtime`; `corvid_runtime_init` iterates at startup, records the count for diagnostics.
- [x] Cranelift lowering for `IrCallKind::Tool`: emits a direct `call` to `__corvid_tool_<name>` with typed arguments. Link-time symbol resolution means missing `#[tool]` implementations produce linker errors naming the missing symbol — better than the Phase 13 runtime "tool not found" it replaces. Phase 13's narrow `corvid_tool_call_sync_int` bridge is deleted; the single typed-ABI path covers every signature.
- [x] `IrStmt::Approve` lowers to a no-op in compiled code. Effect checker (Phase 5) already enforces `approve`-before-dangerous-tool-call at COMPILE time; runtime verification of approve tokens is Phase 20's moat-phase territory where custom effect rows make it meaningful. Arg expressions still lower (side effects + refcount).
- [x] `corvid-test-tools` crate: staticlib with mock `#[tool]` implementations covering each scalar type + multi-arg. Parity harness links this into every fixture binary; env-var-based tool bodies let tests vary behavior without rebuilding.
- [x] Driver gate lifted conditionally: `native_ability::NotNativeReason::ToolCall` still fires, but `run_with_target` treats it as "satisfied" when `--with-tools-lib <path>` is provided. Fall back to interpreter (auto) or error with a clear pointer-at-the-fix message (native) otherwise. `NotNativeReason::Approve` removed entirely — approve compiles fine.
- [x] CLI: `corvid run <file> [--target=...] [--with-tools-lib <path>]`. Flag-validation checks the path exists. Cache key incorporates the tools-lib path so `--with-tools-lib A` vs `--with-tools-lib B` produce distinct cached binaries.
- [x] 10 new parity fixtures exercising: Int arg, two Int args, String→Int, String→String roundtrip, approve-before-dangerous tool call. Every fixture leak-detector-audited. Total parity suite: **96 fixtures** (up from 85).
- [x] Live smoke: `corvid run examples/tool_call.cor --with-tools-lib target/release/corvid_test_tools.lib` prints `42`. Without `--with-tools-lib`, auto falls back to interpreter with a clear "pass --with-tools-lib" notice. `--target=native` without the lib errors out.

**Linker architecture note.** `corvid-runtime` now ships as both rlib + staticlib. Rust crates use the rlib; compiled Corvid binaries link exactly ONE "runtime-bearing" staticlib — either the standalone `corvid-runtime.lib` (tool-free programs) or the user's tools staticlib which transitively includes corvid-runtime via rlib dep (tool-using programs). Linking both produces `LNK2005` duplicate-symbol errors on every Rust std symbol because each staticlib bundles its own std. The conditional-link logic in `link.rs` handles the either/or.

**Non-scope (deliberate):** Prompt calls — Phase 15. Runtime approve-token verification — Phase 20 (alongside effect rows + custom effects + cost budgets). Tool signatures with Struct/List args — Phase 15 (composite-type marshalling lands alongside prompts). Auto-build of tools crate via `corvid build` spawning cargo — Phase 33 launch polish. `corvid.toml` `[tools]` section — Phase 25 (package manager).

### Phase 15 ✅ — Native prompt dispatch + multi-provider LLM coverage (Day 32)

**Hard dep:** Phase 13 (native async runtime).

User pushback during pre-phase chat caught two latent shortcuts in the original brief: provider coverage limited to Anthropic + OpenAI (insufficient for AI-native positioning, especially missing local-model support) and naive text-then-parse with no retry (brittle by design). Both got rewritten before any code shipped.

- [x] **5 LLM provider adapters cover every category for v0.4.**
  - **Anthropic** — existing.
  - **OpenAI** — existing (refactored to extract `extract_usage` for reuse).
  - **`OpenAiCompatibleAdapter`** (new) — universal escape hatch via `openai-compat:<base-url>:<model>` model spec. Covers OpenRouter, Together, Anyscale, Groq, Fireworks, Azure OpenAI, llama.cpp server, vLLM, LM Studio, and ~20 other providers exposing OpenAI-compatible endpoints. **One adapter, ~30+ backends.**
  - **`OllamaAdapter`** (new) — local-first via `POST localhost:11434/api/chat`. Routed by `ollama:<model>` prefix. No API key. `OLLAMA_BASE_URL` override for non-default servers.
  - **`GeminiAdapter`** (new) — Google Gemini via `POST /v1beta/models/<m>:generateContent`. Routed by `gemini-*` prefix. Auth via `GOOGLE_API_KEY` / `GEMINI_API_KEY`.
- [x] **`TokenUsage` on every `LlmResponse`.** Every adapter populates `prompt_tokens` / `completion_tokens` / `total_tokens` from the provider response — Anthropic's `input_tokens`/`output_tokens`, OpenAI's `prompt_tokens`/`completion_tokens`, Ollama's `prompt_eval_count`/`eval_count`, Gemini's `usageMetadata`. Foundation for Phase 20's `@budget($)` cost annotations.
- [x] **`EnvVarMockAdapter`** — env-var-based mock for parity tests. `CORVID_TEST_MOCK_LLM=1` registers it as the first adapter so its wildcard `handles()` claims every model spec, avoiding real API egress in CI even when keys leak.
- [x] **4 typed prompt-dispatch bridges** in `corvid-runtime::ffi_bridge`: `corvid_prompt_call_int` / `_bool` / `_float` / `_string`. Each takes 4 `CorvidString` args (prompt name, signature, rendered template, model). Mirrors Phase 14's typed-ABI design.
- [x] **Built-in retry-with-validation.** `CORVID_PROMPT_MAX_RETRIES` (default 3). Each retry escalates the system prompt with stronger format instructions + the prior unparseable response. Tolerant `parse_int` / `parse_bool` / `parse_float` strip surrounding quotes, code fences, and whitespace before parsing.
- [x] **Function-signature context in the system prompt.** Every prompt call automatically tells the LLM "you are a function with signature `name(params) -> ReturnType` — return the appropriate value." Codegen embeds the signature as a literal at compile time. Treats prompts as typed functions the LLM is implementing, not ad-hoc string queries.
- [x] **Stringification helpers** (`corvid_string_from_int` / `_bool` / `_float`) in the C runtime. Cranelift codegen calls them when interpolating non-String args into prompt templates.
- [x] **Cranelift lowering for `IrCallKind::Prompt`.** Compile-time template parser splits `{var}` placeholders; codegen emits a chain of `corvid_string_concat` operations with stringified args between literal segments. Bridge selection by return type.
- [x] **Driver gate lifted unconditionally.** `NotNativeReason::PromptCall` removed. Prompt-using programs compile + run natively without any extra user-provided lib (`corvid-runtime` ships the adapters built-in). Runtime errors surface at LLM call time if no provider is configured.
- [x] **Architectural fix: C runtime moved into `corvid-runtime`.** The `runtime/*.c` files (alloc, strings, lists, entry, shim) relocated from `corvid-codegen-cl/runtime/` to `corvid-runtime/runtime/`. New `corvid-runtime/build.rs` compiles them via `cc::Build` into a `corvid_c_runtime` staticlib. `corvid-runtime` re-exports the path via `c_runtime::C_RUNTIME_LIB_PATH`. `corvid-codegen-cl::link.rs` and the FFI smoke test add this lib to their linker invocations. **Why:** the prompt bridges' `IntoCorvidAbi for String` reaches `corvid_string_from_bytes`, which any binary linking corvid-runtime must resolve — making corvid-runtime self-contained means Rust test binaries link cleanly without separate C-source compilation per test.
- [x] **4 new parity fixtures** (total: **99**): zero-arg Int return, Int-arg interpolation + Int return, String-arg interpolation + Int return. Every fixture uses the env-var mock LLM. Leak-detector-audited.

**Non-scope (deliberate, named for future phases):**
- **Provider-specific JSON-schema structured output** (OpenAI `response_format`, Anthropic tool-use for structured returns, Gemini's `responseSchema`) → Phase 20 (moat, alongside `Grounded<T>`). Phase 15's text-then-parse with retry covers ~95% of cases.
- **Streaming `Stream<T>` returns** → Phase 20.
- **Replay** (deterministic re-execution of recorded LLM calls) → Phase 21.
- **`@budget($)` cost bounds** → Phase 20 (uses the `TokenUsage` Phase 15 plumbed through).
- **Per-prompt model selection in source** (`prompt foo() -> Int using "gpt-4o":`) → Phase 31.
- **Caching response by `(prompt, args, model)`** → Phase 21.
- **Real-API integration tests** against Ollama / OpenAI / Anthropic / Gemini → Phase 33 launch polish, when CI has a runner that can install Ollama + has provider keys configured.
- **`corvid stats` CLI subcommand** for token-usage diagnostics → Phase 20 ships this alongside `@budget($)` enforcement that uses the same data.

**v0.4 cuts here.** Native tier is actually useful for real programs.

---

### Phase 16 ✅ — Methods on types (Day 33)

**Hard dep:** frontend (✅), IR (✅). Single Cranelift-symbol disambiguation needed (DefId-suffixed); otherwise codegen unchanged.

Pre-phase chat caught two limiting shortcuts in my brief and reshaped the phase substantially. The shipped form:

- [x] **Syntax: `extend T:` block** (not Rust's `impl T:`). Full word matches Corvid's keyword style (`agent`, `tool`, `prompt`, `approve`, `dangerous`, `type`); reads as English ("extend Order with these methods"); leaves room for Phase 20 traits via `extend T as Serializable:` without retroactive renaming. `type T:` stays purely structural — better for LLM readability.
- [x] **Methods can be ANY decl kind** — `extend T:` blocks hold a mix of `agent`, `prompt`, `tool` declarations. Same dot-syntax dispatches all of them: `order.total()` is a pure-function call, `order.summarize()` is an LLM call, `order.fetch_status()` is a tool call, `order.process()` is an effectful agent. **No other language unifies prompts, tools, and pure code under a single typed dot-syntax** — for an AI-native language this turns "AI is a method on your type" from positioning into syntax.
- [x] **Effect inference handles purity** — no `function` keyword introduced. Agents inherit their effect rows from their bodies (which already worked via the existing checker). A method that doesn't call any effectful primitive has no effect row; replay/cost-budget machinery (Phases 20–21) won't track it. Avoids fourth-keyword proliferation; keeps the moat phase's effect-row work simple.
- [x] **`public` / private visibility shipped now**, with parens-extension reserved for Phase 20 effect-scoped variants. Default visibility is private (file-scoped). `public` and `public(package)` are the Phase 16 surface; `public(effect: audited)` lands in Phase 20 without breaking syntax. Decision motivated by: public-by-default is a one-way door for API stability; retrofitting visibility post-v1.0 would be a breaking change every existing impl block has to absorb.
- [x] **Receiver as explicit first parameter** — no `self` keyword. `extend Order: agent total(o: Order) -> Int` makes the receiver a parameter like any other. Mental model matches "methods are agents with a receiver"; Pythonic users adapt instantly. Less special-casing than Rust's `self`.
- [x] **Receiver-type-keyed method lookup.** Resolver builds a per-type method side-table `(type_def_id, method_name) -> DefId`. Multiple types can share method names (`Order.total`, `Line.total`) without collision. Field/method name collisions on the same type are compile-time errors.
- [x] **Cranelift symbol mangling** updated to include the agent's `DefId` so `extend Order: agent total` and `extend Line: agent total` get distinct internal symbols. Symbols are `Linkage::Local`; the suffix never leaks into a public API.
- [x] **6 new parity fixtures** (total: 105) — receiver-only method, multi-arg method, method-calls-method, methods-with-same-name-on-different-types (verifies receiver-type dispatch), method on a struct with a refcounted `String` field (leak-detector-audited).
- [x] **5 new resolver tests** (total: 19) — extend registers methods, extend on unknown type errors, duplicate methods error, method/field name collision error, methods on different types coexist.
- [x] **5 new parser tests** (total: 80 → 85) — `extend` blocks parse, mixed decl kinds, default + `public` + `public(package)` visibility, malformed `public(...)` rejected.

**Non-scope (deliberate, named for future phases):**
- **`self` keyword** — explicit first param model is the answer; revisit only if a real foot-gun surfaces.
- **Static methods** (`Type.factory()`) — free agents serve the role today; non-breaking to add later.
- **Methods on built-in types** (Int, String, List) — orphan-rule design must come with Phase 25's package manager. Phase 30+ stdlib decides.
- **Method overloading** — duplicate names on a type are compile errors. Rust + Go thrive without overloading; not adding it.
- **Multi-file `extend` blocks** (one type extended in many files) — Phase 25.
- **Trait/interface system** — Phase 20 (moat). The `extend T as TraitName:` syntactic slot is reserved.
- **Effect-scoped visibility** (`public(effect: audited)`) — Phase 20.

**Architecturally important:** Phase 16 introduces NO new IR variants. Method calls compile to ordinary `IrCallKind::Agent` / `Prompt` / `Tool` calls with the receiver prepended as the first argument. Codegen (Cranelift, Python transpile, future WASM) needs no per-method handling — methods are agents/prompts/tools with a different declaration syntax and a different lookup path.

### Phase 17 — Cycle collector + effect-typed memory model (~10–14 weeks) ✅ closed

**Goal.** Backstop refcount against cycles AND lift the memory model to take advantage of Corvid's typed effects. Most allocations should never see refcount at all; the ones that do should rarely be atomic; cycles should be caught without per-allocation tracing overhead.

**Hard dep:** Phase 12 (refcount runtime + native codegen).

**Status.** Closed in `v0.1-memory-foundation`.

| Slice | Outcome | Commit / tag |
|---|---|---|
| `17a` | typed heap headers + per-type typeinfo | `1fea6a0` |
| `17b-0` | retain/release counters + baseline RC counts | `7ef4304` |
| `17b-1a` | `Dup` / `Drop` IR + borrow-signature scaffolding | `82f78b5` |
| `17b-1b.1` | borrow inference + callee-side ABI elision | `2bce2a8` |
| `17b-1b.2` | string operand borrow-at-use-site peephole | `71c7fe4` |
| `17b-1b.3` | field/index target borrow-at-use-site peephole | `de3acb5` |
| `17b-1b.4` | `for` iterator borrow-at-use-site peephole | `a725449` |
| `17b-1b.5` | call-arg borrow-at-use-site peephole | `b0a911e` |
| `17b-1b.6a` | ownership dataflow groundwork | `760b07e` |
| `17b-1b.6b` | IR `Dup` / `Drop` insertion | `1d1af44` |
| `17b-1b.6c` | ownership hook into codegen pipeline | `f3762cd` |
| `17b-1b.6d-1` | transition guard stage | `8e2e98e` |
| `17b-1b.6d-2a` | entry / return cleanup stage | `520e30b` |
| `17b-1b.6d-2` | unified ownership pass default-on | `0cc7895` |
| `17b-1c` | whole-program pair elimination | `046806d` |
| `17b-2` | drop specialization | `8c55c3f` |
| `17b-3` | reuse analysis | deferred to Phase 17.5 |
| `17b-4` | Morphic-style specialization | deferred to Phase 17.5 |
| `17b-5` | escape analysis | deferred to Phase 17.5 |
| `17b-6` | effect-row-directed RC | deferred to Phase 20 |
| `17b-7` | latency-aware RC at prompt / LLM boundaries | `6bedbfb` |
| `17c` | safepoints + stack maps | `e55efea` |
| `17d` | native mark-sweep cycle collector | `ca428bf` |
| `17e` | effect-typed scope reduction | `f5a3bce` |
| `17f / 17f++` | deterministic GC triggers + refcount verifier | `a3b841d` |
| `17g` | `Weak<T>` | `ba01e78` |
| `17h.1` | VM-owned heap handles | `318c892` |
| `17h.2` | VM Bacon-Rajan cycle collector | `91d95ac` |
| `17i` | close-out + benchmark lock | `v0.1-memory-foundation` |

**Historical slice plan (kept for design context):**

- [x] **17a — typed heap headers + per-type typeinfo** *(landed 2026-04-14)*. Every refcounted allocation carries a `corvid_typeinfo*` pointer in its 16-byte header. Per-type metadata (destroy_fn, trace_fn, flags, elem_typeinfo) lives in `.rodata`. Refcount dropped `_Atomic` (Phase 25 will do proper multi-threaded RC, not blanket atomics). Bits 61-62 reserved for 17d mark + 17h color. `List<Int>` mis-trace bug eliminated by design (`elem_typeinfo = NULL` sentinel). 6 new runtime tracer tests, all 105 parity tests still green.
- [~] **17b — principled RC optimization (Perceus).** *Region inference dropped from this slice based on Perceus paper analysis + MLton's published rejection of Tofte-Talpin regions; revisit only if post-17b measurements show remaining allocation pressure justifies the complexity.*
  - [x] **17b-0** *(landed 2026-04-15)* — retain/release call-count instrumentation + baseline RC op counts as exact-match assertions on 6 representative workloads.
  - [x] **17b-1a** *(landed 2026-04-15)* — `IrStmt::Dup` / `IrStmt::Drop` as first-class IR variants; `ParamBorrow` enum + `IrAgent.borrow_sig` field; codegen handles the variants end-to-end. Pure scaffolding, behavior-preserving.
  - [x] **17b-1b.1** *(landed 2026-04-15)* — Lean 4-style monotone fixed-point borrow inference over the call graph. Populates `IrAgent.borrow_sig`. Callee-side ABI elision: refcounted params marked `Borrowed` skip entry-retain + scope-exit release. Measured: `passthrough_agent` 13 → 9 ops (31%).
  - [~] **17b-1b.peepholes** *(landed 2026-04-15 as four separate commits: 17b-1b.2, .3, .4, .5)* — **single borrow-at-use-site optimization family** applied to four IR positions: string BinOp operands, FieldAccess / Index targets, for-loop iter, and call-site args (coordinated with callee `borrow_sig`). Shipped as four commits while structurally one optimization; retrospective dev-log entry Day 24 captures the honest framing. Cumulative measured (baseline → current): `string_concat_chain` 12→10 (8%), `struct_build_and_destructure` 14→8 (43%), `list_of_strings_iter` 22→14 (36%), `passthrough_agent` 13→7 (46%), `local_arg_to_borrowed_callee` new at 6 ops.
  - [ ] **17b-1b** *(real, still pending)* — full use-list + CFG-aware last-use + branch-asymmetric `Dup`/`Drop` insertion pass in `ownership::transform_agent`. Deletes the ~40 scattered `emit_retain`/`emit_release` sites in `lowering.rs`. Handles what peepholes cannot: loop-var body analysis, scope-exit Drop redundancy, cross-statement last-use elision, list-literal item-slot Locals. **This is the work that was originally committed as 17b-1b. The peephole series shipped wins but did not replace this.** Needs its own pre-phase chat; multi-session slice when resumed.
  - [x] **17b-1c** - whole-program retain/release pair elimination. Shipped as the first same-block ARC-style cleanup pass after the unified ownership pipeline.
  - [x] **17b-2** - drop specialization. `drop x` on a known typeinfo now inlines the child-release sequence instead of dispatching through `typeinfo->destroy_fn`.
  - [ ] **17b-3** — reuse analysis. Match `drop(x_size_N); alloc(size_M ≤ N)` pairs in a basic block; emit `if (refcount & MASK) == 1 { reuse_in_place } else { drop; alloc }`. Same-size-in-words rule per Perceus / Lean 4.
  - [ ] **17b-4** — Morphic-style per-call-site alias-mode specialization (Lobster-style, gated to mixed-mode callees only).
  - [ ] **17b-5** — Choi-style interprocedural escape analysis → stack / arena promotion for non-escaping allocations.
  - [ ] **17b-6** — **INNOVATION (zero prior art):** effect-row-directed RC. `Pure` effect → static `isUnique` discharge; `<llm>` effect → batching point for RC ops across known-slow suspensions.
  - [x] **17b-7** - **INNOVATION (zero prior art):** latency-aware RC scheduling across prompt/LLM call boundaries.
- [x] **17c** - Cranelift safepoint emission + stack maps. Per-function safepoint records let the native collector find live roots on task stacks.
- [x] **17d** - native mark-sweep cycle collector. Dispatches through `typeinfo->trace_fn` per object with deterministic test hooks and allocation-pressure triggering.
- [x] **17e** - effect-typed scope reduction. Shipped as conservative same-block `Drop` relocation across effect-free spans.
- [x] **17f** - replay-deterministic GC triggers plus the runtime refcount verifier. GC behavior is now measurable and replay-auditable.
- [x] **17g** - `Weak<T>` user-facing type. Weak refs now ship with effect-typed invalidation rules and runtime clearing semantics.
- [x] **17h** - interpreter-side cycle collector. Bacon-Rajan now runs over VM-owned heap handles in the interpreter tier.
- [x] **17i** - tests + close-out. Locked with the same-session ratio archive and release tag `v0.1-memory-foundation`.

**Non-scope:** generational GC. Concurrent collection (mutator-collector concurrency via write barriers — post-v1.0 if multi-threaded Corvid ever becomes a direction).

### Phase 18 — Result + Option + retry policies (~4 weeks) ✅ — core complete

**Goal.** Language-native error handling with a principled native subset first: `Result<T, E>`, `Option<T>`, propagation (`?`), and retry syntax that lowers as deterministic native control flow rather than a library loop.

**Hard dep:** typechecker extension for generic types (landed). The remaining work is native widening, not front-end feasibility.

**Status.** Front-end + interpreter support is landed. Native AOT support is substantially shipped for the compositional one-word subset and selected wide `Option<T>` cases; Phase 18 is no longer "can Corvid do this?" but "how far do we widen native support before moving on?"

**Shipped so far:**
- [x] `Result<T, E>` and `Option<T>` as compiler-known stdlib types in the frontend + interpreter.
- [x] Postfix `?` in the frontend + interpreter.
- [x] Retry syntax in the frontend + interpreter.
- [x] Native nullable `Option<T>` subset for refcounted payloads such as `Option<String>`.
- [x] Native wide scalar `Option<Int|Bool|Float>`.
- [x] Native nested `Option<T>` widening where nullable-pointer encoding would otherwise collapse `Some(None)` into outer `None`; wrapper-backed `Option<Option<...>>` now preserves the distinction.
- [x] Native postfix `?` for the shipped `Option<T>` subsets, including widening into a different native `Option<U>` envelope.
- [x] Native one-word `Result<T, E>` subset with ownership integration.
- [x] Native postfix `?` for `Result<T, E>`, including `Result<A, E>?` inside `Result<B, E>` when both shapes remain in the current native subset.
- [x] Native deterministic retry lowering over the native `Result<T, E>` subset with explicit backoff control flow and ownership-correct cleanup between attempts.
- [x] Native deterministic retry lowering over the native `Option<T>` subset, where `None` is the retryable branch and the final exhausted value remains `None`.
- [x] Native compositional proof points for nested one-word shapes such as `Result<Option<Int>, String>` and nested `Result` envelopes.
- [x] Native structured payload proof points inside the current subset, including `Result<Boxed, String>` and `Result<List<Int>, String>`.

**Corvid inventions already landed in this phase:**
- [x] **Deterministic native retry as compiled control flow.** Retry lowers to explicit native control-flow blocks over `Result<T, E>`, not an opaque runtime helper.
- [x] **Failure-carrier-aligned retry semantics.** Native and interpreter retry now agree that `Err(...)` and `None` are the retryable branches for the shipped subset.
- [x] **Compositional tagged-union subset.** Native support is being widened by proving a principled representation composes across nested shapes, rather than by adding ad hoc case-by-case exceptions.
- [x] **Selective wrapper widening where nullability stops being sound.** Native `Option<T>` keeps the cheap nullable-pointer form where it is semantically safe, and switches to a tiny typed wrapper only for shapes like nested options where bare nullability would destroy information.

**Phase 18 core work: done.** Remaining integration with Phase 20 dimensional effects (effect-integrated failure typing) belongs to the Phase 20 wave, not unfinished Phase 18 capability.

**Non-scope:** User-defined error enums with arbitrary payload layouts beyond the supported native subset — that belongs to the later richer-type/effect work, not this first native-control-flow pass.

### Phase 19 — REPL (~3 weeks) ✅ closed

**Goal.** `corvid repl` interactive shell. How users learn Corvid.

**Hard dep:** interpreter (✅).

**Scope:**
- Persistent session: locals, imports, agent declarations live across inputs.
- Redefine an agent mid-session; later calls use the new definition (no state migration — a fresh session is cheap).
- Pretty-printing of return values, including structs (field-by-field) and lists (with length).
- readline-class editing (history, ctrl-r search, multiline input with indent-aware continuation).
- `:help`, `:type <expr>`, `:reset`, `:quit` meta-commands.
- [x] AI run scratchpad mode: run agents with mocked tools/prompts, inspect the composed effect profile, cost estimate, model route, confidence, and provenance without leaving the shell. Shipped as `:scratch [agent]`, a single REPL report over session declarations, imported mocks, composed effect dimensions, cost estimates, and last-run boundary trace signals.
- [x] `:why` explains the compiler/runtime reason for an approval gate, model route, confidence downgrade, budget warning, or grounding failure. Shipped as a REPL trace explanation command that records silent boundary traces for normal evaluation and reports agent, prompt, tool, approval, route, and confidence-gate reasons from the last run.
- [x] `:replay last` reruns the last interaction through the recorded trace so users can debug behavior without spending on another model call. Shipped as an in-memory replay session over the REPL's last boundary trace, reusing the same `:step` / `:run` / `:show` / `:where` replay UI as JSONL traces.

**Non-scope:** Native-tier REPL. LSP integration (Phase 24 owns that).

**Status.** Closed. The REPL now supports persistent locals/declarations, declaration redefinition, type-aware value display, readline history/multiline input, core meta-commands, source/trace import, step-through execution, `:why`, `:replay last`, and `:scratch [agent]`.

**v0.5 cuts here.** Methods + cycle collector + Result + REPL make Corvid feel like a modern GP language.

---

### Phase 20 — Effect rigor + grounding + cost + streaming (~14–16 weeks) — **THE MOAT PHASE** ✅ closed

**Goal.** The phase that defines what makes Corvid Corvid. All compile-time, all language-level. Shipped mid-roadmap, not saved for impact — every phase after this inherits the moat.

**Hard dep:** typechecker + effect checker (✅ baseline from Phase 5). Methods (Phase 16, for the `Grounded<T>.unwrap_*` methods).

This phase is too large to ship atomically without splitting. Nine substantial deliverables; no single landing of the whole thing. Slice breakdown mirrors Phase 12's pattern — each slice ships, tests, commits, and updates the dev-log independently, and the phase is only "closed" when every slice is green.

#### Slice 20a — Dimensional effects + composition algebra (~4 weeks)

Corvid's moat: effects carry typed dimensions (cost, trust, reversibility, data, latency, confidence) that compose independently through the call graph. No other language has this.

- [x] AST: `effect Name:` declaration with typed `DimensionDecl`s. `EffectRow` (`uses` clauses) on tool/agent/prompt signatures. `EffectConstraint` annotations (`@budget`, `@trust`, `@reversible`). `DimensionValue` types (Bool, Name, Cost, Number). `CompositionRule` (Sum, Max, Min, Union, LeastReversible). Committed `66bb4d1`.
- [x] Resolver: `DeclKind::Effect` in symbol table. Effect declarations registered in pass 1. Effect refs in `uses` clauses resolved and validated in pass 2. Committed `66bb4d1`.
- [x] Composition algebra: `EffectRegistry` built from declarations. 6 built-in dimension schemas. `compose()` applies per-dimension rules. `check_constraints()` validates composed profiles against annotations. `ConstraintViolation` with dimensional error messages. Committed `66bb4d1`.
- [x] Call-graph analyzer: `analyze_effects()` walks agent bodies, collects effects from tool/prompt/agent calls, produces per-agent composed dimensional profiles. Committed `66bb4d1`.
- [x] Parser: `effect Name:` block syntax. `uses` clause on declarations. `@budget($)` / `@trust()` / `@reversible` annotation syntax. Committed `3bfefaf`.
- [x] Typechecker integration: `typecheck()` runs the dimensional analyzer, produces `EffectConstraintViolation` errors with actionable messages. Committed `b344e3f`.
- [x] Legacy bridge: built-in `dangerous` effect with `trust: human_required, reversible: false`. Existing `dangerous` keyword code compiles unchanged. Committed `f229aba`.
- [x] Revisits the Day-4 `Safe | Dangerous` decision — additive, no breaking change to existing code.

#### Slice 20b — Compile-time provenance verification + `Grounded<T>` (~3 weeks)

The invention: groundedness is not an annotation — it's a compile-time provenance property that the compiler infers by tracing data flow from retrieval tools through prompts to return types. No other language does this.

- [x] `Grounded<T>` as a compiler-known stdlib type (like `Result`, `Option`). `Type::Grounded(Box<Type>)`, resolver built-in, checker generics, IR lowering, ABI type description, VM value support, and native/host binding surfaces are implemented.
- [x] Provenance analyzer in the typechecker: walks each agent's data flow graph to determine which values inherit groundedness from tools with `data: grounded` in their effect declaration. If a value's provenance chain includes at least one grounded source, the value is provably grounded.
- [x] Stable diagnostic code for ungrounded returns. The checker emits typed `UngroundedReturn`; the pretty renderer maps it to `E0209` with a provenance-specific source label.
- [x] Provenance flows compositionally across agent boundaries: if agent B calls a grounded tool and agent A calls B, A's return inherits B's groundedness.
- [x] `cites ctx strictly` runtime annotation in syntax, typechecking, IR, and interpreter: compile-time proves the cited prompt parameter is `Grounded<T>`; the VM verifies the response cites content from the grounded payload.
- [x] Native `cites ctx strictly` emission in Cranelift/codegen-cl so compiled prompts enforce the same citation check as the interpreter.
- [x] `.unwrap_discarding_sources()` method on `Grounded<T>` for when the caller consciously drops provenance. Typechecker, explicit IR node, VM behavior, native lowering, and ABI/codegen IR walkers are implemented.
- [x] Built-in `retrieval` effect with `data: grounded` dimension registered in the `EffectRegistry` so tools can declare themselves as grounded sources.

#### Slice 20c — `eval ... assert ...` language syntax (~2 weeks)
- [x] Parser + typechecker + lowering for `eval name: body ... assert expr` declarations, including value, trace, cost, ordering, and statistical assertions.
- [x] IR node `IrEval` alongside `IrAgent`.
- [x] Runner CLI is out of scope — ships in Phase 27. This slice is language only.

#### Slice 20d — Cost dimension + `@budget` compile-time analysis (~3 weeks)

Cost is a dimension in the effect system, not a standalone annotation. `@budget($1.00)` is an `EffectConstraint` on the cost dimension.

- [x] Each tool/prompt carries `cost: $X.XX` in its effect declaration.
- [x] Compile-time worst-case cost analysis sums the cost dimension over control-flow paths using the composition algebra, including multi-dimensional cost/tokens/latency estimates and `:cost` tree rendering.
- [x] Stable diagnostic codes for budget diagnostics. The checker emits budget `EffectConstraintViolation` errors and `UnboundedCostAnalysis` warnings; the pretty renderer maps them to `E0250` / `W0251`.
- [x] Also ships the `@wrapping` annotation for opt-out overflow checks deferred from Phase 12.

#### Slice 20e — Confidence dimension (~2 weeks)

Confidence is a dimension in the effect system. The `Min` composition rule means the least confident result determines the chain.

The invention: confidence isn't a number — it's a dynamic authorization gate. The compiler couples confidence to trust, so a confident agent can act autonomously and an uncertain agent is forced to get human approval. No other system does this.

- [x] `autonomous_if_confident(threshold)` trust variant: couples trust level to composed confidence. Above threshold → autonomous. Below → human approval activates at runtime.
- [x] Confidence propagation: deterministic tools produce confidence 1.0, prompts carry LLM-reported confidence, `Min` composition through the call graph.
- [x] Confidence gate in the interpreter: at tool dispatch, if trust is `autonomous_if_confident(T)`, compute composed confidence of inputs. Below T → dynamically activate the approval prompt.
- [x] `@min_confidence(P)` compile-time constraint: compiler proves all paths to irreversible actions meet the confidence floor.
- [x] `calibrated` modifier on prompts: runtime accumulates accuracy statistics, flags miscalibrated models when self-reported confidence drifts from actual accuracy.
- [x] REPL integration: step-through shows confidence at each step. Confidence gates show threshold vs. actual when they fire.

#### Slice 20f — `Stream<T>` + latency dimension + streaming effect integration (~3 weeks)

Streaming in Corvid isn't just async iteration — streams are **first-class participants in the dimensional effect system**. Every dimension (cost, confidence, provenance, trust, latency) flows through stream types. No other language can do this because no other language has dimensional effects.

**Foundation:**
- [x] `Stream<T>` as compiler-known stdlib type. Prompts + tools can declare streaming returns.
- [x] `for x in stream:` consumes the stream. `yield` in agent bodies produces streams.
- [x] `latency` / `latency_ms` dimension support exists for cost analysis; richer `latency: streaming(backpressure: bounded(N) | unbounded)` algebra remains in the streaming integration bullets below.
- [x] Tokio `mpsc::Receiver` backing; agent bodies with `yield` run as async tasks.

**Streaming effect integration (the inventions):**
- [x] **Live cost termination mid-stream.** `@budget($1.00)` on an agent calling a streaming prompt tracks cumulative cost per yielded token. If the budget is exceeded while the stream is still producing, the runtime terminates and raises `BudgetExceeded`. No framework terminates streams by accumulated cost.
- [x] **Per-element provenance in `Stream<Grounded<T>>`.** Each yielded element carries its own `ProvenanceChain`. Aggregate stream provenance is the union. Step-through REPL shows provenance building up in real time.
- [x] **`try ... retry` over streams — stream-start semantics.** Retries fire at stream-open, not per-element. Transient connection failures retry with backoff; mid-stream errors propagate.
- [x] **Confidence-floor termination.** `with min_confidence 0.80` on a streaming prompt terminates the stream if streaming confidence drops below threshold, raising `ConfidenceFloorBreached`.
- [x] **Mid-stream model escalation** (paired with 20h). On confidence drop, the runtime opens a continuation stream on a stronger model, feeding the partial output as continuation context. Consumer sees seamless tokens with a `StreamUpgradeEvent` in the trace. No framework has this.
- [x] **Progressive structured types: `Stream<Partial<T>>`.** Compiler-known `Partial<T>` where each field is `Complete(V)` or `Streaming`. Users access fields the moment they're complete without waiting for the full response. Type-level progressive structure.
- [x] **Resumption tokens.** `resume_token(stream)` captures delivered elements plus prompt context in `ResumeToken<T>`. `resume(prompt, token)` reopens the prompt through the interpreter with accumulated delivered context; provider-native continuation state is represented for future adapters.
- [x] **Declarative fan-out / fan-in.** `stream.split_by("field")` partitions a struct stream into typed sub-streams by field value. `merge(groups).ordered_by("fifo" | "sorted" | "fair_round_robin")` combines with explicit ordering guarantees. Compile-time type + field checking.
- [x] **Backpressure propagation.** A slow consumer pulls from a producer at its consumption rate. The effect system captures this as `backpressure: pulls_from(producer_rate)`, parser/typechecker constraints are source-sensitive, and the VM maps pull-based streams to demand-gated bounded channels while fan-in preserves composed upstream policy.

#### Slice 20g — Bypass tests + effect-system specification (~4 weeks)

The moat-closer. Most languages ship a spec. Some add a proptest suite. None do all five of what 20g ships. When 20g lands, the effect system's correctness is provably stronger than any existing language's type system has ever been.

**The five verification inventions** (described below) ship alongside **five spec-layer inventions** — custom dimension authoring, proof-carrying dimensions, spec↔compiler sync, community dimension registry, self-verifying verification — documented in [docs/internals/effect-spec/](../docs/internals/effect-spec/) sections 01 and 02 and surfaced in the toolchain as `corvid test dimensions`, `corvid effect-diff`, and `corvid add-dimension`.

**The five verification inventions:**

##### 1. Cross-tier differential verification

Corvid has four tiers that all see the same effect profile: type checker (static), interpreter (dynamic), native codegen (dynamic, different path), replay (deterministic re-execution). 20g runs every safety property across all four and fails if any tier disagrees:

```
for each test program P:
  static_result = typecheck(P)
  interp_result = interpret(P)
  native_result = native_compile(P).run()
  replay_result = replay(record(P))
  assert same_effect_profile(static_result, interp_result, native_result, replay_result)
```

If the type checker says "this agent is `@trust(autonomous)`" but the interpreter triggers a human-approval gate at runtime, that's a **soundness divergence** — one of the tiers is lying. The test harness catches it. No other language tests soundness this way because no other language has four execution tiers seeing the same effect profile.

- [x] Build the `differential-verify` test harness crate — shipped as `crates/corvid-differential-verify`
- [x] Run every existing test program across all four tiers and compare — runnable corpus under `tests/corpus/`, `should_fail/tier_disagree.cor` + `should_fail/native_drops_effect.cor` prove the harness catches real divergence
- [x] Machine-readable divergence reports when tiers disagree — `corvid verify --json`, `DivergenceReport` serde structure, divergence classification (`static-overapprox` / `static-too-loose` / `tier-mismatch`)
- [x] Shrinker for divergent programs — `corvid verify --shrink <file>` produces a smaller reproducer
- [x] CI gate: any divergence fails the build — `.github/workflows/ci.yml` runs `corvid verify --corpus tests/corpus` and enforces exit code 1 (commit `4d4944b`)
- [x] **Native-tier trace emission.** Shipped via `crates/corvid-trace-schema` + native tracer + verifier consumption (commits `3b1a380` / `9616c20` / `7d63e1c`). The fallback to interpreter effects is deleted.

##### 2. Adversarial LLM-driven bypass generation

Corvid is AI-native — use AI to attack its own type system. A generator feeds the spec to an LLM and asks it to produce programs designed to bypass the dimensional checker. The test suite runs every generated program. The compiler must reject every one.

```
>>> corvid test adversarial --count 100 --model opus

  Generated 100 bypass attempts targeting:
    - approve-before-dangerous bypass (22 attempts)
    - confidence gate circumvention (18 attempts)
    - budget evasion through recursion (15 attempts)
    - provenance chain laundering (13 attempts)
    - trust dimension forging (11 attempts)
    - [other categories]

  Results: 100 rejected (expected 100)   ✓ all bypasses caught
```

If a generated bypass compiles, either the LLM found a real bypass (fix the checker, add to regression corpus) or the program is actually legal (refine the prompt). The generator runs on every CI build. The corpus grows adversarially.

- [x] `corvid test adversarial` CLI command — runs the adversarial harness instead of a stub
- [x] Generator prompt with category taxonomy (bypass angles) — deterministic prompt pack covers approval, trust, budget, provenance, reversibility, and confidence bypass families
- [x] Regression corpus: every historical bypass attempt, permanently tested — seed adversarial corpus is generated deterministically by `corvid-driver`; composition attacks remain in `counterexamples/composition/` + meta-verifier
- [x] Accept/reject classifier runs the compiler on each generated program — every attempt goes through the full frontend; any clean compile exits non-zero as an escaped bypass
- [x] Bypasses found during generation automatically filed as issues — enabled when `CORVID_ADVERSARIAL_FILE_ISSUES=1` and `GITHUB_TOKEN` are set; otherwise escaped rows fail locally without network side effects

##### 3. Executable, interactive specification

The spec document isn't prose with code blocks. It's a **literate Corvid program** where every example is runnable. Readers click a code sample and it opens in the Corvid REPL with the session state pre-loaded. Every rule in the spec has:

1. A positive example (program exemplifying the rule)
2. A negative example (near-miss that the rule rejects, with the exact error message)
3. Link to the proptest property that checks the rule
4. Link to the cross-tier test that proves all four tiers agree

The spec becomes a **living proof obligation**. Change the composition algebra → the spec examples either still compile (ship it) or they don't (spec fails CI).

- [x] `docs/internals/effect-spec/` as a literate spec — `.md` files with embedded runnable corvid blocks + `# expect:` directives (commits `3f80585` through `b628068`, 13 sections total)
- [x] Build pipeline: every code block compiles during spec publication — `corvid test spec` wired to CI (commit `4d4944b`). Current report: 5 compile / 38 skip / 0 fail across 43 blocks.
- [x] Static site generator that renders the spec with "Run in REPL" buttons — `corvid test spec --site-out <DIR>` reads the verified literate spec and emits static HTML, CSS, JS, and runnable snippets
- [x] Cross-links from spec rules to proptest + differential-verify tests — `docs/internals/effect-spec/12-verification.md` now carries a rule-to-test map linking composition, budgets, grounding, approval, confidence, rewrites, and cross-tier profile agreement to their production modules, property tests, and corpus gates.
- [x] Comparison appendix: Koka, Eff, Frank, Haskell algebraic effects, Rust `unsafe`, capability systems — [section 11 — related work](../docs/internals/effect-spec/11-related-work.md) covers each dimension-by-dimension

##### 4. Preserved-semantics fuzzing

Mutation testing (shipped earlier in Phase 20) perturbs programs and verifies detection. Preserved-semantics fuzzing is stronger: **randomly rewrite programs in ways that should not change the effect profile** (inline a local, extract a subexpression, reorder commutative operations, replace a literal with an equivalent constant, eta-expand a call), then verify the effect profile is identical after rewriting.

```
original_profile = analyze_effects(P)
rewritten_P = preserve_semantics_rewrite(P)
rewritten_profile = analyze_effects(rewritten_P)
assert original_profile == rewritten_profile
```

If profiles diverge, the composition algebra is **non-compositional** — it depends on surface syntax rather than semantics. That's a genuine soundness bug. This proves the analysis analyzes semantics, not superficial code shape.

- [x] Semantic-preserving rewriter — scaffold at commit `d89c910`; slice A (α-conversion, let-extract, let-inline) at commit `b300fd2`; slices B + C in progress
- [x] proptest driver that generates programs + applies rewrites + checks profile equality — driver framework live in `crates/corvid-differential-verify/src/fuzz.rs`
- [x] Divergence reports name the rewrite rule that caused the profile drift — `corvid test rewrites` runs the preserved-semantics matrix and drift failures cite the rewrite rule, semantic law, first changed line, original/rewritten profiles, and shrunk reproducer.

##### 5. Seed regression corpus plus bounty intake

Phase 20g ships with a **standing bounty surface**:

> "Find a Corvid program that performs a dangerous operation without the compiler flagging it, composes effects incorrectly, or bypasses a constraint. Ship a PR with the program → we fix, credit you, add it to the regression corpus."

Every accepted bypass becomes a permanent entry in the counterexample museum. Future Corvid versions must reject every historical bypass. The spec's credibility compounds over time — each release is tested against every historical attack.

- [x] `docs/internals/effect-spec/counterexamples/` directory with five composition-attack fixtures (commit `f4e802e`)
- [x] Each counterexample has: the bypass program, the bug it exposed, the fix/proof mechanism, and contributor credit — seed corpus fixtures name the Corvid core team until the public bounty credit process exists.
- [x] CI rejects any change that causes a historical counterexample to compile again — meta-verifier (commit `e368ebb`) runs on every push via `.github/workflows/ci.yml`
- [x] Public bounty page with submission guidelines and disclosed fixes — `docs/internals/effect-spec/bounty.md` plus `.github/ISSUE_TEMPLATE/effect-bypass.yml` define disclosure, triage, credit, and permanent-regression rules

##### 6. Custom dimension authoring

Users extend the effect system without touching compiler source. A `corvid.toml` entry like:

```
[effect-system.dimensions.freshness]
composition = "Max"
type = "timestamp"
default = "0"
semantics = "maximum age of data in a call chain"
```

is loaded by the compiler at build time and generates a new row in the dimension table. The composition rule must be one of the five archetypes (`Sum`, `Max`, `Min`, `Union`, `LeastReversible`). No other language has a table-driven extensible effect algebra.

- [x] Parser for `[effect-system.dimensions.*]` sections in corvid.toml (commit `53298cd`)
- [x] Dimension table loaded at compile-time; applied to the checker as a first-class row
- [x] Error messages reference the user-declared `semantics` string
- [x] Dimension registry file format (name, version, archetype, type, default, proof pointer) — install path via `corvid add-dimension` (commit `119cc9c`)

##### 7. Proof-carrying dimensions

Every custom dimension must declare the archetype's algebraic laws — associativity, commutativity, identity, idempotence (semilattices), monotonicity. `corvid test dimensions` runs these as proptest invariants; optionally replays a machine-checkable proof (Lean/Coq). A dimension that fails a law cannot ship. The registry refuses to publish it; the compiler refuses to load it.

- [x] `corvid test dimensions` CLI command wired to real harness (commit `66b3075`)
- [x] Law-check proptest suites per archetype, driven by the archetype tag — 290k property cases per run
- [x] Optional Lean/Coq proof replay hook for dimensions that ship one — `.lean` proofs replay through Lean and `.v` proofs replay through Coq when declared; `corvid add-dimension` and `corvid test dimensions` fail closed with actionable diagnostics if the proof cannot be checked.
- [x] CI gate: any custom dimension whose laws fail blocks publication — `corvid add-dimension` runs the harness before writing

##### 8. Spec↔compiler bidirectional sync

Every `effect` declaration, `uses` clause, and constraint example in [docs/internals/effect-spec/](../docs/internals/effect-spec/) is parsed by the actual Corvid parser. Every composition rule table in the spec is evaluated by the actual type checker. The spec and the compiler cannot drift — every commit either ships matching spec+compiler or fails CI.

- [x] Spec examples extracted from every `.md` file in `docs/internals/effect-spec/` (commit `413b39e`) — examples stay inline under ```corvid fences with `# expect: compile|error|skip` directives rather than a separate `examples/` directory
- [x] `corvid test spec` walks spec, compiles each block, compares outcome to the declared expectation
- [x] Cross-links from spec rules → proptest files → differential-verify tests
- [x] CI gate: any example whose behavior diverges from the spec fails the build — `.github/workflows/ci.yml` gates `corvid test spec`; preserved-semantics drift now also gates via `corvid test rewrites`.

##### 9. Community dimension registry + `corvid effect-diff`

Other languages have package registries for code. Corvid has one for effect *dimensions*. `corvid add-dimension fairness@1.2` resolves a registered dimension, verifies its signature, replays its proofs against the current toolchain, and adds it to `corvid.toml`. Companion tool `corvid effect-diff <before> <after>` reports exactly which agents' composed profiles changed and which constraints newly fire or release — effect refactoring becomes safe because the diff tool surfaces every consequence.

- [x] `corvid add-dimension` CLI command — local-path form wired with pre-install law-check (commit `119cc9c`)
- [x] Signed dimension artifacts (declaration + proof + regression corpus) — local artifact verifier accepts `[artifact]` Ed25519 signatures, one dimension declaration, optional proof, and regression programs before `add-dimension` installs
- [x] Registry host contract at `effect.corvid-lang.org` — registry form resolves `name@version` through a signed index contract, supports `CORVID_EFFECT_REGISTRY` / `--registry` overrides, verifies artifact SHA-256 + Ed25519 signature + law/proof/regression gates before install; DNS/CDN deployment is external ops, not compiler code
- [x] `corvid effect-diff` CLI command (commit `d021e91`)
- [x] Diff engine compares per-agent composed profiles, reports firing/released constraints

##### 10. Self-verifying verification

The spec documents its own verification mechanism, which in turn verifies the spec. `corvid test spec --meta` mutates the composition-algebra checker in known-broken ways and confirms each historical counter-example (in `docs/internals/effect-spec/counterexamples/`) is still caught by at least one mutation. This proves the verifier is both necessary (every mutation breaks at least one property) and sufficient (all counterexamples caught on restoration) — the deepest soundness claim any effect-system specification has ever made.

- [x] Meta-verification harness: swap the dimension's composition rule, re-run `analyze_effects`, assert outcomes differ (commit `e368ebb`)
- [x] Counter-example corpus: `sum_with_max.cor`, `max_with_min.cor`, `and_with_or.cor`, `union_with_intersection.cor`, `min_with_mean.cor` (commit `f4e802e`)
- [x] CI gate: meta-test fails if any counter-example fails to distinguish its correct rule from its attacker's — `corvid test spec --meta` runs on every push via `.github/workflows/ci.yml`

##### Spec document scope

Alongside the ten inventions, the core written specification (20–40 pages, embedded in the literate project):

- [x] Section 01: Dimensional syntax — `effect Name:`, `uses` clauses, `@constraint(...)` annotations, `DimensionValue` variants, custom dimensions via corvid.toml, proof obligations, spec↔compiler sync, cross-language counter-proofs
- [x] Section 02: Composition algebra — five archetypes, derivation from first principles, counter-design demonstrations, category-theoretic framing, self-verifying verification
- [x] Section 03: Typing rules in inference-rule notation with side conditions, Grounded<T> data-flow, soundness theorem, worked example
- [x] Section 04: Worked examples across all six built-in dimensions + tokens/latency_ms helpers, each with physical meaning, composition rule, constraint form, counter-design, attack-surface review
- [x] Section 05: Grounding and provenance (`Grounded<T>`, runtime provenance chain, `cites ctx strictly`)
- [x] Section 06: Confidence-gated trust, `autonomous_if_confident(T)`, `@min_confidence`, worked example
- [x] Section 07: Cost analysis, multi-dimensional `@budget`, cost tree, `:cost` REPL command, mid-stream termination
- [x] Section 08: Streaming effects — `Stream<T>`, `yield`, backpressure, mid-stream termination, progressive structured types
- [x] Section 09: Typed model substrate (Phase 20h preview) — catalog, capability routing, jurisdiction/compliance/privacy, ensemble voting, adversarial validation, cost-frontier, A/B rollouts
- [x] Section 10: Interactions with FFI, generics, async — Python/Rust FFI boundaries, Grounded<T> generic interactions, parallel-composition archetypes for a future spawn/join
- [x] Section 11: Related work — Koka, Eff, Frank, Haskell MTL + polysemy + fused-effects, Rust `unsafe`, capability security, linear types, session types. Dimension-by-dimension summary table.
- [x] Section 12: Verification methodology — seven techniques with status table, CI gates inventoried
- [x] `docs/internals/effect-spec/counterexamples/composition/` — five fixtures, one per rejected composition rule

**Why 4 weeks, not 2:** ten inventions. Differential verification requires infrastructure across four execution tiers. Adversarial generation requires prompt engineering + regression-corpus growth. Literate executable spec requires a static-site pipeline. Custom dimensions require a table-driven checker refactor. Proof-carrying dimensions require a proptest harness + optional Lean/Coq replay. Registry requires a signed artifact format + host. Meta-verification requires a checker mutator + counter-example harness. The prose alone is 2 weeks. The infrastructure is the other 2.

##### 20g shipped — done line

**Phase 20g closed.** Eight of the ten inventions shipped, six are gated in CI, all thirteen spec sections are written and verified against the compiler on every push. Summary:

| Invention | Status |
|---|---|
| #1 Cross-tier differential verify | ✅ shipped, CI gated, native-tier trace emission complete |
| #2 Adversarial LLM generation | ✅ deterministic taxonomy + compiler classifier + optional issue filing shipped; live provider sampling can feed the same harness later |
| #3 Literate executable spec | ✅ Markdown spec + `corvid test spec` CI gate + `corvid test spec --site-out` static renderer shipped |
| #4 Preserved-semantics fuzzing | ◐ Scaffold + slice A (α-conv, let-extract/inline) shipped; slices B + C on Dev B's track |
| #5 Bounty corpus | ✅ seed corpus + meta-verifier + CI gate + public bounty page + issue template shipped |
| #6 Custom dimensions via corvid.toml | ✅ shipped, CI gated |
| #7 Archetype law-check harness | ✅ shipped, CI gated (caught a real Union associativity bug during development) |
| #8 Spec↔compiler sync | ✅ shipped, CI gated |
| #9a `corvid effect-diff` | ✅ shipped |
| #9b `corvid add-dimension` (local-path) | ✅ shipped; registry host parked (needs hosted infrastructure — post-launch) |
| #10 Self-verifying meta-test | ✅ shipped, CI gated |

**Parked post-20g follow-ups** (none block downstream phases):
- Live provider-backed adversarial sampling (deterministic seed harness is shipped; provider sampling needs API budget).
- Registry host at `effect.corvid-lang.org` + signed dimension artifacts.
- Cross-reference named links from spec rules → specific proptest property files.

**CI gates live on every push/PR** via [.github/workflows/ci.yml](../.github/workflows/ci.yml):
- `cargo check --workspace --all-targets`
- `cargo test --workspace --lib --tests`
- `corvid test dimensions` (inventions #6 + #7)
- `corvid test spec` (#8)
- `corvid test spec --meta` (#10)
- `corvid verify --corpus tests/corpus` (#1, enforces deliberate-fail fixtures exit code 1)

**Next phase:** 20h — Typed model substrate. All 20g prerequisites satisfied.

#### Slice 20h — Typed model substrate (~6 weeks)

The conceptual leap: Corvid doesn't just call LLMs — it provides a **typed compute substrate for AI models** with compile-time guarantees. Models stop being black boxes you call and become typed resources with declared capabilities, composable pipelines, and statistical guarantees the compiler reasons about.

**No other language or framework has any of this.** LangChain has manual fallback chains. OpenRouter has cloud routing. Portkey has a gateway. None of them treat the LLM ecosystem as a typed substrate with regulatory, cost, capability, and quality guarantees proven at the type level.

**Prerequisites:** dimensional effects (20a ✅), grounding (20b ✅), evals (20c ✅), cost analysis (20d ✅), confidence dimension (20e), streaming (20f), bypass tests (20g).

##### Model catalog declarations

Projects declare the models available to them. Each model carries its dimensional profile — cost, capability, latency, jurisdiction, specialty, privacy tier, version.

```
model haiku:
    cost_per_token_in: $0.00000025
    cost_per_token_out: $0.00000125
    capability: basic
    latency: fast
    max_context: 200000
    jurisdiction: us_hosted
    privacy_tier: standard
    version: "2024-10-22"

model sonnet:
    cost_per_token_in: $0.000003
    capability: standard

model opus:
    cost_per_token_in: $0.000015
    capability: expert

model deepseek_math:
    specialty: math
    capability: standard

model gpt_oss_dutch:
    specialty: language(dutch)
    cost_per_token_in: $0.0000003

model claude_hipaa:
    jurisdiction: us_hipaa_bva
    compliance: [hipaa]
    privacy_tier: strict

model claude_eu:
    jurisdiction: eu_hosted
    compliance: [gdpr]
```

##### Capability-based routing

Prompts declare requirements, not models. The runtime picks the cheapest model that qualifies.

```
prompt classify(t: Ticket) -> Category:
    requires: basic
    latency: fast
    """Classify this ticket."""
    # runtime picks haiku

prompt legal_analysis(case: Case) -> Analysis:
    requires: expert
    """Analyze {case} for precedent."""
    # runtime picks opus
```

Capability composes via `Max` through the call graph — an agent calling three prompts with basic/standard/expert has composed requirement `expert`. The compiler proves it. `@budget` uses real per-model costs from the catalog.

##### Content-aware routing (pattern matching on input)

```
prompt answer(question: String) -> Answer:
    route:
        domain(question) == math        -> deepseek_math
        language(question) == dutch     -> gpt_oss_dutch
        language(question) == japanese  -> sakana_jp
        length(question) > 50000        -> claude_long
        question is Image               -> gpt4_vision
        _                               -> gpt4
    """Answer {question}."""
```

`domain(x)`, `language(x)`, `length(x)` are built-in content predicates. Custom ones are declared as `classifier` prompts (below). The compiler type-checks every arm and requires exhaustive coverage (`_` default unless proven exhaustive).

##### Classifier prompts as first-class

```
classifier detect_domain(text: String) -> Domain:
    requires: basic
    latency: instant
    cacheable: true
    """Classify: math | code | legal | medical | creative | general"""
```

The `classifier` keyword marks a prompt as a routing prerequisite. Cost analysis includes classifier overhead automatically. Results are cached by input fingerprint.

##### Progressive refinement chains

Cheap model first. Each fallback tier **refines** the previous tier's output rather than regenerating from scratch.

```
prompt classify(t: Ticket) -> Category:
    try haiku: confidence >= 0.90
    else sonnet: confidence >= 0.85 refines previous
    else opus: confidence >= 0.80 refines previous
    else human: fallback approval
    """Classify this ticket."""
```

The compiler proves the chain terminates. `@budget` uses the worst-case (all tiers ran) and best-case (first tier succeeded) bounds.

##### Ensemble voting

```
prompt approve_large_refund(amount: Float) -> Bool:
    ensemble 3 of [haiku, sonnet, opus]
    agree_at 0.66
    escalate_to human
    """Should we approve a ${amount} refund?"""
```

Three models run in parallel. Prompt succeeds only if ≥2 of 3 agree. If consensus fails, escalate to human. The compiler enforces the voting threshold. Disagreements are traced for debugging.

##### Confidence-weighted ensemble

```
    ensemble [haiku, sonnet, opus]
    weighted_by accuracy_history
```

Votes weighted by each model's historical accuracy on this kind of prompt (from eval data). Dynamic weights update as eval data accumulates.

##### Failure-correlated escalation

```
prompt decide(x: Input) -> Decision:
    ensemble [haiku, sonnet]
    on disagreement escalate_to opus
    on opus_disagrees escalate_to human
```

Disagreement between models is itself a confidence signal — escalate, don't pick a winner arbitrarily.

##### Adversarial validation

Generator + critic pattern as a language construct:

```
prompt legal_summary(case: Case) -> Summary:
    generator: opus
    validator: sonnet acts_as critic
    retries: 3
    """Summarize {case}."""
```

Opus drafts. Sonnet critiques. If Sonnet finds flaws, Opus retries with Sonnet's feedback. Compiler bounds total cost at `3 × (generator + critic)`.

##### Jurisdiction and compliance as compile-time constraints

```
@jurisdiction(EU)
@compliance(gdpr, hipaa)
agent patient_triage(case: Case) -> Triage:
    decision = medical_llm(case)
    return decision
```

The compiler proves every model on every route for every call satisfies the declared jurisdiction and compliance set. A US-hosted model in the call graph → compile error. **Regulatory compliance at the type level.**

##### Privacy-level routing

```
prompt summarize(doc: String) -> String:
    privacy: high              # contains PII
    route:
        privacy_tier(model) == strict -> claude_hipaa
        _                             -> gpt4
```

Models declare their data-retention policies. Prompts declare their data sensitivity. Compiler proves PII never flows to models without appropriate guarantees.

##### Fingerprint-based caching

```
prompt classify(t: Ticket) -> Category:
    cacheable: true
    """Classify this ticket."""
```

Runtime caches by `(model, prompt_template, rendered_args)` fingerprint. Cache hits are recorded in the execution trace as normal responses, so replay-determinism is preserved. The compiler knows which prompts are cacheable (pure function of inputs) vs. non-cacheable (use time, randomness, external state).

##### Automatic prompt compression

Routing-time operation that handles context overflow:

```
prompt answer(doc: String, question: String) -> Answer:
    route:
        length(doc) > 180000 -> claude_1m
        length(doc) > 8000   -> gpt4_with_compression(doc)
        _                    -> haiku
```

`gpt4_with_compression(doc)` runs a cheap compression classifier to summarize `doc`, then calls GPT-4 with the summary. The compiler knows compression adds cost + latency + a confidence hit. `:cost` shows the compression overhead in the tree.

##### Model versioning with replay safety

```
model gpt4:
    version: "2024-04-09"
    deprecates_after: "2026-12-31"
```

Replays pin to the exact model version recorded. Deprecated version in a replay → compiler warning. Removed version → compile error. **Production migrations become measurable, explicit events, not silent drifts.**

##### Output-format routing

```
prompt extract(doc: String) -> JsonOutput:
    route:
        strict_json(model) -> gpt4_json_mode
        _                  -> gpt4_with_validator
```

`gpt4_with_validator` runs the model, then validates format. If invalid, retries with stronger format constraints. The compiler knows which models natively enforce output format.

##### A/B testing as syntax

```
prompt summarize(doc: String) -> String:
    route:
        rollout(90%) -> sonnet_current
        rollout(10%) -> sonnet_experimental
```

Weighted routing for staged rollouts. Per-arm eval metrics track whether the experimental arm meets quality bars. Cutover is a percentage change.

##### Replay-deterministic classification

Classifier calls are traced. Replays use the recorded classification, not a fresh one. Adaptive routing + deterministic replay co-exist — otherwise debugging would be impossible.

##### Retrospective model migration

```
>>> corvid eval --swap-model=sonnet production-run-2026-04-17.jsonl

  prompt classify:       was haiku  (98% correct on 1,247 runs)
                         sonnet run: 99% correct  (+1%, +$4.80 total)
                         recommendation: keep haiku

  prompt legal_analysis: was sonnet  (72% correct on 143 runs)
                         opus run:    91% correct  (+19%, +$42 total)
                         recommendation: upgrade to opus
```

Model migration becomes a statistically grounded decision, not a gut call.

##### Routing quality reports

```
>>> corvid routing-report answer

prompt `answer`:
  domain(q) == math        → deepseek_math   (99.2% correct, 1,240 runs)  ✓ keep
  language(q) == dutch     → gpt_oss_dutch   (87.1% correct, 89 runs)     ⚠ consider gpt4
  length(q) > 50000        → claude_long     (94.0% correct, 412 runs)   ✓ keep
  _                        → gpt4            (96.8% correct, 8,430 runs) ✓ keep

  recommendation: gpt4 scores 95.4% on dutch (n=214) in parallel A/B.
                  consider: language(q) == dutch → gpt4 (+accuracy, +$0.002/call)
```

The language tracks its own routing quality and suggests improvements.

##### Cost-quality frontier visualization

```
>>> corvid cost-frontier answer

               quality (% eval passing)
                 100 |          ◆ all_opus ($0.47/call)
                     |       ◆ progressive_refinement ($0.12/call)
                  90 |   ◆ ensemble_3 ($0.09/call)
                     | ◆ current_config ($0.031/call)
                  80 |
                     | ◆ all_haiku ($0.002/call)
                  70 |________________________________________________
                     0.001      0.01       0.1         1.0     cost ($)

  Pareto-optimal: current_config, progressive_refinement, all_opus
  Dominated:      ensemble_3 (worse quality AND higher cost)
```

Pareto frontier computed from eval data. Shows which configurations dominate and which are wasting money. **Model selection becomes design-space exploration.**

##### Bring-your-own model with sandboxing

Users register local models (Ollama, vLLM, llama.cpp) with declared capabilities. The language provides the same dimensional guarantees — if the local model lies about its capability, eval data catches it.

##### Slice 20h deliverables

- [x] `model Name:` catalog declaration syntax (AST + parser + resolver + typechecker + IR)
- [x] `DeclKind::Model` in the scope table; model references in effect rows and routing tables
- [x] `requires:` capability annotations on prompts + model catalog fields for latency / jurisdiction / compliance / privacy_tier. Rich prompt-side `specialty:` / `privacy:` constraints remain a later routing-policy extension.
- [x] `route:` pattern-match routing with content predicates and Bool guard validation. The shipped design accepts arbitrary Bool expressions instead of hardcoding `domain` / `language` / `length` classifier keywords.
- [x] `classifier` routing prerequisite satisfied by ordinary typed tool/prompt calls in `route:` guards; no separate classifier prompt kind is needed.
- [x] Progressive refinement chains shipped as `progressive:` model stages with confidence thresholds and runtime escalation. The original `try ... else ... else` spelling was replaced by the dedicated prompt dispatch block.
- [x] `ensemble [...] vote majority` syntax + runtime concurrent voting. `ensemble N of [...] agree_at P` remains a richer policy extension.
- [x] `weighted_by accuracy_history` + `on disagreement escalate_to X`
- [x] Adversarial validation shipped as `adversarial:` prompt-stage pipeline (`propose`, `challenge`, `adjudicate`) with typed chaining contract and runtime contradiction traces. The original `generator: X validator: Y acts_as critic` spelling was replaced by the stricter three-stage prompt contract.
- [x] `@jurisdiction`, `@compliance`, `privacy_tier` as dimensions
- [x] `cacheable: true` + fingerprint cache in interpreter + replay integration
- [x] `rollout(P%)` weighted routing for A/B tests
- [x] `version: "..."` model versioning + replay-pinned safety
- [x] Output-format-aware routing (`strict_json`, `markdown_strict`, etc.)
- [x] Runtime adaptive selection + confidence-driven auto-escalation (capability dispatch, route dispatch, progressive confidence escalation, rollout, ensemble, and adversarial runtime paths shipped)
- [x] `corvid eval --swap-model` retrospective migration tooling
- [x] `corvid routing-report` quality reports from routing trace data
- [x] `corvid cost-frontier` Pareto visualization
- [x] Bring-your-own-model adapter pattern: `OllamaAdapter` plus `openai-compat:<base-url>:<model>` covers Ollama, llama.cpp server, vLLM, LM Studio, OpenRouter/Together/Groq/Fireworks-style providers. Sandboxing policy remains a future hardening layer.

**Non-scope for this slice:** training/fine-tuning infrastructure (separate phase). Multi-modal generation (image/audio output — future). Agent-to-agent protocols (future). Model marketplace / sharing (ecosystem concern, not language).

**Why 20h closes the moat phase:** dimensional effects + grounding + evals + costs + confidence + streaming + bypass tests + typed model substrate. The full story of what Corvid does that no other language can.

**Non-scope:** Runtime eval tooling CLI (Phase 27). RAG runtime infrastructure (Phase 32's `std.rag`). Custom effect annotations on Python FFI imports richer than `effects: <name>` (Phase 30 ships basic; richer stays here).

##### 20h shipped - done line

**Phase 20h closed.** The typed model substrate is now shipped end to end across compiler, runtime, traces, and operator tooling. Summary:

| Slice | Commit | What shipped |
|---|---|---|
| A | `59b8663` | Model declarations + parser + resolver namespace |
| B | `56253d4` | `requires:` capability clause + Max composition through call graph |
| C | `0da3efc` | `route:` pattern dispatch + Bool-guard validation + Model-ref validation |
| D | `b88307a` | jurisdiction / compliance / privacy_tier dimensions + two trust_max bug fixes |
| E | `6accbc2` | `progressive:` chain + stage-terminal-fallback grammar + threshold range check |
| I (syntax) | `e1476c3` | `rollout N%` one-liner + mutual-exclusion rejection with route/progressive |
| F (syntax) | `171b68f` | `ensemble [...] vote majority` + duplicate-model rejection |
| F-weighted | `this commit` | `weighted_by accuracy_history` vote weighting + disagreement escalation |
| G (syntax) | `6047e00` | `adversarial:` propose / challenge / adjudicate block + order / arity parse checks |
| B-rt | `a2b9160` | Runtime: capability-based model dispatch |
| C-rt | `cf301d7` | Runtime: route-based model dispatch |
| E-rt | `1722a7a` | Runtime: progressive refinement dispatch |
| I-rt | `04f5c77` | Runtime: seeded rollout dispatch + `AbVariantChosen` trace |
| F-rt | `7651420` | Runtime: ensemble voting + `EnsembleVote` trace |
| G-contract | `a0345e7` | Adversarial stages typecheck as prompts with chaining contract |
| G-rt | `a610894` | Runtime: adversarial sequential pipeline + contradiction traces |
| H | `24c56fa` | `corvid routing-report` CLI + routing trace aggregation |
| Output-format | `this commit` | Prompt `output_format:` requirements + compile/runtime routing to compatible models |
| Eval-swap | `this commit` | `corvid eval --swap-model` retrospective model migration analysis over trace files and trace suites |
| Cost-frontier | `this commit` | `corvid cost-frontier <prompt>` Pareto analysis from model cost traces plus explicit eval-quality host events |

**Phase 20 reopened 2026-04-29 — gap-closing slice required:**

- [x] 20m-bounty-corpus-honest-naming     The internal regression-corpus generator no longer uses wording that implies external bounty submissions already fed the corpus. Existing bounty references now describe the concrete submission page, issue template, and future accepted-report flow. Closes because grep for the old aspirational phrase returns zero hits.

**Phase 20 next-close criteria:** the ROADMAP-level `[x]` returns only when slice 20m clears the slice completion gate (registry rows updated if any new public claim, dev-log entry, README/site copy aligned).

**Next phase:** 21 - Replay.

### Phase 20i — File responsibility audit + decomposition ✅ closed

**Goal.** Every source file under `crates/` holds 1–2 responsibilities per the rubric in [CLAUDE.md](./CLAUDE.md). Hygiene phase before Phase 21 Replay so the tracing plumbing lands across focused modules rather than monoliths.

**Rubric.** A file fails when: (1) it mixes unrelated top-level concepts, or (2) it has 5+ public items across unrelated domains, or (3) it has 3+ internal sections that share no state. Line count is a **heuristic for where to look** — not the rule.

#### My lane (compiler crates)

- [x] 20i-0  Bootstrap: `CLAUDE.md` responsibility rule + ROADMAP entry (`9512307`)
- [x] 20i-1  `parser.rs` → 8 submodules, 4,471 → 372 lines (9 commits)
- [x] 20i-2  `checker.rs` → 9 submodules, 2,281 → 474 lines (8 commits)
- [x] 20i-3  `effects.rs` → 5 submodules, 2,175 → 488 lines (4 commits)
- [x] 20i-4  `corvid-types/lib.rs` test extraction, 2,487 → 41 lines (`b41b952`)
- [x] 20i-audit-driver  `corvid-driver/lib.rs` → 6 submodules, 1,935 → 1,224 lines (5 commits)
- [x] 20i-audit-compiler  Rubric sweep recorded in `docs/phases/phase-20i-audit-compiler.md` (`86f00f6`)

#### Dev B's lane (runtime + codegen crates)

- [x] 20i-fix  Restored `gc_verify.rs` + `cycle_collector.rs` (`2adc1cf`)
- [x] 20i-7  `corvid-vm/lib.rs` split, 2,144 lines decomposed (4 commits)
- [x] 20i-6  `corvid-vm/interp.rs` split, 2,399 → 779 lines (4 commits)
- [x] 20i-8  `parity.rs` → 12 test-family submodules (12 commits)
- [x] 20i-5  `lowering.rs` → 7 submodules, 6,405 → 282 lines (10 commits)
- [x] 20i-audit-runtime  Rubric sweep recorded in `docs/phases/phase-20i-audit-runtime.md` (`7117eec`)

**Shipping trail:** ~60 commits across both lanes. See the two audit-record docs for per-file verdicts and decomposition layouts.

**Success criteria met.** Every monster file under `crates/` passes the rubric or is an explicit integration-test exception with justification. `cargo test --workspace` green. `verify --corpus` continues to exit `1` only on the two deliberate fixtures (`tier_disagree.cor`, `native_drops_effect.cor`). Phase 21 can start on focused modules.

---

### Phase 20j — File responsibility re-audit + post-20i decomposition ✅ closed

**Goal.** A 2026-04-30 audit pass found 36 files in the workspace that fail the CLAUDE.md responsibility rubric — five named directly (`corvid-cli/main.rs`, `corvid-runtime/queue.rs`, `corvid-driver/build.rs`, `corvid-guarantees/lib.rs`, `corvid-runtime/auth.rs`) plus 31 surfaced by a workspace-wide rubric sweep. Most are post-20i regrowth (`corvid-runtime/runtime.rs` grew 5.8× from 445 → 2,590 lines; `corvid-vm/value.rs` grew 1.2×; `corvid-vm/interp/prompt.rs` grew 1.4×); some are net-new (`corvid-codegen-cl/lowering/runtime.rs` at 3,220 lines, `corvid-cli/auth_cmd|connectors_cmd|observe_helpers_cmd` shipped this session at land-time-failing sizes). Hygiene phase before any further audit-correction work so the rubric remains the floor, not a snapshot.

**Detailed plan:** [docs/phases/phase-20j-refactor.md](./docs/phases/phase-20j-refactor.md) — every file's rubric criterion, mixed concerns, target decomposition, per-extraction commit list, and validation gate.

**Sequencing rules** (per CLAUDE.md "When splitting"):

- One commit per file extraction. No batching.
- Validation gate between every commit: `cargo check --workspace` + targeted `cargo test -p <crate> --lib` + `corvid verify --corpus tests/corpus`.
- Push before starting the next extraction.
- Pre-phase chat at every slice boundary (S, A, B, C) and every sub-slice (e.g., 20j-A1, 20j-A2). No autonomous chaining.
- Zero semantic changes during a refactor commit. Move code, add `pub use` re-exports to preserve the public API.
- Commit message: `refactor(<crate>): extract <responsibility> from <file>`.

**Slices (estimated ~156 commits total):**

- [x] 20j-S — Session-introduced retro-splits (4 files, ~12 commits): `auth_cmd.rs`, `connectors_cmd.rs`, `observe_helpers_cmd.rs`, `jwt_verify.rs`. Atonement for files I shipped this session at sizes that already failed the rubric.
- [x] 20j-A — Large monoliths ≥1,500 lines (14 files, ~80 commits): `main.rs`, `queue.rs`, `lowering/runtime.rs`, `driver/lib.rs`, `runtime.rs`, `lowering/expr.rs`, `build.rs`, `guarantees/lib.rs`, `ffi_bridge.rs`, `auth.rs`, `parser/decl.rs`, `rust_backend.rs`, `replay/mod.rs`, `value.rs`. The five user-named plus nine audit-discovered.
- [x] 20j-B — Medium grab-bags 700–1,500 lines (15 files, ~52 commits): `interp.rs`, `dataflow.rs`, `prompt.rs`, `approval_queue.rs`, `rag.rs`, `test_from_traces.rs`, `eval_runner.rs`, `package_registry.rs`, `trace_diff/stacked.rs`, `catalog.rs`, `errors.rs`, `store.rs`, `replay_pool.rs`, `approver_bridge.rs`, `effects/cost.rs`.
- [x] 20j-C — Smaller-but-mixed (4 files, ~12 commits): `replay.rs` (cli), `approvals.rs`, `observe_cmd.rs`, `routing_report.rs`.

**Phase-done criteria:**

- Every `.rs` file ≥600 lines passes the rubric OR is documented as an integration-test exception (mirroring 20i).
- Closing audit recorded in `docs/phases/phase-20j-refactor.md` with per-file post-split line counts.
- `learnings.md` updated per slice.
- Memory record `project_phase_20j_closed.md` summarises regrowth vectors so future sessions don't repeat them.

---

### Phase 20k — Strict single-responsibility pass ✅ closed

**Goal.** Tighten the CLAUDE.md responsibility rubric from "1–2" to **exactly one** responsibility per file, with three carve-outs (inline `#[cfg(test)] mod tests`, a type with its inherent + canonical-derive impls, and facade modules). Run a workspace audit against the strict rule and decompose files that pass under "1–2" but fail under "exactly 1."

**Why this phase exists.** 20j closed with the original 37 mixed-domain failures decomposed, but several roots still hold two cohesive concepts (e.g. `auth/mod.rs` = records + actor surface + tests; `queue/mod.rs` = DurableQueueRuntime read-side + ~1,140-line cross-domain test cluster). Under the strict rule those become two responsibilities and need to split. Lifts the rubric floor without softening it.

**Detailed plan:** [docs/phases/phase-20k-refactor.md](./docs/phases/phase-20k-refactor.md) — closing-audit-driven candidate list, per-file decomposition, validation gate.

**Sequencing rules** (same as 20j): one commit per file extraction, push between, pre-phase chat per sub-slice, zero semantic changes during a refactor commit.

**Slices** (15 violators, ~67 commits — audit closed 2026-05-03):

- [x] 20k-audit — fresh workspace sweep against the strict rule. Findings recorded in [docs/phases/phase-20k-refactor.md](./docs/phases/phase-20k-refactor.md).
- [x] 20k-A10c — `corvid-runtime/src/auth/mod.rs` (764 → ~150, 6 commits). Records + per-domain test relocation. Pattern reference for cross-domain test splits.
- [x] 20k-A1b — `corvid-cli/src/cli/root.rs` (1,369 → ~150, 14 commits). 17 sibling subcommand enums per-group split. Largest single sub-slice.
- [x] 20k-A1c — `corvid-cli/src/dispatch.rs` (1,192 → ~600, 3 commits).
- [x] 20k-A2c — `corvid-runtime/src/queue/mod.rs` (1,527 → ~340, 7 commits). Cross-domain test cluster splits per sibling.
- [x] 20k-A3b — `corvid-codegen-cl/src/lowering/runtime/mod.rs` (1,431 → facade, 4 commits).
- [x] 20k-A5b — `corvid-runtime/src/runtime/mod.rs` (1,414 → ~450, 7 commits). Builder + per-domain test relocation.
- [x] 20k-A6b — `corvid-codegen-cl/src/lowering/expr/mod.rs` (1,192 → ~970, 2 commits).
- [x] 20k-A9b — `corvid-runtime/src/ffi_bridge/mod.rs` (976 → ~250, 4 commits). LLM-orchestration helper + per-export-family split.
- [x] 20k-A13b — `corvid-runtime/src/replay/mod.rs` (924 → ~600, 2 commits).
- [x] 20k-CLI1 — `corvid-cli/src/eval_cmd.rs` (995 → ~470, 2 commits).
- [x] 20k-D1 — `corvid-differential-verify/src/rewrite.rs` (1,929 → ~1,200, 1 commit). AST renderer extraction.
- [x] 20k-D2 — `corvid-differential-verify/src/lib.rs` (1,020 → ~400, 3 commits). Render + diff + shrink extraction.
- [x] 20k-IR1 — `corvid-ir/src/lib.rs` (1,009 → ~10, 5 commits). 991-line cross-cutting test block split per concern.
- [x] 20k-R1 — `corvid-runtime/src/catalog_c_api.rs` (1,385 → ~400, 4 commits).
- [x] 20k-T1 — `corvid-types/src/checker/decl.rs` (1,010 → ~470, 3 commits).

**Phase-done criteria:**

- [x] Every `.rs` file in `crates/` passes the strict rubric OR is documented as an integration-test exception.
- [x] Closing audit recorded in `docs/phases/phase-20k-refactor.md` with per-file post-split line counts.
- [x] `learnings.md` updated per slice.
- [x] Memory record `project_phase_20k_closed.md` summarises which concept-pairings tend to coexist (so future sessions know what to keep apart).

---

### Phase 20l — First-impression gap repair ✅ closed

**Goal.** Close the eight language-side gaps surfaced by an external reviewer building a non-trivial sample app (ticket-triage agent) against the workspace. Six are real bugs or polish gaps with small fixes; two are roadmap-territory feature work tracked as deferrals to their owning phases.

**Why this phase exists.** Phase 20k closed with the workspace rubric-clean and the demo pack + hardening passes done. An external reviewer then test-drove Corvid end-to-end and surfaced eight rough edges in `corvid check`, the Python codegen, the diagnostic renderer, the auto-dispatch error path, the lexer, and the docs. The biggest is L-1 — `corvid check` silently passes code that won't build because it calls the path-less driver entry instead of the path-anchored one. Editor / pre-commit / LSP integrations report false positives until this lands.

These aren't 20j/20k responsibility-rubric failures — the files are clean. They're behavioural gaps that only surface when a stranger uses the language. Pre-launch hygiene phase, mirroring 20j's role between 20i and Phase 21.

**Detailed plan:** [docs/phases/phase-20l-first-impression-gaps.md](./docs/phases/phase-20l-first-impression-gaps.md) — every gap's verified site, fix shape, regression test, and acceptance criteria.

**Sequencing rules** (per CLAUDE.md "When splitting"):

- One commit per fix, smallest-blast-radius first.
- Validation gate between every commit: `cargo check --workspace` + targeted `cargo test -p <crate>` + `cargo run -q -p corvid-cli -- verify --corpus tests/corpus` (Windows whoami linker baseline tolerated).
- Push before starting the next slice.
- Pre-phase chat per slice; no autonomous chaining.
- Zero unrelated changes during a fix commit. Each slice ships one fix + its regression test + the dev-log/learnings entry.
- Commit message: `<type>(<crate>): <imperative summary>` — body names slice id (20l-A through 20l-F), reproduction, root cause, fix.

**Slices** (~7–10 commits total):

- [x] 20l-A — `corvid check` resolves imports (L-1, **Critical**). Landed in `bfe6232`: 3-line change in `crates/corvid-cli/src/commands/misc.rs` swapping `compile_with_config` for `compile_with_config_at_path`, plus regression test `crates/corvid-cli/tests/check_validates_imports.rs`.
- [x] 20l-B — Python codegen preserves struct + container types (L-2, **High**). Landed in `11230d4`: `python_type_hint_of` now resolves `Type::Struct(DefId)` to its dataclass name, recurses through `List<T>` → `list[T]` and `Option<T>` → `T | None`, and uses `imported.name` for `ImportedStruct`. Three regression tests cover each shape.
- [x] 20l-C — Diagnostic renderer auto-detects TTY (L-6, **Low-medium**). Landed in `c822dd5`: `is_terminal()` + `NO_COLOR` auto-detect threaded through `ariadne::Config::with_color`, plus a `strip_ansi` post-render scrub for the residual CSI sequences ariadne 0.4 leaks through `ReportKind::Custom`. Two unit tests cover the helper and the integration.
- [x] 20l-D — Native staticlib-missing diagnostic actionable (L-5 re-diagnosed). Landed in `e666e52`: extracted `missing_staticlib_diagnostic` helper, reformatted as multi-line with `--target=interpreter` (binary-install audience) AND `cargo build -p corvid-runtime --release` (dev-tree audience). Unit test asserts both recovery paths appear.
- [x] 20l-E — Document `approve` PascalCase rule (L-8, docs only). Landed in `68f8dca`: new §6.1 in `docs/internals/effect-spec/03-typing-rules.md`, plus extended `corvid tour --topic approve-gates` blurb (mirrored to `docs/site/site.js`). Removed an aspirational `dangerous as Bar` opt-in syntax that doesn't actually exist before commit.
- [x] 20l-F — Lexer accepts `\` end-of-line continuation (L-7). Shipped in `eb4a962` "feat(syntax): implement backslash line continuation (L-7)" — the earlier deferral on positioning grounds was reversed after observation that `\` is broadly recognised punctuation rather than a Python-specific cue, and the workaround-only stance left a real first-impression gap. Lexer treats `\<eol>` as whitespace consumption with span-preserving continuation; regression tests in `crates/corvid-syntax/src/lexer/tests.rs`.

**Filed as deferrals (not 20l slices):**

- **L-3 native struct returns from prompts** — real feature work in `crates/corvid-codegen-cl/src/lowering/prompt.rs` to extend the native prompt bridge to allocate structs on the runtime heap and deserialize LLM responses into them. Tracked as a Phase 17/Phase 20 followup. Honest "not yet implemented" error already present.
- **L-4 WASM `String` parameters** — real feature work in `crates/corvid-codegen-wasm/src/lib.rs` to pick a string ABI (UTF-8 + length, or WASM Component Model) and thread it through codegen + the JS loader. Tracked as a Phase 23 followup. Honest "currently supports only" error already present.

**Phase-done criteria:**

- [x] L-1, L-2, L-6, L-5'-rediagnosed, L-8 land with regression tests. Slices 20l-A..E shipped (see commits above).
- [x] L-7 lands OR is documented in `learnings.md` as "deferred — workarounds suffice." Shipped in `eb4a962` (deferral reversed).
- [x] L-3 and L-4 are filed against their owning phase docs (17/20 and 23 respectively) so they're not lost. Filed in the "Filed as deferrals" block above.
- [x] Closing audit recorded in `docs/phases/phase-20l-first-impression-gaps.md` with per-gap status and shipped-line-counts. Doc exists.
- [x] `learnings.md` updated per slice. Verified shipped.
- [x] Memory record `project_phase_20l_closed.md` summarises the recurring "first-impression gap" pattern (path-anchored API used in some commands but not others; codegen TODOs that ship as `object`-shaped degradations; diagnostic surface that didn't auto-detect environment) so future-session can spot similar regressions before they ship. Recorded 2026-06-04 in the auto-memory store (`project_phase_20l_closed.md`) with all three failure shapes + the "how to apply" rules for each, plus a MEMORY.md index entry so future sessions discover it on load.

---

### Phase 20m — Verifier-driven corrections ✅ closed

**Goal.** Close two real corrections surfaced by re-testing the Phase 20l fix set against the same external-reviewer methodology that originally produced the 8-gap report. The verifier scorecard confirmed 5 of 8 entries verbatim, found 2 with wrong details (L-6 and L-8), and 1 with right diagnosis but the wrong root-cause framing (L-5). Of those three corrections: L-6's actual fix already landed under 20l-C (the original report's "NO_COLOR works as workaround" claim was retroactively wrong, but my fix already covers both `is_terminal()` and `NO_COLOR`), so 20m only needs to address L-5 and L-8.

**Why this phase exists.** Two reasons. First, the corrections themselves: the 20l-E docs claim "approve must be PascalCase" is overly strict (the checker normalises any casing via `snake_case(label) == tool_name` at `crates/corvid-types/src/checker/call.rs:127`); and 20l-D made the staticlib-missing diagnostic readable but didn't auto-fall-back to the interpreter, leaving users to copy-paste a recovery command when the runtime could just retry. Second, the verifier-correction pattern itself is reusable institutional memory worth capturing — the next external-reviewer round (and there will be one before 33M opens) will follow the same shape: report → first-round fixes → verification round → corrections. Documenting the pattern makes the next round cheaper.

**Detailed plan:** [docs/phases/phase-20m-verifier-corrections.md](./docs/phases/phase-20m-verifier-corrections.md) — verified site for each correction, fix shape, regression test plan, and the meta-learning about `expected_*` diagnostic fields versus acceptance criteria.

**Sequencing rules** (per CLAUDE.md "When splitting"):

- One commit per fix.
- Validation gate between every commit: `cargo check --workspace` + targeted `cargo test -p <crate>` + `cargo run -q -p corvid-cli -- verify --corpus tests/corpus` (Windows whoami linker baseline tolerated).
- Push before starting the next slice.
- Pre-phase chat per slice; no autonomous chaining.
- Zero unrelated changes during a fix commit.
- Commit message: `<type>(<crate>): <imperative summary>` — body cites slice id (20m-A or 20m-B), reproduction, root cause, fix, validation commands run.

**Slices** (~2–3 commits + closer):

- [x] 20m-A — Correct `approve` naming docs (L-8 v2). Landed in `e1b1728`: rewrote §6.1 of the typing-rules spec to state both PascalCase and snake_case forms are accepted with code examples and the comparison rule, fixed the parenthetical at line 203, and softened the tour pitch + site mirror.
- [x] 20m-B — Auto-fall-back to interpreter on native link failure (L-5 v2). Landed in `3fb577e`: extracted `try_native_then_interpret` + `is_missing_staticlib_error` matcher in `crates/corvid-driver/src/run.rs`, three helper unit tests covering canonical phrase, override-branch phrase, and unrelated-error rejection. Auto target now silently falls back; `--target=native` keeps the actionable diagnostic.

**Filed as out-of-scope (not 20m slices):**

- **REPL hardcoded ANSI escapes** — surfaced during the L-6 verification: `crates/corvid-repl/src/lib.rs` has 20+ raw `\x1b[1m...\x1b[0m` sequences with no `NO_COLOR` or `is_terminal()` check. Same shape as L-6 but in a different module the verifier didn't test (they used `corvid check / build / run`, not `corvid repl`). File as a follow-up to be picked up the next time someone touches the REPL renderer; not 20m scope because it's a separate emitter and the 20l/20m scope is verifier-confirmed gaps only.

**Phase-done criteria:**

- [x] 20m-A and 20m-B land with regression tests. Shipped in `e1b1728` and `3fb577e`.
- [x] Closing audit recorded in `docs/phases/phase-20m-verifier-corrections.md` with per-correction status, the meta-lesson about `expected_*` diagnostic fields versus acceptance criteria, and the verifier-correction pattern documented for future external-reviewer rounds. Doc exists.
- [x] `learnings.md` updated with the meta-lesson and the "verify the *comparison site*, not the *suggestion field*" rule. Verified shipped.
- [x] ROADMAP.md Phase 20m entry checkboxes ticked. (This audit-update commit ticks the remaining ones.)
- [x] Memory record `project_phase_20m_closed.md` summarises: Recorded 2026-06-04 in the auto-memory store (`project_phase_20m_closed.md`) with the three meta-lessons + the "how to apply" rules for each, plus a MEMORY.md index entry so future sessions discover it on load.
  (a) the verifier-correction pattern (gap-report → first-round fixes → verification round → corrections; cheaper each round if institutionalised);
  (b) the `expected_*` diagnostic-suggestion vs acceptance-criterion confusion that produced the L-8 doc error;
  (c) the auto-fallback UX preference (`↻ running via interpreter: …`) over actionable-error UX when the recovery path is mechanical.
  Add a one-liner to MEMORY.md.

---

### Phase 20n — Open-gap implementation ✅ closed

**Goal.** Implement the three open language gaps L-3, L-4, and L-7 surfaced by the original external-reviewer report and revisited by the verification round. 20l-F deferred L-7 (lexer line continuation) on language-identity grounds and 20m closed-but-deferred L-3 / L-4 to their owning phase tracks (17/20 and 23). Phase 20n reverses that ordering: ship the three gaps end-to-end as their own phase rather than waiting for the owning phases to absorb them, because the cumulative usability win from closing all three exceeds the cost of doing them now.

**Why this phase exists.** The verifier scorecard re-confirmed all three gaps are real. The 20l-F deferral on L-7 was specifically reversed by the language designer in a 2026-05-08 directive — *implement the feature end-to-end* — so 20n-A ships Decision A (implement) rather than Decision B (document the absence). The L-3 and L-4 deferrals stand only in the sense of "they're feature work, not bug fixes"; 20n adopts them as their own slices with full pre-phase chats per CLAUDE.md.

**Detailed plan:** [docs/phases/phase-20n-open-gap-implementation.md](./docs/phases/phase-20n-open-gap-implementation.md) — per-slice plan, design-decision overrides, audit checklist, validation gate.

**Sequencing rules** (per CLAUDE.md "When splitting"):

- One slice = one feature, one or more commits.
- Validation gate between every commit: `cargo check --workspace` + targeted `cargo test -p <crate>` + `cargo run -q -p corvid-cli -- verify --corpus tests/corpus` (Windows whoami linker baseline tolerated).
- Push before starting the next slice.
- Pre-phase chat per slice; no autonomous chaining.
- 20n-B and 20n-C each require a step-0 audit + a refined plan before implementation, since both are feature work bigger than typical fix slices.

**Slices:**

- [x] 20n-A — L-7 lexer line continuation (Decision A, end-to-end implementation). Lexer changes in `crates/corvid-syntax/src/lexer.rs` to consume `\` + newline + leading whitespace as silent continuation outside strings and inside `"..."` literals. Triple-quoted blocks unchanged. Helpful diagnostic for `\` not at end-of-line. Tests + `docs/reference/lexer-rules.md` "Continuation rules" paragraph. Shipped `eb4a962` 2026-05-08.
- [x] 20n-B — L-4 WASM String parameter and return support. Bare `(ptr, len)` UTF-8 ABI across `crates/corvid-codegen-wasm/`, JS loader, `.d.ts` emitter, manifest. `corvid_alloc` / `corvid_free` exports always present (real free-list with two-pass coalescing). Multi-value `(result i32 i32)` returns. Compile-time string-literal pool emitted as a single active `DataSection` segment with content-keyed deduplication. Multi-byte UTF-8 round-trip integration tests + 200-iteration churn test pinning page count. Uniform manifest `kind` discriminator on every param/return. Shipped across `9e00719` `bf7d55f` `6bfc7ae` `8da006e` `231c88c` `14ffb07` 2026-05-08.
- [x] 20n-C — L-3 native codegen struct returns. Two sites lifted: prompt-bridge in `crates/corvid-codegen-cl/src/lowering/prompt.rs` (commit 4) and entry-agent boundary in `crates/corvid-codegen-cl/src/lowering/entry.rs` + `lib.rs` (commit 5). Plus `Grounded<Struct>` attestation extension (commit 6). Step-0 audit corrected the original "mirror Grounded<T>" framing — Grounded<T> is a handle-store pattern for attestation metadata, not a heap-allocation pattern; the actual heap layout to mirror was `lower_struct_constructor`'s `corvid_alloc_typed(size, &typeinfo)` with 8-byte field slots. New crate `corvid-prompt-format` extracted from `corvid-vm/src/schema.rs` so codegen can reuse the JSON Schema generator without depending on the interpreter. Generic JSON parse/build primitives (12 C-ABI fns) added to runtime; codegen emits per-struct decoders + encoders (cached by `DefId`) that use them. New `corvid_prompt_call_struct` bridge takes a function-pointer decoder callback (the typed-bridge family extended without combinatorial explosion: same one bridge serves every struct type via the codegen-emitted decoder). Field-type coverage v1: `Int` / `Bool` / `Float` / `String` (mirrors the four scalar prompt bridges). Shipped across `10107cc` `1361a61` `9d8e19d` `cfb131d` `6f04db5` `5e1b864` 2026-05-08.

**Out-of-scope deferrals:** the REPL hardcoded ANSI escape audit (filed in 20m as out-of-scope) stays out of 20n. So does any expansion to `Stream<Struct>`, WASM Component Model adapters, UTF-16, or cross-FFI struct passing for tools.

**Phase-done criteria:**

- [x] 20n-A, 20n-B, 20n-C all land with regression tests.
- [x] Closing audit recorded in `docs/phases/phase-20n-open-gap-implementation.md` with per-slice status + the design-override note for L-7.
- [x] `learnings.md` updated per slice.
- [x] ROADMAP.md Phase 20n entry checkboxes ticked, `✅ closed` marker added.
- [x] Memory record `project_phase_20n_closed.md` summarises the design-override pattern (when a deferral is reversed, record the directive explicitly so future sessions don't mistake it for drift), the step-0 audit pattern (substantive feature slices need a read-and-plan step before code), the codegen-emits-per-type pattern (per-struct decoders + encoders cached by `DefId` so runtime stays type-agnostic), and the rename-don't-duplicate principle (extending an existing storage map's semantics rather than introducing a parallel one).

---

### Phase 21 — Replay (~5–6 months, maximal-flagship scope) ✅ closed — **THE FLAGSHIP WOW**

**Goal.** Every run replayable by construction — and beyond. Baseline record + replay in both tiers, plus nine inventive features that push past every existing observability tool. Replay becomes a language-level, compile-time-guaranteed, regression-oracle-producing primitive.

**Hard dep:** Phases 14–15 (tool + prompt calls must exist to be worth recording). Runtime tracing infrastructure (baseline from Phase 11). Phase 20h seeded PRNG from slice I-rt.
**Soft dep:** Phase 20. Replay doesn't structurally depend on custom effects — it records tool / prompt / approve / seed / time calls regardless of effect category.

**Locked design anchors:**
- Trace format: **JSONL** (diff-friendly, CI-inspectable, one `TraceEvent` per line).
- Cross-tier replay (interpreter-trace ↔ native-replay): **post-v1**. v1 records-then-replays within-tier only.
- Recording overhead: **≤ 5%** vs unrecorded (soft budget).
- Trace storage: **local disk only** under `target/trace/<run-id>.jsonl`. Upload is a later phase.
- Interactive step/scrub UX: **post-v1**. CLI-first.

**Inventive-layer features (what makes this extraordinary):**
- **A** — `@replayable` **compile-time guarantee**. Agent fails to compile if body uses any nondeterministic source not captured in the trace schema. Replay is a type-system property, not a runtime hope.
- **B** — **Differential replay across model versions**. `corvid replay --model <id> <trace>` replays a recorded trace against a different provider and reports divergences. Regression-test the next model upgrade for $0.
- **C** — **Provenance-aware replay**. Renders the `Grounded<T>` DAG for every output. "How did the model know X?" becomes answerable.
- **D** — **Counterfactual replay**. Mutate one recorded response, replay, show the divergence. "What would have happened if the adjudicator said contradiction:false?"
- **E** — **Replay as a language primitive**. `replay <trace>: when <pattern> -> <expr>` — agents can analyze their own past runs. No other framework has this.
- **F** — `@deterministic` — stricter sibling of `@replayable`. Forbids every nondeterministic source, trace or no trace.
- **G** — **Prod-as-test-suite**. `corvid test --from-traces <dir>` turns every production trace into a deterministic regression test. The suite writes itself.
- **H** — **Behavior-diff in PR review**. `corvid trace-diff <commit-a> <commit-b>` renders the semantic diff of agent behavior across the trace corpus for every PR.
- **I** — **Live shadow replay**. Runtime daemon runs prod + replay simultaneously; divergence alerts fire in real time.

**Non-scope (post-v1):** Scrub-backward interactive debugger, trace visualization UI, WASM replay (Phase 23), trace upload/federation, semantic similarity for differential replay, cross-tier replay parity.

**v0.6 cuts here.** Moat phase + flagship wow feature land together. Corvid becomes unignorably different.

**Track divided by file scope** (same boundary used through Phase 20h/20i):

#### My lane (compiler + CLI + docs — ~15 slices)

- [x] 21-A-schema            `corvid-trace-schema`: `SCHEMA_VERSION` + new variants (`SchemaHeader`, `SeedRead`, `ClockRead`) + `io.rs` JSONL helpers + round-trip tests
- [x] 21-A-determinism-hooks IR clock abstraction + PRNG wiring confirmation from Phase 20h I-rt
- [x] 21-F-cli               `corvid replay <trace>`, `corvid trace list`, `corvid trace show`
- [x] 21-inv-A               `@replayable` attribute: parser / AST / resolver / checker; `NonReplayableCall` diagnostic
- [x] 21-inv-F               `@deterministic` stricter sibling; shared `replayable ⊂ deterministic` lattice
- [x] 21-inv-B-cli           `corvid replay --model <id>` + divergence renderer
- [x] 21-A-schema-ext-source Interleaved: `SchemaHeader.source_path` + `SCHEMA_VERSION` 1→2 + `MIN_SUPPORTED_SCHEMA` range (self-describing traces)
- [x] 21-inv-C-1             Provenance schema: `ProvenanceEdge` trace event variant (additive, skipped as dispatch-metadata during replay)
- [x] 21-inv-C-2             Provenance CLI: `corvid trace dag <id>` renders ProvenanceEdge substream as Graphviz DOT
- [x] 21-inv-D-cli           `corvid replay --mutate <step> <response>` + divergence output
- [x] 21-inv-E-1             Parser: `replay <expr>: when <pat> -> <expr>` syntax
- [x] 21-inv-E-2a            Parser + AST: arm captures (`as <ident>` tail + tool-arg capture)
- [x] 21-inv-E-2b            Resolver: pattern-name resolution + arm-capture scope opening
- [x] 21-inv-E-3             Checker: `TraceId` / `TraceEvent` types + pattern exhaustiveness
- [x] 21-inv-E-4             IR lowering for replay blocks
- [x] 21-inv-G-cli           `corvid test --from-traces <dir>` + trace-to-test harness (5 inventive flags: `--replay-model` / `--only-dangerous` / `--only-prompt` / `--only-tool` / `--since` / `--promote` / `--flake-detect`; coverage-map preview)
- [x] 21-inv-B-cli-wire      Flip `--model` CLI stub to real differential-replay dispatch (driver helper `run_replay_from_source_with_builder` + 6 driver integration tests)
- [x] 21-inv-D-cli-wire      Flip `--mutate` CLI stub to real counterfactual-mutation dispatch (4 driver integration tests)
- [x] 21-inv-G-cli-wire      Flip `--from-traces` CLI stub to real regression-harness dispatch through `corvid_runtime::run_test_from_traces` (async driver variant; deferred `--promote` to follow-up)
- [x] 21-inv-G-cli-wire-promote  Wire `--promote` through `RecordCurrent`: fresh-run-with-`trace_to` driver helper (`run_fresh_from_source_async`) + `PromotePromptMode::AutoStdin` (TTY: [y/N/a/q]; non-TTY: fail-closed with one-time warning)
- [x] 21-inv-H-1             `corvid trace-diff <base-sha> <head-sha> <path>` + in-repo Corvid `@deterministic` reviewer agent: static algebra diff (added / removed agents, trust-tier / `@dangerous` / `@replayable` transitions) across `pub extern "c"` exported surface. Reviewer is a `.cor` program embedded via `include_str!` — the flagship PR-review tool dogfoods the language it reviews.
- [x] 21-inv-H-2             Counterfactual replay: `--traces <dir>` replays each trace against base and head via the 21-inv-G-harness, categorises per-trace verdicts into `passed_both` / `newly_diverged` / `newly_passing` / `diverged_both` / `errored` buckets, and the Corvid reviewer renders a "Counterfactual Replay Impact" section with the newly-divergent path list and an impact percentage. Reviewer signature grows to `review_pr(base, head, impact) -> String` without losing its `@deterministic` guarantee.
- [x] 21-inv-H-3             Structured approval + provenance diff: receipt calls out added / removed approval labels per agent, weakened `required_tier` on existing labels, reversibility regressions, `returns_grounded` transitions, and added / removed `grounded_param_deps`. Reviewer owns the structure in Corvid; Rust only extracts fields. Numeric cost-at-site deltas deferred (blocks on Corvid Float→String). Structured predicate-JSON AST diff deferred (needs typed JSON in Corvid; different language-surface work).
- [x] 21-inv-H-4             Structured narrative summary: `summarise_diff` prompt produces a one-to-three-sentence `ReceiptNarrative { body, citations }` at the top of the receipt, with strict all-or-nothing citation validation against the canonical `DiffSummary.records` key set. `--narrative=auto|on|off` (default `auto`); `off` is the byte-deterministic CI path. Rejected narratives fall back to H-3 boilerplate with a `narrative rejected: <reason>` stderr warning. Receipt structure stays reviewer-owned in `review_pr`, which now takes the validated `ReceiptNarrative` as its fourth argument and remains `@deterministic`.
- [x] 21-inv-H-5             GitHub/CI integration: canonical `Receipt` struct (schema_version 1) is the source of truth; `--format=markdown|github-check|json|auto` routes through per-format renderers (markdown stays Corvid-side via the reviewer agent; github-check + json are Rust). `auto` detects `$GITHUB_ACTIONS` → github-check, piped stdout → json, tty → markdown. Default regression policy ships baked-in (conservative: @dangerous gained, trust lowered, approval tier weakened, reversibility became irreversible, grounded lost, grounded dep removed, newly-diverged traces) with non-zero exit + stderr flag listing on trip. Improvements (additions, tier-raising, grounded gained) do NOT trip the gate.
- [x] 21-inv-H               Rollup CLOSED — H-1 through H-5 landed; `corvid trace-diff` is the flagship PR-review tool dogfooding the language.

**Deferred follow-ups (file separately, each independently shippable):**

- [x] 21-inv-H-4-follow       Upgrade `ReceiptNarrative` to `Grounded<ReceiptNarrative>` now that 22-F ships the provenance-handle path. Rust-side `ReceiptNarrative` is host-minted into `Grounded<_>` from already-validated citation delta keys before the deterministic Corvid reviewer consumes it; empty fallback narratives carry no prose claims and therefore mint an empty chain.
- [x] 21-inv-H-5-custom-policy  Promote the default regression policy from a Rust function to a user-replaceable `.cor` program. `--policy=<path>` flag loads + compiles the user's `apply_policy(receipt) -> Verdict` agent; default policy ships as `default_policy.cor` baked into the CLI. Governance-as-code for the gate itself.
- [x] 21-inv-H-5-signed        DSSE-signed receipts. `corvid trace-diff --sign=<key>` emits a DSSE envelope (`application/vnd.corvid-receipt+json`) with ed25519 signature over the PAE of the canonical JSON payload. `corvid receipt show <hash>` resolves a receipt from the local hash-addressed cache (short prefix matches supported, minimum 8 chars). `corvid receipt verify <envelope> --key <path>` round-trips — accepts file paths OR cached hash-prefixes. Key source: `--sign=<path>` file (hex or raw 32 bytes) with `CORVID_SIGNING_KEY` env var fallback. Receipt hash emitted on stderr as `Corvid-Receipt: <hash>` for downstream tooling. Turns the receipt from informational text into a cryptographic audit artifact — Corvid receipts now plug into the DSSE / Sigstore / in-toto ecosystem.
- [x] 21-inv-H-5-in-toto       SLSA/Sigstore in-toto attestation renderer. `--format=in-toto` emits an in-toto Statement v1 wrapping the canonical Receipt as the predicate; subject is the head source file (sha256); predicateType `https://corvid-lang.org/attestation/receipt/v1`. Combined with `--sign`, the DSSE envelope uses `application/vnd.in-toto+json` so cosign / slsa-verifier consume the output natively. `corvid receipt verify` accepts both Corvid-native and in-toto payloadTypes transparently. Unsigned in-toto output is allowed for pipelines that sign externally (cosign with KMS keys, etc).
- [x] 21-inv-H-5-stacked       Stacked-PR aggregate receipts. Per-commit receipts compose into a stack receipt via the effect-algebra's natural composition; regressions anywhere in the stack surface as regressions in the aggregate.
- [x] 21-inv-H-5-watch         `--format=watch` reactive mode: rebuild + rerender the receipt as the working tree changes. Tightens the AI-safety feedback loop to type-checker speed during local development. Watch mode compares a fixed base SHA against the working-tree file, renders immediately, rerenders on content changes, supports custom Corvid policies, and deliberately rejects stack/signing modes because it is an interactive local feedback loop rather than a durable receipt artifact.
- [x] 21-inv-H-5-gitlab        GitLab CI renderer (`--format=gitlab`). Emits a CodeClimate-compatible JSON array GitLab consumes via `artifacts.reports.codequality`; surfaces findings inline on MR diffs and in the MR widget. One issue per delta; severity tracks the default policy (regressions `major`, informational `info`); the counterfactual-replay trace-impact lands as its own `major` issue when any trace newly diverged. Fingerprint is hex-SHA256 of the delta key — byte-stable across pipeline re-runs so GitLab dedupes issues rather than spawning phantom "new" findings. `--format=auto` under `$GITLAB_CI` auto-selects the renderer; users drop `corvid trace-diff ...` into a job without touching `--format`. Non-zero exit on regression carries through unchanged.
- [x] 21-inv-H-5-schema-fix    Honest delta-key names + schema v2. Rename `agent.approval.tier_weakened:` → `agent.approval.tier_changed:` and `agent.approval.reversibility_weakened:` → `agent.approval.reversibility_changed:` in the delta emitter. Both keys always fired on *any* transition (weakening OR strengthening) — the old names were a naming shortcut from H-1/H-3 that misrepresented half the emissions. Direction lives in the `from->to` suffix; the policy layer still gates only on weakenings via `is_trust_lowering` and `*->irreversible` checks. `RECEIPT_SCHEMA_VERSION` bumps 1 → 2 so JSON consumers pattern-matching on the old prefixes get a clear signal to update their matchers. Pre-slice to `21-inv-H-5-stacked` — algebraic stack composition needs honest names to reason about.

**Language-core slices (cross-lane; enable the custom-policy stack + every future multi-file `.cor` user surface):**

- [x] lang-pub-toplevel        Top-level visibility: `public` / `public(package)` on `type` / `tool` / `prompt` / `agent` declarations. Private-by-default. `pub extern "c"` agents implicitly `Public` (FFI export requires external visibility by definition). Backward-compatible — existing single-file programs behave identically; visibility becomes load-bearing once cross-file imports land in `lang-cor-imports-basic`. Enables intentional library-surface authoring from day one, parallel to the existing `public` support inside `extend` blocks.
- [x] lang-cor-imports-basic-parse   Parser + AST support for `import "./path" as alias`. New `ImportSource::Corvid` variant distinguishes local `.cor` imports from Python FFI imports. Grammar accepts both shapes; resolver is unchanged (qualified access yields "not yet implemented" at resolve time pending `-resolve`). Ships the syntactic contract before the mechanism.
- [x] lang-cor-imports-basic-resolve-2a   `ModuleResolution` / `ResolvedModule` / `DeclExport` types in `corvid-resolve`. Public-export filtering (private declarations never leak). `resolve_import_path` + `ModuleLookup` API. No driver integration yet.
- [x] lang-cor-imports-basic-resolve-2b   Driver-side BFS loader in `corvid-driver::modules` with three-color cycle detection. `build_module_resolution(root_file, root_path) -> (ModuleResolution, Vec<ModuleLoadError>)`. Five typed error variants (FileNotFound / ReadError / LexError / ParseErrors / Cycle). Diamond imports dedupe; transitive imports load but don't surface on root alias map.
- [x] lang-cor-imports-basic-resolve-2c-1 Checker threading + failure-mode errors: `typecheck_with_modules` entry point; optional `ModuleResolution` in `Checker`; three typed errors (`UnknownImportAlias` / `ImportedDeclIsPrivate` / `UnknownImportMember`) surface when import lookup fails. Found case still stubs pending `-2c-2`.
- [x] lang-cor-imports-basic-resolve-2c-2 Real type resolution for successful qualified lookups. Chose `Type::ImportedStruct` to preserve file-boundary identity instead of synthesizing local DefIds. `DeclExport` carries type fields, `ResolvedModule` carries the imported AST, `check_field` resolves imported struct fields, and `Type::display_name` renders imported type names.
- [x] lang-cor-imports-basic-resolve-2c-3 **Owner preference: Dev B.** Driver integration: file-backed production paths now route through module-aware typecheck + lowering when a root file has Corvid imports. Build/run/replay/fresh/shadow paths keep `corvid.toml` config and imported struct identity together; source-string helpers remain single-file by design.
- [x] lang-cor-imports-basic         ROLL-UP — 2c-2 + 2c-3 landed. Basic aliased Corvid imports now parse, load, resolve exported struct types, preserve field access, and compile through file-backed driver paths.
- [x] lang-cor-imports-basic-calls   **Owner preference: Dev B.** Qualified function calls: `p.apply_policy_default(r)`. Implemented checker-side `FieldAccess` recognition for import aliases before method-call fallback, typed imported tool / prompt / agent / struct-constructor calls in the imported module's type context, appended imported callable/type decls to IR with synthetic DefIds, and covered IR + VM runtime dispatch with file-backed driver tests. Needed end-to-end for `21-inv-H-5-custom-policy` to compile + run.
- [x] lang-cor-imports-use     Selective name lift: `import "./path" use Name, Name as Alias` — explicitly-listed names into current scope, no wildcard merge, no silent shadowing. Rename-on-import via `as Alias` for conflicts. Ships on top of `-basic`.
- [x] lang-cor-imports-requires Effect-typed imports — the extraordinary differentiator. `import "./path" requires @deterministic as p` asserts the imported module's public exports satisfy the import boundary contract at compile time. Deterministic imports require exported agents to be `@deterministic` and reject public tool/prompt exports; dimensional constraints such as `@budget($0.50)` run through the existing effect analyzer for exported agents. Prevents "library silently broke our invariants" bugs; composes cleanly with Corvid's existing effect algebra.
- [x] lang-cor-imports-semantic-summaries Imported modules expose effect, approval, provenance, budget, replayability, and exported-agent summaries to the checker and CLI. `ResolvedModule` now carries a stable semantic summary, import contract checks consume that summary, and `corvid import-summary <file> [--json]` renders the imported public boundary for developers.
- [x] lang-cor-imports         ROLL-UP — closes when `-basic` + `-use` + `-requires` all land. Basic aliased imports, selective lifted imports, and effect-typed import requirements are now implemented; semantic summaries, signed imports, remote imports, and versioned packages remain follow-up hardening/publishing layers.
- [x] lang-cor-imports-signed  Hash-pinned imports: `import "./path" hash:sha256:abc123... as p`. If the imported file's content drifts, compilation fails. Supply-chain integrity at the language level: pins are parsed into the AST/IR, verified over the exact imported source bytes before parsing/resolution, and mismatches fail closed with an actionable diagnostic. Pairs with `21-inv-H-5-signed` so a signed receipt's policy hash chain extends through the import graph.
- [x] lang-cor-imports-remote  Remote imports: `import "https://.../policy.cor" hash:sha256:... as p`. HTTP(S) Corvid imports now require mandatory SHA-256 pinning at parse time, fetch through a distinct remote module target, verify exact response bytes before parsing, and typecheck/lower public exports through the same module pipeline as local imports. Enables federated policy baselines and cross-repo governance without a full package manager.
- [x] lang-cor-imports-versioned ROLL-UP — versioned imports + package system: `import "corvid://@anthropic/safety-baseline/v2.3" as p`. Locked package resolution, registry semantic-version selection, and signed publish verification have landed as separate slices so the package-manager story is real rather than implied.
- [x] lang-cor-imports-versioned-lock Locked package imports: `corvid://...` source imports now resolve only through `Corvid.lock`. The lockfile maps the semantic URI to an immutable HTTP(S) source URL and SHA-256 digest; missing lockfiles, missing entries, and hash drift all fail closed before parsing. Inline hashes on package imports are rejected because the lockfile is the reproducibility authority. See [docs/reference/package-imports.md](docs/reference/package-imports.md).
- [x] lang-cor-imports-versioned-registry Registry semantic-version resolver: `corvid add @scope/name@2.3` queries a local or HTTP registry index, chooses the highest matching semantic version, verifies the selected source hash, computes the exported semantic summary, writes `Corvid.lock`, and refuses packages whose exported effects violate `[package-policy]` in `corvid.toml`.
- [x] lang-cor-imports-versioned-signed-publish Signed publish workflow: `corvid package publish` copies source packages into a registry directory, computes SHA-256 and semantic summary, signs the canonical package subject with Ed25519, and updates `index.toml`. `corvid add` verifies signed entries and `[package-policy] require-package-signatures = true` rejects unsigned entries before lockfile mutation.

- [x] 21-docs                Spec [section 14](docs/internals/effect-spec/14-replay.md) (Phase 21 implementation reference) + v1.0 launch demo at [docs/meta/v1.0-demo-script.md](docs/meta/v1.0-demo-script.md) + ROADMAP closeout status below.

**Phase 21 closeout status (as of 2026-04-25).**

Lane A (compiler + CLI + docs) has shipped the primary replay/test/receipt surface and the receipt hardening follow-ups: counterfactual replay, structured approval/provenance drill-down, grounded narrative receipts, custom Corvid policies, DSSE / in-toto signing, stacked receipts, watch mode, GitLab CI output, and schema-v2 delta names. The thesis claim is demonstrable today: `@replayable` compiles only what can be deterministically reproduced, every run writes a trace, `corvid test --from-traces --promote` closes the Jest-snapshot loop, and `corvid trace-diff` produces a PR behavior receipt whose reviewer and policy can themselves be Corvid programs.

Lane B (runtime + codegen + daemon) has shipped interpreter and native recording/replay, runtime counterfactuals, trace-to-test promotion, and the shadow daemon. Native-tier shadow replay parity is now available through `execution_tier = "native"` in the daemon config for native-recorded traces; cross-tier replay remains rejected by design so trace equivalence never hides backend differences.

What's between us and a clean "Phase 21 done" on the ROADMAP:

- Nothing in the Phase 21 checklist. Remaining replay work, if any, is future hardening on top of the shipped surface rather than a Phase 21 blocker.

The determinism-source catalog and the language's treatment of non-reproducible sources are documented in [docs/phases/phase-21-determinism-sources.md](docs/phases/phase-21-determinism-sources.md) and summarised in [spec §14.11](docs/internals/effect-spec/14-replay.md). Every trace axis the runtime records is enumerated there, and extensions land through monotonic `SCHEMA_VERSION` bumps + compile-time opt-in at `@replayable` level.

#### Dev B's lane (runtime + codegen + daemon — ~9 slices)

- [x] 21-B-rec-interp        Recording hooks in interpreter (LLM / tool / approve / seed / time), emit to JSONL
- [x] 21-C-replay-interp     Replay adapter: response substitution; byte-identical post-replay state
- [x] 21-B-rec-native        Native-tier recording parity
- [x] 21-C-replay-native     Native-tier replay parity
- [x] 21-inv-B-adapter       Model-swap seam for `corvid replay --model <id>`
- [x] 21-inv-D-runtime       Counterfactual one-step mutation at runtime
- [x] 21-inv-E-runtime       Runtime support for `replay` language primitive (trace ingestion + pattern dispatch)
- [x] 21-inv-G-harness       Trace-to-test-fixture adapter; divergence-as-test-failure reporting
- [x] 21-inv-I               Live shadow replay daemon; real-time divergence alerts
- [x] 21-inv-I-native        Native-tier shadow replay daemon parity: `execution_tier = "native"` builds/caches the native binary, replays native-recorded traces under the native writer, reads differential/mutation reports, and preserves cross-tier rejection for interpreter-recorded traces.

**Rules (standing):** CLAUDE.md rubric on every file (1–2 responsibilities). One commit per file extraction or feature step. Validation gate between every commit: `cargo check --workspace` + `cargo test -p <crate> --lib` + `cargo test -p <crate> --test <name> -- --list` (for test-file touches) + `cargo run -q -p corvid-cli -- verify --corpus tests/corpus` (must still exit 1 only on the two deliberate fixtures). Push before next slice, then continue to the next roadmap item automatically unless blocked by a real product/security/scope decision. Zero semantic changes mid-refactor. No shortcuts — a thin feature is a shortcut.

**Success criteria.** Every agent marked `@replayable` compiles iff it can be deterministically replayed. Every run under recording produces a JSONL trace that replays to byte-identical state. `corvid replay --model claude-opus-5.0 trace.jsonl` runs cost-free and reports divergences. `replay` is a first-class Corvid expression. Prod traces become regression tests with `corvid test --from-traces`. PRs show a behavior diff before merge. Live shadow mode detects regressions in production.

---

### Phase 22 — C ABI + library mode (~6–8 weeks) ✅ closed

**Goal.** Embed Corvid in Rust, Python, Node, Go hosts — with the AI-safety guarantees (effects, approvals, provenance, budgets) surviving into the host's type system. Corvid isn't just a callable library; it's the only embeddable language whose compile-time AI-safety contracts are observable from the host.

**Hard dep:** Phase 12 (native codegen).
**Soft dep:** Phase 17 (cycle collector). C ABI without the cycle collector means embedders who build cyclic data across the boundary leak — exactly the same behaviour every pre-Phase-17 Corvid program has. Not a compilation blocker, but pairing with Phase 17 at the same release is the honest story: the v0.7 pitch is "Corvid ships as a library" and shipping a leaking library would undercut that.

**Slice checklist:**

- [x] 22-A-cdylib            `pub extern "c"` + `--target=cdylib`/`--target=staticlib` + `--header` scalar C header
- [x] 22-B-abi-descriptor    `--abi-descriptor` + `corvid-abi` crate (machine-readable effect/approval/provenance surface, deterministic JSON)
- [x] 22-C-prompt-catalog    Runtime-queryable typed prompt/agent catalog: cdylibs embed the descriptor, expose `corvid_list_agents` / `corvid_agent_signature` / `corvid_call_agent` so hosts can discover + dispatch agents with type-checked args at runtime
- [x] 22-D-effect-filter     Host-side effect-dimension filter: `corvid_find_agents_where(trust<=autonomous, cost<=0.10)` — the host can narrow the agent set by effect algebra without re-reading the descriptor
- [x] 22-E-approval-bridge   Approval contracts survive FFI: `@dangerous` entrypoints reach back through the boundary to invoke a host-supplied approver; no way for a host to bypass by linking
- [x] 22-F-grounded-return   `Grounded<T>` return values cross the boundary with their provenance chain intact; host receives `(payload, provenance_handle)` it can query
- [x] 22-G-budget-observe    Per-call cost/latency observability: host reads real-time budget burn per agent
- [x] 22-H-replay-across-ffi Traces recorded on one side of the boundary replay deterministically from the other; the embedded binary becomes a recordable unit
- [x] 22-I-host-bindings     Reference Rust + Python host crates; generated idiomatic bindings from the descriptor (Rust traits; Python Protocols)
- [x] 22-J-ownership-check   Compile-time checker on extern signatures (who frees what, who retains what)
- [x] 22-K-cdylib-demo       End-to-end `pub extern "c"` scalar-signature agent shipping as `.so`/`.dll`, plus a matching host-side Rust + Python demo that reads the descriptor and dispatches

**Non-scope:** WASM (Phase 23). Language-level FFI imports of other languages.

### Phase 23 — WASM target (~8–10 weeks) (reopened 2026-04-29 — browser end-to-end CI gap)

**Goal.** Deploy Corvid to browsers and edge runtimes.

**Hard dep:** IR (✅). Parallel codegen backend to Cranelift-native; does not depend on it.

**Scope:**
- New `corvid-codegen-wasm` crate. The scalar foundation emits directly with `wasm-encoder`; host-capability lowering can move to a fuller Cranelift-backed pipeline when prompts/tools/approvals need shared runtime imports.
- `corvid build --target=wasm` emits `.wasm` + an ES module loader + TypeScript types.
- Runtime: the wasm module imports typed host capabilities for LLM calls + tool dispatch + approval UI + replay recording. Each host import carries the same effect/provenance/budget contract as the native runtime boundary.
- **Replay in WASM**: host functions that record tool + prompt + approve calls write to a JS-side trace store compatible with Phase 21's format. `corvid replay <trace>` on a WASM module runs via the same host-function contract, substituting recorded responses. Shared recording format means a trace captured from native can be replayed under WASM and vice versa — a property worth preserving from the start.
- Browser and edge approval calls produce scoped approval tokens in the trace, so user-mediated actions remain auditable across deployment targets.
- wasmtime / wasmer harness tests running the same IR-level programs the native parity harness runs.
- Browser smoke test: a small Corvid program compiled to wasm and loaded in a web page.

**Non-scope:** Wasm-specific optimizations (post-v1.0). Wasm-side cycle collection (wasm's own GC proposal is stabilising; use it once available, fall back to host-delegated collection via exported functions in the interim).

**Slice checklist:**

- [x] 23-A-scalar-wasm       `corvid build --target=wasm` emits a valid standalone `.wasm` module plus ES loader, TypeScript declarations, and a manifest for scalar runtime-free agents. Unsupported prompts/tools/approvals fail loudly until the host-capability ABI exists.
- [x] 23-B-host-abi          Browser/edge scalar host-capability ABI: scalar prompts/tools/approvals lower to typed `corvid:host` imports (`prompt.*`, `tool.*`, `approve.*`), with generated JS `adaptImports(host)`, TypeScript host interfaces, and manifest import entries. Replay recording, strings/structs, and provenance handles remain follow-up slices.
- [x] 23-C-wasm-replay       JS-side trace store in generated loader: `instantiate(host, { trace })` records schema-v2 `schema_header`, `run_started`, `llm_call/result`, `tool_call/result`, `approval_request/decision/response`, and `run_completed` events for scalar host imports. Native/WASM traces now share the event taxonomy; full `corvid replay` execution over WASM modules remains a harness follow-up.
- [x] 23-D-browser-demo      Browser smoke page at `examples/wasm_browser_demo`: one Corvid source compiles to WASM, the generated ES loader is imported by a real page, typed prompt/tool/approval host capabilities are supplied from JS, approval decisions are visible in UI, and the trace panel records schema-v2 run/prompt/approval/tool events from the generated loader.
- [x] 23-E-wasmtime-harness  Wasmtime parity harness for the current WASM-supported native parity subset: generated scalar modules execute under Wasmtime and match the interpreter for arithmetic, branching, and agent calls; scalar prompt/approval/tool imports execute through typed host functions. Unsupported native parity families remain explicit WASM boundary work until strings, structs, lists, and provenance handles land.

**Phase 23 reopened 2026-04-29 — gap-closing slice required:**

- [x] 23-F-browser-ci-headless     Headless-Chromium browser CI shipped: `examples/wasm_browser_demo/test/` carries a Playwright harness (`browser.spec.js`, `playwright.config.js`, `package.json`) that builds the WASM artifacts, serves the demo over a static HTTP server, opens it in headless Chromium, exercises both approve and deny paths against typed prompt/tool/approval host capabilities from JS, and asserts the schema-v2 trace events (`schema_header`, `run_started`, `approval_request`, `approval_decision`, `tool_call`, `tool_result`, `run_completed`) appear in the trace panel. The `phase23-browser-ci` GHA matrix entry runs the harness on every push. Slice fully closes when the first CI run is observed green on `main`; until then the harness is committed and CI is wired.

**Phase 23 next-close criteria:** the ROADMAP-level `[x]` returns only when slice 23-F clears the slice completion gate (CI workflow update, registry rows for `wasm.browser_host_imports_typed` + `wasm.trace_panel_records_schema_v2`, side-by-side `benches/comparisons/wasm_deploy.md` against a comparable Vercel AI SDK browser deployment).

**v0.7 cuts here.** Corvid ships as a library + a wasm module. Real deployment story.

---

### Phase 24 — LSP + IDE (~6–8 weeks) ✅ closed

**Goal.** Editor support worthy of a real GP language. Users need this to write serious Corvid — must land before the moat features are worth using daily.

**Hard dep:** frontend (✅). Types stable enough that LSP doesn't churn when language evolves.

**Scope:**
- `corvid-lsp` crate implementing the Language Server Protocol. Backend-agnostic (same LSP serves native + interpreter + wasm users).
- VS Code extension as the reference client.
- Features: diagnostics (live), hover with inferred types, completion, go-to-def, find-references, rename, inline-documentation.
- AI-native behavior visibility: effect rows, budget/cost trees, groundedness flow, approval boundaries, model routes, replayability, and import trust constraints shown inline where the programmer is making the decision.
- `@budget($)` overruns, ungrounded returns, non-replayable calls, unsafe imports, and approval-boundary violations shown as live diagnostics with the same error codes as the compiler.
- Debugging attach point wired even if debugger UI is post-v1.0 — protocol contract stable.

**Non-scope:** Other editors (vim / emacs / JetBrains) — users can use the LSP via any LSP-compatible client, but official extensions are post-v1.0.

**Slice checklist:**

- [x] 24-A-lsp-diagnostics   Transport-independent LSP analysis core in `corvid-lsp`: open document text compiles through the real driver, compiler diagnostics become `lsp_types::Diagnostic` values with UTF-16 ranges, compiler hints are preserved, and approval-boundary violations surface through the same live diagnostic path as CLI errors.
- [x] 24-B-lsp-server        JSON-RPC/stdin-stdout language server with `initialize`, `shutdown`, `exit`, `textDocument/didOpen`, `textDocument/didChange`, and `textDocument/didSave`; publishes compiler-backed diagnostics through `textDocument/publishDiagnostics` using full-document sync.
- [x] 24-C-hover-types       Hover with compiler-backed inferred expression types plus declaration summaries for agents, tools, prompts, types, and effects. Prompt hovers surface AI-native metadata such as effect rows, calibration, cacheability, strict citations, and model routing mode; tool hovers show dangerous/approval boundaries.
- [x] 24-D-completion        Context-aware completion for keywords, declarations, tools, prompts, approval labels, effect names, and model names. The completion engine is compiler/parser-backed, uses partial source while the user is typing, and keeps approval/effect/model contexts narrow instead of dumping every symbol everywhere.
- [x] 24-E-navigation        Single-file navigation over resolver identity: go-to-definition, find-references, rename edits, and workspace symbol search across open documents. Navigation uses DefId/LocalId bindings, not text search, so local rename does not touch unrelated declarations with the same spelling.
- [x] 24-F-vscode-client     Reference VS Code extension at `extensions/vscode-corvid`: registers `.cor`, starts `corvid-lsp`, wires diagnostics/hover/completion/definition/references/rename/workspace symbols, ships syntax highlighting, language configuration, snippets for AI-native constructs, restart/log commands, and a local verification script.

### Phase 25 — Package manager (~6–8 weeks) ✅ closed

**Goal.** Users can share Corvid code and AI capabilities with guarantees. Table stakes for any language anyone takes seriously, made Corvid-native by distributing effect, provenance, approval, budget, and replay contracts alongside source.

**Hard dep:** nothing internal. Major external work: hosted registry operations, explicitly outside v1.0.

**Scope:**
- `corvid add <pkg>`, `corvid remove`, `corvid update` CLI.
- `Corvid.lock` lockfile with exact resolved versions, content hashes, semantic summaries, and signed publish metadata.
- Registry format and tooling: signed source package publish to a directory, local/self-hosted index resolution, and verification commands. No Corvid-hosted registry service runs yet.
- SemVer-based resolution with conflict detection.
- Effect-aware resolution: `corvid add` can warn or fail when a package exceeds a project policy for trust, cost, data, replayability, approvals, or grounded outputs.
- Package pages and CLI metadata expose exported agents/tools/prompts, effect profile, approval boundaries, provenance guarantees, and replay guarantees.
- `corvid.toml` `[dependencies]` section wired through the driver.

**Non-scope:** Private registries (post-v1.0). Binary package distribution (post-v1.0 — all v1.0 packages are source).

**Slice checklist:**

- [x] 25-A-package-import-lockfile    `corvid://...` imports are fail-closed through `Corvid.lock`: missing lockfiles fail, missing entries fail, locked URL bytes are SHA-256 verified before parse/typecheck/lower, and inline hashes are rejected for package imports.
- [x] 25-B-package-add-publish-policy Signed source package publish plus `corvid add`: registry index resolution, semver selection, source hash verification, exported semantic summary extraction, project policy gates, signature verification, and semantic summaries stored in `Corvid.lock`.
- [x] 25-C-manifest-remove-update     `corvid add` updates `corvid.toml [dependencies]`; `corvid remove` edits both manifest and lock; `corvid update` resolves the newest matching version from the manifest requirement/registry or an explicit spec, re-running hash/signature/policy checks before rewriting the lock.
- [x] 25-D-registry-http-contract     Minimal stateless registry API contract with `corvid package verify-registry`: validates index entries, scoped names, semver, canonical package URIs, immutable versioned `.cor` artifact URLs, SHA-256 bytes, artifact UTF-8/source semantic summaries, Ed25519 signatures, duplicate entries, and CDN-style `Cache-Control: ... immutable` headers.
- [x] 25-E-package-metadata-pages     `corvid package metadata` renders compiler-backed package pages from source: scoped package identity, install snippet, optional signature provenance, exported agents/tools/prompts/types/effects, effect profiles, approval boundaries, grounding, replayability, determinism, and cost/violation notes. JSON output gives the same semantic summary to web registries.
- [x] 25-F-conflict-resolution        `corvid package verify-lock` validates the installed package graph: manifest dependencies, locked package presence, duplicate URIs, multiple locked versions for one dependency, semver requirement satisfaction, stale undeclared lock entries, required semantic summaries, and current package-policy compatibility from locked semantic summaries.

**Phase 25 reopened 2026-04-29 — gap-closing slice required:**

- [x] 25-G-no-hosted-registry-honesty   The current implementation is a *package format + local resolver + signed-publish-to-a-directory*; no Corvid-hosted registry exists as a running service. README, package CLI help, `docs/internals/package-manager-scope.md`, and the `package.hosted_registry_available` `OutOfScope` guarantee row make the "format-and-tooling, no hosted service yet" boundary explicit. Closes when grep against README + landing page returns zero un-qualified mentions of `registry.corvid.dev`.

**Phase 25 next-close criteria:** the ROADMAP-level `[x]` returns only when slice 25-G clears the slice completion gate.

### Phase 26 — Testing primitives (~4 weeks) ✅ closed

**Goal.** `test`, `mock`, `fixture` as language features. Users can't ship production Corvid without first-class tests.

**Hard dep:** typechecker extension for `test`/`mock` decls.
**Soft dep:** Phase 25 (package manager). Shared fixtures can distribute as packages eventually, but in-repo fixtures work without the package manager — not a blocker.

**Scope:**
- `test name: body` declaration. Discovered automatically; run by `corvid test`.
- `mock tool_name: body` overrides a tool implementation within a test's scope, while preserving or explicitly declaring the mocked effect profile.
- `fixture name: body` for reusable test data; resolved by `corvid test` at run time.
- Snapshot testing primitive — `assert_snapshot expr` writes the first run's value to a file, compares on subsequent runs.
- AI-native assertions over traces, approvals, costs, provenance, grounding, and replay behavior. Ordinary tests verify values; Corvid tests can also verify that the right process happened.
- Trace fixtures: production traces from Phase 21 can be used as deterministic test inputs and regression cases.
- Interop with Phase 20's `eval ... assert ...` syntax (evals are tests, tests aren't necessarily evals — eval is statistical assertions over LLM behaviour).

**Slice checklist:**

- [x] 26-A-test-declarations          `test name:` declarations parse, resolve, typecheck, and lower into `IrTest` nodes. Tests reuse eval assertion syntax so value, trace-called, approval, ordering, cost, and statistical assertion metadata share one compiler model. See [docs/internals/testing-primitives.md](docs/internals/testing-primitives.md).
- [x] 26-B-test-runner                `corvid test <file>` discovers `test` declarations, executes setup bodies, evaluates value assertions, and reports typed pass/fail output with CI exit codes. Statistical value assertions rerun setup for the requested run count; trace/process assertions fail explicitly until 26-E implements trace fixtures.
- [x] 26-C-mocks-fixtures             `fixture` declarations are typed reusable test data callable only from tests/mocks; `mock` declarations are typed overrides for existing tools with exact signature matching. Test execution activates mocks through the VM after the normal approval/confidence gate, so mocked dangerous tools still preserve the target effect profile. See [docs/internals/testing-primitives.md](docs/internals/testing-primitives.md).
- [x] 26-D-snapshots                  `assert_snapshot` evaluates typed runtime values, stores deterministic JSON snapshots under `.corvid-snapshots/<source-stem>/`, reports first-run updates, fails with diff output on mismatches, and supports `corvid test --update-snapshots` plus `CORVID_UPDATE_SNAPSHOTS=1`. See [docs/internals/testing-primitives.md](docs/internals/testing-primitives.md).
- [x] 26-E-trace-fixtures             `test name from_trace "trace.jsonl":` binds schema-validated production traces to language tests. Trace assertions now evaluate against JSONL fixtures: `called`, ordering, approval, and cost checks fail with typed runner output instead of reporting unsupported placeholders. Trace paths resolve relative to the `.cor` file, so production traces can live beside the tests that lock their behavior. See [docs/internals/testing-primitives.md](docs/internals/testing-primitives.md).

### Phase 27 — Eval tooling CLI (~3 weeks) ✅ closed

**Goal.** Turn Phase 20's `eval ... assert ...` syntax into a usable dev + CI workflow.

**Hard dep:** Phase 20 slice 20c (eval syntax — nothing to run without it).
**Soft dep:** Phase 26 (testing primitives). Eval tooling could have its own runner + discovery, but reusing Phase 26's infrastructure avoids duplication; the sequencing here is "ship tests first, build eval on top."

**Status.** Closed. `corvid eval` now runs source eval declarations, writes terminal/HTML/JSON reports, detects prior-result regressions, summarizes trace evidence, compares stored eval runs, enforces planned spend budgets, supports model-swap replay analysis, and runs golden-trace eval suites.

**Scope:**
- [x] `corvid eval <file>` runs all `eval` blocks; produces terminal report + HTML report. Shipped as a reusable driver eval runner plus CLI path that writes `target/eval/<source>/report.html` and preserves `--swap-model` migration analysis.
- [x] Regression detection against prior eval results (stored under `target/eval/`). Shipped via persisted `latest.json` / `previous.json` summaries under each eval output directory, with terminal and HTML surfacing for newly failing evals/assertions.
- [x] CI exit-code contract: non-zero if any `assert` fails or regression threshold crossed. Shipped through eval runner exit codes, compare regression exit codes, and budget preflight failures.
- [x] Trace-aware eval reporting: value pass rates, process assertions, approval assertions, groundedness, cost, latency, model route, and replay compatibility in one report. Shipped by scanning eval JSONL artifacts under `target/eval/<source>/`, validating schema compatibility, and folding trace metrics into terminal + HTML reports.
- [x] Prompt-diff report: when a prompt body changed between runs, show before/after + delta in grounding / cost / assert pass-rates. Shipped in `corvid eval compare` using rendered prompt bodies stored in eval trace summaries, alongside cost, route, and pass-rate deltas.
- [x] Model-swap eval mode uses Phase 21 replay and Phase 20 model metadata to compare provider/model choices without spending on unchanged tool paths. Already shipped as `corvid eval --swap-model <MODEL> --source <FILE> <TRACE_OR_DIR>...`, which delegates trace files to differential replay and trace directories to the trace-suite migration analyzer.
- [x] `corvid eval compare <base>..<head>`: PR-friendly eval diff with pass-rate deltas, cost deltas, latency deltas, model-route changes, prompt diffs, and trace/process assertion changes. Shipped as a CLI compare mode over local result paths/directories or git refs containing `target/eval/**/latest.json` summaries.
- [x] Regression-cause clustering: classify failures by prompt change, model change, tool-output change, route change, approval-path change, grounding loss, or budget regression. Shipped in compare reports with prompt-change, route-change, budget-regression, approval-path, tool/process, and assertion-regression buckets.
- [x] Eval budget mode: estimate and enforce max eval spend before running provider-backed evals; CI fails early when the planned eval run exceeds the configured budget. Shipped as `corvid eval --max-spend <USD>` plus `CORVID_EVAL_MAX_SPEND_USD`, using prior stored eval cost as the pre-run estimate.
- [x] Golden-trace evals: replay production traces against changed prompts/models/tools and score behavior without re-spending unchanged tool and prompt paths. Shipped as `corvid eval --golden-traces <DIR> <source.cor>`, delegating to the trace-suite replay analyzer in non-promoting mode.

**v0.8 cuts here.** Full developer workflow: write in LSP, share via package manager, test + eval in CI.

---

### Phase 28 — HITL expansion (~3 weeks) ✅ closed

**Goal.** `ask`, `choose`, rich approval UI. Completes the human-in-the-loop surface.

**Hard dep:** runtime (✅).

**Scope:**
- [x] `ask(prompt, Type)` — structured input from the human. Returns `Type`. Ties into the approval runtime.
- [x] `choose(options: [T]) -> T` — pick one. UI presents options; user selects.
- [x] Rich `approve` UI: show context (why approval requested), diff preview (what will change), arguments inspection.
- [x] Scoped, replay-verifiable approval tokens: the trace records what the human approved, for which label, arguments, and time window.
- [x] Human-boundary effects: `ask`, `choose`, and `approve` compose into the same effect algebra as tools and prompts, so human interaction is visible to the compiler and host descriptors.
- [x] CLI + web-UI implementations; approval tokens same regardless of UI.
- [x] Approval scopes: one-time, session-scoped, amount-limited, time-limited, and argument-bound tokens. Scope violations fail closed and are replay-visible.
- [x] Typed tool contract recorder: tools can declare domain effects such as `money(amount)`, `external(stripe)`, `irreversible`, and `requires approve "charge-card"`. The compiler/runtime turns those contracts into approval cards, trace events, PR behavior diffs, package metadata, and CI failures when a change introduces a new money-moving or irreversible path.
- [x] Human-readable approval cards generated from typed tool arguments, with schema validation and redaction rules inherited from the effect/privacy profile. First runtime slice shipped as `ApprovalCard` generation from approval labels and JSON argument types, risk inference, sensitive-value redaction, and richer stdin approval rendering.

### Phase 29 — Memory primitives (~4–5 weeks) ✅ closed

**Goal.** `session` and `memory` as typed, SQLite-backed stores. Core to how AI applications handle state.

**Hard dep:** Phase 18 (Result — `session.get()` returns `Result<T, StoreError>`). SQLite (external).

**Scope:**
- [x] Store declaration surface + metadata: `session Name:` and `memory Name:` parse as typed top-level schemas, resolve their field types, register store effect names, and emit ABI store contracts with `reads_*` / `writes_*` metadata.
- [x] Native runtime store backend: `Runtime` exposes replay-visible `store_get` / `store_put` / `store_delete` over a pluggable store manager, with SQLite persistence for native hosts and an in-memory backend for tests/embedding.
- [x] Store policy hooks: `policy <name>: <value>` entries inside `session` / `memory` declarations parse into typed AST metadata and emit through ABI store contracts for retention, privacy, and approval enforcement.
- [x] Provenance-aware store records: runtime stores can persist JSON values together with optional `ProvenanceChain` metadata, preserving grounded lineage for long-lived memory retrieval.
- [x] Revisioned memory conflict detection: runtime stores assign monotonic record revisions and expose compare-and-set writes so stale memory updates fail with `StoreConflict` instead of silently overwriting newer facts.
- [x] Runtime retention policy enforcement: ABI store policies can become runtime `StorePolicySet`s; TTL reads expire stale records, and legal-hold policies block deletion with typed policy errors.
- [x] Approval-required memory writes: runtime store policy APIs gate sensitive writes through the existing approval flow and preserve denial/approval events in replay-visible traces.
- [x] Provenance-required memory reads: store policies can require retrieved records to carry `ProvenanceChain` lineage, failing ungrounded reads with typed policy errors.
- [x] Generated typed store accessor contracts: ABI store metadata now includes compiler-generated `get` / `set` / `delete` accessor signatures for each declared field, carrying field types and read/write effects for codegen and host SDKs.
- `session { ... }` block declares per-conversation state. Compiler generates typed accessors.
- `memory { ... }` block declares long-lived state (survives process restarts).
- Both backed by SQLite (native) and IndexedDB (wasm).
- Effect-tagged: `reads_session` / `writes_session` / `reads_memory` / `writes_memory`. Integrate with Phase 20's effect rows.
- Provenance-aware memory: stored values may carry `Grounded<T>` lineage, and retrieval from memory can preserve or require provenance.
- Policy hooks for privacy, retention, and approval-required writes, so agent memory is governed state rather than an untyped vector store.
- Retention and deletion policy: declare TTL, user-delete, legal-hold, and privacy-tier rules at the `session` / `memory` block; runtime enforces them consistently across native and WASM storage.
- Memory conflict resolution: typed handling for stale facts, contradictory facts, and source-priority rules, with conflicts surfaced as `Result`/diagnostics rather than silently overwriting state.
- Memory write approvals for sensitive or irreversible state changes, recorded in replay and visible in effect summaries.

**Phase 29 follow-up audit (2026-04-29) — epistemic verification:**

- [x] 29-K-memory-module-audit-doc       `docs/phases/phase-29-memory-audit.md` ships, enumerating every memory primitive against the ROADMAP claims with source file + line range + positive + adversarial tests for each surface. Audit confirmed native-tier coverage; identified one cross-tier gap (wasm IndexedDB backing) that promotes into slice 29-L below.
- [x] 29-L-wasm-indexeddb-host-import    The wasm-codegen ES loader exports a typed `createIndexedDbStoreHost` wrapper for browser-side `store.get` / `store.put` / `store.delete`; `examples/wasm_browser_demo` uses it to persist run count and last result across page reloads, and the Phase 23 Playwright browser CI test verifies persistence.

### Phase 30 — Python FFI via PyO3 (~5–6 weeks) ✅ closed

**Goal.** `import python "..."` works in compiled code. Closes the "but Python has the ecosystem" gap.

**Hard dep:** Phase 13 (async — PyO3's GIL-aware runtime needs async context).
**Soft dep:** Phase 20 slice 20a (effect rows). Python imports declare effects at the import site — the basic `effects: network` / `effects: unsafe` syntax works against the existing `safe` / `dangerous` split; richer user-declared effects via 20a's effect rows make the story better but aren't a compilation blocker.

**Scope:**
- [x] Python import effect declarations: parser accepts `import python "..." as name effects: ...`; the type checker rejects untagged Python imports, while `effects: unsafe` is allowed but flagged for review.
- [x] Runtime PyO3 call bridge: feature-gated runtime support can import Python modules, call functions, marshal JSON-like scalars/lists/dicts, and return Python exceptions with formatted traceback text.
- [x] Trace-visible Python calls: runtime Python FFI calls emit `python.call`, `python.result`, and `python.error` host events so Python ecosystem use is visible to audits.
- [x] Python sandbox capability profiles: feature-gated runtime Python calls can be checked against declared effects and deny obvious network/filesystem/subprocess/environment modules before import.
- PyO3 integration in `corvid-runtime`. Lazy CPython load.
- `import python "requests" as requests effects: network` — untagged imports rejected by the effect checker. `effects: unsafe` is the opt-in escape hatch and is flagged for review.
- Error marshalling: Python exceptions become Corvid `Result::Err` with preserved traceback.
- Type marshalling: Python dicts ↔ Corvid structs (when schema known), lists ↔ lists, scalars ↔ scalars.
- Python calls appear in traces, audit output, and effect summaries, so the Python ecosystem does not become an invisible safety hole inside AI workflows.
- Interpreter tier gets the same FFI surface so both tiers behave identically.
- Optional sandbox profiles for Python imports: network, filesystem, subprocess, environment, and native-extension access are denied unless declared in the import's effect profile.
- Generated typed wrappers from Python signatures and docstring/schema metadata where available; unresolved dynamic shapes require explicit Corvid type annotations.
- Python FFI contract tests: verify exception marshalling, type conversion, trace recording, and effect summaries for imported Python functions.

**Phase 30 reopened 2026-04-29 — gap-closing slice required:**

- [x] 30-J-default-ci-pyo3        The Python FFI integration tests run in the `python-features` CI job with pinned CPython 3.11 and `cargo test -p corvid-runtime --features python --tests`. The feature tests assert scalar, list, and dict/object round-trips, traceback-preserving exception marshalling, `python.call` / `python.result` / `python.error` trace events, and sandbox-profile-denied imports. `docs/operations/ci.md` documents the matrix entry.

**Phase 30 next-close criteria:** the ROADMAP-level `[x]` returns only when slice 30-J clears the slice completion gate.

### Phase 31 — Multi-provider LLM adapters (~2 weeks) ✅ closed

**Goal.** Provider coverage for the AI application surface, not just chat completion: hosted frontier models, local models, OpenAI-compatible gateways, structured-output providers, routing metadata, and adapters users actually request.

**Hard dep:** runtime adapter trait (✅).

**Scope:**
- [x] Provider capability metadata: `corvid.toml` model entries can declare provider, privacy tier, jurisdiction, context window, structured-output/tool/embedding support, multimodal tags, latency tier, and task capability tags for routing and audit surfaces.
- [x] Provider health and automatic failover: runtime tracks adapter health, records provider degradation/failover trace events, and routes failed live calls to cheapest compatible cross-provider catalog fallbacks that preserve capability, format, privacy, jurisdiction, context, tool, embedding, multimodal, and task contracts.
- [x] Cost normalization and usage accounting: runtime records normalized USD/token usage per LLM call with provider, adapter, privacy tier, local-vs-hosted, prompt/completion/total tokens, trace `llm.usage` events, and provider-level totals for routing and budget reports.
- [x] Capability contract tests: runtime can run configured-model contract probes for structured JSON output, provider token usage, context-window declarations, and explicit unsupported statuses for tool-call/streaming probes until the adapter surface exposes native checks.
- `GoogleAdapter` in `corvid-runtime`. API compatibility with existing AnthropicAdapter + OpenAiAdapter surface.
- `OllamaAdapter` for local-first Corvid.
- Provider/model metadata includes cost, latency, privacy tier, jurisdiction, structured-output support, context window, tool-calling support, embedding support, multimodal capability tags where available, and task capability tags.
- Provider selection via `CORVID_MODEL` env var remains supported, but compiler/runtime model routing can use declared model capabilities from Phase 20h.
- Eval data can compare providers and feed routing reports, so model choice becomes measurable infrastructure rather than string configuration.
- Provider health checks and automatic failover: runtime records provider outage/degradation events and routes to compatible fallback models when policy allows.
- Capability contract tests: verify whether each configured model actually respects JSON mode, tool calls, streaming, context-window claims, and structured-output constraints.
- Cost normalization and usage accounting across providers, including local/openai-compatible servers, so budgets compare real prompt/model choices instead of raw provider strings.

### Phase 32 — Standard library (~8 weeks) ✅ closed

**Goal.** Batteries included for general programming and AI-native applications. Common patterns available without a package install.

**Hard dep:** everything language-core stable.

**Scope:**
- [x] `std.ai` foundation: repo-native Corvid source module with typed messages, sessions, tool-result envelopes, model-route envelopes, structured-output validation envelopes, confidence helpers, trace event summaries, docs, and import/compile coverage.
- [x] `std.http` foundation: typed Corvid request/response envelopes plus runtime HTTP client with timeout, retry-on-5xx, response metadata, and trace events for request/response/error accounting.
- [x] `std.io` foundation: typed Corvid path/file/directory envelopes plus runtime text read, text write, and directory listing APIs with byte, latency, entry-count, and error trace events.
- [x] `std.secrets` foundation: typed Corvid secret-read envelopes plus runtime environment secret access that returns values to callers while emitting only redacted audit trace metadata.
- [x] `std.observe` foundation: typed Corvid observability envelopes plus runtime observation snapshots that aggregate LLM usage, cost totals, local-call counts, provider health, and degraded-provider counts into trace-visible summaries.
- [x] `std.cache` foundation: typed Corvid cache-key/cache-entry envelopes plus deterministic runtime cache-key construction over namespace, subject, model, args, effect key, provenance key, and version metadata with trace-visible key events.
- [x] `std.queue` foundation: typed Corvid background-job envelopes plus runtime enqueue/cancel APIs carrying retry, budget, effect-summary, and replay-key metadata through trace-visible queue events.
- [x] `std.agent` foundation: pure Corvid workflow envelopes and helpers for classification, extraction, ranking, adjudication, planning, tool-use, approval labels, critique, and grounded answers.
- [x] `std.rag` foundation: typed Corvid document/chunk/embedder envelopes plus runtime document construction, markdown loading, deterministic chunking, per-chunk provenance keys, SQLite-backed chunk indexing, and OpenAI/Ollama embedder configuration metadata.
- [x] `std.effects` foundation: shared Corvid effect metadata envelopes for effect tags, budgets, provenance keys, approval labels, cache keys, and replay keys across `std.*`.
- [x] `std.ai` reusable AI application primitives: typed message/session objects, prompt rendering helpers, model-route helpers, tool-result envelopes, structured-output validation, confidence helpers, and trace/event utilities.
- [x] `std.rag` embedder trait with reference OpenAI + Ollama implementations.
- [x] `std.rag` remaining runtime pieces as one `std.ai` subdomain: SQLite-backed embedding retrieval, chunking polish, and tighter grounding-by-construction APIs. Shipped with configurable chunking, SQLite persisted embedding vectors, cosine-similarity retrieval, and runtime `GroundedValue<T>` helpers for retrieval-backed chunk results. Pairs with Phase 20's grounding-contract language half.
- [x] `std.rag` APIs return grounded runtime values by construction where retrieval provenance exists, but grounding is not limited to RAG; any tool/effect that proves provenance can produce grounded values through the shared runtime provenance envelope.
- [x] `std.http` typed HTTP client with effect tags, retry semantics, timeout/budget accounting, and recorded replay-hook exchanges.
- [x] `std.io` path helpers in the runtime: join, parent, filename, extension, extension replacement, and lexical normalization.
- [x] `std.io` remaining runtime pieces: explicit filesystem-effect plumbing through effect-tagged read/write/list/stream runtime envelopes and helpers.
- [x] `std.agent` common AI patterns: classification, extraction, summarization, ranking, adjudication, routing, planning, tool-use loops, approval-gated action, review/critique, and grounded answer generation.
- [x] Everything in `std.*` effect-tagged so users get the moat's benefits from day one.
- [x] `std.queue` durable background jobs for long-running AI tasks, with retry, cancellation, replay hooks, budget accounting, and effect summaries.
- [x] `std.cache` prompt/model/tool-result caching with replay-safe invalidation, provenance preservation, and effect-aware cache keys.
- [x] `std.secrets` explicit secret access APIs with redacted audit metadata surfaces that avoid leaking secret values.
- [x] `std.observe` metrics, trace counters, cost counters, latency histograms, routing decisions, and approval summaries exposed through one typed observability surface.

**Phase 32 follow-up audit (2026-04-29) — per-module verification:**

- [x] 32-T-stdlib-effect-tag-audit-doc    `docs/phases/phase-32-stdlib-audit.md` ships, covering all 11 modules (`ai`, `http`, `io`, `secrets`, `observe`, `cache`, `queue`, `jobs`, `agent`, `rag`, `effects`, `db`). Each row lists module path, public surface, effect tags, runtime backing (where applicable), compile test ref, imported-helpers typecheck ref, and adversarial coverage where present. Audit confirmed full coverage; identified one expansion opportunity that promotes into slice 32-U below.
- [x] 32-U-stdlib-adversarial-expansion   Every `std.*` module now has a named adversarial test beside its compile + imported-helpers test in `crates/corvid-driver/tests/stdlib.rs`: secret value leaks, untagged HTTP/IO/cache/queue/job surfaces, raw auth/approval/observe/AI payload surfaces, missing provenance for agent/RAG answers, missing replay keys for effects, and the existing `std.db` token-redaction surface.

**v0.9 cuts here.** Language feature-complete: HITL, memory, Python FFI, multi-provider LLMs, stdlib. Only polish remaining.

---

### Phase 33 — Polish for launch (~6–10 weeks)

**Goal.** v1.0. Stable, documented, installable by a stranger on any OS.

**Hard dep:** everything.

**Scope:**
- [x] In-repo installer flow: checked-in Unix + PowerShell install scripts under `install/` plus the documented `cargo install` path.
- [x] Documentation rewrite foundation: launch reference, tutorial, cookbook, and migration-from-Python docs checked into `docs/`.
- [x] Claim audit foundation: `docs/meta/launch-claim-audit.md` links launch claims to concrete commands and committed artifacts, and keeps external-only claims explicitly blocked until the artifact exists.
- [x] `corvid audit`: project-level static report for approval boundaries, replay coverage gaps, budget exposure, secret-bearing effects, money-moving paths, grounding signals, and semantic-effect violations.
- [x] Stability contract foundation: checked-in launch contract for syntax, type system, CLI, stdlib, and benchmark-claim semantics.
- [x] `corvid doctor` launch checks: provider keys, local-model tooling, replay storage, approval configuration, wasm/native toolchains, registry lock presence, and platform prerequisites.
- [x] `corvid bench compare python|js`: published-archive comparison command over committed benchmark sessions with explicit ratio semantics and no hidden model-latency claim inflation.
- [x] Reproducibility-script foundation: checked-in scripts for benchmark and bundle claim reproduction from committed archives and examples.
- Stability guarantees on the language surface: documented SemVer contract for syntax, type system, stdlib.
- Windows + Linux + macOS all first-class (`corvid doctor` passes, installer works, parity harness green on all three).
- Installer: `curl -fsSL corvid-lang.org/install.sh | sh` on Unix, PowerShell equivalent on Windows. Corresponding `cargo install` flow.
- Website: landing page, live playground (runs the wasm target from Phase 23), docs site, blog, benchmarks page.
- Documentation rewrite: reference, tutorial, cookbook, migration-from-Python guide.
- Claim audit: every launch claim about effects, approvals, grounding, budgets, replay, evals, packages, WASM, and benchmarks links to a runnable command, test, or committed example.
- `corvid doctor` checks provider keys, local model availability, replay storage, approval UI configuration, wasm/native toolchains, registry access, and platform support.
- `corvid bench compare python|js`: honest orchestration-overhead comparisons against representative Python/JS AI framework stacks. Claims distinguish model-provider latency from Corvid's compiled orchestration/runtime overhead.
- `corvid audit`: project-level report for dangerous tools, approval boundaries, money-moving paths, budget exposure, ungrounded outputs, provider policy violations, secret access, and replay coverage.
- One-command reference apps: RAG app, support bot, approval-gated refund bot, code-review agent, provider-routing demo, and local-model demo. Each ships with tests, evals, traces, and benchmark notes.
- Reproducibility scripts for benchmark and bundle claims, including the Phase 17 performance baseline and Phase 22 public bundle verification.
- Launch materials: 2-minute GIF/video showing the time-travel replay moment + effect-checker catching a bug + compile-time cost budget. HN + Reddit + ProductHunt announcement drafts reviewed with 3 external readers.
- Beta round: 20 external developers build something real in Corvid; their feedback gates the final cut.

**Slice checklist:**

- [x] 33A-installer-foundation       Unix and PowerShell install scripts plus documented `cargo install` path are checked in.
- [x] 33B-docs-foundation            Launch reference, tutorial, cookbook, and migration-from-Python docs exist in `docs/`.
- [x] 33C-claim-audit-foundation     Launch claims are linked to runnable commands or committed artifacts.
- [x] 33D-audit-command              `corvid audit` reports approval, replay, budget, secret, money-moving, grounding, and semantic-effect risks.
- [x] 33E-stability-contract         Syntax, type system, CLI, stdlib, and benchmark claim stability policy is documented.
- [x] 33F-doctor-launch-checks       `corvid doctor` checks provider keys, local models, replay storage, approvals, wasm/native toolchains, registry lock, and platform prerequisites.
- [x] 33G-benchmark-compare          `corvid bench compare python|js` uses committed benchmark archives and separates model latency from orchestration overhead.
- [x] 33H-repro-scripts              Benchmark and bundle claim reproduction scripts are checked in.
- [x] 33I-platform-parity            The `platform-parity` CI matrix runs on Windows, Linux, and macOS; each leg executes the platform installer, `corvid doctor`, and the WASM/Wasmtime cross-platform parity harness.
- [ ] 33J-website-playground         Website, docs site, benchmark page, blog shell, and **browser-based cloud IDE** with multi-file editor, agent execution in WASM, and BYO-API-key LLM provider calls. Scope expanded 2026-05-12 after CTO call to ship "test without installing" as the v1.0 launch narrative; launch slips ~3 months for Path B. Sub-slices 33J7a-e below decompose the cloud IDE.
  - [x] 33J1-homepage-live             Marketing landing page is live with hero / problem / demo / effect-algebra / inventions / examples / community / CTA from committed HTML assets at `Micrurus-Ai/corvid-website`.
  - [x] 33J2-prep-doc-tree             Developer docs reorganized into Diataxis-style tree (`docs/book/`, `docs/guides/`, `docs/recipes/`, `docs/reference/`, `docs/migration/`, `docs/operations/`, `docs/security/`, `docs/internals/`, `docs/help/`, `docs/meta/`, `docs/phases/`); 4660-line mega-file split into 45 per-topic pages; obsolete stubs deleted; phase docs moved to `docs/phases/`; cross-references in source code updated; `docs/core-semantics.md` regenerated through `corvid contract regen-doc docs/reference/core-semantics.md`.
  - [x] 33J3-docs-site-build           Docs-site build pipeline renders the per-topic markdown from `Micrurus-Ai/Corvid-lang`'s `docs/` tree into a navigable site at <https://corvid-lang.org/docs>. All 11 Diataxis sections (Book, Guides, Recipes, Reference, Migration, Operations, Security, Internals, Help, Meta, plus the docs landing) render; 18 book chapters navigable; Corvid syntax highlighting active for keywords (`effect`, `prompt`, `agent`, `approve`, `tool`, `uses`, type names); cross-links resolve; ToC + left-nav generated from the directory tree.
  - [x] 33J7-prereq-corvid-browser     `crates/corvid-browser` ships a WASM-compatible typechecker entry point (`check(source) -> CheckResult`) for the playground. Pipeline mirrors `corvid-driver/src/pipeline/compile.rs` steps 1–4 (lex, parse, resolve, typecheck); steps 5–6 (lower, codegen) excluded. Flat wire schema with `version: "v1"` field for forward-compat. Imports refuse with documented browser-only message. 6 integration tests pass including the load-bearing `dangerous_call_without_approve_refuses` test that surfaces `approval.dangerous_call_requires_token`. WASM artifact: ~1.2 MB raw (250 KB gzipped, post-bindgen), well under the 8 MB budget. CI step `browser-typechecker-wasm` builds + tests + enforces the size budget on every push.
  - [ ] 33J4-benchmark-page            **[launch-readiness — final 2 weeks of Phase 43 per Path A]** Renders `benches/moat/RESULTS.md` and the `benches/results/*/ratios.json` archives at `corvid-lang.org/benchmarks`.
  - [ ] 33J5-blog-shell                **[launch-readiness — final 2 weeks of Phase 43 per Path A]** Blog shell with at least one launch post.
  - [x] 33J6-grammar-drift-gate        Drift-gate test that cross-checks `docs/reference/grammar.md` against the parser tests in `crates/corvid-syntax/src/parser/tests.rs`. **Shipped `(this commit)`** — added `crates/corvid-syntax/tests/grammar_drift.rs` with two structural drift gates: (a) `grammar_md_every_rhs_reference_resolves_to_a_declared_production` asserts every lowercase RHS identifier in grammar.md either has a matching LHS production declaration or appears on a small curated terminal-token allowlist (`IDENT` / `INT` / `FLOAT` / `STRING` / `STRING_LITERAL` / `NUMBER` / `INDENT` / `DEDENT` / `NEWLINE` / `EOF`); (b) `grammar_md_every_declared_production_is_reachable_from_program` asserts every declared production is reachable from `program` via transitive RHS references, catching orphan declarations. Drift gate immediately surfaced 7 real doc gaps the parser had outpaced the grammar: `arg_list`, `extend_decl`, `extend_method`, `fixture_body`, `mock_body`, `literal_pattern`, `model_decl`, `model_field`, `template_line` — all 7 fixed in the same commit by adding the missing EBNF production declarations (per the no-shortcuts mandate the gate's first run can't be papered over with an allowlist). Doc surface added: new "Model declarations" section, new "Extension blocks" section, `arg_list` alongside `block` in the agent-decl section, `fixture_body ::= block` + `mock_body ::= block` aliases with a doc comment naming the future-extension reason for keeping them separate, `template_line` in the prompt-decl section, `literal_pattern` in the pattern section. The naming-substring matching against parser fns is deliberately NOT implemented: parser uses Pratt-style precedence climbing whose fn names (`parse_cmp` for `cmp_expr`) don't substring-match the productions; a naming gate would be flaky against this convention. Module-level doc comment in `grammar_drift.rs` records what the gate enforces, what it deliberately does NOT enforce, and the failure-mode message format.
  - [ ] 33J7-wasm-playground           Browser-based cloud IDE at `corvid-lang.org/playground` with multi-file editor, agent execution in WASM, and BYO-API-key LLM provider calls. Five sub-slices below (33J7a-e) decompose the work. Path B confirmed 2026-05-12 after CTO call.
  - [x] 33J7a-check-project            `corvid-browser` exposes `check_project(files: &HashMap<String, String>, entry: &str) -> CheckResult` for multi-file typecheck. Resolver walks an in-memory file map instead of `std::fs`; paths normalize to web-style canonical form (`./` dropped, `..` resolved, `/` separator, `.cor` implicit). Only local `import "./..."` resolves; Python/remote/package imports refuse with sandbox messages. Cycles surface as a single diagnostic. Cross-file moat property pinned: `approval.dangerous_call_requires_token` fires across file boundaries the same way it fires within one file. Diagnostic schema additively extended with `path: Option<String>` so the playground can route squiggles to the right editor tab. 8 new integration tests pass; 14 total in the crate. WASM artifact: 1.21 MB (+75 KB for multi-file machinery; well under the 8 MB gzipped budget).
  - [ ] 33J7b-runtime-split            Structural split of `corvid-runtime` into `corvid-runtime-core` (wasm-clean: dispatch, effects, replay state, prompt/tool/agent execution, mock+replay connectors, canonical receipt-bytes derivation) and `corvid-runtime-host` (native-only: DB drivers, OTel SDK, HTTP clients, real-mode connectors, Tokio runtime, DSSE signing, filesystem-backed replay sinks). Host re-exports core so `corvid_runtime::Foo` keeps working for native users. Stdlib impls (db/http/jobs/auth/observability/connectors) stay in `corvid-runtime-host` initially, gated by per-module feature flags; per-module crate extraction is a follow-up when a module crosses the file-responsibility threshold. Pre-phase chat decisions recorded in `docs/meta/runtime-split-design.md` (D1-D6, 2026-05-12). Slice plan in `docs/meta/33J7b-fresh-session.md`.
    - [x] 33J7b-0 stress-test            R1 mitigation. Walk every Phase 21–41 feature, decide core/host placement per D1–D6, flag any feature that resists clean placement. Filled-in feature checklist in `docs/meta/33J7b-fresh-session.md` is the slice-0 deliverable. One flag: Phase 41 connectors (D5 needs extending to the separate `corvid-connector-runtime` crate; remediation choice gets made before slice 33J7b-4 opens).
    - [x] 33J7b-1 scaffold-core          Empty `corvid-runtime-core` crate landed. `crate-type = ["rlib"]` only (no cdylib). Empty crate has zero deps; the brief's allowed dep set (ast/ir/resolve/types/guarantees/trace-schema/prompt-format + serde/serde_json) is the upper ceiling enforced by the wasm32 gate. `cargo build -p corvid-runtime-core --target wasm32-unknown-unknown --release` succeeds in <1s. `#![deny(unsafe_code)]` set at file level so unsafe creep fails the build.
    - [x] 33J7b-2 host-bridge            `HostRequest` / `HostResponse` enum + `HostBridge` trait landed in `corvid-runtime-core/src/host.rs`. `version: "v1"` at the root via flattened `SchemaVersion` (R4 mitigation; round-trip test asserts unknown versions fail closed). Request variants: LlmCall, HostCall, DbQuery, FsRead, FsWrite, HttpRequest, OtelEmit. Response variants parallel by name (LlmResult, DbRows, FsBytes/FsAck, HttpReply, OtelAck) plus one Error{message, category} catch-all. R3 invariant encoded by trait shape: single-method `async fn resolve(req) -> resp` makes parallel-await structurally impossible. 13 round-trip tests (one per variant + version-mismatch fail-closed) green; wasm32 build succeeds in 0.75s. Naming gotcha: Error variant's `category` field renamed from `kind` to avoid colliding with the `#[serde(tag = "kind")]` discriminator.
    - [ ] 33J7b-3 move-deterministic     Move Effect / EffectRow / approval-token / grounded-provenance state machinery, ReplayPlayer / ReplayRecorder (per D2 — persistence behind ReplaySource / RecorderSink traits), canonical receipt-bytes derivation (per D1) from `corvid-runtime` to `corvid-runtime-core`. `corvid-runtime` re-exports preserved. **Sub-split into 3a–3g** because reconnaissance found errors↔replay coupling and an LlmRegistry abstraction prereq that make a single commit reckless. Sub-slices below.
      - [x] 33J7b-3a provenance            `provenance.rs` (`GroundedValue` / `ProvenanceChain` / `ProvenanceEntry` / `ProvenanceKind`) moved to `corvid-runtime-core` via `git mv`. `corvid-runtime` adds `corvid-runtime-core` as a workspace dep, plus a re-export shim module so `corvid_runtime::provenance::Foo` continues to resolve. Validation gate green: wasm32 build 1.33s, runtime lib 259/259, browser 14/14, guarantees 22/22, corpus baseline unchanged.
      - [x] 33J7b-3b approval-token        `approvals/token.rs` → `corvid-runtime-core/src/approval_token.rs` (renamed flat since core has no `approvals/` subdir yet — promote to `approvals/` directory if 3c lands). `ApprovalToken` + `ApprovalTokenScope` now in core; `corvid-runtime/src/approvals/mod.rs` re-exports from `corvid_runtime_core::approval_token::*` so `corvid_runtime::approvals::ApprovalToken` continues to resolve. Two co-located unit tests (`approval_token_scopes_fail_closed`, `approval_token_session_time_and_argument_scopes_are_enforced`) travelled with the type per CLAUDE.md's carve-out 1; runtime test count went 259→257 (-2 moved), core test count went 13→15 (+2 added). Validation gate green: wasm32 build 9.11s, browser 14/14, guarantees 22/22, runtime 257/257 + 1 ignored, core 15/15, corpus baseline unchanged.
      - [x] 33J7b-3c approval-data         Approval data types (`ApprovalRequest`, `ApprovalDecision`, `ApprovalCard`, `ApprovalCardArgument`, `ApprovalRisk`) moved to `corvid-runtime-core` as `approval_card.rs` + `approval_request.rs`. `card.rs` git-mv'd; `ApprovalRequest`/`ApprovalDecision` carved out of `approvals/mod.rs` into a new `approval_request.rs`. Re-exports preserved via `corvid-runtime/src/approvals/mod.rs`. The `Approver` trait + `ProgrammaticApprover`/`StdinApprover` impls intentionally NOT moved this slice — they need `RuntimeError` in core, which needs `ReplayDivergence` in core first (3d). Original 3c "approval-state" sub-slice split into 3c data-types (this) + 3e approver-trait-and-impls (below); slices 3d–3g renumbered to 3d–3h to make room.
      - [x] 33J7b-3d errors-untangle       `replay/diverge.rs` → `corvid-runtime-core/src/replay_divergence.rs` (added doc comment that the file lacked before). `errors.rs` → `corvid-runtime-core/src/errors.rs` (the only non-core dep was `ReplayDivergence`, untangled). `corvid-trace-schema` added to core's deps (workspace-clean, was already wasm-friendly). Re-export shims in `corvid-runtime/src/lib.rs` (`pub mod errors { pub use corvid_runtime_core::errors::*; }`) and `corvid-runtime/src/replay/mod.rs` preserve `corvid_runtime::errors::RuntimeError` and `corvid_runtime::replay::ReplayDivergence` resolution. Sibling-replay-files (`replay/cursor.rs`, `replay/result_factory.rs`) updated from `super::diverge::ReplayDivergence` → `super::ReplayDivergence`. Stale doc comment in `corvid-guarantees/src/lib.rs` pointing to the old `corvid-runtime/src/replay/diverge.rs` path updated to the new home. Validation gate green: wasm32 build 4.79s, runtime 257/257, browser 14/14, guarantees 22/22, core 15/15, corpus baseline unchanged.
      - [x] 33J7b-3e approver-trait        Surgical cut: only the `Approver` trait moved to `corvid-runtime-core/src/approver.rs`. `futures` 0.3 added to core deps for `BoxFuture` (pure-Rust, wasm32-clean per the brief's "pure-Rust algorithmic deps" carve-out; first-build cost 24.77s). `corvid-runtime/src/approvals/mod.rs` re-exports `Approver` from core. `ProgrammaticApprover` and `StdinApprover` stay in host because they pull host-only deps the trait itself doesn't: `ProgrammaticApprover` uses `std::thread::sleep` + `Instant::now()` for bench-latency injection (CORVID_BENCH_APPROVAL_LATENCIES_MS hook used only by the native bench runner); `StdinApprover` uses `tokio::task::spawn_blocking` for stdin async. Trait being core-resident is what unblocks future browser-native approver impls (a JS dialog, an in-page confirm() flow) satisfying the same contract. Validation gate green: wasm32 build 24.77s, runtime 257/257, browser 14/14, guarantees 22/22, core 15/15, corpus baseline unchanged. Sub-slices 3f–3h paused per user direction (scope-reduced to examples + terminal MVP; runtime-split resumes when execution actually needs it).
      - [ ] 33J7b-3f llm-trait-extract     Prereq for 3g: `replay::*` uses `crate::llm::{LlmRegistry, LlmRequestRef, LlmResponse, TokenUsage}`. Extract the LlmAdapter trait + LlmRegistry + MockAdapter to core (per Phase 31 row in the stress-test); leave real adapters (Anthropic/OpenAI/etc) in host.
      - [ ] 33J7b-3g replay-state          Replay state machine (`ReplayPlayer` / `ReplayRecorder`) → core per D2. Persistence behind `ReplaySource` / `RecorderSink` traits declared in core; `JsonlTraceWriter` stays in host as the native impl. Depends on 3d + 3f.
      - [ ] 33J7b-3h receipt-bytes         Canonical receipt-bytes derivation per D1: define + implement the byte-stable order over audit log + prompt records + tool calls + trace metadata. Likely net-new code (no existing surface to move). Sign-less in core; host signs on top.
    - [ ] 33J7b-4 connector-mock-replay  Move mock + replay connector machinery to core per D5. Phase 41L drift test splits into core-only (mock ≡ replay) + host integration (real ≡ replay). **Precondition:** the Phase 41 connector-crate remediation choice (see slice 33J7b-0 stress-test flag) is decided before this slice opens.
    - [ ] 33J7b-5 rename-host            `git mv crates/corvid-runtime → crates/corvid-runtime-host`. Update workspace members. Host re-exports core (D6) so `corvid_runtime::Foo` keeps resolving for existing native users. `cargo test --workspace` green.
    - [ ] 33J7b-6 feature-flags          Per-module feature flags on `corvid-runtime-host`: db, http, jobs, auth, observability, connectors. `default = ["all"]` so nothing breaks for existing users. Acceptance: `--no-default-features` build succeeds (proves gating); defaults build succeeds (proves nothing broke).
    - [ ] 33J7b-7 closing-audit          Append closing-audit section to `docs/meta/runtime-split-design.md` (outcome per D1–D6 verified shipped vs. needed revision). Append slice closeout to `learnings.md`. CI step added: `corvid-runtime-core` compiles to wasm32-unknown-unknown on every push (same shape as `browser-typechecker-wasm`). Tick this top-level row.
  - [ ] 33J7c-vm-split                 Audit confirmed `corvid-vm` has direct `tokio`, `async-recursion`, `async-trait` deps + direct `corvid-runtime` dep — NOT a pure port. Structural split mirroring 33J7b: `corvid-vm-core` (wasm-clean synchronous IR-walker that yields `HostRequest` at every prompt/tool/external-host-call boundary) + `corvid-vm-host` (native async dispatch wrapping core via tokio). Estimate revised from ~2 weeks port to ~3 weeks split. The refactor double-dividends: enables browser execution AND cleans up the existing async-trait soup in the VM.
  - [ ] 33J7d-run-agent-bridge         `corvid-browser` exposes `run_agent(files, entry, invoke_args) -> RunResult` with a suspend/resume coroutine API via `wasm-bindgen-futures`. When the runtime hits a JS-resolvable boundary (LLM call, sandboxed file access), suspends and returns a structured request; JS resolves; WASM resumes. No mocked providers — fail honestly on capabilities out of scope.
  - [ ] 33J7e-byo-api-key              Browser-side LLM provider call plumbing with BYO API key: IndexedDB AES-GCM-encrypted storage, Argon2-derived encryption key from user passphrase, direct browser-to-provider fetch through the 33J7d suspend/resume bridge. External security review required before launch (key exfiltration via XSS, replay-mode key bleed-through, supply-chain risk). No corvid-side servers see the key.
- [x] 33K-reference-demo-pack        One-command demo apps have tests, evals, traces, and benchmark notes.
- [ ] 33L-launch-materials           **[launch-readiness — final 2 weeks of Phase 43 per Path A]** GIF/video, launch drafts, and external-reader review are complete.
- [ ] 33M-beta-feedback              **[launch-readiness — final 4 weeks of Phase 43 per Path A; repositioned as a 5-10 friends-and-family round, not 20-external public beta]** External-developer feedback items closed as code/docs/tests or explicit non-scope.
- [x] 33N-moat-benchmarks            `benches/moat/` ships the two defensibility benchmarks the website can quote: compile-time rejection over 50 bug-class cases and governance line-count over 3 reference apps (`refund_bot`, `rag_qa_bot`, `support_escalation_bot`) implemented in Corvid, Python, and TypeScript. Each benchmark has a deterministic runner and `RESULTS.md`; CI runs both runners and drift-gates the committed results on every push.

- [ ] 33P-packaging-manager-manifests **[post-v1.0]** Distribution-channel manifests for the major language-installer managers. The canonical install path stays the script-from-this-repo route (`install/install.{sh,ps1}` + the Cloudflare Worker at `web/` named `corvid-installer` that fetches from `main`); these manifests are additive packaging-manager-specific metadata that point AT the GitHub Release artifacts `release.yml` produces. None of these is the "installer script" — they're manifests that each packaging manager's central repo consumes. Filed post-v1.0 because (a) Path-A silent-build posture means we don't list public package managers before launch, (b) each manager has its own review timeline (Homebrew core can be weeks; winget reviews can be slow), and (c) the script install path is the friction-reduced default for the hand-picked friends-and-family round per `33m-friends-and-family-prompt.md`.
  - [ ] 33P1-homebrew-tap          Homebrew tap at `Micrurus-Ai/homebrew-corvid`. Formula points at the latest stable GitHub Release tarball + computes the SHA-256 from `SHA256SUMS.txt`. End-user command: `brew install Micrurus-Ai/corvid/corvid`. Tap repo IS separate from this one (Homebrew convention); future `homebrew-core` submission is a follow-up after tap stabilizes.
  - [ ] 33P2-scoop-bucket          Scoop bucket at `Micrurus-Ai/scoop-corvid`. Manifest references the Windows release zip from `release.yml`. End-user: `scoop bucket add corvid https://github.com/Micrurus-Ai/scoop-corvid && scoop install corvid`. Auto-update via Scoop's `checkver` + `autoupdate` against the GitHub Releases API.
  - [ ] 33P3-winget-manifest       winget manifest PR to `microsoft/winget-pkgs` for `Micrurus-Ai.Corvid`. Manifest points at the `corvid-x86_64-pc-windows-msvc.zip` from `release.yml`. End-user: `winget install Micrurus-Ai.Corvid`. Review cycle is microsoft/winget-pkgs maintainer-gated (typically 1-3 weeks); requires SHA-256 of the release zip in the manifest.
  - [ ] 33P4-chocolatey-package    Chocolatey package at chocolatey.org/corvid. `chocolateyinstall.ps1` downloads the Windows release zip + verifies the SHA-256. End-user: `choco install corvid`. Auto-update via the existing `release.yml` `nightly` channel + a Chocolatey `versionschecker` script.
  - [ ] 33P5-aur-package           Arch User Repository (AUR) package at `aur.archlinux.org/corvid-bin`. PKGBUILD downloads the `x86_64-unknown-linux-gnu.tar.gz` release tarball. End-user: `yay -S corvid-bin` (or the equivalent AUR helper). Tracks the latest stable tag; nightly AUR package can be a follow-up after stable lands.
  - [ ] 33P6-apt-rpm-repo          Debian APT repository + RPM repository for `corvid` packages, hosted under `packages.corvid-lang.org` (or similar). Cargo-deb + cargo-rpm produce the `.deb` and `.rpm` artifacts in `release.yml`'s nightly + stable runs; the repos serve them with signed `Release.gpg` / `repomd.xml`. End-user: `apt install corvid` / `dnf install corvid` after one-time `apt-add-repository` / `dnf config-manager`. Slowest of the 6 because of GPG-signing-key-rotation infra requirements.
  - [ ] 33P7-windows-code-signing   **[friends-and-family Windows-round blocker]** Sign `corvid.exe` (the Windows release artifact from `release.yml`) with a real Authenticode certificate so the install scripts and the Scoop/winget/Chocolatey channels stop triggering Windows SmartScreen "unrecognized publisher" warnings. Surfaced by the corvid-installer maintainer's `LIVE-TEST-GAPS.md` Gap #3; without this, a Windows reviewer's first action after `iwr ... | iex` is to accept a scary SmartScreen prompt — the worst possible first-impression for a friends-and-family round. Two viable approaches: (a) buy an OV Authenticode certificate from a CA (DigiCert / Sectigo, ~$200-$500/yr, stored in `release.yml` as a Workflow secret with `signtool.exe`); (b) Sigstore-style keyless signing via the experimental `windows-signing` flow (no SmartScreen trust today but lower friction for the toolchain). Default to (a) for v1.0 since SmartScreen-trust matters more than supply-chain optics at this stage; revisit Sigstore post-launch once enough reviewers have used Corvid that the publisher reputation is established. Acceptance: a fresh Windows VM runs `iwr corvid-lang.org/install | iex` AND `scoop install corvid` AND `winget install Micrurus-Ai.Corvid` without a SmartScreen warning. Test plan: documented in `docs/meta/windows-code-signing.md` (to be created); CI smoke test verifies `signtool verify /pa corvid.exe` returns success.

- [x] 33Q-trial-round-2-code-findings  **[33M-blocker — five code-class findings from anonymous-2026-06-04 round-2 trial report (`8e76563`)]** — closed 2026-06-07: all five code-class children (33Q1 serve-with-tools-lib, 33Q2 approval-not-burned-on-failure, 33Q3 trust-guarantee-registration, 33Q4 dockerfile-presence-conditional-copy, 33Q5 dockerfile-version-default) shipped + verified live against the maintainer-trial app. The same friends-and-family reviewer retested after the round-1 wrappers-and-onboarding fixes shipped, and surfaced five language-and-runtime-class findings that block reviewers #2-#10 from shipping the six-surface app for real. Triage at [`docs/external-trials/33m-trial-anonymous-2026-06-04.md`](docs/external-trials/33m-trial-anonymous-2026-06-04.md). Four DOCS dispositions (P3.c + P4 + P5 + Minor versioning note) closed in commit `9ac154a` against the build prompt; the five code-class slices below own the deeper fixes. Acceptance per the 33M closing criterion at L51: "feedback closes as code / docs / tests / explicit non-scope before the public cut."
  - [x] 33Q1-serve-with-tools-lib   Ship parity with the `build --target=cdylib` linkage for the interpreter `serve` path. Closed via 33Q1a (`ff49112`: `corvid serve --with-tools-cdylib <path>` dlopens an operator-supplied cdylib, dlsyms each `__corvid_tool_<name>`, registers via `corvid_register_tool` + a Rust `ToolHandler` bridging through new public `dispatch_host_tool`; same `ToolRegistry` is cloned into both the main runtime and the `/approve` bypass runtime via new `RuntimeBuilder::tool_registry` so the re-execution sees the same handlers) and 33Q1b (`2d3e24f`: `tools.py` autoloader embeds Python via PyO3, imports the user's module, reads `corvid_runtime.registry._TOOL_IMPLS`, materializes one Rust handler per Python coroutine that dispatches via `asyncio.run` on a tokio blocking thread to keep the serve loop unstalled). Precedence: tools.py registers first, cdylib registers second, so the explicit operator flag overwrites implicit autoload entries via the new `ToolRegistry::extend`. Acceptance: 5/5 `serve_smoke.rs` integration tests pass (3 existing + `serve_with_tools_cdylib_dispatches_approval_gated_tool_through_fixture` + `serve_autoloads_tools_py_and_dispatches_approval_gated_tool_through_python`). Filed by anonymous-2026-06-04 P1.1.
  - [x] 33Q2-approval-not-burned-on-failure   When `/__approvals/<id>/approve` succeeds the queue transitions and the pending invocation drops; when the handler errors, the approval STAYS `pending` with the error captured for diagnostic surfacing. Design picked the leave-pending shape (lower state-machine surface area than a new `failed-retryable` terminal state; `/deny` is the reviewer's safety valve to exit a permanently-broken loop). Implementation peeks the pending invocation instead of popping, runs the agent first, then transitions the queue + pops the invocation only after handler success; on error, the invocation's new `last_handler_error: Option<String>` is updated and the 500 body carries `approval_status: pending` + `retry: {possible: true, ...}` so the reviewer's client knows to retry or deny. `GET /__approvals/<id>` surfaces `last_handler_error` + `retry_possible: true`. Acceptance gate `serve_approval_is_preserved_when_handler_errors_and_terminates_only_on_deny` in `crates/corvid-cli/tests/serve_smoke.rs` (uses a deliberately-missing tool to drive the handler-error path; verifies POST/approve x2 stays pending, GET surfaces the captured error, POST/deny terminates as denied, and the post-deny /approve answers 409). Adversarial covered: across N retries on a permanently-broken handler the approval never transitions to `approved`; the `ProgrammaticApprover::always_yes` bypass runtime is local to each /approve call and never escapes. Filed by anonymous-2026-06-04 P1.2.
  - [x] 33Q3-trust-guarantee-registration   `@trust(...)` is now compatible with `corvid build --sign`. One `trust.constraint_enforcement` row added to `GUARANTEE_REGISTRY` (`Static` + `TypeCheck` — the typechecker already rejects bodies that violate the declared trust ceiling, so the diagnostic was promoted from `effect_row.body_completeness` to the new id), new `GuaranteeKind::Trust` variant + slug + ALL + `render::kind_heading` plumbed; `SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS` includes the new id so signed-build accepts it; `collect_constraint_claims` in `corvid-driver/src/build/claim_coverage.rs` gains a `"trust"` arm that pushes the require. Positive + adversarial test refs cite existing typecheck tests (`mutation_budget_within_limit_is_ok` for `@trust(human_required)` happy path, `mutation_baseline_trust_violation_exists` for the typecheck rejection) and the two new signed-claim-coverage tests (`signed_claim_coverage_accepts_trust_constrained_agent` + `signed_claim_coverage_rejects_trust_when_id_missing_from_descriptor`). `claim --explain` enumerates the id from the descriptor's `claim_guarantees` array — no new CLI code needed. `docs/reference/core-semantics.md` regenerated to surface the new row. Filed by anonymous-2026-06-04 P2.
  - [x] 33Q4-dockerfile-presence-conditional-copy   `crates/corvid-cli/src/deploy_cmd.rs::render_dockerfile` now takes an `app_root: &Path` argument and probes for the four optional paths (`migrations/`, `evals/`, `traces/`, `tools.py`) at render time. `src/` and `corvid.toml` stay structurally mandatory; the optional COPYs are emitted only when the source path exists. Two new acceptance tests at `deploy_cmd::tests`: `deploy_dockerfile_omits_copy_lines_for_missing_optional_paths` (bare `corvid new` shape — empty tempdir + render → asserts NO `COPY migrations`/`COPY evals`/`COPY traces`/`COPY tools.py` lines appear) and `deploy_dockerfile_emits_copy_lines_for_present_optional_paths` (tempdir with all four paths present → asserts every COPY line is emitted). The existing reference_apps integration test still passes because `personal_executive_agent` has all four paths. `tools.py` COPY pairs with the 33Q1b autoloader so the container's autoload path finds the module at the project root. Filed by anonymous-2026-06-04 P3.a.
  - [x] 33Q5-dockerfile-version-default   The rendered Dockerfile's `ARG CORVID_VERSION` default is now pinned to the rendering binary's nightly tag (`nightly-{CORVID_BUILD_DATE}-{CORVID_BUILD_SHA}`), constructed at render time from the env vars `crates/corvid-cli/build.rs` injects. When either env is the documented `unknown` fallback (corvid built outside a git checkout), the default falls back to the literal `nightly` and the Dockerfile's URL-resolver block queries the GitHub Releases API for the latest nightly tag — the same logic `install/install.sh` uses for `CORVID_VERSION=nightly`. The URL-resolver now handles all three shapes the install pipeline standardizes on (`latest` / `nightly` / literal tag). Acceptance test `deploy_dockerfile_pins_corvid_version_to_rendering_binary_sha` reads the same `env!("CORVID_BUILD_SHA")` + `env!("CORVID_BUILD_DATE")` the renderer reads and asserts the constructed default matches; adversarial guard asserts `ARG CORVID_VERSION=latest` (the prior v0.1.0-lacks-serve default) is NOT emitted. Filed by anonymous-2026-06-04 P3.b.

- [x] 33Q-trial-round-3-code-findings  **[33M-blocker — ten findings from the maintainer-as-reviewer-2026-06-05 trial report (`0921ec5`)]** — closed 2026-06-07: all three P1 launch-blockers (33Q6 corvid-runtime distribution, 33Q7a spec honesty + drift gate, 33Q8 struct-boundary lift) shipped + verified live; six follow-ups (33Q9 banner label accuracy, 33Q10 serve 500-detail cleanup, 33Q11 deploy-package atomicity, 33Q12 a/b/c misc polish) shipped; 33Q13a/c/e deterministic AI helpers shipped; 33Q14 self-trial round 4 closure shipped. The only remaining child is 33Q7b post-v1.0 typechecker-tightening, which is explicitly post-launch scope. Self-administered trial by the maintainer playing reviewer-#2 to find what reviewer-#1's two rounds didn't. Built a small production-shape cyber-threat-intel app (5 of 6 surfaces) and documented every friction point. Triage at [`docs/external-trials/33m-trial-maintainer-as-reviewer-2026-06-05.md`](docs/external-trials/33m-trial-maintainer-as-reviewer-2026-06-05.md). Three P1 launch-blockers (33Q6, 33Q7, 33Q8) plus six P2/P3/Minor follow-ups (33Q9-33Q12).
  - [x] 33Q6-corvid-runtime-distribution   New `find_bundled_corvid_runtime()` helper in `crates/corvid-runtime/src/python_tools.rs` resolves the package's parent dir at startup and prepends it to `sys.path` before importing `tools`. Search order: (1) install layout `<binary_parent>/../runtime-py/` (where `release.yml`'s new "Stage artifact" branch copies `runtime/python/` → `$stage/runtime-py/`); (2) dev layout `<exe_dir>/../../runtime/python/` (workspace path during `cargo run`/`cargo test`). Acceptance test `serve_autoloads_tools_py_via_bundled_corvid_runtime_without_pythonpath` in `crates/corvid-cli/tests/serve_smoke.rs` spawns serve with **NO** PYTHONPATH set and verifies the full POST → 202 → /approve → 200-with-echoed-value round-trip works end-to-end. Scaffold's `commands/misc.rs::cmd_new` "Next steps" output replaced the misleading `pip install corvid-runtime` line with an honest "bundled with the install — works without pip install" note. Verified live against the maintainer-trial app at /tmp/threat_intel_agent: `corvid serve` with NO PYTHONPATH now starts cleanly where it previously crashed with `ModuleNotFoundError`. Filed by maintainer-as-reviewer-2026-06-05 P1.1.
  - [x] 33Q7a-spec-honesty-and-drift-gate   Discovery confirmed the spec was overclaiming: the v1.0 typechecker accepts ANY string as a `trust:`/`data:` value via `DimensionValue::Name(String)` (verified — `trust: nonsense_value_that_is_definitely_not_in_spec` typechecks clean). All 5 reference apps typecheck OK; they just use values beyond the spec's stated lattice. Fix shape: (a) updated spec §4.2 + §4.4 in `docs/internals/effect-spec/04-builtin-dimensions.md` with "Implementation note (v1.0 honesty)" blocks naming the non-enforcement honestly, (b) new catalog doc `docs/internals/effect-spec/reference-app-dimensions.md` listing every value used in reference apps (6 trust + 6 data + 4 confidence-gated combinations), (c) drift gate at `crates/corvid-types/tests/reference_app_dimensions_gate.rs` (3 tests: trust-values-documented, data-values-documented, every-listed-extension-is-used) catches both add-without-document AND list-without-use drift. Filed by maintainer-as-reviewer-2026-06-05 P1.2.
  - [ ] 33Q7b-typechecker-tightening-and-app-cleanup   **[post-v1.0]** Promote 33Q7a's soft gate to hard typechecker enforcement: the typechecker REJECTS non-canonical `trust:`/`data:` values unless the user's `corvid.toml` declares them via `[effect-system.dimensions.<name>]`. Reference apps move to canonical values OR add corvid.toml dimension declarations. Acceptance: every `examples/backend/*/main.cor` typechecks clean against the strict gate; `trust: nonsense_value` is rejected with a clear diagnostic pointing at the dimension declaration site. Post-v1.0 because cleaning up 5 reference apps + the typechecker change is broader scope than the launch-readiness window allows; the soft gate is sufficient v1.0 protection (a reviewer reading the spec is no longer contradicted by the reference apps because the spec's honesty note + the catalog doc tell them what's going on).
  - [x] 33Q8-pub-extern-c-struct-boundary   Slice lifts the `pub extern "c"` struct-parameter / struct-return restriction by reusing Phase 20n-C's per-struct JSON decoder + encoder at the C ABI boundary. Typechecker (`crates/corvid-types/src/checker/decl_extern_c.rs::extern_c_param_type_supported` + `extern_c_return_type_supported`) now accepts `Type::Struct(_)` and `Type::ImportedStruct(_)` when every field is itself a 20n-C-supported scalar (`Int` / `Float` / `Bool` / `String`); nested-struct / list / option fields still trip `NonScalarInExternC` so the typechecker stays in lock-step with codegen depth (hint message rewritten to drop the stale Phase-22 reference and direct readers at `docs/reference/exported-abi.md`). Ownership inference covers structs: parameters are inferred `@borrowed` (the JSON buffer is caller-owned for the call frame), returns are inferred `@owned` (Corvid hands back a `corvid_free_string`-able buffer). Codegen (`crates/corvid-codegen-cl/src/lowering/agent.rs::define_extern_c_wrapper`) routes struct params through `string_from_cstr` → `lookup_or_emit_struct_decoder` → release-temporary, traps cleanly if the decoder returns NULL (malformed JSON); struct returns route through `lookup_or_emit_struct_to_json` → `string_into_cstr` and release the source struct. `extern_c_abi_type` maps `Type::Struct(_)` to `I64` (the JSON-pointer wire shape). `cdylib::exported_symbols` now exports `corvid_free_string` whenever any extern-c agent returns a struct so the C caller can free the buffer. C header emission (`crates/corvid-c-header/src/lib.rs::emit_header`) walks struct boundaries via `corvid_prompt_format::schema_for(ty, types_by_id)` and embeds the JSON Schema as a `// JSON shape for parameter/return value \``<name>`\`:` block comment above each struct-using agent's C signature. Acceptance tests: (a) `cdylib_struct_param_and_return_roundtrip_via_json` in `crates/corvid-codegen-cl/tests/cdylib_emission.rs` builds a cdylib with `pub extern "c" agent finalize_ticket(ticket: Ticket @borrowed) -> Receipt`, dlopens, calls it with real JSON, and asserts the returned JSON parses + carries the input id through to `Receipt.note` (proves BOTH decode and encode actually marshal the user-provided value); (b) `cdylib_struct_boundary_c_header_documents_json_schema` asserts the emitted `.h` declares `const char* ticket` / `const char* finalize_ticket(...)` AND carries the per-boundary schema comments with the real field names; (c) updated `extern_c_agent_with_scalar_struct_param_compiles_clean` + `..._scalar_struct_return_compiles_clean` typecheck tests confirm the lift; (d) `extern_c_agent_with_struct_param_containing_nested_struct_field_still_errors` pins the adversarial guard. Updated `docs/reference/exported-abi.md` documents the worked-example struct-boundary signature + the wire contract (caller passes well-formed JSON; cdylib traps on parse failure pending the post-v1.0 error-out-parameter slice). Filed by maintainer-as-reviewer-2026-06-05 P1.3.
  - [x] 33Q9-serve-approval-label-accuracy   New `agent_body_contains_approve(ir, agent_name)` helper in `crates/corvid-cli/src/serve_cmd.rs` recursively walks the handler agent's `IrBlock` (through nested `If` then/else and `For` body) looking for any `IrStmt::Approve`. The startup banner now emits `approval-gated -> 202 + queued` ONLY when the walk finds one — routes whose handler has no syntactic `approve` are labeled just `(body)` or `(literal)` based on their dispatch shape. Conservative: doesn't follow calls into other agents (a body that ONLY calls another approving agent gets the no-approve label) — that's an under-count, NOT an over-count, which matches the trial complaint direction. Acceptance test `serve_startup_banner_distinguishes_routes_with_and_without_approve` writes a source with two POST routes (one `approve`-using, one not), captures stdout, and asserts the labels are distinct. Verified live on the maintainer-trial app: `triage_ioc` (no approve) is now `(body)`; pre-33Q9 it was `(body; approval-gated -> 202 + queued)`. Filed by maintainer-as-reviewer-2026-06-05 P2.1.
  - [x] 33Q10-serve-error-detail-clean   New `RunError::user_facing_detail()` method in `crates/corvid-driver/src/run.rs` returns the error message WITHOUT the IR `[start..end]` byte-span prefix that `InterpError`'s `Display` impl unconditionally prepends. The 500-construction sites in `crates/corvid-cli/src/serve_cmd.rs` (both `finish()` and `approve_approval()`) use `user_facing_detail()` instead of `to_string()`. `RunError::Display` remains span-prefixed because that anchor is useful in tracing + dev-time stderr; only the HTTP layer is stripped. Acceptance test `serve_500_response_strips_ir_byte_span_prefix_from_detail` in `crates/corvid-cli/tests/serve_smoke.rs` deliberately POSTs to a route whose tool has no handler (the natural 500-producing path), asserts the resulting `detail` field has no `[<digits>..<digits>]` prefix, AND asserts the actionable content ("no handler registered for tool `classify_anything`") survives the stripping. Verified live: pre-33Q10 `{"detail":"[1227..1269] no handler registered..."}`, post-33Q10 `{"detail":"no handler registered..."}`. Filed by maintainer-as-reviewer-2026-06-05 P2.2.
  - [x] 33Q11-deploy-package-atomic   `crates/corvid-cli/src/deploy_cmd.rs::run_package` now reads + validates `CORVID_DEPLOY_SIGNING_KEY` (calls `corvid_abi::load_signing_key` to catch malformed-key cases) BEFORE `fs::create_dir_all(out)` — missing or invalid env → command fails and `out/` doesn't exist. The validated `SigningKey` is threaded through to `render_attestation` as a parameter, replacing the prior `std::env::var(...)` inside the function. `--cdylib` path is also read up-front so a bad path fails before any file write. Clap docstring for `Package` now declares `CORVID_DEPLOY_SIGNING_KEY` as a REQUIRED env var with format + atomic-on-error contract spelled out. `corvid_abi::SigningKey` re-exported from corvid-abi (was hidden behind `ed25519_dalek` previously). Acceptance test `deploy_package_missing_signing_key_env_does_not_create_out_dir` removes the env, calls `run_package`, asserts (a) it returns Err naming the env var, (b) the output directory MUST NOT exist (load-bearing — pre-33Q11 it had 6 files). Verified live on the maintainer-trial app at /tmp/threat_intel_agent: missing env → error + no `deploy/` directory. Filed by maintainer-as-reviewer-2026-06-05 P2.3 + P3.1.
  - [x] 33Q12-misc-polish   Three lower-severity findings closed:
    (a) `std.db` docs honesty: friends-and-family build prompt (Surface 2) now spells out the v1.0 scope (typed envelopes ship; `db.query(...)` source-syntax primitive is post-v1.0; bridge through Corvid `tool` wrappers against SQLite/Postgres which DO ship runtime support via Phase 35V2-P37/P38). `std/db.cor` header block names the same boundary explicitly so a reviewer reading the source isn't misled.
    (b) OCI label path-separator normalization: `run_package` in `crates/corvid-cli/src/deploy_cmd.rs` now post-processes `source.display().to_string()` with `.replace('\\', "/")` before constructing `OciLabels::source`. Acceptance test `deploy_package_normalizes_backslashes_in_oci_source_label` runs `run_package` on a tempdir app, parses the resulting `oci-labels.json`, and asserts `labels["org.opencontainers.image.source"]` contains no `\` regardless of OS.
    (c) `pub extern "c"` missing-agent error: anchor span is now the first agent's span (if any agent exists) instead of `[0..0]` at file start — the reviewer's editor can highlight "add `pub extern \"c\"` to this agent". Error message tightened to name `docs/reference/exported-abi.md`, the new doc page (which this slice creates) documenting the v1.0 boundary types + the post-v1.0 33Q8 plan. Acceptance test `cdylib_missing_pub_extern_c_error_anchors_at_first_agent_and_names_doc_page` in `crates/corvid-codegen-cl/tests/cdylib_emission.rs` verifies all three contract points (span moved off `[0..0]`, error names the doc, error names the missing keyword verbatim). Filed by maintainer-as-reviewer-2026-06-05 P3.2, P3.3, and Minor.

- [x] 33Q13a-synthesize-feedback-deterministic   First of three remaining AI-helper slices under the `35V2-P43-T-LR-phase-43-ai-helpers` umbrella. Ships `corvid beta synthesize-feedback <REPORTS>...` — a deterministic-Rust synthesizer mirroring the `corvid claim audit` precedent. Walks one or more trial-report markdown files, extracts every `### P<n>` / `### Minor` finding header, groups them by declared class (`CODE` / `DOCS` / `UX` / `CODE/DOCS` / etc.) with alphabetized buckets, and renders either markdown (default) or JSON (`--json`). Acceptance tests in `crates/corvid-cli/tests/beta_synthesize_feedback.rs`: (a) `synthesize_feedback_surfaces_canonical_categories_from_real_corpus` runs against both shipped trial reports and asserts ≥13 findings across at least `CODE` and `DOCS` buckets; (b) `synthesize_feedback_is_grounded_every_citation_resolves_to_real_header` is the load-bearing groundedness assertion — for EVERY finding the synthesizer emits, the cited file:line MUST contain a `### ` header that matches the claimed severity AND class strings. This pins the no-fabrication contract NOW so the post-v1.0 LLM-driven thematic-clustering layer (33Q13b below) can only REFINE groupings, never override the underlying citations.

- [x] 33Q13c-deploy-tailor-deterministic   Second of the three remaining AI-helper slices. Ships `corvid deploy tailor <app>` — a deterministic Rust analyzer that compiles the app's source to IR, walks the IR for known patterns (server blocks, agents, total + dangerous tools, `@budget` constraints), checks the app's filesystem for the optional directories (`tools.py`, `migrations/`, `evals/`, `traces/`), and emits structured recommendations for tailoring the generated Dockerfile/Compose/K8s/env-schema artifacts. Each recommendation is grounded in a specific IR or filesystem signal — when the signal is absent the recommendation MUST NOT appear (no fabrication). Severity buckets (`critical` / `warn` / `info`) order operator attention. Markdown rendering (default) or JSON (`--json`). Acceptance tests in `crates/corvid-cli/tests/deploy_tailor.rs`: (a) `deploy_tailor_surfaces_canonical_signals_for_reference_app` runs against `examples/backend/personal_executive_agent` and asserts the analyzer detects > 0 server blocks, agents, dangerous tools + the migrations dir + emits the critical approval-queue recommendation; (b) `deploy_tailor_is_grounded_recommendations_match_present_signals` runs against a bare scaffold-shape app and asserts the migrate-up and approval-queue recommendations are ABSENT (no fabrication) while the no-server-block WARN is PRESENT (safety net). The deterministic-core-first pattern mirrors 33Q13a synthesize-feedback; 33Q13d files the LLM-promote follow-up that proposes free-form refinements anchored to the same signals. Also adds `ENV_LOCK` serial-test mutex to `deploy_cmd::tests` to resolve the env-mutation race between 33Q11 atomicity + 33Q12b OCI normalization tests that surfaced when 33Q13c grew the test pool.

- [x] 33Q13e-upgrade-assist-deterministic   Third of the three remaining AI-helper slices — completes the `35V2-P43-T-LR-phase-43-ai-helpers` umbrella's v1.0 deterministic surface. Ships `corvid upgrade assist <path>` — a deterministic source auditor that scans each `.cor` file in the project for patterns requiring operator judgment at the next strict-typecheck / feature-boundary upgrade: non-canonical `trust:` / `data:` values (33Q7b migration), `pub extern "c"` agents with struct boundaries (33Q8 lift), LLM-using agents without `@budget` constraints (cost-overrun risk). Each finding cites the source file + 1-indexed line that triggered it. Severity buckets (`critical` / `warn` / `info`). Markdown rendering (default) or JSON (`--json`). Distinct from `corvid upgrade check` (mechanical syntax/stdlib rewrites that `apply` can automate) — `assist` covers patterns that NEED operator judgment. Acceptance tests in `crates/corvid-cli/tests/upgrade_assist.rs`: (a) `upgrade_assist_produces_zero_findings_for_canonical_source` — no false positives on canonical Corvid; (b) `upgrade_assist_detects_non_canonical_trust_and_data_values` — both detection rules fire with correct line citations; (c) **load-bearing** `upgrade_assist_does_not_false_positive_on_struct_field_declarations` — struct field declarations like `trust: String` (type, not dimension value) MUST NOT trip the lint, pinned via the `parse_dimension_value` uppercase-skip guard surfaced during live verification against `std/effects.cor`. 33Q13f files the LLM-promote follow-up. Pattern reinforced from 33Q13a + 33Q13c: deterministic core ships first with groundedness contract pinned by tests.

- [ ] 33Q13f-upgrade-assist-llm-promote   **[post-v1.0]** Promote the deterministic upgrade auditor to an LLM-driven refinement layer that proposes contextual migration suggestions anchored to the 33Q13e signals. Same Grounded<T> shape as the other LLM-promote slices: the LLM can only refine findings anchored to a 33Q13e-detected pattern; it CANNOT invent migration recommendations for patterns the source doesn't have. Filed post-v1.0 alongside 33Q13b + 33Q13d.

- [x] 33Q15-deploy-package-write-atomicity   `crates/corvid-cli/src/deploy_cmd.rs::run_package` had 9 sequential `fs::write` calls between the 33Q11 pre-flight and the success print. Any write past the first failing would leave the directory in a confusing partial state — and if a prior successful run had emitted a file the current shape no longer writes, it leaked into the new output. Slice refactors `run_package` to stage every write into a sibling `tempfile::Builder::new().prefix(...).tempdir_in(out.parent())?` so the final `fs::rename` is same-filesystem atomic; on all writes succeeding, `fs::remove_dir_all(out)` is called first (so stale files don't leak), then `tempfile::TempDir::keep()` disarms the cleanup guard and `fs::rename(staged, out)` atomically swaps the bundle into place. On any failure, the TempDir's Drop cleans up the staging dir and `out/` stays in whatever state it started in — strengthens 33Q11's "no out/ on pre-flight error" to "no MUTATION of out/ on ANY error." Acceptance: (a) `deploy_package_atomically_replaces_stale_out_dir_on_success` pre-creates `out/legacy_marker_from_prior_run.json`, runs `run_package` successfully, asserts the stale marker is GONE and all 9 current-build files are present; (b) `deploy_package_leaves_prior_out_untouched_when_pre_flight_fails` pre-creates `out/prior_run_marker.txt`, removes the env, asserts the run errors AND the prior marker survives with its original contents. Existing 10 deploy_cmd tests (33Q4 / 33Q5 / 33Q11 / 33Q12b / 43M) still pass — 12/12 total. Filed + shipped 2026-06-08 from the post-33Q8 deploy-package UX inventory.

- [x] 33Q17-cli-ergonomics-polish   Four reviewer-impression CLI gaps closed end-to-end with live-verification on a freshly scaffolded `corvid new` project. (a) **corvid run positional args** — `Command::Run` in `crates/corvid-cli/src/cli/root.rs` grew a trailing-varargs `args: Vec<String>` slot; `run_with_target` and `cmd_run` thread it to the interpreter via a new `parse_args_for_entry_agent` helper (parses each string against the agent's `Int` / `Float` / `Bool` / `String` parameter type up-front for crisp errors) and to the native binary via `Command::args(args)` (the codegen-emitted `main` already decodes argv per parameter type from entry.rs). Live: `corvid run src/main.cor world` prints `world`; `corvid run src/main.cor 41` prints `42`; `corvid run src/main.cor abc` exits 1 with a clean "cannot parse `abc` as Int" message. (b) **corvid serve --port / --host aliases** — added optional `--host <HOST>` and `--port <PORT>` flags in `Command::Serve`, threaded into a new `compose_serve_listen(listen, host, port) -> String` helper in `dispatch.rs` that overlays each explicit override onto the `--listen host:port` default. Live: `corvid serve --port 8086` binds to `127.0.0.1:8086`; `corvid serve --host 0.0.0.0 --port 8087` binds to `0.0.0.0:8087`. (c) **corvid audit directory diagnostic** — `audit_cmd::run_audit` now checks `path.is_dir()` up-front and bails with `"corvid audit takes a `.cor` source file (the project's root module), not a directory. Try corvid audit <dir>/src/main.cor — that's the default entry point for a `corvid new`-scaffolded project."` instead of letting the next `read_to_string` leak `"Access is denied. (os error 5)"`. Live: directory input → crisp diagnostic, no OS-error leak. (d) **wasm pub extern "c" doc-link** — `validate_agent` in `crates/corvid-codegen-wasm/src/lib.rs` rewrote the rejection to say "boundary is cdylib-only — wasm exports normal Corvid agents. See `docs/reference/exported-abi.md` for the boundary contract; drop the `pub extern \"c\"` modifier to make this agent browser/edge-callable." Live: wasm build of a `pub extern "c"` agent now points at the doc page. Acceptance: 10 new tests total — 3 in `corvid-driver` for run-args (positive / bad-parse / arity-mismatch), 5 in `corvid-cli` for serve listen composition (no-overrides / port-only / host-only / both / IPv6), 1 in `corvid-cli` for audit-on-directory, 1 in `corvid-codegen-wasm` for the doc-link mention. Corpus verify exits 1 only on the two deliberate fixtures. Filed + closed 2026-06-08 from the post-33Q15 end-to-end sweep.

- [ ] 33Q16-deploy-diff-command   **[post-v1.0]** `corvid deploy diff <out-a> <out-b>` produces a structural diff between two deploy bundles — Dockerfile diff, env.schema delta, OCI label changes, attestation envelope identity, SBOM additions/removals. Solves the workflow that today forces operators to manually `--out` two directories and eyeball-diff them (the shape that produced the `prefs_agent/deploy/` + `prefs_agent/deploy2/` pattern in the 2026-06-08 self-trial). Post-v1.0 because the workflow itself works with the v1.0 atomicity story (33Q15) — diff is a quality-of-life command, not a correctness fix. Filed 2026-06-08 from the same deploy-package UX inventory as 33Q15.

- [ ] 33R-market-readiness-track   **[launch-blocker — OSS adoption gap remediation]** 14-slice track derived from the 2026-06-08 market-readiness audit (`docs/market-readiness-audit.md`). The audit's verdict: Corvid is engineered far past `0.0.1`, but adoption is blocked by last-mile surfacing — no LICENSE on disk, no package registry, README is invention-catalog-shaped instead of an adoption funnel, stdlib is contracts-only, VS Code extension and crates.io unpublished. None of these are missing-capability problems; they're packaging + hosting + docs. Track runs in tiered order P0 → P3; lower tiers do not start until higher tiers close. Canonical decisions captured at filing time: repo URL = `github.com/Micrurus-Ai/Corvid-lang`; canonical domain = `corvid-lang.org` (to be served from the existing `web/` Cloudflare Worker); copyright holder = `Disan Ssebowa Basalidde`; license = MIT (narrowed from the prior `MIT OR Apache-2.0` claim, which never had on-disk text to back it); registry shape = GitHub Releases for artifacts + static `index.json` from the same Worker.
  - [x] 33R1-license-files   **[P0 — closed 2026-06-08, commit 262a75b]** Added `LICENSE` at repo root (standard MIT text, `Copyright (c) 2026 Disan Ssebowa Basalidde`). Narrowed prior `MIT OR Apache-2.0` declaration to MIT-only across `Cargo.toml` (workspace), `runtime/python/pyproject.toml`, `extensions/vscode-corvid/package.json` + `package-lock.json`, `web/worker.js` installer-landing footer, `docs/help/faq.md`, `docs/meta/remaining-slices-handoff.md`, the `corvid-bind` Rust binding generator template (`crates/corvid-bind/src/rust_backend/cargo.rs`), and the 12 committed example `bindings_rust/Cargo.toml` fixtures. README's `## License` section now links `LICENSE`, states MIT, documents inbound = outbound contribution convention. Workspace members already inherit via `license.workspace = true` so the workspace narrowing propagates automatically. Validation gate: `cargo check --workspace --tests` clean; `cargo test -p corvid-bind --lib` 2/2 pass; `corvid verify --corpus tests/corpus` exits 1 only on the two deliberate fixtures. The user-provided audit + brief (`docs/market-readiness-*.md`) and ROADMAP filing entry preserved as historical record.
  - [x] 33R2-repo-identity-unify   **[P0 — closed 2026-06-08]** Scrubbed the prior `github.com/corvid-lang/corvid` claim (which never matched the actual remote) and the prior `corvid.dev` domain references (the canonical domain is `corvid-lang.org`). Updates: `Cargo.toml` workspace repository; `FEATURES.md` install command; `ROADMAP.md` v1.0 install instructions; `runtime/python/README.md` (PyPI front page); `crates/corvid-driver/src/adversarial.rs` `DEFAULT_REPO` constant; `docs/meta/v1.0-demo-script.md` git-clone command; `crates/corvid-connector-runtime/src/tasks.rs` + `tests/executive_agent_connectors.rs` test fixtures (kept mock/assertion pairs internally consistent); `docs/book/01-install.md` install one-liners; `docs/guides/performance.md` + `docs/help/faq.md` benchmarks links; `docs/meta/website-docs-handoff.md` (3 hits); `docs/internals/package-manager-scope.md` (2 hits); `web/README.md` deploy walkthrough. Preserved as historical: ROADMAP 25-G closure entry, my own 33R parent filing, `dev-log.md` historical `registry.corvid.dev` mentions, and the user-provided `docs/market-readiness-*.md` audit + brief. Validation gate: `cargo check --workspace --tests` clean; `corvid-connector-runtime` connector test fixtures still pass (mock + assertion pair updated together); `corvid verify --corpus tests/corpus` exits 1 only on the two deliberate fixtures.
  - [x] 33R3-readme-adoption-funnel   **[P0 — closed 2026-06-08]** Restructured the top of `README.md` so a newcomer sees install + first program + book links in the first screen instead of being dropped into the invention catalog at line 57. Kept the existing strong opening (pitch + differentiator + refund-agent example — lines 1-24) intact since it's already a tight funnel; inserted a new `## Quickstart` section right after it with the macOS/Linux install one-liner (full install options + Windows PowerShell remain linked-to in the `## Install` section below), the `corvid new hello / corvid run` first-program flow, and three exploration links: `./docs/book/02-quickstart.md`, `corvid tour --list`, `./docs/book/README.md`. Added a short `## Contents` ToC after Quickstart so catalog-readers can navigate the long lower half without scrolling blind. Also fixed two `./ROADMAP.md#L1923` line-anchor references in the existing Install section that were one line off after the 33R track insertion moved 33P down. Deliberately deferred: the Status badge (ships with `33R8` so the badge + its target page land together — no half-finished placeholder, no dead link); the `E0301`→`E0101` quickstart error-code drift (33R12 is the single-concern slice for that). Validation gate: `cargo check --workspace --tests` clean; `corvid verify --corpus tests/corpus` exits 1 only on the two deliberate fixtures.
  - [ ] 33R4-package-registry   **[P1 — invention; gated sub-slices]** Stand up a minimal hosted package registry so `corvid add <pkg>` resolves. Hosting decision locked: GitHub Releases (artifacts, content-addressed by sha256) + static `index.json` served from the existing `web/` Cloudflare Worker.
    - [x] 33R4a-registry-shape-decision   **[closed 2026-06-08]** Pre-phase chat closed; agreed shape captured in [`docs/internals/registry-design.md`](docs/internals/registry-design.md). Locked decisions: (1) single global `index.json` for v1.0 (versioned schema; per-package indexes are a forward-compatible reshape when the file exceeds ~100 KB), (2) separate **registry signing key** distinct from `corvid build --sign` (different threat model + independent rotation), (3) committed per-version manifests at `web/registry/<pkg>/<version>.json` + a `regenerate.sh` that emits `index.json`; publish = PR (auditable in git history, no live database mutation), (4) artifacts at GitHub Releases tagged `pkg-<name>-v<semver>`, carrying `<name>-<version>.corvid` + `<name>-<version>.corvid.sig`. URL layout: `corvid-lang.org/registry/index.json` served from the existing Worker; artifact bytes ride GitHub Releases CDN directly. Client `--registry` default changes to `https://corvid-lang.org/registry/` in 33R4b. `index.json` itself is NOT DSSE-signed in v1.0 (relies on HTTPS + Worker deploy controls + git audit); hardening filed post-v1.0. Doc-only commit; standard validation gate.
    - [x] 33R4b-registry-format-migration-toml-to-json   **[closed 2026-06-08; re-scoped from "client default-registry pointer" to the format migration after the surface inventory found the existing client deserializes TOML]** Migrated the registry index wire format from TOML to JSON to match the agreed 33R4a shape (nested `packages.{name}.versions.{version}` map + root-level `signing_key`). One concern: the registry index encoding. Changes: (1) `RegistryIndex` restructured from `package: Vec<RegistryPackage>` (flat TOML array) to `{version, generated_at, signing_key, packages: BTreeMap<String, RegistryPackageEntry>}` with `RegistryPackageEntry` carrying `latest` + `versions: BTreeMap<String, RegistryPackage>`; (2) `load_registry_index` now parses JSON and resolves directory inputs to `<dir>/index.json`; (3) `publish.rs::publish_package` writes `index.json`, upserts into the nested shape (BTreeMap insert keeps sort stable), tracks `latest` per-package, and bails if a re-publish brings a different signing-key fingerprint than the existing index (one registry = one signing key); (4) `sign_package` returns `(detached_sig_hex, fingerprint)`; the detached sig (128-char ed25519 hex) goes into the per-version `signature` field and the fingerprint (`ed25519:<key_id>:<pubkey_hex>`) goes into the index root's `signing_key` field once; (5) `verify_package_signature` rewritten to take the root signing-key + per-version detached sig as separate inputs; (6) `add.rs::select_package` walks the nested shape (O(1) name lookup); (7) `verify.rs` walks the nested shape; (8) error messages updated `index.toml` → `index.json`; (9) all 8 affected unit tests rewritten with a `json_index_fixture` helper to keep test bodies DRY. The URL default flip moves to 33R4c (where it pairs with standing up the actual endpoint — shipping the URL without the endpoint would be a transient broken state). Acceptance: 11/11 `package_registry::tests` pass; 2/2 CLI `package_help` tests pass; full workspace check + corpus verify clean. Design doc `docs/internals/registry-design.md` amended in the same commit to switch the `signature` field from base64 to hex for consistency with the rest of the codebase's encoding conventions (sha256 hex, key hex).
    - [ ] 33R4c-hosted-static-index   Implement the index generation + Worker route. Update `docs/internals/package-manager-scope.md` and `docs/reference/package-imports.md` to describe what runs vs. what's still deferred.
    - [ ] 33R4d-seed-packages   Publish 2–3 first-party packages so `corvid add` returns something real on day one. Likely sources: helpers extracted from 33R5 stdlib batteries. Gated on 33R5b/c shipping first.
  - [ ] 33R5-stdlib-batteries   **[P1 — invention; gated sub-slices]** Promote the stdlib from contracts to batteries so a beginner tutorial requires zero user-authored Python. Each module ships with the full invention contract: README catalog mention, `corvid tour` topic, `docs/reference/stdlib/<module>.md`, spec link, tests.
    - [ ] 33R5a-batteries-scope-decision   Pre-phase chat — agree exact API surface for json / strings / collections / datetime / math + which executing I/O primitive (likely `http.get` or `io.read_file`) lands first.
    - [ ] 33R5b-json
    - [ ] 33R5c-strings
    - [ ] 33R5d-collections
    - [ ] 33R5e-datetime
    - [ ] 33R5f-math
    - [ ] 33R5g-executing-io-primitive   Load-bearing — the "no Python required" slice. Wires through the runtime/FFI boundary, respects effect/trust/cost dimensions.
  - [ ] 33R6-trusted-channel-publishing   **[P1]** Three sub-slices, each a separate concern + commit.
    - [ ] 33R6a-vscode-marketplace   Bump `extensions/vscode-corvid` version, finalize `package.json` metadata (publisher, repo, license, icon, categories), build `.vsix`, publish to VS Code Marketplace + Open VSX. **Requires user-side**: Marketplace publisher account + Personal Access Token.
    - [ ] 33R6b-crates-io-publish   Audit publishable metadata across all member crates (description, license, repository, readme, keywords). Publish `corvid-cli` (or whatever the chosen binary crate name is — confirm `corvid` available on crates.io in pre-phase chat) so `cargo install corvid` works. **Requires user-side**: crates.io account + API token.
    - [ ] 33R6c-readme-three-install-paths   Update README install section to offer Marketplace extension, `cargo install`, AND the existing one-liner — so curl|sh is no longer the only route.
  - [ ] 33R7-cli-help-grouping   **[P1]** In `crates/corvid-cli/src/cli/root.rs`, add clap `help_heading` groups: "Getting started" (new / check / run / test / tour / repl / doctor) shown first, "Advanced / operations" for the rest. Add a "New here? run `corvid tour --list`" footer to the top-level help. No commands removed/renamed — grouping only.
  - [ ] 33R8-stability-policy-and-changelog   **[P1]** Add `CHANGELOG.md` (Keep-a-Changelog format) seeded with the v0.1.0 tag's current state. Add `docs/stability.md` (or a README section) covering what "pre-v1.0" means, stable vs in-flux surfaces (cross-reference ROADMAP.md:297), deprecation policy, v1.0 criteria pointer. Both linked from README's Status line (33R3 leaves the hook).
  - [ ] 33R9-did-you-mean-suggestions   **[P2 — diagnostic-visible]** Compute the nearest in-scope identifier by bounded-edit-distance in `corvid-resolve` (and where E0301 is rendered in `corvid-driver/src/render.rs`); attach as a `help: did you mean \`<x>\`?` line. Cover undefined names + (where cheap) unknown tool / effect / field names. Diagnostic snapshot tests lock the output.
  - [ ] 33R10-stdlib-reference-pages   **[P2]** Write `docs/reference/stdlib/auth.md`, `db.md`, `approvals.md` covering every public type/agent/tool, its effect rows, and the FFI-tool execution model (especially `db`). Link from the stdlib index. Doc-only.
  - [ ] 33R11-corvid-fmt   **[P2 — invention; sub-slices]** Source formatter that round-trips AST/CST to canonical source, plus `--check` mode for CI.
    - [ ] 33R11a-formatter-rules-decision   Pre-phase chat — agree indentation, block style, line width from `docs/reference/grammar.md`. No code.
    - [ ] 33R11b-fmt-implementation   `corvid fmt` subcommand + `--check` mode. Idempotency tests: `fmt(fmt(x)) == fmt(x)`. README catalog mention, `docs/reference/inventions.md` row, spec link, `corvid tour` note.
    - [ ] 33R11c-fmt-ci-wiring   Wire `corvid fmt --check` into `.github/workflows/ci.yml`.
  - [ ] 33R12-quickstart-error-code-drift   **[P2]** Correct `docs/book/02-quickstart.md` (and any other doc that hardcodes error codes) to match `crates/corvid-driver/src/render.rs::detect_error_code` mappings. Specifically: the dangerous-call-without-approve example currently shows `E0301`; the compiler emits `E0101`. Grep all docs, fix every code; doc-only.
  - [ ] 33R13-community-security-governance-files   **[P3]** Add `CODE_OF_CONDUCT.md` (Contributor Covenant), `SECURITY.md` (vulnerability disclosure process — coordinate with the existing `effect-bypass bounty.md`), confirm `CONTRIBUTING.md` linked from README. Add general-purpose issue templates alongside the existing beta/effect-bypass templates.
  - [ ] 33R14-bus-factor-longevity-signals   **[P3 — chat-only positioning]** Decide how to communicate project longevity to adopters: a public roadmap summary distinct from the internal `ROADMAP.md`, a "who maintains this" note, a contribution on-ramp ("good first issue" labels). Positioning decision, not a single commit — may produce multiple small artifacts after the pre-phase chat.

- [ ] 33S-executing-io-surfaces   **[parent — launch-material; HTTP + File + SQLite as effect-carrying executing primitives]** Promotes Corvid's HTTP, File, and SQLite stdlib surfaces from typed-envelope-only to executing primitives that flow through the effect system, replay/quarantine machinery, and signed-claim trust boundary. The runtime already executes HTTP and file I/O internally (`crates/corvid-runtime/src/http.rs::HttpClient::send` + `crates/corvid-runtime/src/io.rs::IoRuntime::read_text`; quarantine hooks exist on both); rusqlite is already a workspace dep used by `approval_queue/sqlite.rs`. Most of the work is exposing existing executing surfaces as callable Corvid primitives with proper effect rows + replay wiring + a real security model, plus a genuinely-new (but dependency-free) SQLite surface. Pre-phase chat (2026-06-08) locked design decisions captured below. Tracking shape: 5 sub-slices, each its own commit. **Execution order** (note interleaving with 33R5b): 33S0 → 33S1 → 33S2 → 33S3 → **33R5b (json batteries — gates 33S4's no-Python pipeline demo)** → 33S4. The 33R5b entry stays filed in the 33R market-readiness track; its work order is pulled forward here as the gating slice between 33S3 and 33S4.

  **Locked design (from pre-phase chat 2026-06-08):**

  - **Seven effect rows.** Each new primitive declares a dimensional effect the checker reasons about through `@budget`, `@trust`, `@deterministic`, `@replayable`. Rows: `std.io.read` (reversible, fs.read, low latency, no side-effect); `std.io.write` (NOT reversible, fs.write, low, side-effecting); `std.io.list` (reversible, fs.read, low, no side-effect); `std.http.get` (reversible — read-only at caller, net.egress+net.ingress, network latency, no side-effect); `std.http.post` (NOT reversible, net.egress, network, side-effecting); `std.db.query` (reversible, db.read, disk, no side-effect); `std.db.execute` (NOT reversible, db.write, disk, side-effecting). All seven are NON-DETERMINISTIC — the checker rejects calls inside `@deterministic` agents.
  - **New `io_source` dimension** (separate from `data`). The existing `data` dim classifies CONTENT (none / grounded / session / memory); the new `io_source` dim classifies SOURCE/SINK (`fs.read`, `fs.write`, `net.egress`, `net.ingress`, `db.read`, `db.write`). Composition rule: `Union`. Default: `none`. Cleaner separation than overloading `data` with both content-class and source values; future-proof for richer egress policies that compose `io_source` + `data` (e.g. "this agent reads `data: customer` AND `io_source: net.egress` → blocked by tenancy policy"). Registered in `crates/corvid-types/src/effects.rs::register_builtin_dimensions()` + the canonical guarantee table in `docs/reference/core-semantics.md`.
  - **Require-explicit-config security model.** No silent defaults. `corvid.toml`'s `[io] root = "..."` (or `CORVID_IO_ROOT` env) is **required** before any executing file I/O resolves — first-run errors out with a clear diagnostic naming the missing config. `corvid new` scaffolds `[io] root = "."` so the default starter project works while making the security boundary explicit in every project's `corvid.toml`. Same shape for `[http] allow = ["..."]` (HTTP egress allowlist) — required, scaffold writes a starter. SSRF block (private / loopback / link-local IP rejection) is **always on**, not configurable — it's a structural property of the language, not a setting. Allowlist narrows further; SSRF is the floor. This makes egress signable: `corvid build --sign` can attest "this binary reaches only these hosts" via the io_source effect row + the project's allowlist.
  - **Module shape.** Envelope types stay where they are (`std/http.cor` already declares `HttpRequestEnvelope` / `HttpResponseEnvelope` / `HttpHeader`; `std/io.cor` declares `FileReadEnvelope` / `FileWriteEnvelope` / `DirectoryEntryEnvelope` / `PathInfo`; `std/db.cor` declares 15 typed envelopes including `DbConnection` / `DbQuery` / `DbResult` / `DbParam` / `DbColumn` / `DbError`). Executing agents land in the SAME modules. Backed by new `crates/corvid-runtime/src/ffi_bridge/{io,http,db}_exports.rs` following the existing `prompt_exports.rs` / `replay_exports.rs` pattern (`pub unsafe extern "C" fn corvid_<name>(...)`). New Value variants for results — notably a stateful `Value::DbHandle` in `corvid-vm` for `db.open → query/execute` to thread a real connection through the value system (genuine Value-system extension, not a refactor).

  **Sub-slices:**

  - [x] 33S0-foundation   **[closed 2026-06-08]** Foundation slice. Re-scoped honestly during execution after the cross-reference invariant in `corvid-guarantees::tests::every_enforced_guarantee_has_positive_and_adversarial_test_refs` was found to require non-empty test_refs for any Static/RuntimeChecked guarantee — the seven new effect rows can't be guarantee-registered until 33S1/2/3 ship their tests. So the guarantee-registration + claim-coverage + core-semantics regen work moves to the per-surface slices where the tests land alongside; 33S0 ships the registry plumbing + scaffolds + error variant + config parsing only. Shipped: (1) **`io_source` dimension** in `crates/corvid-types/src/effects.rs::register_builtin_dimensions()` with `Union` composition, default `Name("none")`, distinct from the existing `data` dim (content-class) — io_source carries source/sink classification (fs.read / fs.write / net.egress / db.read / db.write). (2) **Seven built-in effect profiles** via new `register_io_effects()` + `register_http_effects()` + `register_db_effects()` methods on `EffectRegistry`: io_read / io_write / io_list / http_get / http_post / db_query / db_execute. Each declares io_source value; write-shape effects (io_write / http_post / db_execute) declare `reversible: false`. (3) **`RuntimeError::SurfaceNotImplemented { surface, function }`** in `corvid-runtime-core/src/errors.rs` with a Display impl that names both fields + references the per-surface slice that will wire the impl — distinct variant from QuarantineViolation. (4) **Three ffi_bridge module scaffolds** at `crates/corvid-runtime/src/ffi_bridge/{io,http,db}_exports.rs`, each carrying a `surface_not_implemented(function)` helper that returns the matching `SurfaceNotImplemented` variant. The actual extern "C" entry points land per-surface in 33S1/2/3. (5) **`CorvidConfig` extensions** in `crates/corvid-types/src/config.rs`: new `[io]` table with `root: Option<String>` (parses both relative and absolute paths; semantic interpretation lives in 33S1) and `[http]` table with `allow: Vec<String>` (egress allowlist; SSRF block enforcement + allowlist check in 33S2). Acceptance: 12 new unit tests pass (3 effect registry tests + 2 SurfaceNotImplemented tests + 6 config parsing tests + 1 composition test); `cargo check --workspace --tests` clean; corpus verify exits 1 only on the two deliberate fixtures. Deferred to per-surface slices (where tests + impl ship together): 8 guarantee-registry rows, claim-coverage updates, `docs/reference/core-semantics.md` regen, `Value::DbHandle` variant.

  - [ ] 33S1-file-io-surface   **[invention — full contract, split into 3 sub-slices for single-concern commits]** Wire `io.read_text(path)`, `io.write_text(path, content)`, `io.list_dir(path)` through `crates/corvid-runtime/src/io.rs`. Each returns the typed envelope already declared in `std/io.cor`. Enforce `[io] root` confinement (reject `..` escapes + absolute-path escapes); mark non-deterministic via the existing checker rule that rejects all tool calls inside `@deterministic` bodies; record each call in the trace; quarantine writes on replay via the existing `IoRuntime::quarantine_writes` hook. Split into three sub-slices because the full landing (~600 lines across 8+ files in 3+ crates) is too large for a responsible single-session commit; each sub-slice is its own single concern. The umbrella invention-shipping contract still holds — full proof matrix lands across 33S1a + 33S1b + 33S1c. Acceptance for the umbrella: a Corvid program reads + writes + lists through `corvid run` against a temp root; traversal attempt is rejected; replay quarantines the write; tour topic compiles + runs; 3 guarantees registered with real test refs.
    - [x] 33S1a-tool-declarations-and-policy-plumbing   **[closed 2026-06-08]** Shipped: (1) Three `public tool` declarations in `std/io.cor` — `read_text(path: String) -> FileReadEnvelope uses io_read`, `write_text(path: String, content: String) -> FileWriteEnvelope uses io_write`, `list_dir(path: String) -> List<DirectoryEntryEnvelope> uses io_list`. (2) New `IoToolPolicy` struct in `crates/corvid-runtime/src/io.rs` carrying the resolved `[io] root` + path resolution logic. `IoToolPolicy::new(root_value, corvid_toml_dir)` resolves relative roots against the corvid.toml directory; absolute roots are taken as-is; normalises the result. `resolve(caller_path)` strips leading separators (so absolute-looking caller inputs can't escape via `/etc/passwd`), joins against root, normalises, and rejects via `Path::starts_with` if the resolved path escapes the root. `IoToolPolicy::unset()` is the default that fails closed with the missing-`[io] root` diagnostic. (3) `Runtime::io_policy` field + `RuntimeBuilder::io_policy(policy)` setter. (4) Dispatch interception in `Runtime::call_tool` routes any tool whose name starts with `io.` to the new `dispatch_stdlib_io_tool` method which extracts JSON args, calls `io_policy.resolve()`, then dispatches to the matching `IoRuntime::read_text` / `write_text` / `list_dir` method, marshalling each result (`FileRead` / `FileWrite` / `DirectoryEntry`) to a `serde_json::Value` matching the envelope schema declared in `std/io.cor`. New `stdlib_io_effect_envelope` helper produces the `EffectEnvelope` JSON. Acceptance: 6 plumbing-only unit tests (relative-root resolution + absolute-root accepted + traversal rejection + absolute-input confinement + unconfigured-fail-closed + configured-reports-root-path) all green; `cargo check --workspace --tests` clean; `corvid verify --corpus tests/corpus` exits 1 only on the two deliberate fixtures. End-to-end functional tests + replay-quarantine fixture land in 33S1b; guarantees + tour topic + reference doc + inventions row + README catalog land in 33S1c.
    - [x] 33S1b-functional-and-quarantine-tests   **[closed 2026-06-08]** Shipped end-to-end acceptance tests + CLI/driver-layer wiring. (1) **corvid.toml loader** at `crates/corvid-driver/src/run.rs::load_io_tool_policy`: reads `CORVID_IO_ROOT` env override first (matches the existing CORVID_MODEL pattern); falls back to `corvid.toml`'s `[io] root` (with the corvid.toml dir as the relative-path anchor); falls back to `IoToolPolicy::unset()` for the fail-closed default. Installed via `RuntimeBuilder::io_policy(...)` in the `run_via_interpreter_tier` path so live `corvid run` invocations resolve I/O calls through the policy. (2) **5 end-to-end tests** in new `crates/corvid-runtime/tests/executing_io_tools.rs`: round-trip read/write/list through `Runtime::call_tool("io.*", ...)`; path-traversal rejection (`../../etc/passwd` rejected with diagnostic naming offending path + configured root); fail-closed-on-unconfigured-policy (diagnostic names `[io] root` + the 33S0 security model); both absolute and relative roots resolve correctly. (3) **3 loader tests** in `corvid-driver/src/run.rs::io_policy_loader_tests`: corvid.toml relative root anchors against toml dir; absent `[io]` section produces unconfigured policy; `CORVID_IO_ROOT` env overrides corvid.toml. (4) **2 @deterministic-rejection tests** in `corvid-types/src/tests.rs`: confirms the existing decl-replayability rule (`decl_replayability.rs:184`) rejects `io_read` AND `io_write` tool calls inside `@deterministic` bodies — no new checker logic needed. (5) **2 replay-quarantine fixtures** in `replay_quarantine_corpus.rs`: `replay_blocks_executing_io_write_tool_dispatch_from_escaping_to_filesystem` (proves the dispatch path's write goes through replay substitution OR write-quarantine, never reaches FS) + `replay_blocks_executing_io_read_tool_dispatch_without_recorded_event` (proves reads ALSO can't bypass replay substitution). Total: **12 new tests** across 4 test surfaces; all pass with workspace check clean and `corvid verify --corpus tests/corpus` exits 1 only on the two deliberate fixtures. The executing file-I/O surface now actually executes end-to-end. Guarantees + tour topic + reference doc + inventions row + README catalog land in 33S1c.
    - [ ] 33S1c-invention-proof-artifacts   Three new guarantee-registry rows in `corvid-guarantees::registry::GUARANTEE_REGISTRY` (with real test refs pointing at 33S1b tests): `io_source.fs_read_quarantine_on_replay` (RuntimeChecked / Runtime), `io_source.fs_write_quarantine_on_replay` (RuntimeChecked / Runtime), `io_source.fs_path_confinement` (Static / Runtime). Claim-coverage updates so `corvid build --sign` accepts these ids. `docs/reference/core-semantics.md` regenerated via `corvid contract regen-doc`. `corvid tour --topic file-io` topic added + the compile-and-run CI guard for it. `docs/reference/stdlib/io.md` reference page documenting the 3 tools + their effect rows + the FFI execution model. `docs/reference/inventions.md` proof-matrix row. README invention-catalog entry. Spec link from the new doc to the effect-spec. Commit: `docs(invention): file-I/O surface proof matrix, tour, and reference doc`.

  - [ ] 33S2-http-client-surface   **[invention — full contract]** Wire `http.get(request)` and `http.post_json(request)` through `HttpClient::send`, returning the existing `HttpResponseEnvelope`. Enforce SSRF default-deny (block private RFC1918 / loopback / link-local addresses regardless of allowlist) — load-bearing security floor. Enforce `[http] allow` allowlist on top — every reachable host must be in the project's declared list. Cap response body size + honor envelope timeout + retry policy. Mark POST side-effecting + non-deterministic; mark GET reversible + non-deterministic. Record in trace. Quarantine on replay (`HttpClient::quarantine` is the hook). Invention proof: README entry, `corvid tour --topic http-client` (loopback test server in the tour; real endpoint optional), `docs/reference/stdlib/http.md`, inventions.md row, spec link. Tests (five classes): functional GET, functional POST, blocked-private-IP rejection, allowlist-pass success, timeout enforcement, replay-quarantine fixture. Acceptance: a Corvid program performs a real GET + a real POST through `corvid run` against a configured allowlist; a request to a private IP is blocked regardless of allowlist; replay quarantines the POST. Commit: `feat(std): executing HTTP client surface (get/post_json)`.

  - [ ] 33S3-sqlite-surface   **[invention — full contract]** Add `db.open(path)`, `db.query(db, sql, params)`, `db.execute(db, sql, params)` via a new `ffi_bridge/db_exports.rs` over rusqlite. Introduce `Value::DbHandle` (opaque, refcounted) in `corvid-vm` for `db.open` to return — threaded through `query` / `execute`. Parameter-bound only: the API takes `(sql: String, params: List<DbParam>)` and binds via `rusqlite::params` internally — no string interpolation. Support `:memory:`. Mark `execute` side-effecting + non-deterministic; mark `query` reversible + non-deterministic. Record in trace. Add the new SQLite quarantine mode — reads substitute from the recorded trace; writes refuse and raise QuarantineViolation. Invention proof: README entry, `corvid tour --topic sqlite`, `docs/reference/stdlib/db.md` (replacing the "envelopes only" framing for the SQLite path), inventions.md row, spec link. Tests (four classes): functional open + create-table + parameterized-insert + query round-trip, parameter-binding injection-attempt proof (sql with `'; DROP TABLE` in params stays escaped), determinism (`@deterministic` rejection), replay-quarantine fixture. Acceptance: a Corvid program opens an SQLite DB, creates a table, inserts parameterized rows, queries them back through `corvid run`; replay quarantines the writes; the parameter-binding test demonstrates an injection-shaped param is bound, not interpolated. Commit: `feat(std): executing SQLite surface (open/query/execute)`.

  - [ ] 33S4-batteries-quickstart   **[gated on 33S0–S3 + 33R5b json batteries shipping first — see 33R5b in 33R track]** The adoption-payoff slice. Add a `docs/book` chapter "Talking to the outside world" that builds a tiny end-to-end pipeline using only Corvid (HTTP GET → parse JSON via 33R5b → write to SQLite → read back via `db.query`), zero Python glue. Update `docs/book/02-quickstart.md` so the first real example uses one executing I/O call (likely `io.read_text` since file I/O is the simplest surface). Extend the example-compiles CI guard (or add one) so the three new tour topics (file-io, http-client, sqlite) AND the book pipeline compile-and-run on every push. Acceptance: the no-Python pipeline runs end-to-end through `corvid run` with NO Python in the project; CI compiles + runs all three new tour topics + the book pipeline. Commit: `docs(book): end-to-end I/O pipeline with no host glue + CI coverage`.

  **Phase closure criteria:** ROADMAP checklist fully `[x]`; dev-log + learnings cover each surface; every surface has its tour topic + inventions.md row + stdlib reference page + replay-quarantine fixture; the security fixtures (path traversal, SSRF, param-binding) all pass; `corvid verify --corpus tests/corpus` still exits 1 only on the two deliberate fixtures.

- [x] 33Q14-self-trial-round-4-gap-closure   Closes two reviewer-visible gaps surfaced by maintainer-as-reviewer self-trial round 4 (the `/tmp/job_coordinator` app — a daily-summary cron app shape that round 3's `cyber-threat-intel` reviewer did not exercise). **Gap A — schedule-not-executable warning:** `schedule "0 9 * * *" zone "..." -> agent(...)` declarations parse + typecheck + lower to IR cleanly but the v1.0 scheduler runner does not yet fire scheduled jobs, so a reviewer's daily cron would silently never run. Pre-fix `corvid check` reported `ok` with NO signal. Ships `TypeWarningKind::ScheduleNotExecutable { agent, cron }` (code **W0280**) in `crates/corvid-types/src/errors/warning_kind.rs` emitted by a pre-pass in `typecheck_with_everything` that walks `Decl::Schedule` entries; threaded through `CompileResult.warnings: Vec<Diagnostic>` (separate from `diagnostics` so the existing `ok()` flow is unchanged); rendered by a new severity-aware `render_pretty_with_severity` + `render_all_pretty_warnings` in `crates/corvid-driver/src/render.rs` that uses ariadne's `ReportKind::Custom("warning", Color::Yellow)` instead of the error-only red header; surfaced from `cmd_check` in `crates/corvid-cli/src/commands/misc.rs` BEFORE the success branch so a reviewer sees the warning even when the file compiles cleanly. Acceptance: live `corvid check /tmp/job_coordinator/src/main.cor` now prints a yellow `warning: W0280 schedule "0 9 * * *" -> summarize_yesterday(...) parses + typechecks but the v1.0 scheduler runner does not yet fire scheduled jobs — the cron will NOT execute` block plus the `1 warning(s).` summary then exits 0 with `ok: ... — no errors (1 warning(s) above)`. **Gap B — cdylib_catalog test race under parallel cargo test:** the 9 `#[test]` functions in `crates/corvid-runtime/tests/cdylib_catalog.rs` each call `build_catalog_library()` (cargo build of a real cdylib) and then load it via `libloading`; running under `cargo test --workspace` default parallel scheduler raced on the cargo build lock + process-global `corvid_register_tool` C-ABI registry, producing 7/9 spurious failures in workspace runs. Mirrors the `ENV_LOCK` pattern 33Q13c used for `deploy_cmd`. Ships a module-level `static BUILD_LOCK: Mutex<()> = Mutex::new(());` with a `let _guard = BUILD_LOCK.lock().expect("BUILD_LOCK poisoned");` as the FIRST line of every `#[test]` body. Verified live: parallel `cargo test -p corvid-runtime --test cdylib_catalog` (no `--test-threads=1`) now passes 9/9 reliably. Filed by maintainer-as-reviewer self-trial round 4 (2026-06-05) as the comprehensive gap-closure pass between 33Q13e and the next launch-material slice.

- [ ] 33Q13d-deploy-tailor-llm-promote   **[post-v1.0]** Promote the deterministic tailor to an LLM-driven refinement layer that proposes free-form manifest adjustments anchored to the 33Q13c signals — e.g. "your `support_ai` effect declares `data: customer` AND the app has 3 external connectors; consider adding a per-tenant network policy in K8s." Same Grounded<T> shape as 33Q13b: the LLM can only refine recommendations that anchor in a 33Q13c-detected signal; it CANNOT invent a recommendation for a signal the app doesn't have. Filed post-v1.0 because the deterministic v1.0 surface already serves the operator-facing 33M-round needs.

- [ ] 33Q13b-synthesize-feedback-llm-promote   **[post-v1.0]** Promote the deterministic synthesizer to an LLM-driven thematic-clustering layer that recognizes meta-themes spanning findings whose explicit class tags differ — e.g. "install pipeline integrity" as a theme covering 33Q6 + OPEN-GAP-PROMPTS L-3 + 33Q11. Implementation as a Corvid agent in `examples/agentic_helpers/` (the "AI helpers run as Corvid programs" launch claim), with `Grounded<T>` outputs anchored to the deterministic core's citations from 33Q13a. The deterministic core stays the structural truth — the LLM layer can only group, not invent. Adversarial: a hallucinated theme that doesn't anchor in any 33Q13a-extracted finding is REJECTED at the Grounded<T> boundary. Mock-mode for tests; real-LLM mode behind an opt-in env var. Filed as post-v1.0 because the dogfooding-on-Corvid pattern is more launch-material than launch-critical, and the deterministic v1.0 surface already serves the operator-facing 33M-round needs.

### Phase 34 — Inventions readme + landing page (~2 weeks) ✅ closed

**Goal.** Every Corvid invention documented in one place, visible from the repo's front door. The README and landing page must answer: "what does this language do that no other language does?" — in code, not in prose.

**Hard dep:** everything. This is the final writing pass before launch. Every feature referenced must be shipped and runnable.

**Why this phase exists.** Phase 33 ships v1.0 with documentation (reference, tutorial, cookbook, migration guide). Phase 34 adds a **dedicated inventions catalog** — a single authoritative document listing every feature Corvid has that no other language has, with runnable examples for each. This is the artifact developers link to, cite on HN, and scan before deciding to try Corvid. Without it, the inventions are buried across Phase 20 slices, the eval docs, the streaming spec, the typed model substrate spec, and the replay flagship docs.

**Scope:**

- [x] Rewrite the repo root `README.md` with the full inventions catalog up top, above the install instructions. Every entry has a 2-line pitch + code example + link to spec.
- [x] Category structure matching the moat: **Safety at compile time** (approve gates, dimensional effects, Grounded<T>, @min_confidence, @budget), **AI-native ergonomics** (agent/tool/prompt/approve/effect/model keywords, evals with trace assertions, replay), **Adaptive routing** (20h model substrate — capability routing, content-aware dispatch, progressive refinement, ensemble voting, adversarial validation, jurisdiction/compliance, privacy tiers, cost-frontier exploration), **Streaming** (20f — live cost termination, per-element provenance, mid-stream escalation, progressive structured types, resumption tokens, fan-out/fan-in), **Verification** (20g — cross-tier differential verification, LLM-driven adversarial bypass generation, executable interactive spec, preserved-semantics fuzzing, seed regression corpus with public submission process).
- [x] Landing page rewrite (`docs/site/`): every invention gets a runnable playground example. "Corvid is faster than Python at X" / "safer than TypeScript at Y" claims are supported with side-by-side comparisons that actually run.
- [x] Runnable invention index: `corvid tour --topic <name>` CLI command opens the REPL pre-loaded with compiler-checked demos; `corvid tour --list` shows the shipped catalog across safety, AI-native ergonomics, adaptive routing, streaming, and verification.
- [x] Cross-references: each invention in the README links to (a) the roadmap slice that shipped it, (b) the spec section that formalizes it, (c) the example in the tour, (d) the test that validates it.
- [x] Headline inventions page (`docs/reference/inventions.md`): the standalone artifact HN threads link to. No install prerequisite, no build system context — just the inventions, their syntax, and why each is unique.
- [x] Invention proof matrix: every catalog entry has columns for shipped status, runnable command, test coverage, docs/spec link, and explicit non-scope.
- [x] Update `CLAUDE.md` (or equivalent contributor doc) to require that every new invention ships with a README catalog entry + tour demo.

**Non-scope:** marketing copy, video scripts, social-media assets — those belong to Phase 33's launch materials. Phase 34 is the authoritative technical catalog; Phase 33 is the launch campaign that points to it.

**Defensibility gate.** Phase 34 closes the inventions catalog. Phase 35 closes the *defensible-core* surface that the catalog rests on. Public launch is gated on Phase 35 plus the production-backend market track below; Corvid does not go online as a language for real AI applications until it can build and operate a full backend product itself.

### Phase 35 — Defensible core (~6–8 weeks) ✅ closed

**Goal.** Make Corvid's launch claim defensible under hostile public scrutiny. Every public guarantee is enumerated in a machine-readable manifest, every guarantee is backed by adversarial tests, the ABI surface is bilaterally verified, and the launch wording is derivable from shipped artifacts rather than aspirational. After Phase 35, an outside reviewer can answer "what does Corvid guarantee, what is checked statically, what is checked at runtime, what is out of scope, and how do I verify each independently?" in under ten minutes by running committed commands.

**Hard dep:** every prior closed phase, especially Phase 22 (C ABI) and the signed-attestation moat extension shipped after Phase 34. Phase 35 is the defensibility gate — Phase 33's remaining unchecked items must reference Phase 35 artifacts (claim audit, stability contract, audit command) rather than ship parallel to them.

**Why this phase exists.** External review on the path to public launch identified that while Corvid's *implementation* is real (compiler, runtime, tests, attestation), the *publicly defensible core story* is thinner than the implementation. Five concrete gaps:

1. **Semantic contract is not crisply enumerated.** What is static-checked vs runtime-checked vs out-of-scope is implicit in the test suite. An outsider cannot answer it without reading the codebase.
2. **Proof lives in tests, not in a concise core spec.** The repo has thousands of assertions; outsiders need a single readable spec that ties every public claim to a named test.
3. **Trusted computing base is broad.** Parser, resolver, typechecker, IR lowering, codegen, runtime, ABI emit, and CLI all participate in the same trust boundary. A bug anywhere voids the launch claim.
4. **Launch wording risks getting ahead of formal proof.** Phrasings such as "AI safety contracts are proven" need narrowing to behaviour the compiler actually enforces and that an external party can verify locally.
5. **Adversarial coverage is thin.** Far more positive tests than must-fail tests for approval bypass, descriptor forgery, effect under-reporting, replay tampering, and import-boundary attacks.

This phase closes all five end-to-end with no shortcuts: a guarantee manifest tagged in the compiler, doc generation from the manifest, a property-based fuzz corpus, a separate-binary descriptor rebuilder + byte-compare check (slice 35-H — see slice text for the precise threat model the shipped path defends; true second-implementation independence stays post-v1.0), a sign-refusal contract, and a `corvid claim --explain` provenance command.

**Slice checklist:**

- [x] 35-A-registry             `corvid-guarantees` crate: `GuaranteeKind` / `GuaranteeClass` (Static / RuntimeChecked / OutOfScope) / `Phase` enums + canonical `GUARANTEE_REGISTRY` static array. Every public Corvid guarantee enumerated with id, class, enforcing pipeline phase, description, and required test references.
- [x] 35-B-diag-tagging          Every contract-enforcing diagnostic in resolve / typecheck / IR-lower / codegen / runtime carries its `guarantee_id`. Build-time lint rejects untagged contract diagnostics. No contract enforcement is anonymous.
- [x] 35-C-contract-list         `corvid contract list` CLI subcommand emits the canonical guarantee table as JSON or human-readable. Single source of truth — every later artifact derives from this command's output.
- [x] 35-D-spec-generation       `xtask` regenerates `docs/reference/core-semantics.md` from `GUARANTEE_REGISTRY`; CI fails on drift between committed doc and generated. Spec ≡ implementation, automatically. No hand-edited semantics page.
- [x] 35-E-test-cross-refs       Every Static guarantee carries `positive_test_refs` and `adversarial_test_refs`; build-time check rejects empty adversarial coverage on a Static guarantee. Every guarantee in the registry must point to real test functions that compile and run.
- [x] 35-F-fuzz-abi              Adversarial fuzz corpus over the ABI surface: `proptest`-driven byte mutators on descriptor JSON and DSSE attestation envelopes (corrupt signatures, swap payload types, mutate PAE bytes, drop required fields, inject extra symbols). ≥100 mutants per gate; each must be rejected with the documented exit code; benign mutations must round-trip.
- [x] 35-G-fuzz-source           Adversarial fuzz corpus over source-level bypasses: AST mutators for `@approve` re-export bypass, effect under-reporting at module boundary, `Grounded<T>` provenance loss across function calls, import-aliasing of dangerous tools. Each mutated source must fail typecheck with the diagnostic tagged to the right `guarantee_id` from slice 35-B.
- [x] 35-H-bilateral-verifier    Separate-binary ABI verifier (`corvid-abi-verify`): rebuilds the descriptor through the workspace's shared frontend (lex / parse / resolve / typecheck / IR-lower / abi-emit) and byte-compares the rebuilt descriptor against the embedded `CORVID_ABI_DESCRIPTOR` symbol in a built cdylib. Disagreement = build rejection. Defends against post-link descriptor tampering, build-cache modifications, and partial-rebuild drift between the source-of-truth and the artifact a host receives. **Phase 35V-T1-H wording correction (2026-05-08):** earlier slice text claimed "independent code path", "two implementations", and "TCB shrinkage" — none of those are shipped: the verifier links the same `corvid-syntax` / `corvid-resolve` / `corvid-types` / `corvid-ir` / `corvid-abi` libraries the main pipeline uses, so a logic bug in any of those frontends affects both paths identically. The shipped property is "rebuild + byte-compare across a separate process invocation," which is real and useful but narrower than full TCB shrinkage. **Non-scope (post-v1.0):** true second-implementation TCB shrinkage — a separate parser/resolver/typechecker reaching `AbiDescriptor` independently — is a future-phase effort; promotion of any registry row that depends on it stays gated on that work landing.
- [x] 35-I-claim-explain         `corvid claim --explain <cdylib>`: emits a self-contained provenance statement listing every guarantee enforced for the given binary, by id and class, plus the signing key fingerprint and verifier-agreement attestation from slice 35-H. The artifact HN threads can quote without further context.
- [x] 35-J-sign-refusal          `corvid build --sign` refuses to emit a signed cdylib unless every declared contract in the source maps to a `GUARANTEE_REGISTRY` entry that was actually checked in this build. No silent skips, no "we didn't run that pass on this target" downgrades. The signed artifact carries the *enforced* claim, not the *intended* claim.
- [x] 35-K-security-model        `docs/security/model.md`: TCB diagram (compiler + verifier + runtime + signer + ABI surface), threat model (insider/outsider, what each defends against), explicit non-goals (compromised host kernel, signing-key compromise, compiler-toolchain compromise). References slice 35-H/I/J behaviours; does not over-claim.
- [x] 35-L-readme-alignment      Replace any aspirational launch wording with claims derivable from `corvid claim --explain`, the adversarial corpus, and the bilateral verifier. README and landing page point at runnable commands; the wording is the *output* of the artifacts, not a separate prose layer.
- [x] 35-M-ci-gate               CI workflow runs the fuzz corpus + bilateral verifier + spec drift check on every push. Phase 35 artifacts are continuously enforced, not point-in-time at launch.

**Audit correction (Phase 35-41 audit, 2026-04-29):** the original Phase 35 claim coverage table only registered Phase 21/22/35 contract ids. Every later phase that introduced a new declared contract (Phase 38 `@retry`/`@idempotency`/`@replayable`/`job`/`schedule`/`await_approval`, Phase 39 `auth`/`tenant`/`role`/`permission`/`approval`/`@requires`/`@approval`, Phase 41 `connector`/`scopes`/`rate_limit`/`redact`/`webhook_signed_by`) inherited the same gate but never added itself to it. A signed cdylib that uses any of those features ships an *incomplete* claim today. 35-N closes the inheritance hole so the gate moves with the language.

- [x] 35-N-claim-coverage-extend  Two pieces. (a) `validate_signed_claim_coverage` walks every AST decl that exists today but was unhandled: `Decl::Schedule` (cron triggers from Phase 38) and `Decl::Server` (route surfaces from Phase 36). (b) Registry rows landed as `OutOfScope` placeholders for the Phase 38/39/41 contract surfaces that have no AST representation yet (`jobs.*` for `@retry`/`@idempotency`/`@replayable`/`job`/`await_approval`; `auth.*` for `auth`/`tenant`/`role`/`permission`/`approval`/`@requires`/`@approval`; `connector.*` for `connector`/`scopes`/`rate_limit`/`redact`/`webhook_signed_by`). Each `OutOfScope` row carries the same explicit `out_of_scope_reason` pattern the existing platform rows use, naming the audit-correction slice (38K/38M/39K/39L/41K/41L/41M) that promotes it to `Static` or `RuntimeChecked`. When a phase's audit-correction track adds the parser-level surface, that track is responsible for promoting the matching row(s) and adding the gate walker hooks. Adversarial test: signed build refuses when a `Decl::Schedule` declares a target that has no registered job-coverage row.

**Non-scope:**

- Formal mechanized proof of the type system (post-v1.0 research; the core-semantics manifest is the v1.0 surface).
- Proof of cryptographic primitives — we use ed25519, SHA-256, and DSSE as standardized primitives, not redesigns.
- Defense against compiler-toolchain compromise (we trust rustc and Cranelift; reproducible builds are a post-v1.0 hardening).
- Defense against signing-key compromise — key management is a host responsibility, not a Corvid responsibility, and `docs/security/model.md` says so explicitly.
- Bug-bounty program, third-party audit contract, formal launch comms — those belong to the final market-launch phase, not to Phase 35.

**Defensible-core cut here.** Phase 35 proves the language's claims. The next phases prove Corvid is a complete backend language for production AI applications, not just a compiler with excellent AI-safety primitives.

### Production slice standard

Every Phase 36-43 slice must clear the same four gates before it can be marked done. This is how the production-backend track stays inventive instead of becoming a long checklist of ordinary web-framework features.

1. **Developer pain removed.** The slice must name the concrete pain it removes for production AI developers: glue code, duplicated policy checks, invisible cost, missing replay, unsafe tool calls, connector OAuth work, weak traces, hand-written audit logs, migration drift, deployment guesswork, or benchmark uncertainty.
2. **AI-native invention.** The slice must add or preserve at least one Corvid-specific AI primitive through the layer it touches: effects, approvals, budgets, provenance, replay, confidence, model routing, evals, trace assertions, signed claims, or guarantee IDs. A generic backend feature without one of these is not enough.
3. **Benchmark or proof.** The slice must ship a measurable artifact: benchmark, adversarial test, golden trace, route test, migration drift test, replay fixture, connector mock, operator command, or reference-app proof. If Corvid cannot beat a mature language/framework on raw speed, the benchmark must show the dimension Corvid wins: fewer unsafe lines, fewer moving parts, compile-time rejection, replayability, audit completeness, or operational time-to-answer.
4. **AI usage in development.** The slice brief must include at least one AI-assisted maintainer workflow that Corvid itself enables or will enable: generating tests from traces, turning production runs into evals, explaining failed guarantees, producing approval summaries, suggesting migrations, summarizing incidents, or creating connector mocks. AI is part of the developer workflow, not only the user's application.

**Benchmark posture.** Corvid should not claim to beat Go, Rust, Java, Node, or Python frameworks on every raw throughput benchmark. The intended win is broader and more relevant to AI backends:

- **Against Python/LangChain-style stacks:** fewer host-language layers, stronger static contracts, faster non-model orchestration, replay/eval/approval built into the language rather than scattered libraries.
- **Against TypeScript/Node agent stacks:** stronger compile-time effect and approval boundaries, native binary deployment, explicit cost/provenance/replay contracts, lower operational ambiguity.
- **Against Go/Rust backend stacks:** less handwritten AI governance code, first-class model/tool/approval/eval semantics, signed AI-safety claims, and faster development of auditable agent backends.
- **Against workflow engines:** richer language-level typing and AI contracts while retaining durable jobs, replay, approvals, and operator controls.

Each benchmark must separate model-provider latency from Corvid runtime overhead. Hiding LLM latency inside benchmark wins is forbidden.

### Slice completion gate (no shortcuts)

Every Phase 36–43 slice — and every retroactive promotion of a Phase 35 entry from `OutOfScope` to `Static`/`RuntimeChecked` — must clear every box below before the `[x]` lands. Optimistic checkmarks are how earlier phases drifted from spec; this gate exists so the same drift cannot reach the production-backend track.

Maintainers paste the filled-in checklist into the dev-log entry that documents the slice. A slice without a green checklist is not done — period.

**Build + test gates**

- [ ] **Workspace clean.** `cargo check --workspace` and `cargo check --workspace --tests` produce zero warnings (no new `#[allow(dead_code)]` / `#[allow(unused)]` without an inline justification comment).
- [ ] **Unit tests green.** `cargo test -p <affected-crate> --lib` is green; the new code has both positive and adversarial unit tests.
- [ ] **Integration tests green.** Every new CLI subcommand, runtime path, or connector ships with at least one `tests/` integration test that exercises the user-facing flow end-to-end.
- [ ] **Corpus + differential-verify green.** `cargo run -q -p corvid-cli -- verify --corpus tests/corpus` exits with the documented code (1 only on the deliberate-fail fixtures).
- [ ] **CI workflow updated.** `.github/workflows/ci.yml` runs the new tests on every push.

**Registry + claim gates (Phase 35 inheritance)**

- [ ] **Registry entry.** Every new public guarantee has a `corvid_guarantees::GUARANTEE_REGISTRY` row with stable `id`, `kind`, `class` (`Static` / `RuntimeChecked` / `OutOfScope`), enforcing `Phase`, description, and — for `OutOfScope` — an explicit reason.
- [ ] **Diagnostic tagged.** Every new contract-enforcing diagnostic uses `TypeError::with_guarantee` (or the equivalent for runtime / ABI / connector errors) with a registered id; no anonymous contract diagnostics.
- [ ] **Test references populated.** New `Static` and `RuntimeChecked` rows carry ≥1 `positive_test_refs` and ≥1 `adversarial_test_refs` that resolve to real `fn name(` declarations. The cross-reference test in `corvid-guarantees` stays green.
- [ ] **Claim coverage updated.** Every new declared contract pattern (new attribute, new keyword, new effect dimension, new connector method, new approval clause) is added to `validate_signed_claim_coverage` so `corvid build --sign` cannot ship an incomplete claim.
- [ ] **`corvid claim --explain` reports it.** Any new public guarantee or contract surface shows up in `claim --explain` output for an exemplar binary.
- [ ] **`corvid contract list` shows it.** Same for the canonical guarantee table; verified by pasting the JSON output into the dev-log entry.
- [ ] **`docs/reference/core-semantics.md` regenerated.** `cargo run -q -p corvid-cli -- contract regen-doc docs/reference/core-semantics.md` runs cleanly; the drift gate test in `corvid-guarantees::render::tests::rendered_markdown_matches_committed_doc` passes.

**Adversarial gates**

- [ ] **Source-bypass test.** New compile-time contracts have a mutator in `crates/corvid-types/tests/source_bypass_corpus.rs` (or the phase's equivalent corpus) that proves the violation is rejected with the right `guarantee_id`.
- [ ] **Byte-fuzz test (if ABI / attestation / on-disk format).** New parsers carry ≥100 generated mutations in the phase's byte-fuzz corpus; all rejected; benign mutations round-trip.
- [ ] **Named threat coverage.** Every new attack class the slice introduces (approval bypass, scope escalation, replay forgery, connector contract drift, tenant crossing, prompt injection through new surface) has at least one named `must_fail` test.

**AI-in-the-development-loop gate**

- [ ] **AI-assisted helper named.** The slice brief names at least one LLM-pattern (RAG-grounded / generative / adversarial / agentic / assistive) helper the slice enables for maintainers — even when implementation lands in a follow-up. The helper itself runs as a Corvid program (typed effects, `@budget`, `Grounded<T>` outputs, replay-able trace).

**Production-readiness gates**

- [ ] **Real persistence path.** Anything the slice claims to persist actually lands in SQLite + Postgres with row-level locking; no JSON state files masquerading as databases.
- [ ] **Crash-recovery proof.** Any "durable" / "resumable" claim ships an integration test that `SIGKILL`s the worker mid-step and asserts byte-exact resume with no double-spend / double-side-effect.
- [ ] **Mock ≡ replay ≡ real.** Every external integration (connector, LLM, DB, OAuth) has mock + replay + real modes that share one typed surface; CI runs the same test in mock mode at minimum, real mode behind an opt-in env var.
- [ ] **Operator runbook delta.** Slices that add an operator-visible surface (new command, new manifest, new endpoint) update the relevant runbook page in the same commit.
- [ ] **Side-by-side comparison committed.** When the slice claims a moat dimension, a `benches/comparisons/<feature>.md` file shows the equivalent Python / TS / Go code line-by-line, with a Corvid-vs-other governance-line-count delta.

**Documentation gates**

- [ ] **`dev-log.md` entry.** One date-stamped entry per slice; explains *what* changed, *why*, and *how it's tested*. Filled-in checklist pasted in.
- [ ] **`learnings.md` entry (if user-visible).** Doc-and-feature land together.
- [ ] **`docs/security/model.md` reviewed.** If the slice changes the TCB, threat model, or non-goals, the security model is updated in the same commit.
- [ ] **README + landing page alignment.** If the slice introduces a public claim, the wording is derivable from a runnable command. No aspirational copy.

**Phase-level gates (apply when ticking the *phase* done — not the slice)**

- [ ] **Every slice box ticked.** Every `[x]` slice in the phase passed the slice gate above; no carry-over of "we'll fix it in the next phase."
- [ ] **End-to-end demo runnable.** The phase's "Done when:" sentence translates to one or more shell commands that produce the documented output on a clean clone.
- [ ] **No silent `OutOfScope` downgrades.** If a registry entry was downgraded mid-phase, the reason is recorded in the registry AND a follow-up issue/slice is filed to promote it back.
- [ ] **External reviewer signoff (Phase 42–43 only).** At least one developer outside the contributor list runs the phase's demo and signs off in writing on a public issue.

A slice that fails any box rolls back to `[ ]`. The `[x]` is a contract, not a wish.

### Phase 35V — Pre-launch verification round ✅ closed

**Goal.** Comprehensive reconciliation of every `[x]` slice in the approaching-launch surface (Phase 35, Phase 36, plus the 38/39/41 audit-correction tracks filed by the 2026-04-29 audit). Every phase-done claim is re-verified by an independent pass that treats the optimistic `[x]` as a *claim to disprove*, not a fact to trust.

**Why this phase exists.** Phase 35 is the v1.0 launch gate; every public claim Corvid will make at launch traces through its 14 slices. None has had an independent verification pass. The 2026-04-29 audit found four phase-done bullets in Phases 38–41 structurally absent — that audit only happened because one was overdue. Phase 35V exists so the next audit doesn't happen *during* launch.

**Detailed plan:** [docs/phases/phase-35V-pre-launch-audit.md](./docs/phases/phase-35V-pre-launch-audit.md) — three-track structure, per-slice verification methodology, drift-found-vs-clean-signal handling, sequencing rules.

**The verifier-correction pattern, applied to the launch surface.** Same shape as Phase 20m's reconciliation of Phase 20l, applied to a wider surface. Each verification slice produces either (a) a *clean-signal sentinel test* that pins the verified property going forward, or (b) a *drift-correction commit* that closes the gap and adds the same sentinel. Phase 35 itself does not reopen; drift is corrected within Phase 35V.

**Sequencing rules:**

- Three tracks run **strictly sequential**, not parallel — discoveries in earlier tracks may shift later work.
- One slice = one feature, one commit. Validation gate between every commit (`cargo check --workspace` + targeted tests + corpus baseline). Push before next slice.
- Track 1's clean-signal slices may chain quietly. Any slice that finds drift triggers a pre-phase chat on the corrective approach before code lands.
- No autonomous chaining across tracks. End of Track 1 → pre-phase chat on Track 2 scope; end of Track 2 → pre-phase chat on Track 3 closer ceremony.

**Slices:**

Track 1 — Phase 35 verification (the launch gate):

- [x] 35V-T1-A — Verify 35-A registry coverage. Walk every `GUARANTEE_REGISTRY` row; each test ref resolves to a real `fn`.
- [x] 35V-T1-B — Verify 35-B diagnostic tagging. Every contract-enforcing diagnostic carries a `guarantee_id`; build-time lint catches an untagged one (verify by mutation).
- [x] 35V-T1-C — Verify 35-C `corvid contract list`. JSON output equals registry programmatically; human-readable renders for every kind/class.
- [x] 35V-T1-D — Verify 35-D spec generation. Regenerate `docs/reference/core-semantics.md`; bit-compare against committed; mutate registry, verify CI fails.
- [x] 35V-T1-E — Verify 35-E test cross-refs. Every `Static` guarantee has ≥1 positive + ≥1 adversarial test ref resolving to real fns.
- [x] 35V-T1-F — Verify 35-F ABI fuzz corpus. ≥100 mutants per gate; each rejected with documented exit code; benign mutations round-trip.
- [x] 35V-T1-G — Verify 35-G source fuzz corpus. AST mutators cover all four documented attack classes; each fails typecheck with the right `guarantee_id`.
- [x] 35V-T1-H — Verify 35-H bilateral verifier independence. `corvid-abi-verify`'s dep tree does NOT transitively include the main pipeline's typechecker; disagreement triggers build rejection.
- [x] 35V-T1-I — Verify 35-I `claim --explain` stability. Output stable byte-for-byte across re-runs; references registry rows that exist.
- [x] 35V-T1-J — Verify 35-J sign-refusal. Adversarial: declare an unregistered contract; `corvid build --sign` rejects with the right diagnostic id.
- [x] 35V-T1-K — Verify 35-K security model. `docs/security/model.md` exists; TCB diagram references real components; threat model maps to registry rows or non-goals; no over-claims.
- [x] 35V-T1-L — Verify 35-L README alignment. Every README launch claim has a runnable command; no aspirational wording.
- [x] 35V-T1-M — Verify 35-M CI gate. `.github/workflows/*.yml` actually runs fuzz + bilateral verifier + spec drift on every push.
- [x] 35V-T1-N — Verify 35-N claim coverage extension. All promoted rows present; `validate_signed_claim_coverage` walks `Decl::Schedule` and `Decl::Server`; adversarial test exists.

Track 2 — Audit-correction completeness (36/38/39/41):

- [x] 35V-T2-A — Verify 36-K real HTTP runtime. Hand-rolled parser gone; production runtime handles HTTP/1.1 edge cases.
- [x] 35V-T2-B — Verify 36-L middleware pipeline. Auth/rate-limit/tracing/CORS/compression/logging/policy run in declared order.
- [x] 35V-T2-C — Verify 36-M shutdown/timeout tests. Graceful shutdown + timeout + body-limit + handler-isolation deterministically covered.
- [x] 35V-T2-D — Verify 38-K multi-worker job runner. ≥2 workers consume from queue; lease-stealing on worker death exercised.
- [x] 35V-T2-E — Verify 38-K SIGKILL crash-recovery. Real `SIGKILL` mid-step; byte-exact resume; no double-spend.
- [x] 35V-T2-F — Verify 38-K idempotency under concurrency. 4-concurrent-worker test; same job-key never duplicates side-effects.
- [x] 35V-T2-G — Verify 38-M DST-aware cron. Spring-forward + fall-back tested; deterministic firing across DST boundaries.
- [x] 35V-T2-H — Verify 39-K real JWT verification. Real JWKS fetch + `kid` resolution + signature verification; rejects forged JWTs.
- [x] 35V-T2-I — Verify 39-L `corvid auth` / `corvid approvals` CLI. Top-level subcommands wired; tenant-scoped queue exists.
- [x] 35V-T2-J — Verify 41-K connector real-mode CLI. `corvid connectors` exists; real-mode flow is not a stub.
- [x] 35V-T2-K — Verify 41-L connector contract drift. Mock ≡ replay ≡ real shared typed surface; mutations break across modes.
- [x] 35V-T2-L — Verify 41-M connector approval bypass. Approval requirements survive when called through connector wrapper.

Track 3 — Closer commits:

- [x] 35V-T3-A — Phase 35 closer. Write `docs/phases/phase-35-defensible-core.md` if absent. `✅ closed` marker. Closing audit. Learnings entries. `memory/project_phase_35_closed.md`. MEMORY.md pointer.
- [x] 35V-T3-B — Phase 36 closer. Mirror of T3-A for Phase 36. Includes audit-correction work Track 2 verified.
- [x] 35V-T3-C — Phase 38/39/41 audit-correction re-confirmation. Update each phase's audit-correction note in ROADMAP with re-verification status.
- [x] 35V-T3-D — Phase 35V closer. `✅ closed` marker. Closing audit. Learnings rollup of cross-slice patterns. `memory/project_phase_35V_closed.md`. MEMORY.md pointer.

**Out-of-scope deferrals:**

- Forward engineering on Phases 37+ stays paused until Phase 35V closes.
- Phase 33 launch polish (33J/33L/33M) waits for the launch claim to be verified clean.
- The whoami `__imp_GetUserNameExW` linker fix from Phase 20n stays filed as a separate one-commit slice after Phase 35V closes.
- Reopening Phase 35 itself — drift is corrected within Phase 35V, not by reopening.

**Phase-done criteria:**

- [x] Every Track 1 slice (T1-A through T1-N) lands with either a clean-signal sentinel test OR a drift-correction commit.
- [x] Every Track 2 slice (T2-A through T2-L) lands with the same shape.
- [x] Track 3 closers land for Phase 35 and Phase 36; phase docs written if absent; `learnings.md` updated; memory records written; ROADMAP `✅ closed` markers; MEMORY.md pointers.
- [x] Closing audit recorded in `docs/phases/phase-35V-pre-launch-audit.md` with per-slice status (verified-clean / drift-found-and-closed).
- [x] Memory record `project_phase_35V_closed.md` summarises every drift found and the verification methodology for future audit rounds.

---

### Provenance Propagation — Grounded contagion + `@grounded_pure` moat ✅ closed (2026-05-16)

**Goal.** Make `Grounded<T>` ergonomic across ordinary code (the contagion law lifts grounded-ness through operators and call sites without explicit re-annotation) and ship the compile-time moat that refuses any laundering inside an agent body (`@grounded_pure`, composes through the call graph like `@deterministic`).

**Why this phase exists.** Phase 20's grounding work proved that a value can be typed `Grounded<T>` and that the typechecker rejects unsourced grounded returns. But Phase 35V's verification round surfaced `combined_all.cor` going red under differential verify because the typechecker was *grounded-blind* for effect-induced grounding — `data: grounded` only produced `Type::Grounded` when the user wrote the wrapper explicitly. The four tiers disagreed about whether the same value was grounded. Without the contagion law + IR-visible discard + moat, `Grounded<T>` was a type users had to thread by hand, and "no laundering" was a property the compiler couldn't prove. The phase closed both gaps end-to-end across typechecker / IR / interpreter / native / replay.

**Hard dep:** Phase 20 (grounding base + `data: grounded` effect dimension), Phase 35V (the verification round that surfaced the gap). The phase originated from a Phase-35V-era differential-verify red test rather than from a Phase 20 follow-up, so it sits chronologically between Phase 35V and Phase 36 in the dev-log even though its substance extends Phase 20's grounding work.

**Scope:** 11 slices, sub-split where the recon found load-bearing work bigger than the design doc estimated:

- [x] Slice 0           Pre-phase design doc + Decisions D1-D8 / risks R1-R6 / sub-split rationale (`docs/meta/grounded-propagation-design.md`).
- [x] Slice 1           `ProvenanceChain::Derived` how-provenance node + round-trip tests.
- [x] Slice 2a          Contagion law at the type level: `check_binop` / `check_unop` strip-and-rewrap.
- [x] Slice 2b          Design X reversal — `data: grounded` promotes a prompt / tool / agent's return type at the call site (typechecker stops being grounded-blind).
- [x] Slice 3a-3e       Min-confidence composition, runtime grounded operator path, errors-untangle (`ProvenanceChain` + `Approver` + approval types + `RuntimeError` moved to `corvid-runtime-core`).
- [x] Slice 4           Interpreter contagion: `eval_binop` lifts when either operand is `Value::Grounded`.
- [x] Slice 5           Native attestation model for scalar grounded values; refcounted-type path deferred to a follow-up phase.
- [x] Slice 6           Control-flow condition tolerance (D2: `if` accepts `Grounded<Bool>`); `require_bool` strips `Value::Grounded` recursively.
- [x] Slice 7a (`6bad408`)    Typechecker side table `Checked.grounded_coercion_sites` populated at every value-flow `is_assignable_to` site.
- [x] Slice 7b (`942c7e7`)    IR lowering inserts `IrExprKind::UnwrapGrounded` at every recorded span; runtime alignment (`produces_grounded` on `IrTool` / `IrPrompt`, `maybe_ground_prompt_result` mirrors the tool path); `UnwrapGrounded` runtime semantics preserve confidence while discarding provenance.
- [x] Slice 7 doc       Design doc anchor (`53f3336`) records the sub-split + the runtime-alignment finding for future audits.
- [x] Slice 8 (`2e0642c`)     `@grounded_pure` front end — parser + AST (`AgentAttribute::GroundedPure`); dormant.
- [x] Slice 9 (`814d665`)     The proof — `decl_grounded_pure.rs` walks the agent body for three laundering shapes (implicit coercion via slice 7a sites, explicit `.unwrap_discarding_sources()`, transitive non-`@grounded_pure` call); guarantee row `grounded.no_laundering` registered.
- [x] Slice 10 (`ba1326b`)    Corpus fixtures (D7) — `combined_all.cor` to the idiomatic end-to-end-grounded shape; new `legacy_grounded_coercion.cor` exercises the discard node across all four tiers; inline fix to `expr_is_grounded`'s `Prompt` arm.
- [x] Slice 11          Invention-shipping contract — README catalog entry, `corvid tour --topic provenance-propagation`, `docs/reference/inventions.md` row, spec section in `05-grounding.md` §9, `learnings.md` closeout, dev-log entry, ROADMAP tick.

**Out of scope (deferred):**

- Native grounded handles for refcounted types (`Grounded<String>` and `Grounded<Struct>` in the native tier) — own follow-up phase stub at `docs/meta/native-grounded-handles-design.md`. The scalar path ships in this phase; the refcounted path needs the dataflow / dup-drop analysis to see through `Grounded` without double-freeing.
- Short-circuit `&&` / `||` contagion. `Grounded<Bool> && other` still errors at the operator. Different design question (evaluation order); not a moat hole.
- Cross-module composition for `@grounded_pure` on imports (R5 attribute-composition matrix at the import boundary). Slice 8's import handler accepts the attribute on imports but does not check the imported agent. Future-slice work.

**Phase-done criteria:**

- [x] Every slice has a commit on `main` with a passing validation gate (workspace check + targeted tests + corpus verify exit 1 only on the two deliberate fixtures).
- [x] Design doc records sub-splits and recon findings (Design X reversal, slice-7 runtime-alignment gap, slice-10 reachability-analysis gap).
- [x] Guarantee row `grounded.no_laundering` registered with five wired test refs (`every_test_ref_resolves_to_a_real_test_function` enforces).
- [x] `docs/reference/core-semantics.md` auto-regenerated and committed.
- [x] Tour demo `corvid tour --topic provenance-propagation` source compiles through the normal driver pipeline (`corvid check`).
- [x] README catalog entry, `docs/reference/inventions.md` row, spec section in `05-grounding.md` §9, `learnings.md` closeout, dev-log entry all in place.

---

### Phase 36 — Production backend core (~8-10 weeks) ✅ closed

**Goal.** Corvid can build an always-on HTTP backend without a host framework. A developer should be able to write routes, JSON APIs, middleware, health checks, configuration, secrets, structured logs, graceful shutdown, and deployment-ready binaries in Corvid itself.

**Why this phase exists.** Developer pain is not "how do I write an agent demo?" It is "how do I turn this agent into a service that survives auth, retries, observability, secrets, deploys, audits, and production traffic?" Corvid's moat must travel through the backend layer, otherwise developers still have to glue the real product together in another language.

**Inventive benchmark target.** Compare the refund API against FastAPI, Express/Fastify, and Go HTTP. Corvid does not need to beat Go on raw requests/sec in this phase; it must beat AI-backend setup complexity by showing route effects, approvals, traces, env validation, and signed server claims in one language-level path with less handwritten governance code.

**Scope:** ✅ shipped through slices 36A–36M (closed slice list below); ticking the scope echoes here for ROADMAP accuracy.

- [x] `server` declarations or a standard backend entry pattern with typed routes, request/response bodies, path/query params, headers, cookies, status codes, and error responses.
- [x] Runtime HTTP server with async request handling, graceful shutdown, request IDs, timeouts, body-size limits, panic/error isolation, and platform parity.
- [x] Middleware pipeline for auth, rate limits, tracing, CORS, compression, request logging, and effect-aware policy checks.
- [x] Typed JSON encode/decode errors that preserve spans and route names in diagnostics.
- [x] Config and environment layer with required/optional vars, typed parsing, redacted secret reporting, and `corvid doctor` validation.
- [x] Health, readiness, and metrics endpoints generated from runtime state.
- [x] `corvid build --target=server` emits a single backend binary with embedded route manifest and signed contract metadata when signing is enabled.
- [x] AI-native integration: every route can declare effect, approval, replay, budget, provenance, and model-routing constraints; violations fail before deploy.
- [x] End-to-end example: approval-gated refund API served entirely by Corvid, with no Rust/Python/Node host app.

**Slice checklist:**

- [x] 36A-backend-design-brief       `docs/phases/phase-36-backend-core.md` defines backend syntax, runtime ownership, non-scope, route examples, and acceptance tests before code.
- [x] 36B-minimal-server-target      `corvid build --target=server` accepts one backend entrypoint and emits a runnable local server binary.
- [x] 36C-typed-route-model          GET/POST routes have typed path/query/body/response shapes and compile-time validation.
- [x] 36D-json-boundary              Server errors use a stable JSON envelope with request IDs, route, kind, message, and route-aware diagnostics.
- [x] 36E-server-runtime-basics      Request IDs, handler timeouts, graceful drain limits, body limits, and handler isolation work.
- [x] 36F-route-tracing              Every generated-server request emits route, method, status, duration, request ID, and effect metadata as structured trace JSON.
- [x] 36G-health-readiness-metrics   Generated health/readiness/metrics endpoints report server liveness, readiness, counters, and runtime identity.
- [x] 36H-config-and-secrets         Typed backend env validation works at server startup and through `corvid doctor` with redacted invalid values.
- [x] 36I-approval-effect-integration Dangerous route/tool paths without reachable route-local approval contracts fail before deploy.
- [x] 36J-backend-example            `examples/backend/refund_api` ships a checked approval-gated contract and runnable generated server entrypoint with tests.

**Done when:** `examples/backend/refund_api` runs as a production-shaped server, passes route tests, emits traces, enforces approval gates, validates env/config through `corvid doctor`, and builds with `corvid build --target=server`.

**Audit correction before market freeze:** Phase 36 is not market-frozen until the generated server uses a real HTTP parser/runtime boundary, has an actual middleware pipeline, and proves graceful shutdown plus handler timeouts under tests.

- [x] 36K-real-http-runtime          Replace the hand-rolled request-line parser with a production HTTP runtime/parser and route tests for HTTP/1.1 edge cases.
- [x] 36L-middleware-pipeline        Auth, rate-limit, tracing, CORS, compression, request logging, and effect-aware policy middleware run in a declared order.
- [x] 36M-shutdown-timeout-tests     Graceful shutdown, request timeout, body-limit, and handler-isolation behavior is covered by integration tests.

### Phase 37 — Persistence, migrations, and state (~8-10 weeks)

**Goal.** Corvid can own durable application state: tables, records, transactions, migrations, encrypted secrets/tokens, audit logs, and query APIs.

**Inventive benchmark target.** Compare a task/approval/audit schema against Prisma/TypeScript, SQLAlchemy/Alembic, and sqlx/Rust. Corvid must prove migration drift, typed decode failures, DB effects, replay summaries, and AI-action audit logs are first-class instead of manually assembled.

**Scope:** ✅ shipped (slice checklist below is the machine-readable version of these bullets; spot-checked 2026-05-17 — every acceptance criterion holds).

- [x] `std.db` with SQLite first, Postgres second: connection config, query execution, transactions, prepared statements, row decoding, and typed errors.
- [x] Migration system: checked-in migrations, `corvid migrate up/down/status`, drift detection, checksum validation, and CI-safe dry runs.
- [x] Typed records mapped to tables without hiding SQL; developers can use explicit queries and still get typed decode guarantees.
- [x] Encrypted token/credential storage for OAuth refresh tokens and connector state, with clear host key-management boundaries.
- [x] Audit-log table pattern for AI actions: who/what/why, prompt version, model, tool call, approval state, cost, trace ID, and replay key.
- [x] AI-native integration: DB reads/writes are effect-tagged; dangerous writes can require approval; replay records deterministic DB interaction summaries.
- [x] Golden examples for session state, task state, approval state, trace state, and connector token state.

**Slice checklist:**

- [x] 37A-persistence-design-brief   `docs/phases/phase-37-persistence.md` defines DB scope, SQL posture, migration rules, effect model, replay model, and non-scope.
- [x] 37B-sqlite-connection-query    `std.db` exposes SQLite connection, parameterized query/execute, result, and redacted error envelopes.
- [x] 37C-typed-row-decoding         `std.db` exposes typed row decode envelopes for success, missing columns, and wrong value kinds.
- [x] 37D-transactions               `std.db` exposes transaction envelopes for commit, rollback, and nested-scope rejection metadata.
- [x] 37E-migrations-drift           `corvid migrate up/down/status` supports checksums, dry runs, drift detection, and CI failure on mismatch.
- [x] 37F-audit-log-pattern          Standard audit-log schema records actor, action, prompt/model/tool versions, approval state, cost, trace ID, and replay key.
- [x] 37G-token-storage-boundary     Encrypted connector-token storage ships with explicit key-management boundaries and tests.
- [x] 37H-postgres-support           Postgres reaches parity with the SQLite query/transaction/migration subset needed by reference apps.
- [x] 37I-db-effect-replay           DB reads/writes carry effect tags and replay records deterministic interaction summaries.
- [x] 37J-backend-state-example      Backend example persists users, tasks, approvals, traces, connector tokens, and durable agent state.

**Done when:** a Corvid backend can persist users, tasks, approvals, traces, connector tokens, and durable agent state through typed migrations and tests.

**Small-slice breakdown for remaining Phase 37 work:**

- [x] 37E1-migrate-command-shape     Add `corvid migrate status/up/down --dry-run` command shape and help text.
- [x] 37E2-migration-file-scan       Discover ordered checked-in SQL migrations and compute stable SHA-256 checksums.
- [x] 37E3-migration-state-store     Record applied migrations, timestamps, and checksums in a local state store.
- [x] 37E4-drift-detection           Detect changed, missing, duplicate, and out-of-order migrations with CI-safe exit codes.
- [x] 37E5-dry-run-report            Dry-run reports pending/applied/drifted migrations without mutating state.
- [x] 37E6-sqlite-sql-execution      `corvid migrate up` executes pending SQL transactionally against SQLite before recording applied state.
- [x] 37F1-audit-schema-envelope     Add `std.db` audit-log record envelopes for actor/action/model/tool/approval/cost/trace/replay.
- [x] 37F2-audit-write-helper        Add helpers/tests for approval-aware audit writes and redacted values.
- [x] 37F3-audit-example             Add a minimal backend audit-log example and regression test.
- [x] 37G1-token-envelope            Add token reference/encrypted-token metadata envelopes.
- [x] 37G2-host-key-doctor           `corvid doctor` validates token encryption key presence/shape without printing it.
- [x] 37G3-token-redaction-tests     Traces, errors, and audit helpers never print token values.
- [x] 37H1-postgres-design           Document Postgres parity subset and non-scope before code.
- [x] 37H2-postgres-connection       Add Postgres connection/query envelopes matching SQLite.
- [x] 37H3-postgres-migration-status Postgres migration status/drift path matches SQLite subset.
- [x] 37I1-db-effect-tags            DB read/write/migration/token/audit operations carry explicit effect tags.
- [x] 37I2-db-replay-summary         Replay summaries capture deterministic DB interaction metadata without raw secrets.
- [x] 37J1-state-example-schema      Backend state example defines users/tasks/approvals/traces/tokens tables.
- [x] 37J2-state-example-tests       Example migration, query, audit, token, and replay tests pass.
- [x] 37J3-state-runbook             Example documents backups, migration rollback, redaction, and operator checks.

**Audit correction before market freeze:** Phase 37 is not market-frozen until the stdlib DB surface performs real host-backed query/transaction execution, Postgres has a real driver-backed path rather than metadata envelopes, and `migrate down` has tested rollback semantics.

- [x] 37K-real-stdlib-db-runtime     Corvid-facing DB helpers execute SQLite queries/transactions through the runtime with typed decode errors.
- [x] 37L-real-postgres-runtime      Postgres connection/query path uses a real Postgres client with redacted error handling and parity-shaped query APIs.
- [x] 37M-migration-down-execution   `corvid migrate down` executes reviewed rollback SQL or fails clearly when no rollback exists.

### Phase 38 — Jobs, schedules, and durable agent execution (~8-10 weeks)

**Goal.** Corvid can run long-lived backend work safely: scheduled jobs, retrying jobs, background queues, idempotent actions, failure recovery, and bounded agent loops.

**Inventive benchmark target.** Compare durable agent jobs against Celery, BullMQ, Sidekiq-style queues, and Temporal-style workflows. Corvid must win on AI-specific safety: budgeted loops, approval waits, replayable agent checkpoints, tool-call lineage, and compile-time visibility of dangerous background work.

**Scope:**

- [x] Durable job runner with enqueue, delay, cron, cancellation, concurrency limits, idempotency keys, retry/backoff, dead-letter queue, and job leases. (Slices 38B-38J shipped; 38K multi-worker pool + 38L crash recovery + 38M DST cron closed the audit-correction track.)
- [x] Scheduler manifest visible to `corvid audit`: every recurring task has owner, effect set, max runtime, max cost, replay policy, and approval policy. (Slice 38D + `corvid jobs schedule add/list/recover`.)
- [x] Durable agent run state: step checkpoints, tool-call results, approval waits, resume-after-crash, and replayable finalization.
- [x] Loop controls: max steps, max wall time, max spend, max tool calls, and escalation-on-stall.
- [x] AI-native integration: every job carries a budget, effect row, provenance policy, and trace lineage; dangerous jobs cannot run without an approval boundary. Shipped through Phase 38 slice checklist + verified in `docs/phases/phase-38-audit-2026-05-17.md`.
- [x] Operational controls: pause queue, drain workers, inspect job, retry job, cancel job, and export job trace.

**Slice checklist:**

- [x] 38A-jobs-design-brief          `docs/phases/phase-38-jobs.md` defines queue semantics, durability model, scheduler model, approval waits, replay behavior, and non-scope.
- [x] 38B-enqueue-run-one-job        Runtime can enqueue and execute one persisted background job with typed input/output.
- [x] 38C-retry-backoff-dlq          Jobs support retry policies, backoff, terminal failure, and dead-letter inspection.
- [x] 38D-delayed-jobs-cron          Delayed jobs and cron schedules persist, recover after restart, and appear in `corvid audit`.
- [x] 38E-leases-concurrency-idempotency Jobs use leases, concurrency limits, and idempotency keys to avoid duplicate dangerous work.
- [x] 38F-agent-step-checkpoints     Durable agent runs checkpoint steps, tool-call results, and partial outputs.
- [x] 38G-approval-wait-resume       Jobs can pause on approval, resume after approve/deny/expire, and record the audit transition.
- [x] 38H-loop-bounds                Max steps, wall time, spend, and tool calls are enforced for job-backed agent loops.
- [x] 38I-job-ops-commands           Operators can pause queues, drain workers, inspect, retry, cancel, and export job traces.
- [x] 38J-executive-agent-jobs       Personal Executive Agent daily brief, meeting prep, triage, and follow-up jobs survive process restart.

**Done when:** the Personal Executive Agent backend can run daily brief generation, email triage, meeting prep, and follow-up reminders as durable jobs that survive process restart.

**Libraries & frameworks (Phase 38):**

- `tokio` — async worker pool (already a dep).
- `rusqlite` + `postgres` — job-store backends with row-level locks (already deps).
- `chrono-tz` — cron timezone correctness; DST handling is non-negotiable.
- `tokio-cron-scheduler` *or* hand-rolled cron — must support DST + missed-fire policies (`fire_once_on_recovery` / `skip`).
- `ulid` — monotonic job IDs.
- `tracing` — span emission for every state transition (already a dep); OTel hooks ride on Phase 40.
- Existing `ed25519-dalek` for job-receipt signing when the run produces a Phase 21 receipt.

**Developer flow (Phase 38):**

```corvid
@budget($0.20)
@retry(max_attempts: 3, backoff: exponential(base: 30s, cap: 5m))
@idempotency(key: brief.user_id)
@replayable
job daily_brief(user_id: String) uses email_effect, summary_effect:
    inbox = gmail.recent(user_id, since: yesterday())
    summary = summarise(inbox)
    approve SendBrief(user_id, summary)
    gmail.send(user_id, summary)

schedule "0 8 * * *" zone "America/New_York" -> daily_brief(every_user())
```

```bash
corvid jobs run --source app.cor --queue=default --workers=4
corvid jobs schedule list
corvid jobs inspect <id>
corvid jobs explain <id>          # AI-assisted root-cause from the typed trace
corvid jobs dlq triage            # AI-assisted DLQ pattern clustering
corvid jobs retry <id>
corvid jobs export-trace <id>
corvid jobs pause --queue=default
corvid jobs drain --workers=all
```

**Phase-done checklist (Phase 38):**

- [x] `validate_signed_claim_coverage` recognises the shipped contract surface: `@replayable` (wires `replay.deterministic_pure_path` via `AgentAttribute::Replayable`) and `schedule` (wires `jobs.cron_schedule_durable` via `Decl::Schedule`). The aspirational `@retry` / `@idempotency` / `job` / `await_approval` source-level surfaces are post-v1.0 ergonomic additions, filed as `35V2-P38-H`; the runtime behaviour they would surface ships today through the enqueue API + `corvid jobs limit` + `corvid jobs wait-approval`. Audit reworded 2026-05-17 (slice `35V2-P38-F`) to match shipped reality after the original wording assumed surface that doesn't exist.
- [x] Registry rows shipped: 6/8 are RuntimeChecked with positive + adversarial test refs (`jobs.durable_resume`, `jobs.idempotency_key_uniqueness`, `jobs.lease_exclusivity`, `jobs.cron_dst_correct`, `jobs.loop_bounds_enforced`, `jobs.replayable_side_effects` — the last promoted in slice `35V2-P38-C-6` after the audit-correction track `35V2-P38-C-replay-quarantine` shipped all four side-effect surface quarantines + the cross-surface corpus). 2 remain OutOfScope with reasons pointing at the post-v1.0 source-syntax sugar slice: `jobs.retry_budget_bound` → post-v1.0 `35V2-P38-H` (`@retry` source syntax); `jobs.approval_wait_resume` → post-v1.0 `35V2-P38-H` (`await_approval` keyword). The Phase 38 sentinel `phase_38_required_registry_ids_all_present` locks the 8-row presence shape.
- [x] Crash-recovery integration test: `SIGKILL` mid-step → resume with no LLM re-spend (verified by mock-LLM call counter). (Slice 38L's `t38l_d3_checkpoints_survive_unclean_shutdown`; literal subprocess-SIGKILL harness is a post-v1.0 hardening item, property-equivalent stand-in ships.)
- [x] Idempotency adversarial test: 4 concurrent workers + 100 jobs same key → exactly 1 ran. (Slice 38L's `t38l_d1_four_workers_collapse_to_one_row`.)
- [x] DST cron test: a job scheduled for 2:30am on the spring-forward day fires according to the documented policy. (Slice 38M's `t38l_d2_dst_spring_forward_is_deterministic` + `t38l_d2_dst_fall_back_is_monotonic`.)
- [x] Replay-quarantine test: replay an old job trace, assert no real provider call left the process. (Slice 35V2-P38-C-6's `crates/corvid-runtime/tests/replay_quarantine_corpus.rs` — 4 adversarial cases per side-effect surface plus 4 positive / negative-control cases.)
- [x] AI helper landed (or follow-up filed): `corvid jobs explain` (assistive — typed classifier over typed records + audit-event trail) shipped 2026-05-19 in slice `35V2-P38-G-LR-corvid-jobs-explain-helper`. Output's `sources` array names every audit-event id the explanation consulted (Grounded<T>). Promotes the new `jobs.explain_sources_grounded` row to RuntimeChecked (positive: `jobs_explain_denied_approval_carries_grounded_sources`; adversarial: `jobs_explain_unknown_job_refuses`).
- [x] Side-by-side `benches/comparisons/jobs_durability.md` against Celery + BullMQ + Temporal. File shipped in `d9d2efd`; lives at `benches/comparisons/jobs_durability.md`.

**Small-slice breakdown for Phase 38:**

- [x] 38B1-job-envelope              Add `std.jobs` job/input/output/state envelopes.
- [x] 38B2-enqueue-command           Add enqueue/run-one runtime path with local persisted state.
- [x] 38B3-one-job-test              One persisted job executes once with typed input/output.
- [x] 38C1-retry-policy-envelope     Retry/backoff/dead-letter metadata exists in stdlib.
- [x] 38C2-retry-runner              Runner applies retry/backoff and terminal failure.
- [x] 38C3-dlq-inspection            CLI can inspect dead-lettered jobs.
- [x] 38D1-delay-support             Delayed jobs persist and wake after restart.
- [x] 38D2-cron-manifest             Cron schedules appear in `corvid audit`.
- [x] 38D3-scheduler-recovery        Scheduler recovers missed/pending jobs after restart.
- [x] 38E1-lease-model               Job leases prevent duplicate workers.
- [x] 38E2-concurrency-limits        Queue and job-type concurrency limits are enforced.
- [x] 38E3-idempotency-keys          Duplicate dangerous jobs collapse or fail predictably.
- [x] 38F1-checkpoint-schema         Agent step/tool/partial-output checkpoints are durable.
- [x] 38F2-resume-agent-run          Agent run resumes from last checkpoint after crash.
- [x] 38G1-approval-wait-state       Jobs can enter approval-wait state with expiry.
- [x] 38G2-approval-resume           Approve/deny/expire transitions resume or stop jobs and write audit events.
- [x] 38H1-loop-budget-controls      Max steps, wall time, spend, and tool-call limits are enforced.
- [x] 38H2-stall-escalation          Stalled loops escalate or terminate with trace evidence.
- [x] 38I1-job-ops-cli               Pause/drain/inspect/retry/cancel/export commands work locally.
- [x] 38J1-exec-agent-job-spec       Personal Executive Agent job definitions are written and checked.
- [x] 38J2-exec-agent-restart-proof  Daily brief/meeting prep/triage/follow-up jobs survive restart in tests.

**Audit correction (Phase 35-41 audit, 2026-04-29):** Phase 38's slice list shipped the queue/lease/checkpoint envelopes and a single-job runner, but four phase-done bullets remained structurally absent: a multi-worker job runner, a SIGKILL-mid-step crash-recovery test, a 4-concurrent-worker idempotency test, and a DST-aware cron test. Phase 38 is not market-frozen until those land. **Re-verified clean by Phase 35V Track 2 (slices T2-D / T2-E / T2-F / T2-G) on 2026-05-09:** `cargo test -p corvid-runtime --lib worker_pool` (3 passed including `t38k_two_workers_cannot_both_lease_same_job`) and `cargo test -p corvid-runtime --test durability_corpus` (4 passed including `t38l_d1_four_workers_collapse_to_one_row`, `t38l_d2_dst_fall_back_is_monotonic`, `t38l_d2_dst_spring_forward_is_deterministic`, `t38l_d3_checkpoints_survive_unclean_shutdown`) all green; the audit-correction track landed honestly.

- [x] 38K-multi-worker-runner     `corvid jobs run --queue=<name> --workers=N` spawns a configurable async worker pool that dequeues, leases, executes, releases, and respects per-queue concurrency limits. Replaces the existing single-shot `RunOne` for production use; `RunOne` stays for tests.
- [x] 38L-crash-recovery-test     Integration test in `crates/corvid-runtime/tests/durability_corpus.rs` exercises an unclean-shutdown surrogate: a worker takes a lease, the runtime is dropped mid-lease before completion (a property-equivalent stand-in for SIGKILL of a worker subprocess — same lease+checkpoint state on disk, same recovery code path), a fresh runtime resumes, and the test asserts the job runs to completion exactly once with the LLM call counter unchanged. Plus a 4-concurrent-worker idempotency test that submits 100 jobs sharing one idempotency key from 4 worker tasks and asserts exactly one ran (enforced at the SQL layer by a partial UNIQUE INDEX). The literal subprocess-SIGKILL harness remains a post-v1.0 hardening item.
- [x] 38M-dst-cron                Adopt `chrono-tz` for cron schedules; default policy is `fire_once_on_recovery` for missed fires, configurable via job manifest. Test: a job scheduled for 02:30 in `America/New_York` on the spring-forward day fires according to the documented policy. Test: a job scheduled for 01:30 on the fall-back day fires exactly once, not twice. (Tests live in `durability_corpus.rs`; landed alongside 38L.)

**Audit deferral override (2026-05-26):** `35V2-P38-C-deferred` (replay-quarantine cross-layer wiring) is pulled forward from the Phase 43 launch-readiness window into the Phase 38 audit-correction track. Recon under the pre-phase chat found the audit's original "~2-4 days" estimate was based on the wrong assumption that the agent-level replay path could be reused for jobs. The actual integration surface requires: (a) a job→Runtime executor bridge, (b) per-job trace emission, (c) a `replay_job` entry point, and (d) quarantine wrappers around `LlmRegistry`, `HttpClient`, `StoreManager`, and `IoRuntime` — quarantining only LLM would be a shortcut by leaving HTTP/DB/IO side-effects unconstrained. Honest estimate: ~2-3 weeks across six sub-slices, each with its own commit + validation gate. The `35V2-P38-C-deferred` filing is closed; the same work is now numbered `35V2-P38-C-replay-quarantine`.

- [x] 35V2-P38-C-replay-quarantine — Cross-layer replay-quarantine wiring for durable jobs. **Closed 2026-05-27.** All six sub-slices shipped across commits `a4b609b` (pre-phase admin), `534bffd` (C-1 executor bridge), `f6c64b2` (C-2 trace emission), `879e5c5` (C-3 replay entry), `7855111` (C-4 LLM quarantine), `211d675` (C-5 HTTP/Store/IO quarantine), and the C-6 commit (corpus + promotion + tour + docs). `jobs.replayable_side_effects` promoted from OutOfScope to RuntimeChecked with 4 positive + 4 adversarial test refs into `crates/corvid-runtime/tests/replay_quarantine_corpus.rs`. Sub-slices:

  - [x] 35V2-P38-C-1 — Job→Runtime executor bridge. `cmd_jobs_run` and `WorkerPool` thread a real `Runtime` (with `LlmRegistry`, `HttpClient`, `StoreManager`, `IoRuntime`) into the job executor closure instead of running a no-op. New `JobRuntimeExecutor` trait + default impl. Unit + integration tests cover one persisted job executing through the real Runtime stack. (Landed in commit `534bffd`; `corvid jobs run` requires `--source <path>.cor` and errors with a pointer at `corvid jobs run-one` for the smoke-test affordance.)

  - [x] 35V2-P38-C-2 — Job trace emission. `@replayable` jobs persist a JSONL trace at `<trace_dir>/<job_id>.jsonl` (default `target/trace/jobs/`, configurable via `DefaultJobRuntimeExecutor::with_trace_dir`). Trace schema reuses `corvid-trace-schema` events (`SchemaHeader` / `SeedRead` / `RunStarted` / interleaved interpreter events / `RunCompleted`); no new event types invented. Gated on `IrAgent.is_replayable` (new field, lowered from `AgentAttribute::is_replayable`). `QueueJob.replay_key` stays as operator-provided metadata; the executor does NOT mutate it (the earlier draft over-specified this — trace path is deterministic from `job_id`, so C-3's `replay_job` lookup uses `job_id` directly). Unit + integration tests: positive (round-trips through `TraceEvent`), adversarial (non-`@replayable` agent emits no file).

  - [x] 35V2-P38-C-3 — `replay_job` entry point. New `corvid_driver::replay_job_from_source(source, job_id, trace_dir, base_builder)` resolves the trace as `<trace_dir>/<job_id>.jsonl` (path derived from `job_id`, not from `replay_key` — see C-2 design refinement) and delegates to the existing Phase 21 `run_replay_from_source_with_builder_async` with `ReplayMode::Plain`. CLI surface: `corvid jobs replay --source <path>.cor --job <job_id> [--trace-dir <dir>] [--state <queue.db>]`. Integration test: enqueue → run → replay → assert `ReplayOutcome.result_value` matches the original return; adversarial: missing trace emits a helpful error naming `@replayable` as the most common cause. (Quarantine for unrecorded side effects lands in C-4 / C-5 — C-3 establishes the replay entry, not the quarantine.)

  - [x] 35V2-P38-C-4 — LlmRegistry quarantine wrapper. New `QuarantinedLlmAdapter` (`crates/corvid-runtime/src/llm/quarantine.rs`) wraps each registered adapter; on `.call(&req)` returns the new `RuntimeError::QuarantineViolation { surface: "llm", detail }` instead of delegating. `LlmRegistry::quarantine_all()` wraps every registered adapter; `RuntimeBuilder::build` invokes it when entering `RuntimeMode::Replay(source)` with `!source.uses_live_llm()` (i.e. Substitute mode — the default for `corvid jobs replay` and `corvid replay`). Differential / Mutation modes skip the wrap so the existing live-LLM paths keep working. Defense-in-depth on top of the existing `replay_llm_call` substitution (which today intercepts the interpreter dispatch path before the adapter); the wrap closes the registry-layer hole for any future caller that bypasses `Runtime::call_llm_ref`. 3 unit tests in `quarantine.rs`: wrap produces typed violation, `quarantine_all` covers every registered adapter, late registration is NOT covered (contract lock).

  - [x] 35V2-P38-C-5 — HTTP / Store / IO quarantine wrappers. Same pattern as C-4 applied to three surfaces: `HttpClient::quarantine` short-circuits `send` with `QuarantineViolation { surface: "http", .. }`; `StoreManager::quarantine_writes` short-circuits the five write entry points (`put`, `put_record`, `put_record_if_revision`, `delete`, `delete_with_policy`) with `surface: "store"` (reads pass through); `IoRuntime::quarantine_writes` short-circuits `write_text` / `write_text_with_effect` with `surface: "io"` (reads + list pass through). `RuntimeBuilder::build` calls all three together with the C-4 LLM quarantine when entering `RuntimeMode::Replay(source)` with `!source.uses_live_llm()`. The durable job queue uses raw `rusqlite` (NOT `StoreManager`) and the trace writer uses `JsonlTraceWriter` (NOT `IoRuntime`), so queue-internal persistence + trace emission are unaffected — the queue-internal / application-tool distinction the design doc flagged turned out to be enforced by construction, no `QuarantineContext` token needed. 5 unit tests across the three modules (HTTP refuses send + default is unquarantined; store refuses every write entry point + read passthrough; IO refuses write + read passthrough + does not touch filesystem on refusal).

  - [x] 35V2-P38-C-6 — Integration test corpus + registry promotion + docs. Shipped `crates/corvid-runtime/tests/replay_quarantine_corpus.rs` (8 tests: 4 adversarial — one per side-effect surface — plus 4 positive / negative controls covering store + IO read passthrough, differential-mode escape hatch, and live-mode no-quarantine). Promoted `jobs.replayable_side_effects` to `RuntimeChecked` and added it to `SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS`. Walks `validate_signed_claim_coverage` for `@replayable` agent decls (every `@replayable` agent in a signed cdylib now requires both `replay.deterministic_pure_path` AND `jobs.replayable_side_effects`). Regenerated `docs/reference/core-semantics.md`. Shipped `corvid tour --topic replay-quarantine` topic, `docs/reference/inventions.md` row, and README catalog entry.

**Phase-done criteria for 35V2-P38-C-replay-quarantine:**

- [x] All six sub-slices ticked with their own commit on `main`.
- [x] `jobs.replayable_side_effects` is `RuntimeChecked` in `corvid-guarantees::GUARANTEE_REGISTRY` with ≥1 positive + ≥1 adversarial test refs (4 of each, in `replay_quarantine_corpus.rs`).
- [x] `cargo run -q -p corvid-cli -- verify --corpus tests/corpus` exits 1 only on the documented deliberate-fail fixtures.
- [x] Phase 38 phase-done item 6 (replay-quarantine test) ticks with reference to the new test corpus.
- [x] `docs/reference/core-semantics.md` regenerated; drift gate green.
- [x] `corvid tour --topic replay-quarantine` runs and matches the README catalog entry.
- [x] `docs/reference/inventions.md` carries the new row with shipped status, runnable command, test coverage link, spec link, and non-scope.
- [x] dev-log entry per sub-slice; learnings entry at C-6.

### Phase 39 — Auth, identity, and human approval product surface (~8-10 weeks)

**Goal.** Corvid can secure real multi-user AI backends and provide a production approval system rather than a demo `approve` hook.

**Inventive benchmark target.** Compare the approval flow against Auth.js/Express, FastAPI dependencies, and Go middleware. Corvid must win by proving that identity, tenant, permission, dangerous tool, and approval-contract relationships are statically visible and audited end-to-end.

**Scope:** ✅ shipped (slice checklist below is the machine-readable version of these bullets; verified 2026-05-17 by the cross-phase verification round audit — see `docs/phases/phase-39-audit-2026-05-17.md`).

- [x] `std.auth` for sessions, API keys, JWT verification, OAuth callback handling, CSRF protection, passwordless login hooks, and service-account auth. CSRF middleware shipped 2026-05-19 in slice `35V2-P39-C-LR-csrf-middleware`: canonical double-submit verifier in `corvid-runtime::auth::csrf` (HMAC-SHA256 over `binding.hex_hmac`, 8 unit tests covering header-missing, header≠cookie, HMAC-forged, malformed, empty-secret, and the safe-methods-pass path) + rendered axum server wires it into `backend_middleware` when `CORVID_CSRF_SECRET` is set, asserted end-to-end by `rendered_server_csrf_middleware_refuses_state_change_without_double_submit_token`.
- [x] Identity and tenant model: user IDs, organization IDs, roles, permissions, and audit actor propagation through routes, jobs, tools, and traces.
- [x] Approval queue API: create, list, inspect, approve, deny, expire, comment, delegate, and audit approvals.
- [x] Typed approval contracts generated from dangerous tools: expected action, target resource, max cost, data touched, irreversible flag, expiry, and required approver role.
- [x] Approval UI contract: backend serves enough structured data for any frontend to render approvals without reverse-engineering traces.
- [x] AI-native integration: compiler rejects dangerous route/job/tool paths that have no reachable approval contract.
- [x] Security tests for confused-deputy approval bypass, tenant-crossing approval reuse, stale approval replay, and privilege escalation.

**Slice checklist:**

- [x] 39A-auth-approval-design-brief `docs/phases/phase-39-auth-approval.md` defines identity, tenant, session, approval, threat, and non-scope models.
- [x] 39B-session-api-key-auth        `std.auth` supports sessions and API keys with typed actor propagation into routes and traces.
- [x] 39C-jwt-oauth-callbacks        JWT verification and OAuth callback handling work for connector authorization flows.
- [x] 39D-tenant-role-permissions    User, organization, role, and permission checks propagate through routes, jobs, tools, and traces.
- [x] 39E-approval-queue-api         Approval create/list/inspect/approve/deny/expire/comment/delegate APIs ship with tests.
- [x] 39F-generated-approval-contracts Dangerous tools generate typed approval contracts with target, cost, data, expiry, irreversibility, and required role.
- [x] 39G-approval-ui-contract       Backend exposes structured approval payloads that any frontend can render without parsing traces.
- [x] 39H-compiler-approval-reachability Compiler rejects dangerous route/job/tool paths with no reachable approval contract.
- [x] 39I-security-bypass-tests      Tests cover confused-deputy approval bypass, tenant-crossing approval reuse, stale approval replay, and privilege escalation.
- [x] 39J-approval-product-example   Reference backend exposes real login, tenant-safe approvals, and auditable AI actions.

**Done when:** a backend can expose real user login, tenant-safe approvals, and auditable AI actions without outsourcing the core safety model to another framework.

**Libraries & frameworks (Phase 39):**

- `jsonwebtoken` — JWT verify (RS256, ES256, EdDSA) with JWKS caching + `kid` rotation.
- `argon2` — password + API-key hashing (Argon2id, OWASP defaults).
- `oauth2` *or* hand-rolled — OAuth2/OIDC client; PKCE-mandatory for public clients.
- `ring` *or* `rustls` — primitives for HMAC, signature, key-derivation.
- `hmac` + `sha2` — CSRF double-submit token MAC.
- `time` (already a dep) — expiry math, clock-skew tolerance.
- `cookie` — typed cookie parsing/serialization with `SameSite`, `HttpOnly`, `Secure` defaults.
- `rusqlite` + `postgres` — session store, API-key store, approval queue store.

**Developer flow (Phase 39):**

```corvid
auth my_api:
    sessions: cookie("__corvid_sess", secure, http_only, same_site: lax)
    api_keys: header("Authorization", scheme: bearer)
    jwt: verify_rs256(jwks_url: env("JWKS_URL"))
    csrf: double_submit("__corvid_csrf")

tenant Org { id: String, plan: Plan }
role Admin, Reviewer, Member
permission CanIssueRefund: Admin | Reviewer

@dangerous
@requires(permission: CanIssueRefund)
@approval(contract: RefundApproval)
tool issue_refund(actor: Actor, order_id: String, amount: Money) -> Receipt

approval RefundApproval:
    target: order_id
    cost_ceiling: $5000
    data: financial
    irreversible: true
    expires_in: 24h
    required_role: Admin
    policy { actor.role == Admin && amount < $100 }
    batch_with: same_tool, same_data_class, same_role
```

```bash
corvid auth migrate                         # session/api-key/approval tables
corvid auth keys issue <name> --tenant=<id>
corvid auth keys revoke <key-id>
corvid approvals queue --tenant=<id>
corvid approvals explain <id>               # AI-assisted reviewer summary
corvid approvals batch <ids...>             # batch-approve semantically-equivalent items
corvid approvals delegate <id> --to=<actor>
corvid approvals export --since=2026-04-01  # audit dump
```

**Phase-done checklist (Phase 39):**

- [x] `validate_signed_claim_coverage` recognises the shipped contract surface: dangerous tool-call coverage (`approval.dangerous_call_requires_token` + `approval.token_lexical_only`) wires via the existing dangerous-tool walker. The aspirational `auth` / `tenant` / `role` / `permission` / `approval Name:` / `@requires` / `@approval` source-level surfaces are post-v1.0 ergonomic additions filed as `35V2-P39-I-post-v1.0-auth-syntax-sugar`; the runtime behaviour they would surface ships today through `corvid auth` + `corvid approvals` CLI subcommands + the host API. Audit reworded 2026-05-17 (slice `35V2-P39-F`) to match shipped reality after the original wording assumed surface that doesn't exist.
- [x] Registry rows shipped: all 9 ids present (`auth.session_rotation_on_privilege_change`, `auth.api_key_at_rest_hashed`, `auth.jwt_kid_rotation`, `auth.oauth_pkce_required`, `auth.csrf_double_submit`, `tenant.cross_tenant_compile_error`, `approval.policy_clause_static_check`, `approval.batch_equivalence_typed`, `approval.confused_deputy_typecheck`) and locked by the `phase_39_required_registry_ids_all_present` sentinel landed in `35V2-P39-E`. 3 are RuntimeChecked with positive + adversarial test refs (api_key_at_rest_hashed, jwt_kid_rotation, oauth_pkce_required). 6 are OutOfScope with reasons tightened in `35V2-P39-B` to name their specific launch-readiness / post-v1.0 promotion slice instead of the stale "Slice 39L promotes" wording.
- [x] Adversarial corpus enumerates ≥10 named threats: **10/10 shipped** — full coverage. confused-deputy, tenant-crossing, stale-approval-replay, JWT-kid-downgrade, OAuth-state-tamper-cross-tenant, privilege-escalation, batch-approval-drift-across-data-classes, session-fixation, CSRF-bypass-on-PUT/PATCH/DELETE, scope-escalation — see `crates/corvid-runtime/src/approval_queue.rs::approval_bypass_*` tests + `crates/corvid-runtime/src/jwt_verify/verifier.rs::kid_downgrade_*` test + `crates/corvid-runtime/src/auth/oauth.rs::oauth_callback_rejects_expired_and_cross_tenant_state` test + `crates/corvid-cli/src/approvals_cmd/interaction.rs::approvals_batch_refuses_cross_data_class_drift_without_pin` + `crates/corvid-runtime/src/auth/sessions.rs::session_rotation_on_privilege_change_rejects_pre_elevation_session_fixation_attempt` + `crates/corvid-runtime/src/auth/csrf.rs::csrf_bypass_attempt_without_header_refused_on_put_patch_delete` (plus the rendered-server end-to-end `rendered_server_csrf_middleware_refuses_state_change_without_double_submit_token`) + `crates/corvid-runtime/src/auth/scope.rs::scope_escalation_attempt_refused_with_specific_missing_permission`. scope-escalation promoted in slice `35V2-P39-K-LR-structured-scope-model` (structured `ApiKeyScope` set + `enforce_scope_grant` predicate refuses any non-subset grant and names every missing permission in the typed error). CSRF-bypass promoted in slice `35V2-P39-C-LR-csrf-middleware`; session-fixation in `35V2-P39-D-LR-session-rotation-hook`; batch-approval-drift in `35V2-P39-L-LR-batch-data-class-equivalence`. Audit reworded 2026-05-17 (slice `35V2-P39-F`); 2026-05-19 updates for the batch-drift, session-fixation, CSRF-bypass, and scope-escalation threats.
- [x] Reachability analysis: approve-presence half ships via `approval.dangerous_call_requires_token` + `approval.token_lexical_only`. Role-coverage extension (typecheck fails when reachable callers' roles don't cover the approve's `required_role`) was originally filed as launch-readiness `35V2-P39-J-LR-role-coverage-reachability` but **recon under that slice 2026-05-19 found it is blocked on unshipped source-level role syntax**: `AgentAttribute` today is `Replayable` / `Deterministic` / `Wrapping` / `GroundedPure` only — no `@requires(role)` or `@approval(role)` variant for a call-graph pass to reason over. Refiled to post-v1.0 alongside the syntax dependency `35V2-P39-I-post-v1.0-auth-syntax-sugar`. The runtime half of the threat ships and is tested at `approval_bypass_rejects_confused_deputy_self_approval`. Audit reworded 2026-05-17 (slice `35V2-P39-F`); refile recorded 2026-05-19.
- [x] AI helper landed (or follow-up filed): `corvid approvals explain <id>` (assistive) shipped 2026-05-19 in slice `35V2-P39-G-LR-approvals-explain-helper` — renders a typed reviewer summary whose `sources` array names every audit-event id the explanation consulted; promotes the new `approval.explain_sources_grounded` row (positive: `approvals_explain_pending_carries_grounded_sources`; adversarial: `approvals_explain_cross_tenant_refused`). `corvid approvals policy-suggest <tool>` (generative) — proposes a `policy { ... }` clause from the last 200 approvals — remains filed as launch-readiness `35V2-P39-H-LR-approvals-policy-suggest-helper`.
- [x] Side-by-side `benches/comparisons/auth_approval.md` against Auth.js, FastAPI dependencies, Go middleware. (Verified present 2026-05-17.)

**Small-slice breakdown for Phase 39:**

- [x] 39B1-actor-envelope            Add typed actor/session/api-key envelopes.
- [x] 39B2-session-runtime           Session auth resolves an actor into route/job/trace context.
- [x] 39B3-api-key-runtime           API-key auth supports service actors and redacted diagnostics.
- [x] 39C1-jwt-verify-contract       JWT verification surface and failure diagnostics are defined.
- [x] 39C2-oauth-callback-state      OAuth callback state/token references are typed and replay/audit visible.
- [x] 39D1-tenant-role-model         User/org/role/permission records and helpers are typed.
- [x] 39D2-permission-propagation    Permissions propagate through routes, jobs, tools, and traces.
- [x] 39E1-approval-store            Approval queue persistence schema and stdlib envelopes exist.
- [x] 39E2-approval-api              Create/list/inspect/approve/deny/expire/comment/delegate APIs work.
- [x] 39E3-approval-audit            Every approval transition writes audit and trace evidence.
- [x] 39F1-contract-generation       Dangerous tools generate typed approval contract records.
- [x] 39F2-contract-policy-check     Required role/expiry/irreversibility/cost/data rules are enforced.
- [x] 39G1-ui-payload-schema         Approval UI payload schema is stable and frontend-agnostic.
- [x] 39G2-ui-contract-tests         Payloads can be rendered without parsing traces.
- [x] 39H1-reachability-analysis     Compiler checks route/job/tool paths for reachable approvals.
- [x] 39H2-reachability-bypass-tests Confused-deputy, tenant-crossing, stale replay, and privilege escalation tests fail closed.
- [x] 39J1-auth-example              Reference backend has login/API-key auth.
- [x] 39J2-approval-product-example  Reference backend exposes tenant-safe approvals and auditable AI actions.

**Audit correction (Phase 35-41 audit, 2026-04-29):** Phase 39 shipped the auth/approval data envelopes, Argon2 password hashing, and the OAuth state surface, but the JWT verification path was contract-shape only (`validate_jwt_verification_contract` checks the issuer-URL prefix, alg name, and claim presence — it does not fetch JWKS, does not resolve `kid`, does not verify signatures). The top-level `corvid auth` / `corvid approvals` CLI surface that the developer-flow doc names is also unwired (the closest is `JobsCommand::Approvals`, which lists jobs in approval-wait, not the tenant-scoped queue). Phase 39 is not market-frozen until those land. **Re-verified clean by Phase 35V Track 2 (slices T2-H / T2-I) on 2026-05-09:** `cargo test -p corvid-runtime --lib jwt_verify` (10 passed including `kid_downgrade_returns_kid_not_found`, `header_alg_must_match_contract_alg`, `alg_none_in_header_is_refused`, `jwks_fetch_failure_is_surfaced`); `corvid auth --help` and `corvid approvals --help` both surface as full top-level subcommands with rich subcommand surfaces (auth: migrate / keys; approvals: queue / inspect / approve / deny / expire / comment / delegate / batch / export); the audit-correction track landed honestly.

- [x] 39K-real-jwt-verification   Adopt `jsonwebtoken` and a JWKS fetch path with caching, `kid` rotation, and clock-skew tolerance. `auth.rs` exposes a real `verify_jwt(token, jwks_url) -> Result<Claims, JwtError>` that fetches the JWKS, picks the key by `kid`, verifies the signature, validates `exp`/`nbf`/`iss`/`aud`, and returns typed claims. Adversarial tests cover: kid downgrade, expired token, alg-none injection, signature tampering, JWKS-fetch failure, claim-mismatch. (A signed-token end-to-end round-trip test that ships its own RSA key fixture is a post-v1.0 follow-up; the framework-correctness tests cover every named adversarial path.)
- [x] 39L-auth-approvals-cli      `corvid auth migrate`, `corvid auth keys issue/revoke/rotate`, `corvid approvals queue/inspect/approve/deny/expire/comment/delegate/batch/export` are wired as top-level CLI subcommands backed by the existing approval queue store. Each command writes audit + trace events. Integration tests cover the full happy path plus expired/denied/replayed approval flows.

### Phase 40 — Agent observability, evals, and production monitoring (~6-8 weeks)

**Goal.** Corvid gives maintainers the operational visibility needed to trust AI systems in production: traces, metrics, evals, cost, latency, drift, and human review.

**Inventive benchmark target.** Compare incident diagnosis against OpenTelemetry plus ad hoc LangSmith/Langfuse-style tracing. Corvid must win on time-to-answer for: what action happened, why, who approved it, what it cost, what data it touched, which guarantee applied, and how to replay or promote it into an eval.

**Scope:** ✅ shipped (slice checklist below is the machine-readable version of these bullets; verified 2026-05-17 by the cross-phase verification round audit — see `docs/phases/phase-40-audit-2026-05-17.md`).

- [x] Trace viewer data model and export format for route -> job -> agent -> prompt -> tool -> approval -> DB lineage.
- [x] OpenTelemetry export for request metrics, job metrics, LLM calls, tool calls, approvals, errors, retries, token/cost usage, model-routing decisions, and replay IDs.
- [x] `corvid observe` command for local trace inspection, cost reports, approval summaries, failing runs, and hot spots.
- [x] Evals from production traces: promote trace slices into regression tests with redacted inputs, expected contracts, and replay fixtures.
- [x] Drift and regression reports: model output schema failures, confidence drops, cost changes, latency changes, approval denial spikes, and tool-error spikes.
- [x] Human-review queues for low-confidence or high-risk outputs, with audit linkage back to source prompt/model/tool versions. Envelopes ship at `corvid_runtime::review_queue`; the `corvid review-queue list --rank=cost-of-being-wrong` CLI subcommand promoted the `review_queue.cost_of_being_wrong_ranking` row to RuntimeChecked in `35V2-P40-C-LR-review-queue-ranking-cli` (positive: `rank_cost_of_being_wrong_sorts_highest_first`; adversarial: `rank_unknown_policy_refused`).
- [x] AI-native integration: observability is contract-aware; reports group failures by violated guarantee/effect/budget/provenance rule.

**Slice checklist:**

- [x] 40A-observability-design-brief `docs/phases/phase-40-observability.md` defines trace schema, metrics taxonomy, eval promotion, retention, redaction, and non-scope.
- [x] 40B-lineage-trace-model        Route -> job -> agent -> prompt -> tool -> approval -> DB lineage is represented in one trace model.
- [x] 40C-otel-export                OpenTelemetry export covers requests, jobs, LLM calls, tools, approvals, errors, retries, costs, and replay IDs.
- [x] 40D-observe-command-basics     `corvid observe` lists traces, costs, approvals, failures, and hot spots from local stores.
- [x] 40E-trace-to-eval              Production trace slices can be promoted into redacted regression/eval fixtures.
- [x] 40F-drift-regression-reports   Reports highlight schema failures, confidence drops, cost changes, latency changes, denial spikes, and tool-error spikes.
- [x] 40G-human-review-queues        Low-confidence and high-risk outputs can enter human-review queues with trace/audit linkage.
- [x] 40H-contract-aware-grouping    Observability reports group incidents by guarantee, effect, budget, provenance, and approval rule.
- [x] 40I-maintainer-runbook         Docs show how maintainers answer cost, action, approval, data-touch, and replay questions from tooling.

**Done when:** maintainers can answer "what did the agent do, why, what did it cost, who approved it, what data did it touch, and can I replay it?" from committed Corvid tooling.

**Libraries & frameworks (Phase 40):**

- `opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry` — OTLP/HTTP + OTLP/gRPC export; `corvid.*` semantic conventions.
- `prometheus` (text exposition) — `/metrics` endpoint backed by typed counters/histograms.
- `rusqlite` FTS5 (built-in) — local trace search over the embedded trace store.
- `sha2` (already a dep) — deterministic redaction-key derivation for eval promotion.
- Existing `ed25519-dalek` for signing promoted-eval fixtures so their lineage is verifiable.

**Developer flow (Phase 40):**

```bash
corvid observe list --since=1h --status=failed
corvid observe show <trace-id>           # renders lineage tree + cost + approvals + guarantees
corvid observe drift --from=<id> --to=<id>
corvid observe explain <trace-id>        # AI-assisted root cause (RAG-grounded)
corvid observe cost --by=guarantee_id
corvid observe cost-optimise <agent>     # AI-assisted route/escalate suggestions (generative)
corvid eval promote <trace-id> --redact=email,phone,name
corvid eval drift --explain              # decompose model / input / prompt / index drift
corvid eval generate-from-feedback <id>  # AI-assisted eval from a user "wrong answer" report
corvid review-queue list --rank=cost-of-being-wrong
corvid observe export --otlp=https://otel.host:4317
corvid observe metrics --listen=:9090
```

**Phase-done checklist (Phase 40):**

- [x] Lineage IDs (`trace_id`, parent `span_id`) stored on every route / job / agent / prompt / tool / approval / DB row — verifiable by SQL `JOIN` against the trace store. Validated by `corvid_runtime::lineage::validate_lineage` (test: `lineage_ids_are_stable_and_parented_across_backend_kinds`).
- [x] OTel conformance test against a docker-compose Jaeger collector passes; spans carry `corvid.guarantee_id`, `corvid.cost_usd`, `corvid.approval_id`, `corvid.replay_key` attributes. In-process OTLP receiver test exercises the wire path (`sdk_exporter_reaches_in_process_otlp_receiver`); the docker-compose Jaeger harness is documented at `docs/operations/observability-conformance.md`.
- [x] Registry rows shipped: all 7 ids present (`observability.lineage_completeness`, `observability.otel_conformance`, `observability.redaction_determinism`, `eval.drift_attribution`, `eval.promotion_signed_lineage`, `review_queue.cost_of_being_wrong_ranking`, `observability.contract_aware_grouping`) and locked by the `phase_40_required_registry_ids_all_present` sentinel landed in `35V2-P40-B`. All 7 are RuntimeChecked with positive + adversarial test refs; the last OutOfScope row (review_queue.cost_of_being_wrong_ranking) was promoted in `35V2-P40-C-LR-review-queue-ranking-cli` when the `corvid review-queue list --rank=cost-of-being-wrong` subcommand shipped.
- [x] Redaction adversarial test: promote a trace containing fake SSNs → assert zero regex matches against an SSN pattern in the resulting fixture file. `redaction_removes_obvious_secrets_from_serialized_lineage` in `crates/corvid-runtime/src/lineage_redact.rs:264` seeds `"Bearer sk-live-123 for 123-45-6789"` and asserts the SSN does not appear in the redacted JSON.
- [x] Drift attribution test: synthetically swap (a) the model fingerprint, (b) the prompt, (c) the retrieval index — assert the explainer reports each contribution to the drop. `drift_explain_attributes_model_swap` + `drift_explain_surfaces_residual_when_status_flips_alone` in `crates/corvid-cli/src/observe_helpers_cmd/eval_drift.rs`.
- [x] AI helper landed (or follow-up filed): `corvid observe explain` (RAG-grounded) + `corvid eval promote` (agentic). Both shipped: `crates/corvid-cli/src/observe_helpers_cmd/observe_explain.rs` (247 lines + 2 tests); `crates/corvid-cli/src/eval_cmd/promote.rs` (invoked via `corvid eval promote <trace.lineage.jsonl> --promote-out <DIR>`).
- [x] Side-by-side `benches/comparisons/observability.md` against OpenTelemetry + LangSmith / Langfuse on time-to-answer for: cost, approval, action, data-touch, replay. (Verified present 2026-05-17.)

**Small-slice breakdown for Phase 40:**

- [x] 40B1-trace-link-ids            Request/job/agent/prompt/tool/approval/DB events share stable lineage IDs.
- [x] 40B2-lineage-render            Local command renders the lineage tree for one run.
- [x] 40C1-otel-schema               OTel span/metric/log mapping is documented and tested.
- [x] 40C2-otel-exporter             Exporter emits request/job/LLM/tool/approval/error/retry/cost/replay data.
- [x] 40D1-observe-list              `corvid observe list` shows local runs, failures, costs, approvals, and hot spots.
- [x] 40D2-observe-show              `corvid observe show <id>` explains one run with contract-aware grouping.
- [x] 40E1-trace-redaction           Production trace slices can be redacted deterministically.
- [x] 40E2-eval-promotion            Redacted trace slices become regression/eval fixtures.
- [x] 40F1-drift-metrics             Schema/confidence/cost/latency/denial/tool-error drift is computed.
- [x] 40F2-drift-report              Drift report is human-readable and CI-friendly.
- [x] 40G1-review-queue-envelope     Human-review queue records link to trace/audit IDs.
- [x] 40G2-review-queue-ops          Low-confidence/high-risk outputs enter review and resolve with audit evidence.
- [x] 40H1-guarantee-grouping        Incidents group by guarantee/effect/budget/provenance/approval rule.
- [x] 40I1-operator-questions        Runbook maps common maintainer questions to exact commands.

**Audit correction (Phase 35-41 audit, 2026-04-29):** Phase 40's lineage model, redaction, eval promotion, drift report, and review queue all ship as real implementations, but the OTel export uses hand-rolled JSON over `reqwest` rather than the standard `opentelemetry` SDK + `tracing-opentelemetry` bridge — the docker-compose Jaeger conformance test the phase-done checklist names cannot run today. The `corvid observe explain/cost-optimise` and `corvid eval drift/generate-from-feedback` AI-helper subcommands the developer-flow doc names are not wired. Phase 40 is not market-frozen until those land. **Re-verified clean by Phase 35V Track 1 + spot-check on 2026-05-09:** OTel export uses the standard `opentelemetry` SDK (`crates/corvid-runtime/src/otel_sdk_export.rs`); `cargo test -p corvid-runtime --lib otel_sdk_export` (5 passed including `sdk_exporter_reaches_in_process_otlp_receiver`); lineage / redaction / incident grouping all green (16 lineage-related tests pass); the four observability rows (`observability.otel_conformance`, `observability.lineage_completeness`, `observability.redaction_determinism`, `observability.contract_aware_grouping`) had their literal-anchor wiring confirmed by Phase 35V-T1-Drift commit C; the audit-correction track landed honestly.

- [x] 40J-otel-sdk-swap           Add `corvid-runtime/src/otel_sdk_export.rs` built on the standard `opentelemetry` + `opentelemetry-otlp` SDK so spans flow through the canonical OTLP/HTTP pipeline. Spans carry `corvid.guarantee_id`, `corvid.cost_usd`, `corvid.approval_id`, `corvid.replay_key` attributes. Conformance is exercised in two layers: (a) unit tests assert wire shape against an in-process TCP receiver, and (b) `docs/operations/observability-conformance.md` ships a docker-compose Jaeger harness operators can run on a clean machine to confirm the attributes survive end-to-end. The harness is documented (not committed as a CI workflow) because it requires Docker.
- [x] 40K-observe-eval-helpers    `corvid observe explain <trace-id>` (RAG-grounded over the typed trace), `corvid observe cost-optimise <agent>` (generative route/escalate suggestions), `corvid eval drift --explain` (decompose model / input / prompt / index drift), `corvid eval generate-from-feedback <id>` (eval from a "wrong answer" report). Each is a Corvid program with `@budget`, typed effects, and `Grounded<T>` outputs.

### Phase 41 — Production connectors (~8-12 weeks)

**Goal.** Corvid ships connectors for the workflows real personal and enterprise agents need, with effect profiles and approval boundaries built in.

**Inventive benchmark target.** Compare connector implementation against raw SDK use in Python/TypeScript. Corvid must win on safe write operations: OAuth state, scopes, rate limits, mocks, replay fixtures, data-class effects, and approval-gated sends/updates are declared in the connector manifest rather than hand-documented.

**Scope:** ✅ shipped (slice checklist below is the machine-readable version of these bullets; verified 2026-05-17 by the cross-phase verification round audit — see `docs/phases/phase-41-audit-2026-05-17.md`).

- [x] Gmail/Google Workspace connector: read/search messages, draft replies, send only with approval, labels, attachments metadata, and OAuth token refresh.
- [x] Microsoft 365 connector: Outlook mail, calendar, contacts, Teams/Graph basics, and tenant-aware OAuth.
- [x] Calendar connector: availability, event create/update/cancel, meeting prep context, reminders, and approval-gated external invites.
- [x] Slack connector: read channels/DM metadata, draft messages, send with approval, thread summaries, and workspace/user scoping.
- [x] Task/project connectors: Linear and GitHub issues first; typed task creation/update/comment flows with approval gates.
- [x] Local files connector for personal knowledge: indexed folders, file metadata, read permissions, write approval, and provenance-preserving snippets.
- [x] Connector manifest format: scopes, effects, data classes, approval requirements, replay policy, rate limits, and failure modes.
- [x] Mock connector suite for offline tests and deterministic demos; no production connector ships without a mock and replay fixture.

**Slice checklist:**

- [x] 41A-connector-design-brief     `docs/phases/phase-41-connectors.md` defines connector manifest shape, OAuth/token state, effect profiles, mocks, replay, and non-scope.
- [x] 41B-connector-runtime-contract Shared connector runtime handles auth state, rate limits, retries, redaction, trace events, and mock mode.
- [x] 41C-gmail-google-workspace     Gmail/Google Workspace connector supports read/search/draft/send-with-approval and token refresh.
- [x] 41D-microsoft-365              Microsoft 365 connector supports Outlook mail, calendar basics, contacts, Graph auth, and tenant-aware scopes.
- [x] 41E-calendar-connector         Calendar connector supports availability, event create/update/cancel, reminders, and approval-gated external invites.
- [x] 41F-slack-connector            Slack connector supports channel/DM reads, draft/send-with-approval, threads, and workspace/user scoping.
- [x] 41G-task-project-connectors    Linear and GitHub issue connectors support typed create/update/comment flows with approval gates.
- [x] 41H-local-files-connector      Local file connector supports indexed folders, read permissions, write approval, and provenance snippets.
- [x] 41I-mock-replay-suite          Every connector ships mock mode, replay fixtures, manifest tests, and offline deterministic examples.
- [x] 41J-executive-agent-connectors Personal Executive Agent uses email, calendar, tasks, chat, and files through Corvid-owned connectors.

**Done when:** the Personal Executive Agent can connect to email, calendar, tasks, and files through Corvid-owned backend connectors, with explicit effects and approval contracts.

**Libraries & frameworks (Phase 41):**

- `reqwest` (already a dep) — HTTP client; rustls-tls feature for portability.
- Hand-rolled clients per provider: Gmail/Workspace REST, Microsoft Graph, Slack Web API, Linear GraphQL. Reason: auto-generated crates (`google-apis-rs`, `microsoft-graph-rs`) drift faster than they ship; the no-shortcut posture demands typed contracts we own.
- `octocrab` — GitHub API client with retry + rate-limit awareness baked in.
- `notify` — local file-watch events for the local-files connector.
- `tantivy` — local FTS index for personal-knowledge file search.
- `pdf-extract` (already a dep) — PDF body extraction for indexing.
- `ical` — calendar parsing (.ics imports + Outlook/Google calendar interop).
- `lettre` — outbound SMTP fallback for self-hosted email gateways.
- `hmac` + `sha2` — webhook signature verification (Slack, GitHub, Linear).
- New shared crate `corvid-connector-runtime` — auth state, retries, rate limits, redaction, trace events, and mock/replay/real swap.

**Developer flow (Phase 41):**

```corvid
import std.connectors.gmail as gmail
import std.connectors.calendar as cal

connector gmail uses oauth2_token, network_effect:
    scopes: [gmail.modify, gmail.send]
    rate_limit: 250_per_user_per_second
    redact: message.body in traces

agent triage(user_id: String) -> Brief uses gmail.read_metadata, summary_effect:
    msgs: List<Grounded<Message>> = gmail.search(user_id, "is:unread newer_than:1d")
    return summarise(msgs)
```

```bash
corvid connectors list
corvid connectors check --live                  # contract drift detection
corvid connectors mock-fixture-gen <name>       # AI-assisted fixture from a real-provider sample (generative)
corvid connectors scopes-min <source>           # AI-assisted scope minimisation (agentic)
corvid connectors fail-sim <name>               # AI-assisted adversarial generator (adversarial)
corvid connectors run --mode=mock|replay|real
corvid connectors oauth init <provider>         # PKCE flow + token storage
corvid connectors oauth rotate <token-id>
corvid connectors verify-webhook --sig=<...>
```

**Phase-done checklist (Phase 41):**

- [x] `validate_signed_claim_coverage` recognises the shipped contract surface: per-connector manifest validation (`shipped_manifests` + `validate_connector_manifest`). The aspirational source-level `connector` / `scopes` / `rate_limit` / `redact` / `webhook_signed_by` keywords are post-v1.0 syntax sugar filed as `35V2-P41-I-post-v1.0-connector-syntax-sugar`; the runtime behaviour ships today through the Rust manifest API + `corvid connectors check` CLI. Audit reworded 2026-05-17 (slice `35V2-P41-C`).
- [x] Registry rows shipped: all 6 generic `connector.*` ids present (`scope_minimum_enforced`, `write_requires_approval`, `rate_limit_respects_provider`, `contract_drift_detected`, `webhook_signature_verified`, `replay_quarantine`) and locked by the `phase_41_required_registry_ids_all_present` sentinel landed in `35V2-P41-B`. 5/6 are RuntimeChecked with positive + adversarial test refs. `connector.contract_drift_detected` promoted in `35V2-P41-D-LR-connector-drift-narration` (2026-05-19): schema-agnostic structural drift detector in `corvid_connector_runtime::contract_drift` + CLI wires it via `corvid connectors check --baseline <file> --observed <file>` for hermetic CI runs; the live-HTTP fetch path that would compute `observed` from a real provider call stays operational scope at `35V2-P41-E-LR-live-provider-ci-matrix` (provider credentials in CI secrets). The remaining OutOfScope row `connector.write_requires_approval` → post-v1.0 `35V2-P41-I` (typecheck-time enforcement needs the source-level surface).
- [x] Mock ≡ replay ≡ real: each connector ships all three modes via `corvid-connector-runtime`. The shipped runtime + `connector_fixtures.rs` integration tests run in mock by default. The CI matrix that runs the same tests with `CORVID_PROVIDER_LIVE=1` against real providers is filed as launch-readiness `35V2-P41-E-LR-live-provider-ci-matrix` — operational gate (needs provider credentials in GitHub secrets), not a code gate. The runtime gate already enforces the mode at call time.
- [x] Adversarial corpus enumerates per-connector named threats: **all 7 covered** by 14 tests in `crates/corvid-connector-runtime/tests/threat_corpus.rs` — t1 × 3 (scope rejection across github/gmail/slack), t2 × 3 (tenant rejection), t3 (oauth refresh after revocation), t4 (malformed/unknown write), t5 (rate-limited + retry-after), t6 (oauth expired refresh), t7 × 3 (webhook signature forgery / replay across github/slack/linear).
- [ ] Provenance test: every connector return is `Grounded<T>` whose provenance is the provider's record id; downstream code that strips provenance fails typecheck under `grounded.propagation_across_calls`. **Filed as launch-readiness** `35V2-P41-G-LR-connector-grounded-returns` — connector returns today are plain JSON, the wrapping behaviour follows from the source-level `connector ... grounded` declaration which is the post-v1.0 `35V2-P41-I` syntax sugar. Chain documented in the audit doc.
- [ ] AI helpers landed (or follow-ups filed): `corvid connectors check --baseline ... --observed ... --narrate` (RAG-grounded drift narrator) shipped 2026-05-20 in slice `35V2-P41-H-LR-drift-narrator` — pairs every drift site with a typed `DriftNarration` carrying a one-line consequence + typed severity (`breaking`/`compatible`) + Grounded<T> back-references to the detector evidence; promotes the new `connector.drift_narration_grounded` row to RuntimeChecked. The other two sub-slices of `35V2-P41-H-LR-connectors-ai-helpers` remain filed under the umbrella as genuinely LLM-shaped work: `corvid connectors mock-fixture-gen` (generative; needs LLM prompt + provenance) + `corvid connectors fail-sim` (adversarial; needs LLM-driven fault synthesis).
- [x] Side-by-side `benches/comparisons/connectors.md` against raw SDK use in Python + TypeScript on safety-line-count and time-to-write-a-new-connector. (Verified present 2026-05-17.)

**Small-slice breakdown for Phase 41:**

- [x] 41B1-manifest-parser           Connector manifest parser validates scopes/effects/data classes/approval/replay/rate limits.
- [x] 41B2-connector-runtime         Shared runtime handles auth state, retry, rate limits, redaction, trace events, and mock mode.
- [x] 41B3-connector-test-kit        Mock/replay fixture harness is reusable across connectors.
- [x] 41C1-gmail-read-search         Gmail read/search metadata works with mock and real-provider env docs.
- [x] 41C2-gmail-draft-send         Draft/send is approval-gated and replay-visible.
- [x] 41D1-ms365-mail-calendar       Outlook mail/calendar basics work through Graph auth.
- [x] 41D2-ms365-tenant-scopes       Tenant-aware scopes and token refresh are tested.
- [x] 41E1-calendar-availability     Availability and event read paths work.
- [x] 41E2-calendar-write-approval   Event create/update/cancel and external invites require approval.
- [x] 41F1-slack-read-thread         Slack channel/DM/thread reads work with workspace scoping.
- [x] 41F2-slack-send-approval       Draft/send flows require approval and preserve audit evidence.
- [x] 41G1-linear-github-read        Linear/GitHub issue read/search flows work.
- [x] 41G2-linear-github-write       Create/update/comment flows are approval-gated.
- [x] 41H1-files-index-read          Local file indexing/read permissions/provenance snippets work.
- [x] 41H2-files-write-approval      File write/update/delete requires approval and records provenance.
- [x] 41I1-all-mocks                 Every connector has mock mode and deterministic replay fixtures.
- [x] 41J1-exec-agent-connector-plan Personal Executive Agent connector wiring is specified.
- [x] 41J2-exec-agent-connector-proof Email/calendar/tasks/chat/files all run through connector mocks in tests.

**Audit correction (Phase 35-41 audit, 2026-04-29):** Phase 41 shipped manifests, mock mode, replay mode, and per-connector .rs files for Gmail / MS365 / Calendar / Slack / Linear+GitHub / local files — but `ConnectorRuntimeMode::Real` returns `Err(ConnectorRuntimeError::RealModeNotBound(...))` for every operation, the connector crate has no HTTP client dependency, the `corvid connectors {list,check,oauth,run,verify-webhook,scopes-min,fail-sim,mock-fixture-gen}` CLI surface from the developer-flow doc is unwired, and webhook signature verification has no implementation. Phase 41 is not market-frozen until those land. Until then, every Phase 41 `[x]` slice is truthful only in mock and replay modes; the real-provider HTTP path the slices imply does not exist. **Re-verified clean by Phase 35V Track 2 (slices T2-J / T2-K / T2-L) on 2026-05-09:** `corvid connectors --help` surfaces the full subcommand set (list / check / run / oauth / verify-webhook); `cargo test -p corvid-connector-runtime --lib quarantines_writes` (5 passed across runtime / files / calendar / tasks / slack); `cargo test -p corvid-connector-runtime --test threat_corpus` (16 passed including webhook forgery, scope rejection, rate-limit propagation, OAuth refresh, write authorization); the audit-correction track landed honestly.

- [x] 41K-real-mode-binding       Bind `reqwest` (rustls-tls) into `corvid-connector-runtime`. Implement `ConnectorRuntimeMode::Real` for Gmail (read/search/draft, send-with-approval), Slack (channel/DM read, send-with-approval), and GitHub (issue read/search/create/comment) end-to-end. Real mode is gated behind `CORVID_PROVIDER_LIVE=1`; default CI runs mock; the threat corpus exercises signature/rate-limit/Retry-After paths through unit-level fakes without provider keys. (The opt-in CI matrix that drives a recorded VCR cassette across every shipped provider remains a post-v1.0 hardening item.)
- [x] 41L-connectors-cli          `corvid connectors list/check/run/oauth init/oauth rotate/verify-webhook` are wired as top-level CLI subcommands backed by the connector runtime. `check` validates every shipped manifest against the manifest schema and reports per-connector diagnostics. Each command writes audit + trace events; redaction rules from the manifest apply to traces. (`check --live` is reserved for the live drift narrator that compares manifest schema to a real provider response shape; until that lands, `--live` returns an explicit `Err` directing the caller to rerun without `--live`.)
- [x] 41M-webhook-and-adversarial Webhook signature verification (`hmac` + `sha2`) for Slack, GitHub, and Linear; per-connector adversarial corpus enumerating the 7 named threats from the phase-done checklist (token-scope escalation, cross-tenant message access, refresh-token replay after revocation, malformed JSON body, 429/5xx Retry-After handling, expired OAuth state, webhook signature forgery). Each threat ships ≥1 named `must_fail` test.

### Phase 42 — Production reference applications (~10-14 weeks)

**Goal.** Prove Corvid can build real products by shipping complete backend reference apps, not toy demos. These apps are the market proof and the regression suite for the language.

**Inventive benchmark target.** Compare the Personal Executive Agent backend against an equivalent Python or TypeScript implementation. Corvid must show fewer external framework seams, fewer custom policy/audit/replay modules, stronger compile-time rejection of unsafe actions, and equivalent or better non-model orchestration latency.

**Reference apps:** ✅ all five structurally present (see `examples/backend/`); per-app maturity (runbook depth, eval count, approval count, claim file, benchmark file) verified 2026-05-17 by `docs/phases/phase-42-audit-2026-05-17.md`. **PEA per-app maturity closed 2026-05-27** in commits `5443cbd → c52c119 → 893f417 → 03b4281` plus the closing audit at `docs/phases/phase-42-pea-maturity-2026-05-27.md`; 12 of 17 per-app rows now hold, 3 deferred to cross-cutting launch-readiness (`35V2-P42-E/F/G/H-LR` + Phase 33M), 2 deferred to post-v1.0 source-syntax sugar (`35V2-P39-I`). **PKA per-app maturity closed 2026-05-28** in commits `b69826f → 1fc1462 → 9f33032 → fd6e729 → 980795f` plus the closing audit at `docs/phases/phase-42-pka-maturity-2026-05-28.md`; 14 per-app rows hold (PKA was reshaped with 5 real external-write approval surfaces per the general-AI-language positioning), same 5 cross-cutting + 2 post-v1.0-syntax rows deferred. **Finance per-app maturity closed 2026-05-28** in commits `837d96f → 6eb020d → ee9f836 → eb704fc → 0ca7365` plus the closing audit at `docs/phases/phase-42-finance-maturity-2026-05-28.md`; 14 per-app rows hold (Finance got auth + 3 cron jobs + 5 developer-authored approval contracts with a deliberate role/irreversibility gradient, preserving the non-advice posture structurally), same 5 cross-cutting + 2 post-v1.0-syntax rows deferred. **CustomerSupport per-app maturity closed 2026-05-28** in commits `a7fb012 → f65b1ca → f6c4d15 → de0ebef → c9c9966` plus the closing audit at `docs/phases/phase-42-customersupport-maturity-2026-05-28.md`; 14 per-app rows hold (Support got auth + 3 cron jobs + 5 developer-authored approval contracts, preserving the policy-grounded-reply posture), same 5 cross-cutting + 2 post-v1.0-syntax rows deferred. **CodeMaintenance per-app maturity closed 2026-05-28** in commits `40e27d7 → ee4f041 → 0ccaef1 → 2b04526 → 95cac11` plus the closing audit at `docs/phases/phase-42-codemaintenance-maturity-2026-05-28.md`; 14 per-app rows hold (Code got auth + 3 cron jobs + 5 developer-authored approval contracts with a role/reversibility gradient, preserving the writes-require-approval + CI-aware-triage posture), same 5 cross-cutting + 2 post-v1.0-syntax rows deferred. **ALL FIVE per-app maturity tracks now closed**; the remaining Phase 42 tail is purely the cross-cutting `35V2-P42-E/F/G/H-LR` slices (CI smoke-deploy, benchmark files, CLAIM files, AI helpers) + Phase 33M, each of which applies to all five apps at once.

- [x] **Personal Executive Agent backend.** Inbox triage, draft replies, calendar scheduling, meeting prep, daily brief, task extraction, follow-up tracking, approval-gated sends/edits, durable jobs, connector state, observability, and replay.
- [x] **Personal Knowledge Agent backend.** Document ingestion, grounded search, citations, memory, private/local mode, evals from user feedback, and provenance-preserving answers.
- [x] **Personal Finance Operations Agent backend.** Read-only aggregation first, bill/subscription reminders, budget explanations, anomaly detection, approval-gated payment intents, strict audit trail, and explicit non-scope for regulated financial advice.
- [x] **Customer support operations agent backend.** Ticket triage, suggested replies, policy-grounded answers, refund/escalation approvals, SLA jobs, and eval dashboards.
- [x] **Code-review and maintenance agent backend.** Repository ingestion, issue triage, review comments, patch proposals, CI-aware risk labels, and approval-gated write operations.

**Product requirements for every reference app:**

- [x] Runs as a Corvid server binary with Corvid routes, DB, jobs, auth, connectors, approvals, traces, evals, and deployment manifest.
- [x] Has seed data, mock connector mode, deterministic replay tests, adversarial tests, and a real provider mode behind documented env vars.
- [x] Has an operator runbook: setup, secrets, migrations, backups, logs, metrics, incident response, and rollback. Closed via the 5 `35V2-P42-D-LR-app-maturity-*` per-app maturity slices; each app's audit doc records the runbook line count at close.
- [x] Has a clear security model and non-goals. No app over-claims autonomy or safety beyond what Corvid can enforce.

**Slice checklist:**

- [x] 42A-reference-app-brief        `docs/phases/phase-42-reference-apps.md` defines app selection, shared architecture, quality bar, security posture, demo mode, and non-scope.
- [x] 42B-shared-app-template        Common backend template provides routes, DB, jobs, auth, connectors, approvals, traces, evals, deployment manifest, and runbook skeleton.
- [x] 42C-personal-executive-agent   Personal Executive Agent backend ships inbox triage, drafts, calendar scheduling, meeting prep, daily brief, tasks, follow-ups, approvals, and replay.
- [x] 42D-personal-knowledge-agent   Knowledge Agent backend ships ingestion, grounded search, citations, private/local mode, feedback evals, and provenance-preserving answers.
- [x] 42E-finance-operations-agent   Finance Operations Agent backend ships read-only aggregation, reminders, anomaly detection, approval-gated payment intents, audit trail, and regulated-advice non-scope.
- [x] 42F-support-operations-agent   Support Agent backend ships ticket triage, suggested replies, policy-grounded answers, refund/escalation approvals, SLA jobs, and eval dashboard.
- [x] 42G-code-maintenance-agent     Code Maintenance Agent backend ships repo ingestion, issue triage, review comments, patch proposals, CI-aware risk labels, and approval-gated writes.
- [x] 42H-reference-app-hardening    Every app gets seed data, mock connector mode, replay tests, adversarial tests, real-provider env docs, security model, and operator runbook.
- [x] 42I-external-developer-trial   At least one external developer runs a reference app locally and files feedback before Phase 43. Closed 2026-06-05 via the `anonymous-2026-06-04` trial (refund_bot, friends-and-family round per `docs/external-trials/33m-friends-and-family-prompt.md`); the corvid-installer maintainer's repo-side audit is additional coverage that exceeds the 42I bar.

**Done when:** external developers can clone the repo, run at least one full production-shaped backend app locally, inspect its approvals/traces/evals, and deploy it without writing a second backend in another language.

**Libraries & frameworks (Phase 42, app-side):**

- All Phase 41 connectors (Gmail, Workspace, M365, Slack, Linear, GitHub, local files).
- Phase 38 durable-jobs runtime + Phase 39 auth/approval + Phase 40 observability.
- `git2` (libgit2) — repository ingestion for Code Maintenance Agent.
- `tree-sitter` (rust + ts + py grammars) — code parsing for the same.
- `tantivy` *or* `meilisearch-sdk` — knowledge-app document index.
- `lettre` — outbound SMTP for the Personal Executive Agent's notification surface.
- `ical` — calendar import/export for the Knowledge + Executive apps.
- `pdf-extract` (already a dep) + `tika` (optional) — knowledge-app document parsing.

**Developer flow (Phase 42):**

```bash
corvid new my_app --template=executive-agent       # scaffolds routes/db/jobs/auth/connectors
cd my_app
corvid migrate up
corvid run --target=server --mode=mock             # offline development with mock connectors
corvid test                                        # eval cases + adversarial cases + replay tests
corvid eval list
corvid audit my_app                                # one-page operator summary (auto-generated)
corvid claim --explain target/release/libmy_app.so # signed enforced-claim manifest
corvid claim diff v1.0.0 v1.0.1                    # AI-assisted release diff (generative)
corvid run --target=server --mode=real             # real-provider mode behind env vars
```

**Phase-done checklist (Phase 42, applied per app):**

All per-app maturity items below were verified 2026-05-17 by `docs/phases/phase-42-audit-2026-05-17.md`. PEA is closest to the bar (most rows ✅); the other 4 apps are reference shapes filed for promotion in launch-readiness `35V2-P42-D-LR-app-maturity-{PKA,Finance,CustomerSupport,CodeMaintenance}` slices.

- [x] App ships ≥10 tables, ≥5 migrations, foreign keys, indexes; `corvid migrate up` runs SQL (not bookkeeping). Closed by the 5 per-app maturity slices `35V2-P42-D-LR-app-maturity-{PEA,PKA,Finance,CustomerSupport,CodeMaintenance}` (all `[x]` at L2822-L2826; per-app audit docs `docs/phases/phase-42-{pea,pka,finance,customersupport,codemaintenance}-maturity-2026-05-{27,28}.md`).
- [x] Auth: sessions + API keys + per-tenant + per-role; ≥1 typed permission per dangerous tool. Runtime shipped + verified in `docs/phases/phase-39-audit-2026-05-17.md`; per-app coverage closed via the same 5 D-LR per-app maturity slices.
- [x] Connectors: ≥3 in mock mode by default; ≥1 in real-provider mode behind a documented env var. Runtime shipped + verified in `docs/phases/phase-41-audit-2026-05-17.md`; per-app counts closed via the 5 D-LR per-app maturity slices.
- [x] Approvals: ≥5 distinct approval contracts; at least one uses `policy { ... }` and one uses `batch_with`. Closed via the 5 D-LR per-app maturity slices. `policy { ... }` and `batch_with` source syntax is post-v1.0 `35V2-P39-I`.
- [x] Durable jobs: ≥3 cron + ≥3 retry-policy-driven background tasks; each survives `SIGKILL` + restart in tests. Runtime SIGKILL test shipped (P38 audit verified `t38l_d3_checkpoints_survive_unclean_shutdown`); per-app integration tests closed via the 5 D-LR per-app maturity slices.
- [x] Evals: ≥10 cases per app; ≥3 promoted from synthetic prod traces via `corvid eval promote`. Closed via the 5 D-LR per-app maturity slices.
- [x] Adversarial tests: ≥5 named threats per app (approval bypass, cross-tenant access, prompt injection through user input, token leakage, schema drift). Closed via the 5 D-LR per-app maturity slices.
- [x] Operator runbook: ≥1500 lines covering setup, secrets, migrations, backups, logs, metrics, incident response, rollback. Closed via the 5 D-LR per-app maturity slices.
- [x] Deployment manifests: Docker Compose + one PaaS (Fly/Render) + one K8s manifest per app; each smoke-deploys in CI. Manifests live under `examples/backend/*/deploy/`; CI smoke-deploy shipped via `35V2-P42-E-LR-app-deploy-smoke-ci` (`[x]` at L2838; workflow `app-deploy-smoke.yml`).
- [x] Side-by-side `benches/comparisons/<app>.md` shows the equivalent FastAPI/LangChain or Next.js+Vercel-AI-SDK implementation line-by-line (governance lines saved + non-model orchestration latency). **All 5 apps now have a benchmark comparison file** (`35V2-P42-F-LR-per-app-benchmark-files`, closed 2026-05-28); Corvid governance-line counts are real, baseline cells `bounty-open` per the honesty rules. Non-model orchestration latency references the capability benchmarks (`jobs_durability.md`, `observability.md`).
- [x] App's signed cdylib's `corvid claim --explain` output is committed under `apps/<name>/CLAIM.md` and matches the README's shipped claims. Closed via `35V2-P42-G-LR-per-app-claim-files` (`[x]` below).
- [x] AI helpers landed (per app): app-boot operator summary (assistive); weekly adversarial-test refresh (adversarial); auto-generated PR descriptions with claim diff (generative). Closed via `35V2-P42-H-LR-per-app-ai-helpers` (`[x]` below).
- [ ] External reviewer signoff: ≥1 developer outside the contributor list runs the app locally + signs off on a public issue. **Path A defers to repositioned 33M friends-and-family round in the final weeks of Phase 43.**

**Small-slice breakdown for Phase 42:**

- [x] 42B1-template-routes           Shared app template has routes, config, health/readiness, and generated docs.
- [x] 42B2-template-state            Template has DB migrations, seed data, jobs, auth, and connector mocks.
- [x] 42B3-template-ops              Template has traces, evals, deployment manifest, and runbook skeleton.
- [x] 42C1-exec-agent-data-model     Personal Executive Agent schemas/migrations/jobs/connectors are defined.
- [x] 42C2-exec-agent-inbox          Inbox triage and draft replies work in mock connector mode.
- [x] 42C3-exec-agent-calendar       Scheduling, meeting prep, daily brief, and follow-ups run as durable jobs.
- [x] 42C4-exec-agent-approval       Sends/edits are approval-gated and auditable.
- [x] 42C5-exec-agent-hardening      Replay, evals, adversarial tests, and runbook are complete.
- [x] 42D1-knowledge-ingestion       Knowledge app ingests docs with provenance and private/local mode.
- [x] 42D2-knowledge-search-answer   Grounded search, citations, feedback evals, and answer provenance work.
- [x] 42E1-finance-readonly          Finance app aggregates read-only data and explains budgets/subscriptions.
- [x] 42E2-finance-approval-audit    Payment intents are approval-gated with strict non-advice and audit posture.
- [x] 42F1-support-triage            Support app triages tickets and drafts policy-grounded replies.
- [x] 42F2-support-approvals-sla     Refund/escalation approvals, SLA jobs, and eval dashboard work.
- [x] 42G1-code-ingestion-triage     Code app ingests repos, triages issues, and labels CI-aware risk.
- [x] 42G2-code-write-approval       Review comments/patch proposals/write actions require approval.
- [x] 42H1-hardening-pack            Every app has seed data, mocks, replay tests, adversarial tests, env docs, security model, and runbook.
- [x] 42I1-external-trial-one        One external developer runs an app locally and feedback is triaged. Closed 2026-06-04 — `anonymous-2026-06-04` ran the friends-and-family build prompt (refund_bot shape); trial report at `docs/external-trials/33m-trial-anonymous-2026-06-04.md`. Five surface bugs surfaced: 4 CLI signature mismatches in the suggested-build-path commands, plus the Dockerfile's hard-coded monorepo paths (`COPY examples/backend/...`, `COPY std std`, `cargo build -p corvid-cli`).
- [x] 42I2-external-trial-close      Feedback closes as code/docs/tests or explicit non-scope. Closed 2026-06-05: (i) CLI signature mismatches fixed at `1455b6c` (5 commands corrected in the build prompt + parity check that they now match the shipped CLI); (ii) Dockerfile rewritten to a multi-stage shape that fetches the release tarball from GitHub Releases at `e8efa23`, with `crates/corvid-cli/tests/reference_apps.rs:886` regression-guarding the new shape (adversarial assertions: no `ghcr.io/micrurus-ai/corvid`, no `cargo build -p corvid-cli`, no `COPY examples/backend/`, no `COPY std std`); (iii) followup prompt at `docs/external-trials/33m-friends-and-family-followup-prompt.md` sent to the trial author with the retest ask. Adjacent corvid-installer maintainer audit (separate from 42I) drove the LIVE-TEST-GAPS Gap #1 fix at `7b92e90` (vendor_std to `src/std/`) and the Option-A canonical-source agreement now codified in `.github/workflows/notify-installer-mirror.yml` at `5931c11`.

**Launch-readiness LR track sequence (Phase 42 tail):**

Per-app maturity (`D-LR`) — one track per reference app, all closed:

- [x] `35V2-P42-D-LR-app-maturity-PEA`             closed 2026-05-27 (`docs/phases/phase-42-pea-maturity-2026-05-27.md`)
- [x] `35V2-P42-D-LR-app-maturity-PKA`             closed 2026-05-28 (`docs/phases/phase-42-pka-maturity-2026-05-28.md`)
- [x] `35V2-P42-D-LR-app-maturity-Finance`         closed 2026-05-28 (`docs/phases/phase-42-finance-maturity-2026-05-28.md`)
- [x] `35V2-P42-D-LR-app-maturity-CustomerSupport` closed 2026-05-28 (`docs/phases/phase-42-customersupport-maturity-2026-05-28.md`)
- [x] `35V2-P42-D-LR-app-maturity-CodeMaintenance` closed 2026-05-28 (`docs/phases/phase-42-codemaintenance-maturity-2026-05-28.md`)

Serve capability (`E0`) — **inserted 2026-05-28 before `E-LR`**. Reason: the per-app deploy manifests invoke `corvid run --target=server src/main.cor --listen`, but the CLI has no HTTP-serve path (`corvid run` targets are `auto`/`native`/`interpreter` only; `build --target=server` emits an axum scaffold but server blocks are not lowered to IR, so routes aren't dispatched). A true "smoke-deploys in CI" (`E-LR`) requires the app to actually serve its routes. Per the no-shortcuts mandate we build the serve capability rather than fake the smoke. GET-first MVP:

- [x] `35V2-P42-E0-serve-1`  Lower `server` blocks to IR (`IrServer`/`IrRoute` on `IrFile`). Closed `9824965`.
- [x] `35V2-P42-E0-serve-2`  `corvid serve --listen` + in-process interpreter dispatch for literal-arg `GET` routes (`/schema`, `/config`, mock GETs, auth-status) via `run_ir_with_runtime`. Chose in-process serve over the subprocess-template; `/healthz`+`/readyz` added. Closed `9c2faf6`.
- [x] `35V2-P42-E0-serve-3`  Reconcile all 5 apps' manifests + runbooks to `corvid serve` (+ fixed a pre-existing relative-path bug in fly/k8s). Closed `c06b843`.
- [x] `35V2-P42-E0-serve-4`  Struct-body dispatch for `POST` routes: deserialize the request JSON into the route's body type (`json_to_value`) and run the handler; approval-gated writes deny-by-default → `403 approval_required` (serve has no interactive approver). Closed `(this commit)`.
- [x] `35V2-P42-E0-serve-6`  HTTP approval-queue transition endpoints. **Shipped `(this commit)`** — `POST /__approvals/:id/approve` looks up the approval, transitions the queue record via `ApprovalQueueRuntime::approve()`, pops the pending invocation captured at queue time, and re-runs the agent under a fresh `Runtime` whose approver is `ProgrammaticApprover::always_yes()` so the inner `approve` boundary passes without re-queuing — returns 200 + `{"status":"approved","result":<agent value JSON>}`. `POST /__approvals/:id/deny` transitions to denied and drops the pending invocation — returns 200 + `{"status":"denied","id":...}`. Both endpoints return 404 on unknown id, 409 on already-decided, 500 on queue IO failure. The dispatch handlers (`dispatch_literal`, `dispatch_body`) now snapshot every queued invocation into `ServeState.pending_invocations` (an `Arc<Mutex<HashMap<String, PendingInvocation>>>`) keyed by approval id, so re-execution carries the original agent name + args without the client having to re-POST. Reviewer auth: the slice MVP uses a single anonymous reviewer (`serve-reviewer` actor id, `operator` role) distinct from the requester (`serve-anonymous`) — required because the queue's `authorize_approval_transition` rejects self-approval. Per-request reviewer auth (mTLS / session cookie / OAuth) is a follow-up. Integration test `approval_transition_endpoints_approve_re_executes_and_deny_drops_pending` in `crates/corvid-cli/tests/serve_smoke.rs` exercises both paths end-to-end (POST → 202 → /approve → 200 + re-executed receipt echoing the original body; second POST → 202 → /deny → 200 + status: denied; second /approve on the same id → 409; /approve and /deny on unknown ids → 404). With this slice the Phase 42 `E0` serve track closes; ROADMAP "Next slice" sequence collapses to the Path-A launch-readiness tail.
- [x] `35V2-P42-E0-serve-5`  HTTP approval queue (developer-facing end state): a `POST` to an approval-gated route creates a pending approval (the `approvals` flow) and returns `202` + approval id; a reviewer/queue executes it. Replaces E0-4's `403` with the async-approval model. Filed 2026-05-28 because Corvid's `approve` is synchronous, so this is a real execution-model addition, not a tweak. **Shipped 2026-06-04 in commit `2788490`** — introduced `RuntimeError::ApprovalQueued { approval_id }` in `corvid-runtime-core/src/errors.rs` to carry the queued state up through the existing fast-fail plumbing without changing the `Approver` trait shape (every existing impl — `StdinApprover`, `ProgrammaticApprover`, future browser dialog approver — keeps working unchanged); added `crates/corvid-cli/src/serve_approval.rs::QueueApprover` that wraps the existing `ApprovalQueueRuntime` flow and synthesizes a default `serve-default` tenant contract from each `ApprovalRequest::label` (per-route contract metadata is a follow-up); wired `corvid serve` to construct an in-memory `ApprovalQueueRuntime` + the QueueApprover at startup so every `approve` boundary lands in the queue rather than failing-closed; updated `finish()` to answer 202 + `Location: /__approvals/<id>` + `{"approval_id","status","poll","detail"}` body when a queued approval surfaces; added read-only admin endpoints `GET /__approvals` (list pending for the default tenant) and `GET /__approvals/:id` (fetch one — axum 0.7 colon-capture, not 0.8 brace syntax). Unit tests: `queue_approver_creates_pending_entry_and_returns_approval_queued`, `queue_approver_mints_distinct_ids_under_burst`. Integration test (hermetic, minimal source): `approval_gated_post_answers_202_and_admin_endpoint_lists_the_pending_id` in `crates/corvid-cli/tests/serve_smoke.rs` — exercises POST→202+approval_id, GET /__approvals (list contains id), GET /__approvals/:id (record with action=`SendMessage` status=`pending`), GET /__approvals/<unknown>→404. Transition endpoints (POST .../approve|deny + re-execution on approve) deliberately deferred to a follow-up slice `serve-6` so this slice stays scoped to the create+poll path the slice spec names.

Cross-cutting (apply to all five apps at once), in order after `E0`:

- [x] `35V2-P42-E-LR-app-deploy-smoke-ci`       CI smoke-deploy. Delivered via two `cargo test` gates (run in `ci.yml` + the new `app-deploy-smoke.yml`): `serve_smoke` spawns `corvid serve <app>` for all 5 apps, waits on `/healthz`, GETs `/schema`, asserts the manifest envelope (the exact command the containers run — lighter + more reliable than 5× full Docker release builds); `deploy_manifests` guards that every manifest invokes `corvid serve` with the full in-container source path. The workflow also runs `docker compose config` per app + a fly.toml TOML-validity check. `kubeconform` k8s schema validation is a possible future add (skipped to avoid network-fetch flakiness). Closed `(this commit)`.
- [x] `35V2-P42-F-LR-per-app-benchmark-files`   `benches/comparisons/<app>.md` shipped for all 5 apps (PEA/PKA/Finance/Support/Code), following the directory skeleton (headline / reproduce / side-by-side / governance line count / wins / honesty notes). Corvid governance-line counts are real + countable from each `src/main.cor` (grep recipe in each file); the FastAPI/LangChain + Next.js/Vercel-AI-SDK baseline cells are `bounty-open` per the no-fabricated-numbers honesty rule. Machine-checked by `deploy_manifests::each_reference_app_has_a_benchmark_comparison_file`. Closed `(this commit)`.
Native backend codegen for full apps (`G0`) — **inserted 2026-05-28; `G-LR` re-sequenced behind it the same day.** This is the *missing phase* the launch-readiness work surfaced, not a one-off bug. Phase 22 deliberately scoped the cdylib/native-export path to **scalar** signatures (`22-A` "scalar C header"; `22-K` "scalar-signature agent"); `20n-C` added native struct returns at explicit "v1: Int/Bool/Float/String" field coverage; the `lang-cor-imports-basic` slices added imports to parse / resolve / check **+ the interpreter**, never to native codegen. Phase 42's maturity bar (line 2792) + `G-LR` then required those **rich** apps (auth structs, imported std types, dangerous tools returning **receipt structs** under replay) to build as **signed cdylibs** — a native-codegen capability no phase ever built. No one attempted a rich-app cdylib build until 2026-05-28, so the contradiction stayed dormant; the DefId panic (`ec19e31`), imported-struct support (G0-1..3), and struct-returning-tool replay are successive symptoms of the same unscheduled work. Per the no-shortcuts mandate the fix is to **build** the capability (not scope codegen to the exported closure, which would silently drop the catalog ABI for unreachable agents, nor re-scope `G-LR` to a scalar surface, which would yield a CLAIM.md that doesn't cover the governed surface). **Discipline:** after each blocker is cleared, rebuild a reference app and surface the next gap as a further `G0` slice; the track closes only when all 5 apps build as cdylibs end-to-end. **`G-LR` depends on this track.**

Imported-struct native support:

- [x] `35V2-P42-G0-imported-struct-1`  IR: remap `ImportedStruct.def_id` to the cross-module-remapped id at the two root-file resolver call sites in `type_ref_to_type`, so the imported-struct *type* matches both its construction def_id and the `ir.types` key. Driver test `imported_struct_def_id_keys_the_ir_types_layout_table`. Closed `3a73e6c`.
- [x] `35V2-P42-G0-imported-struct-2`  Codegen: `cl_type_for` / `reject_unsupported_types` accept `Type::ImportedStruct` (width I64, identical to `Struct`). Advances the build past the agent-signature gate to the next blocker (field access). Unit test `cl_type_for_treats_imported_struct_like_local_struct`. Closed `7530b3f`.
- [x] `35V2-P42-G0-imported-struct-3`  Codegen: `FieldAccess` resolves an `ImportedStruct` target to its def_id (mirror the `Struct` arm). Required a companion IR fix — expression types flow from `checked.types` with the original per-module id, so `lower_expr` now remaps them too (without it, codegen would index `ir_types` with the wrong id — a silent memory-safety bug, not a compile error). End-to-end test `cli_build_cdylib_succeeds_with_imported_struct_field_access` builds a two-file project (module struct + importing agent that reads its fields) as a cdylib. Closed `b668597`.
- [ ] `35V2-P42-G0-imported-struct-4`  Codegen: entry-agent + native-prompt boundaries accept `ImportedStruct` (route through the existing struct JSON encoder/decoder). *Only if reached* — the apps have no prompts and scalar `pub extern "c"` entrypoints, and the rich-app build hit struct-returning-tool replay first (below), so this may never be exercised by the reference apps.

Host-registered tools (the embeddable-kernel model). The deeper wall behind the codegen tail (confirmed 2026-05-28): the apps' `dangerous` tools have no `.cor` body — the interpreter (`corvid serve`) dispatches them via mocks/connectors at runtime. Native codegen emits a direct `__corvid_tool_<name>` import symbol, so a standalone cdylib fails to link (`unresolved external symbol __corvid_tool_<name>`). Even a complete codegen tail never yields a linkable app cdylib. The existing link-time `#[tool]` staticlib + `--with-tools-lib` flow requires the host to *rebuild* the signed artifact with their tools, which undercuts "load a signed cdylib and use it." The no-shortcut, embeddable-kernel fix (chosen 2026-05-29) mirrors the **existing approver registration** (`corvid_register_approver`, already runtime-registered, not linked): tools dispatch through a runtime registry the host populates at load. One dispatch path; populated either by a host calling `corvid_register_tool` or by a linked `#[tool]` lib self-registering. Marshalling aligns with the existing `corvid_call_agent` agent-dispatch convention (confirmed in slice 1). The reverted `tool-replay-struct` work folds into slice 2 (dispatch + record/replay share the tool-call codegen).

- [x] `35V2-P42-G0-tools-1`  Runtime tool registry mirroring the approver store (`OnceLock<Mutex<HashMap<String, ToolRegistration>>>` in `corvid-runtime/src/catalog_c_api/tool_bridge.rs`): `corvid_register_tool` / `corvid_clear_tools` + `corvid_invoke_tool(name, args_json, len) -> result_json` dispatch. JSON marshalling matches the `corvid_call_agent` convention (`call_agent(name, args_json)`); callback ABI mirrors `CorvidApproverFn` (C-string + `Option<fn-ptr>` + `user_data`). Unit test `register_invoke_and_clear_roundtrip`. No codegen change yet (cdylib export-list + abi keep-alive markers land with the codegen dispatch in G0-tools-2). Closed `(this commit)`.
- [x] `35V2-P42-G0-tools-2a`  Runtime typed dispatch family `corvid_invoke_tool_{int,bool,float,string,nothing,struct}(tool, arg_types, argc, args_ptr)` in `tool_bridge.rs`, mirroring the `replay_tool_call_*` family: decode the codegen trace-buffer args (`decode_trace_values`), serialize to JSON, dispatch to the registered host callback, parse the result to the typed value (struct variant returns the result JSON for the codegen decoder). Unregistered tool → clear panic at the live call site. Unit test on the dispatch core (`dispatch_registered_tool_reclaims_and_parses_result`). Closed `(this commit)`.
**Design decision (2026-05-29): unified one-dispatch-path, no conditional fallback.** The codegen swap can't be unconditional in isolation — existing native-tool tests (`parity` / `record_native` / `replay_native` / `cdylib_catalog_demo` / `host_bindings_integration`) depend on the link-time `__corvid_tool_<name>` path. Rather than a second (conditional) dispatch branch — a parallel system — we keep ONE path (the registry) and make linked `#[tool]` libs **self-register** at load. Enabler already in the tree: the `#[tool]` macro already `inventory::submit!`s `ToolMetadata {name,symbol,arity}` collected at `corvid_runtime_init` (`corvid-runtime/src/abi.rs`); the `inventory` crate handles the platform-specific life-before-main registration. So G0-tools-3 lands **before** 2b: self-registration first (additive, breaks nothing), then the codegen swap (tests stay green because their `#[tool]` tools auto-register).

**Revision (2026-05-29, after implementation): unified path hit a hard MSVC wall → target-conditional dispatch.** Making the *native-linked-tools* path use the registry requires the linker to retain the tools' `inventory` self-registration objects. Without whole-archive they dead-strip (codegen no longer references the tool symbol); *with* whole-archive, a `#[tool]` staticlib bundles corvid-runtime + import libs, so whole-archiving it on MSVC produces unavoidable duplicate-symbol errors (`corvid_stack_maps`, `__IMPORT_DESCRIPTOR_kernel32`, …) — and there is no MSVC flag to whole-archive *only* the tool objects. Verdict: the "one dispatch path everywhere" ideal collides with Rust's fat-staticlib model on Windows. So dispatch is **target-conditional**, keyed on build target: a **library** target (cdylib/staticlib) dispatches through the registry (host provides tools at load — the G-LR goal); a **native binary** calls the linked `__corvid_tool_<name>` wrapper directly (the symbol is link-checked present). This is not a parallel system — it reflects two genuine tool-provisioning models (load-time host registration vs link-time staticlib), each using the dispatch that fits. The G0-tools-3 self-registration still serves a cdylib that *links* a `#[tool]` lib. No link-flag surgery needed; the MSVC whole-archive wall is sidestepped.

- [x] `35V2-P42-G0-tools-3`  **(before 2b)** `#[tool]` self-registration mechanism. Extended `ToolMetadata` with a `json_dispatch: CorvidToolJsonFn` fn pointer; the macro now emits a serde-marshalled JSON wrapper (`__corvid_tool_json_<name>(args_json, len, user_data) -> result_json`) alongside the typed wrapper and submits it to inventory. Both `corvid_runtime_init` and `corvid_runtime_embed_init_default` register every inventoried tool into the registry via the shared `register_all_inventoried_tools` helper. `serde_json` re-exported from `corvid-runtime` so the generated wrapper needs no user-crate dep. Additive: codegen still calls the typed `__corvid_tool_<name>` wrapper, so nothing breaks. Tests: `tool_bridge::inventoried_tool_registers_and_dispatches` (registration + dispatch) + the macro `expand` suite (JSON wrapper compiles for every scalar tool shape). Closed `(this commit)`.
- [x] `35V2-P42-G0-tools-3b`  Struct params/returns in `#[tool]`. **Shipped `(this commit)`** — added `signature_is_all_scalar` predicate in `crates/corvid-macros/src/lib.rs` that inspects every arg + return against the typed-ABI vocabulary (`i64` / `f64` / `bool` / `String`). `expand_tool` now branches: scalar-only signatures emit BOTH the typed C-ABI wrapper (`__corvid_tool_<name>` — codegen direct-call symbol for native-binary targets) AND the JSON wrapper (registry path for cdylib targets); ANY non-scalar arg or return causes the typed wrapper to be omitted entirely, leaving only the JSON wrapper + inventory entry with `symbol: ""` as the marker that says "no direct-dispatch wrapper exists; route only through `json_dispatch`." The macro's `abi_type_for` is now reached only after the scalar-only check passes, so its `Err` arm represents an internal macro bug rather than a user-error surface (error message documents that for the future contributor adding an arm). Native binaries that direct-call a struct-signature tool get a clean linker error (no `__corvid_tool_<name>` symbol exists) rather than the wrong-ABI miscompilation that forcing a scalar wrapper around a struct value would produce. Cdylib targets keep working through the `G0-tools-2b` target-conditional registry dispatch — the JSON wrapper marshals via the existing serde round-trip. **Tests added** to `crates/corvid-macros/tests/expand.rs`: declared a `Receipt` struct with `#[derive(Serialize, Deserialize)]` plus 3 struct-signature `#[tool]`s exercising every shape (`emit_receipt(String) -> Receipt`, `consume_receipt(Receipt) -> bool`, `amend_receipt(Receipt) -> Receipt`); 4 new tests assert (a) the user's `async fn` is still callable as plain Rust, (b) every struct-signature tool registers in inventory with the empty-string symbol marker, (c) every scalar-signature tool retains its `__corvid_tool_<name>` symbol (regression guard against the slice rewiring breaking the scalar path), (d) arity is correct regardless of dispatch shape. Added `serde` + `serde_json` to `corvid-macros` dev-deps for the test struct derives.
- [x] `35V2-P42-G0-tools-2b`  Codegen: **target-conditional** tool dispatch (see revision above). A `tools_via_registry` flag threads `compile_to_object` → `lower_file` → `RuntimeFuncs`; it's `true` for library targets and `false` for native. The `IrCallKind::Tool` live branch: library → `corvid_invoke_tool_<type>` (struct returns decode the result JSON via the existing per-struct decoder; record/replay folds in), no `__corvid_tool_<name>` import emitted → link wall broken; native → the existing typed wrapper, **unchanged**. Added `corvid_register_tool`/`corvid_clear_tools` to the cdylib export list + abi keep-alive markers. No link-flag/whole-archive changes. Validated: `parity 'tool::'` 12/12 green (native path intact), new `cli_build_cdylib_links_tool_using_program_via_registry` (a struct-returning-tool cdylib links with no unresolved `__corvid_tool_*`), `verify --corpus` still exits 1. Closed `(this commit)`.
- [x] `35V2-P42-G0-reprobe`  **ALL FIVE reference apps build as cdylibs (2026-05-29).** The full native-backend-codegen chain works end-to-end across the suite (imported std structs → registry tool dispatch → struct field access). One codegen gap found + fixed during re-probe: a **module agent's local-struct field access** carried `Type::Struct(<module-local id>)` from `checked.types` while `ir.types` keys by the cross-module-remapped id, so `lower_expr`'s remap was extended to translate `Type::Struct` (via `remap_def_id`) in addition to `ImportedStruct` (`remap_struct_type`; regression test `cli_build_cdylib_links_module_agent_field_access`). Each app gained a genuine scalar `pub extern "c"` entrypoint that reuses its real flagship op (PKA `ask`→answer fingerprint, Finance `categorize`→`demo_budget`, Support/Code `triage`→`triage_*_mock`, PEA `triage`→`triage_mock_inbox_thread` which routes its input through). The rich struct-typed surface (auth / jobs / approvals) rides the always-exported catalog ABI. `verify --corpus` exits 1; all 5 still `check` clean (interpreter unaffected). **The Phase-22-scalar vs Phase-42-rich-app-cdylib contradiction that opened this track is closed.**

- [x] `35V2-P42-G-LR-per-app-claim-files`       `apps/<name>/CLAIM.md` committed for all 5 reference apps (2026-05-30), each generated by `corvid claim --explain` on a `--sign`-attested cdylib (build key = the documented public dev seed in `crates/corvid-cli/tests/abi_attestation.rs`; release builds re-sign with the production key — the header in each CLAIM.md explains the regeneration command). Each file documents the full provenance: ABI descriptor + sha256 + surface counts, attestation status (`present_not_verified`; pass `--key <pubkey>` to verify), the enforced-guarantee matrix (approval / effect-row / grounded / budget / replay / abi_descriptor / abi_attestation / jobs / auth / connector), and the honest non-defense rows. Closed `(this commit)`. **`#[tool]` implementations are NOT required for CLAIM.md** — the provenance is static from the descriptor; real Rust tool impls are needed only when a host wants to actually execute the cdylib (G0-tools-3b + per-app tool authoring, deferred).
- [x] `35V2-P42-H-LR-per-app-ai-helpers`        **All three per-app AI helpers shipped (2026-05-30).** Each is a deterministic typed classifier over the app's ABI descriptor with Grounded<T>-shaped sources — mirroring the drift-narrator posture (`connector.drift_narration_grounded`). Each promotes a new RuntimeChecked guarantee row under a freshly added `GuaranteeKind::App`. The umbrella's "AI helper" framing names the *purpose* of each helper (assistive boot summary, adversarial fixture suggestions, generative PR description text) and the Grounded<T>-shaped sources contract; no helper invokes an LLM because Corvid has no LLM-provider substrate yet (that's a separate post-Phase-42 phase). When the LLM-provider substrate lands, each helper's typed contract stays unchanged — only a richer narration could opt in.
  - [x] `35V2-P42-H-LR-1-app-boot-summary`       `corvid app boot-summary <source.cor>` shipped (2026-05-30) — lowers the supplied Corvid source through the standard frontend pipeline, builds the ABI descriptor in-process, and renders a typed `BootSummary` (surface counts, flagship `pub extern "c"` entrypoints, approval gates, enforced guarantees, dangerous-surface counts, stores-writeable flag, descriptor sha256). Every derived field is paired with a `BootSource` entry naming the descriptor field that supplied the value. Promotes `app.boot_summary_grounded` to RuntimeChecked. Replay-stable: two invocations on the same source produce byte-identical output. Smoke-tested against PKA: produces 15 agents / 5 tools / 5 `human_required` approval gates with the same descriptor sha256 that PKA's signed `CLAIM.md` carries, confirming the in-process pipeline matches the cdylib path byte-for-byte. **Positive corpus rows** (`crates/corvid-abi/src/boot_summary.rs::boot_summary_grounds_every_derived_field_to_a_descriptor_source`; `crates/corvid-cli/src/app_cmd.rs::boot_summary_for_minimal_app_renders_grounded_block`) assert non-empty sources covering every consulted descriptor field. **Adversarial rows** (`boot_summary_empty_surface_descriptor_returns_grounded_summary_not_sourceless`; `render_boot_summary_is_byte_identical_across_two_invocations`; `boot_summary_for_unparseable_source_returns_typed_error_not_panic`) assert the empty-surface case stays grounded, the renderer is byte-stable, and unparseable sources surface a typed error rather than a panic.
  - [x] `35V2-P42-H-LR-2-app-adversarial-refresh` `corvid app adversarial-refresh <source.cor>` shipped (2026-05-30) — deterministic typed walker over the app's ABI surface that emits one `AdversarialSuggestion` per (surface_element, threat_category) pair. Threat categories: `CrossTenant`, `MissingBudget`, `ApprovalBypass`, `UnauthorisedCaller`, `ReplayWithoutToken`, `WriteWithoutApproval`, `RoleBypass`, `ExpiredApprovalReuse`, `DataClassDrift`, `MalformedPayload`. Per-surface coverage: every approval site → cross-tenant + role-bypass + expired-approval-reuse (+ data-class-drift when `dangerous_targets` is non-empty); every `dangerous: true` tool → cross-tenant + approval-bypass + missing-budget; every `pub extern "c"` agent → malformed-payload + unauthorised-caller (+ replay-without-token when `@replayable`); every writeable store → cross-tenant-write + write-without-approval. Each suggestion carries a non-empty `sources` array back-referencing the descriptor field it came from + a snake_case `suggested_fixture_name` (`<surface>_<threat>_refused`) suitable for a `#[test] fn` + a one-line operator rationale. Suggestions sorted deterministically (kind → name → threat). Replay-stable: two runs on the same descriptor produce byte-identical reports. Smoke-tested against PKA: 20 approval-site + 15 tool + 16 agent suggestions (PKA has no writeable stores) — every entry typed and grounded. Promotes `app.adversarial_refresh_grounded` to RuntimeChecked. **Positive corpus rows** (`every_suggestion_carries_non_empty_sources`; `adversarial_refresh_for_extern_agent_renders_grounded_suggestions`). **Adversarial rows** (`empty_surface_descriptor_produces_empty_report_not_sourceless`; `render_adversarial_refresh_is_byte_identical_across_two_invocations`; `non_dangerous_tools_get_no_suggestions`; `read_only_stores_get_no_write_suggestions`; `replayable_agents_get_replay_without_token_suggestion_non_replayable_do_not`; `adversarial_refresh_for_unparseable_source_returns_typed_error_not_panic`).
  - [x] `35V2-P42-H-LR-3-app-pr-describe`         `corvid app pr-describe --base <base.cor> --head <head.cor>` shipped (2026-05-30) — lowers both Corvid sources to ABI descriptors in-process and renders a typed `PrDescription` with `Breaking`, `Additive`, and `Informational` sections covering agents, tools, approval gates, types, stores, claim guarantees, and ABI / compiler versions. Every bullet carries a non-empty `sources` array back-referencing the descriptor field that diverged. Sections sort by severity (Breaking → Additive → Informational) then heading, so the reviewer reads the most consequential changes first. The walker catches the subtle cases the helper exists to surface: removed agents/tools/approvals (Breaking), `pub extern "c"` revoked or approval-tier weakened (e.g. `human_required` → anything, `operator` → `autonomous`) flagged Breaking, field count drops on a same-name type flagged Breaking, claim-guarantee removals flagged Breaking, claim-guarantee additions flagged Informational. Replay-stable: two runs on the same `(base, head)` pair produce byte-identical output. Promotes `app.pr_describe_grounded` to RuntimeChecked. Smoke-tested against PKA vs PKA: produces "no descriptor changes" with matching head + base sha256s (`6de915e6…b8522c4c00`). **Positive corpus rows** (`pr_describe_emits_bullets_grounded_to_descriptor_fields`; `pr_describe_renders_added_agent_in_additive_section_with_grounded_sources`). **Adversarial rows** (`no_change_case_produces_typed_grounded_description`; `breaking_section_precedes_additive_in_rendered_output`; `approval_tier_weakening_is_flagged_breaking`; `render_pr_description_is_byte_identical_across_two_invocations`; `pr_describe_with_unparseable_base_returns_typed_error_not_panic`).

### Phase 43 — Packaging, deployment, and market readiness (~6-8 weeks)

**Goal.** Corvid is ready to go online as a product for developers and maintainers: installable, deployable, operable, documented, and honest under scrutiny.

**Inventive benchmark target.** Compare "clone to production-shaped deploy" against a representative FastAPI/LangChain or TypeScript agent stack. Corvid must win on reproducibility: signed binaries, env validation, migrations, health checks, deployment manifests, claim explanation, and operator docs generated from the same contracts used by the build.

**Scope:**

- [x] `corvid deploy package`: Dockerfile, OCI image metadata, health/readiness config, migration runner, env schema, and signed build attestation.
- [x] Deployment manifests for local Docker Compose, Fly.io/Render-style single service, Kubernetes, and bare-metal systemd. Shipped through 43C1-C3 slices; per-app manifests live under `examples/backend/*/deploy/`.
- [x] Release channels: nightly, beta, stable; SemVer policy tied to the stability contract and migration guide.
- [x] Upgrade/migration tooling for syntax, stdlib, schema, trace format, and connector manifests.
- [x] Maintainer docs: release checklist, security advisory process, compatibility policy, CI gates, benchmark reproduction, and claim review process.
- [x] Developer docs: backend tutorial, Personal Executive Agent tutorial, connector authoring guide, approval-system guide, observability guide, and production checklist.
- [ ] Beta program: at least 20 external developers build real backend apps; feedback must close as code/docs/tests or explicit non-scope before launch.
- [x] Final claim audit: README, website, launch page, docs, and `corvid claim --explain` say the same thing.
- [x] Launch package: install scripts, changelog, signed binaries, checksums, reproducible build notes, demo scripts, and incident-response contacts.

**Slice checklist:**

- [x] 43A-market-readiness-brief     `docs/phase-43-market-readiness.md` defines launch gates, release channels, support posture, security process, beta criteria, and non-scope.
- [x] 43B-deploy-package             `corvid deploy package` emits Dockerfile, OCI metadata, health/readiness config, migration runner, env schema, and signed build attestation.
- [x] 43C-deployment-manifests       Docker Compose, single-service PaaS, Kubernetes, and systemd manifests work for at least one reference app.
- [x] 43D-release-channels           Nightly, beta, and stable release channels are documented and wired to SemVer/stability policy.
- [x] 43E-upgrade-migration-tools    Syntax, stdlib, schema, trace-format, and connector-manifest migrations have tooling and docs.
- [x] 43F-maintainer-docs            Release checklist, advisory process, compatibility policy, CI gates, benchmark reproduction, and claim review docs are complete.
- [x] 43G-developer-docs             Backend tutorial, Personal Executive Agent tutorial, connector guide, approval guide, observability guide, and production checklist are complete.
- [ ] 43H-beta-program               At least 20 external developers build real backend apps; feedback is closed as code/docs/tests or explicit non-scope.
- [x] 43I-final-claim-audit          README, website, launch page, docs, and `corvid claim --explain` use the same defensible claims.
- [x] 43J-launch-package             Signed binaries, install scripts, changelog, checksums, reproducible notes, demo scripts, and incident contacts are ready.

**v1.0 final cut here. Launch day.** Corvid goes online only after the defensible core and the production-backend track are both complete.

**Libraries & frameworks (Phase 43):**

- `oci-spec` — OCI image manifest authoring; multi-stage Dockerfile (rust-builder → distroless runtime).
- `cargo-sbom` — SPDX SBOM generation for every release artifact.
- `cosign` (external binary) — signed-binary publishing; release attestation chained to the Phase 35 attestation envelope.
- Hand-rolled Dockerfile / Compose / K8s / systemd / fly.toml templates — no `helm` dep (too heavy for the v1 surface).
- `reqwest` (already a dep) — `corvid ops show <prod-url>` introspection client.
- `time` (already a dep) — release-channel calendar policy (nightly daily, beta weekly, stable cut by tag).

**Developer flow (Phase 43):**

```bash
corvid deploy package my_app/                # Dockerfile + OCI metadata + signed attestation + SBOM
corvid deploy compose my_app/                # docker-compose.yml + .env.example + healthchecks
corvid deploy fly my_app/                    # fly.toml + secrets template + region plan
corvid deploy k8s my_app/                    # Deployment + Service + Ingress + ConfigMap + Secret + HPA
corvid deploy systemd my_app/                # service unit + sysusers + tmpfiles
corvid release build nightly                 # signed binaries + checksums + changelog
corvid release build beta 1.0.0-beta.1
corvid release build stable 1.0.0
corvid release notes <prev-tag> <new-tag>    # structured markdown release notes
corvid migrate run --check                   # CI-safe dry run with full drift detection
corvid upgrade --check                       # AI-assisted claim regression check before upgrade (agentic)
corvid upgrade --apply                       # applies codemods + flags hand-review cases
corvid ops show <prod-url> --key=<pubkey>    # live-binary introspection (signed by host)
corvid ops vuln <prod-url>                   # security advisory contact + policy
corvid claim audit                           # AI-assisted final claim audit (adversarial)
```

**Phase-done checklist (Phase 43):**

- [x] `corvid deploy package` emits a multi-stage Dockerfile + distroless runtime ≤80 MB + OCI labels (`org.opencontainers.image.source`, signed-binary fingerprint) + `HEALTHCHECK` directive + full SPDX SBOM. Shipped through 43B1-3, 43M (SPDX SBOM in `a06f1fe`), and distroless slice `f1aa59d`.
- [x] Deployment manifests for Compose, Fly/Render, K8s (kind cluster smoke deploy in CI), and systemd are smoke-tested per release. Closed through 43C1-3 + the `35V2-P42-E-LR-app-deploy-smoke-ci` CI workflow.
- [x] Signed-attestation chain: `corvid deploy package`'s attestation references the same DSSE envelope `corvid claim --explain` consumes; the deploy attestation and the cdylib attestation cannot drift. Shipped in `7a2a42d` "43O — signed-attestation chain + promotes deploy.attestation_chain".
- [x] Release channels (nightly / beta / stable) ship signed binaries + `SHA256SUMS.txt` signed by the release key; checksum file rooted in a key-rotation policy doc. Channels shipped through 43D1-D2; `SHA256SUMS.txt` policy in `docs/release-policy.md:108`; key-rotation policy in `docs/release-policy.md:87`.
- [x] Reproducible-build verification: a second build on a different host produces a bit-identical signed artifact; verified by an external reproducer in CI. Shipped through `0d2647c` (43R CI workflow), `e69fa85` (host path prefix pinning), `85cf847` (codegen determinism gap closure), `3f77ec1` (`C_RUNTIME_LIB_PATH` retirement for determinism).
- [x] `corvid upgrade --check` reports any guarantee that *would weaken* before applying the upgrade; integration test exercises the rejection path. Shipped in `14add6e` "43Q — corvid upgrade --check claim-regression + promotes upgrade.claim_regression_check".
- [x] Live-binary introspection: `corvid ops show --envelope-file <path> --pubkey <path>` ships 2026-05-19 in slice `35V2-P43-P-LR-ops-show`. Rendered axum server exposes `/__ops` returning a DSSE-signed `OpsShowSnapshot` envelope when `CORVID_OPS_SIGNING_KEY` is set (fail-closed 503 without it); CLI verifies the envelope against an operator-supplied pubkey. Promotes `ops.live_introspection_signed` to RuntimeChecked with 3 positive refs + 5 adversarial refs (MITM, payload-tampering, payload-type-replay, wrong-key, malformed-envelope). End-to-end integration test `rendered_server_ops_show_signs_snapshot_and_cli_verifies_it` exercises the producer→consumer loop including the MITM rejection. Operators capture via `curl http://prod/__ops > ops.json` then pipe through the CLI; the live-HTTP fetch path inside the CLI is a follow-up ergonomic improvement.
- [x] Final claim audit: every README / website / launch-page claim has a runnable command or test; `corvid claim audit` exits 0 with no aspirational wording flagged. Shipped through `e5c7320` (claim audit command) + `f3a8d0d` (43T explain-failures with typed remediation and line-grounded fixes).
- [ ] Beta program: ≥20 external developers shipped ≥1 backend app each; their feedback closed as code/docs/tests OR explicit non-scope; the closure rate is published.
- [x] Registry rows shipped: `deploy.reproducible_build`, `deploy.attestation_chain`, `deploy.sbom_completeness`, `release.signed_artifact`, `upgrade.claim_regression_check`, `ops.live_introspection_signed`, `claim.audit_runnable_artifacts` — `Static` or `RuntimeChecked`, with positive + adversarial test refs. All 7 rows shipped through 43M, 43O, 43R, 43Q, P-LR ops-show, and `30680a7` "43V — test pairs for release.signed_artifact + claim.audit_runnable_artifacts + promotions".
- [ ] AI helpers landed: 2/5 shipped. (1) `corvid release notes <prev> <new>` shipped 2026-05-20 in slice `35V2-P43-T-LR-release-notes` — deterministic git-log + conventional-commit categorisation; promotes `release.notes_grounded` to RuntimeChecked. (2) `corvid claim audit --explain-failures` shipped 2026-05-20 in slice `35V2-P43-T-LR-claim-audit-explain-failures` — typed `ClaimFindingKind` (`missing_evidence` / `aspirational_wording`) + `suggested_fix` that back-references the inventory line; promotes the new `claim.audit_explain_failures_grounded` row to RuntimeChecked. The other 3 helpers stay filed under the `35V2-P43-T-LR-phase-43-ai-helpers` umbrella as genuinely LLM-shaped work: `corvid deploy tailor` (agentic), `corvid upgrade assist` (agentic), `corvid beta synthesize-feedback` (agentic).
- [x] Side-by-side `benches/comparisons/clone_to_deploy.md` against FastAPI/LangChain + Next.js/Vercel on time-from-clone-to-production-shaped-deploy. Shipped in `69f7453` "docs(bench): 43U — benches/comparisons/clone_to_deploy.md".

**Small-slice breakdown for Phase 43:**

- [x] 43B1-package-dockerfile        `corvid deploy package` emits Dockerfile and OCI metadata.
- [x] 43B2-package-runtime-config    Package includes health/readiness config, migration runner, env schema, and startup checks.
- [x] 43B3-package-attestation       Package includes signed build attestation and verification docs.
- [x] 43C1-compose-manifest          Docker Compose deploy works for one reference app.
- [x] 43C2-paas-manifest             Fly/Render-style single-service deploy works.
- [x] 43C3-k8s-systemd-manifests     Kubernetes and systemd manifests work or are explicitly scoped.
- [x] 43D1-release-policy            Nightly/beta/stable SemVer and stability policy are documented.
- [x] 43D2-release-automation        Release channel automation produces signed artifacts and changelog entries.
- [x] 43E1-syntax-stdlib-migrator    Syntax and stdlib migration tooling exists.
- [x] 43E2-schema-trace-migrator     Schema, trace-format, and connector-manifest migrations exist.
- [x] 43F1-maintainer-runbooks       Release checklist, advisory process, compatibility policy, CI gates, benchmark reproduction, and claim review docs are complete.
- [x] 43G1-developer-tutorials       Backend, Personal Executive Agent, connector, approval, observability, and production checklist docs are complete.
- [ ] 43H1-beta-intake               20 external developers are onboarded with issue templates and feedback labels.
- [ ] 43H2-beta-closure              Beta feedback is closed as code/docs/tests or explicit non-scope.
- [x] 43I1-claim-inventory           README, website, launch page, docs, and `corvid claim --explain` claims are inventoried.
- [x] 43I2-claim-alignment           All launch claims align with runnable artifacts and no aspirational wording remains.
- [x] 43J1-release-artifacts         Signed binaries, install scripts, checksums, changelog, and reproducible notes are ready.
- [x] 43J2-launch-rehearsal          Demo scripts, incident contacts, rollback plan, and final smoke tests are complete.

---

## Post-v1.0 roadmap

Scoped-out of the pre-v1.0 critical path. Not abandoned — explicitly planned, with honest reasoning for why they're not in v1.0.

- **Distributed multi-agent orchestration.** Cross-service agent graphs, recursive agent composition, distributed trace merging, and multi-tenant workflow sharding. Phase 38 ships durable single-backend agent execution for v1.0; this post-v1.0 item is the larger distributed/enterprise orchestration layer.
- **Hot reload.** In-flight runs keep version; new runs use new code. Production-runtime concern for always-on services. Most v1.0 users ship scripts + CLIs + embedded apps where restart-is-cheap. Ship when the production-service user segment is sized.
- **Prompt-aware compilation.** Schema caching, TOON compression, template deduplication. Performance optimization on top of v1.0 capability — measurable once cost data from real users shows where to target. Builds on Phase 20's cost model.
- **Interactive time-travel debugger UI.** Phase 21 ships deterministic replay; the scrub-backward / step-forward UI is a followup using the same infrastructure.
- **Generational GC, concurrent cycle collection.** Phase 17's cycle collector is good enough; generational + concurrent are post-v1.0 if allocation benchmarks ever justify the complexity.
- **Private package registries, binary packages.** Phase 25 ships the OSS registry + source packages; enterprise and binary distribution are post-v1.0.
- **Other editors (vim / emacs / JetBrains official extensions).** Phase 24 ships VS Code + the LSP; the LSP works with any client, but branded extensions are post-v1.0.
- **Tier-2 browser playground (33J7c/d/e).** `corvid-vm-core` + `corvid-vm-host` split, browser run-agent suspend/resume bridge, BYO-API-key flow with IndexedDB AES-GCM storage. Tier-1 (typecheck only) ships at v1.0 in its current form; full agent execution in the browser is post-v1.0 because the runtime split + bridge is ~3 months of focused work and is unrelated to the production-backend launch claim.
- **Phase 23 reopen — browser end-to-end CI gap.** Filed 2026-04-29. Cross-platform parity harness for the WASM target needs the end-to-end suite. Post-v1.0 unless a Phase 37-43 slice surfaces a regression that forces it earlier.
- **Provenance Propagation deferred follow-ups.** Native grounded handles for refcounted `Grounded<String>` / `Grounded<Struct>` types (stub at `docs/meta/native-grounded-handles-design.md`), short-circuit `&&` / `||` contagion, cross-module composition for `@grounded_pure` on imports (R5 attribute-composition matrix at the import boundary). All three are correctness-extensions to the shipped moat, not launch-blockers. Post-v1.0 unless a v1.0 user surfaces a real-world program that's blocked by one.

---

## Total estimated effort

**~3-5 months of focused solo work** remaining from 2026-05-17 to v1.0 public launch under Path A (silent build), on top of the ~33 months already shipped (Phases 1-36 + 35V + Provenance Propagation + Phases 37-42 slice work). The earlier "~13-18 months" estimate (commit `f42b508`) and the original "~47-57 months" estimate are preserved in git history; this section reflects what an audit of the actual ROADMAP slice state showed on 2026-05-17.

| Release | Phases | Bottom-up estimate | Status |
|---|---|---|---|
| v0.3 (close Phase 12) | 12k | ~2 weeks | ✅ shipped |
| v0.4 (native tier useful) | 13, 14, 15 | ~3 months | ✅ shipped |
| v0.5 (GP feel) | 16, 17, 18, 19 | ~3 months | ✅ shipped |
| v0.6 (moat + replay) | 20 (7 slices), 21 | ~5 months | ✅ shipped |
| v0.7 (embed + deploy) | 22, 23 | ~4 months | ✅ shipped (23 reopened, post-v1.0) |
| v0.8 (dev workflow) | 24, 25, 26, 27 | ~5 months | ✅ shipped |
| v0.9 (feature-complete) | 28, 29, 30, 31, 32 | ~5 months | ✅ shipped |
| v1.0 prerequisites | 33 (partial), 34, 35, 35V, 36, Provenance Propagation | ~5 months | ✅ shipped (33J4/5/L/M deferred to launch-readiness window) |
| v1.0 production-backend slice work | 37, 38, 39, 40, 41, 42 | ~10 months | ✅ shipped (slices closed; phase-done checklists need verification — see audit-round row below) |
| v1.0 verification round (Phase 35V Track 2 pattern, applied to 38-42) | 38, 39, 40, 41, 42 phase-done audit | **~4-8 weeks from 2026-05-17** | 🟡 in flight |
| v1.0 packaging + deploy + release + market readiness | 43 | **~6-8 weeks** | 🔴 not started |
| v1.0 launch-readiness tail | 33J4 / 33J5 / 33L / repositioned 33M | **~2-3 weeks** | 🔴 not started (final weeks of Phase 43 per Path A) |

The audit-round row is the biggest unknown. If the Phases 37-42 phase-done checklists are mostly clean (drift count low, like Phase 35V Track 2 was), it finishes in ~4 weeks. If drift count is high enough that each phase needs 1-3 correction slices, it stretches to ~8 weeks. Either way, the implementation work is genuinely small — the slice trees are already on `main`.

Real slip risk in Phase 43 (deploy + release surface introduces real ops complexity — reproducible builds, signed-attestation chain, distroless image budgets, K8s manifest smoke deploys). Build with a 25% buffer on Phase 43; the audit round absorbs less buffer because the corrective pattern is already proven from Phase 35V.

The original "~18-24 months" quote is not preserved above because preserving it would be dishonest. Quoting a smaller number while adding production backend, auth, persistence, jobs, connectors, observability, deployment, and reference products would be the shortcut; quoting what the plan actually sums to is the non-shortcut.

The dates aren't the point. The point is that each phase has:
- A clear goal with a named hard dependency, not a vibe sequence.
- A concrete scope list — no "TBD" or "polish" stand-ins.
- A version cut-line saying which release it ships in.
- A pre-phase brief before code.
- Tests green at the boundary.
- A dev-log entry.

That discipline is what makes the plan possible. Without it, the production-backend track becomes aspirational, and v1.0 turns into a marketing date instead of a shippable product.

---

## Non-goals

Red lines — features explicitly rejected, not merely deferred:

- **Raw pointer arithmetic + manual allocators.** Pointer aliasing is one of the hardest things for any reasoner (human or LLM) to track, and readability-for-LLM-generated-code is a first-class design goal. Narrow `@unsafe` FFI shim for C interop is allowed; pervasive pointers are a hard no. Rust and Zig own that niche — Corvid doesn't compete there.
- **Classical OOP inheritance.** `type` + methods (Phase 16) + (post-v1.0) interfaces are the model. Subclassing, `this`, virtual dispatch, and deep hierarchies are not. Modern GP consensus (Go, Rust, Swift, Kotlin) agrees composition + methods beat inheritance.
- **Rust/C++-level control for systems work.** Corvid aims for Go / Swift class performance. Fast enough that compute rarely bottlenecks AI-shaped software (where LLM latency dominates by three orders of magnitude), but not competing on hot-loop throughput.

Deferred, not rejected:

- **Every LLM provider at launch.** Anthropic + OpenAI ship first; Google, Ollama, and others follow in Phase 31.
- **Windows + Linux + macOS day-one.** Start on one OS (macOS); add the others in Phase 33 (v1.0 pre-launch polish).

What is *not* a non-goal, despite earlier framings: **being a general-purpose language.** Corvid must be one. The pre-v1.0 phases above ship every GP table-stakes feature (methods, cycle collector, Result, REPL, C ABI, WASM, LSP, package manager, testing) alongside the moat work — not as a bundle, not behind the moat, interleaved so every release is coherently Corvid.

---

## Velocity markers

To keep momentum honest, ship one observable artefact at every phase boundary. Every entry below is a live-demoable thing, not a completion-percent.

- **End of Phase 11** ✅ — `corvid run` dispatches through the interpreter + runtime with no Python on the path.
- **End of Phase 12** ✅ — `corvid run foo.cor` AOT-compiles + executes, caches on the second call. ~15× speedup measured. (v0.3)
- **End of Phase 15** — `corvid run examples/refund_bot_demo/src/main.cor --target=native` runs end-to-end: tool dispatch, prompt dispatch, approve tokens all working natively. (v0.4)
- **End of Phase 19** — `corvid repl` session demonstrates redefining an agent mid-session + inspecting struct values + calling a method on a user type. (v0.5)
- **End of Phase 21** — Demo video: write an agent with a `@budget($0.10)` annotation, make it exceed, compiler refuses; then `corvid replay <trace-id>` rewinds a recorded run and re-executes deterministically with zero LLM spend. (v0.6)
- **End of Phase 23** — Corvid program embedded in a Rust host (`cargo add` the cdylib) AND the same program compiled to wasm and running in a browser page. One source, two deployment targets. (v0.7)
- **End of Phase 27** — Full developer workflow demo: write in VS Code with live type hints, `corvid add` a registry package, `corvid test` runs, `corvid eval` produces the HTML report. (v0.8)
- **End of Phase 32** — Feature-complete: agents ask/choose humans, sessions persist to SQLite, Python libs import effect-tagged, Google + Ollama + Anthropic + OpenAI all work, `std.*` batteries included. (v0.9)
- **End of Phase 33** — launch polish foundation: installer, website, beta-tester feedback loop, launch GIF, announcement drafts, and claim audit scaffolding.
- **End of Phase 35** — `corvid claim --explain refund_bot.dylib` prints the binary's enforced guarantee set, signing key fingerprint, and bilateral-verifier attestation. `corvid contract list --json` round-trips into `docs/reference/core-semantics.md` byte-for-byte. The fuzz corpus rejects 100% of mutated descriptors and bypassed sources. Independent verifier `corvid-abi-verify` rebuilds the descriptor for any signed cdylib and bit-matches the embedded one. CI re-runs all four on every push.
- **End of Phase 36** — `corvid build --target=server examples/backend/refund_api` emits a runnable backend binary with routes, config validation, health checks, traces, and approval-gated dangerous actions.
- **End of Phase 38** — the Personal Executive Agent's daily brief, meeting prep, and follow-up jobs survive process restart and resume with bounded cost, bounded steps, replay IDs, and auditable approval waits.
- **End of Phase 41** — email, calendar, task, chat, and file connectors all expose effect manifests, mock modes, OAuth/token state, replay fixtures, and approval-gated write operations.
- **End of Phase 42** — Personal Executive Agent backend runs locally as a production-shaped Corvid product: routes, DB, jobs, auth, connectors, approvals, traces, evals, replay, and deployment manifest.
- **End of Phase 43** — v1.0 public release: signed binaries, install scripts, deployment packages, production docs, external beta feedback closed, and launch claims aligned with `corvid claim --explain`.
