# Launch Claim Audit

> **Charter.** Every launch-facing claim points at a runnable command, a
> committed test, or a tracked external dependency. Zero aspirational
> wording. Last re-audit: **2026-06-04** (v1.0 launch criterion
> [`ROADMAP.md` L49](../../ROADMAP.md)).
>
> The audit is the canonical cross-reference between the **moat
> claims** in
> [`docs/reference/inventions.md`](../reference/inventions.md) +
> [`docs/reference/core-semantics.md`](../reference/core-semantics.md),
> the **production-backend surface** shipped across Phases 36-43, the
> **per-app maturity** coverage closed in the Phase 35V2 LR track,
> and the **launch infrastructure** shipped in the 43-letter
> closer slices. Each row carries the runnable command the public
> can replay against `main`, the test or doc that's the evidence
> file, and the explicit non-scope.

## How this is enforced

| Claim | Evidence |
|---|---|
| 22-row Proof Matrix is canonical for the moat | [`inventions.md` Proof Matrix](../reference/inventions.md#proof-matrix) + drift gate `corvid-guarantees::render::tests::rendered_markdown_matches_committed_doc` |
| `corvid claim audit` walks this doc + refuses 0-exit on aspirational wording | `corvid claim audit --explain-failures` (43I `e5c7320` + 43T `f3a8d0d`) returns typed `ClaimFindingKind` + `suggested_fix` (`claim.audit_explain_failures_grounded` promoted to `RuntimeChecked`) |
| Per-app `apps/<name>/CLAIM.md` matches the signed cdylib's `claim --explain` | `35V2-P42-G-LR-per-app-claim-files` + CI gate re-runs `corvid claim --explain` and diffs against the committed file |
| Signed-attestation chain: deploy attestation = cdylib attestation envelope | 43O `7a2a42d` — `corvid deploy package`'s attestation references the same DSSE envelope `corvid claim --explain` consumes |
| Reproducible build: second-host bit-identical artifact | 43R `0d2647c` + `reproducible-build.yml` + determinism patches (`e69fa85`, `85cf847`, `3f77ec1`) |
| `corvid upgrade --check` rejects upgrades that weaken any guarantee | 43Q `14add6e` — promotes `upgrade.claim_regression_check` to `RuntimeChecked`, integration test exercises the rejection path |

## 1. Moat — safety-at-compile-time claims

Source-of-truth: [`docs/reference/inventions.md`](../reference/inventions.md) §1 + the Proof Matrix.

| Claim | Runnable command | Evidence | Explicit non-scope |
|---|---|---|---|
| Approve-before-dangerous boundaries are compiler-visible | `corvid tour --topic approve-gates` | `crates/corvid-types/src/lib.rs` + `crates/corvid-types/tests/source_bypass_corpus.rs` | Proves the boundary exists, not the approval quality. |
| Dimensional effects compose through declared algebra | `corvid tour --topic dimensional-effects` | `crates/corvid-types/src/effects.rs` + `effects-spec/02-composition-algebra.md` | Proves declared contracts, not provider honesty. |
| `Grounded<T>` carries source linkage through composition | `corvid tour --topic grounded-values` + `corvid trace dag <trace>` | `crates/corvid-types/src/effects/grounded.rs` + `tests/corpus/combined_all.cor` + `tests/corpus/legacy_grounded_coercion.cor` | Proves source linkage, not source truth. |
| Strict citations are checked at the prompt boundary | `corvid tour --topic strict-citations` | `crates/corvid-vm/src/tests/dispatch.rs` + `effects-spec/05-grounding.md` | Checks citation evidence, not factual correctness. |
| Compile-time budgets reject budget-busting programs | `corvid tour --topic cost-budgets` | `crates/corvid-types/src/effects/cost.rs` + `effects-spec/07-cost-budgets.md` | Static declared costs, not invoice reconciliation. |
| Confidence gates fail-closed below threshold | `corvid tour --topic confidence-gates` | `crates/corvid-types/src/tests.rs` + `effects-spec/06-confidence-gates.md` | Depends on calibrated adapter confidence. |
| `@grounded_pure` forbids laundering inside a body | `corvid tour --topic provenance-propagation` | `crates/corvid-types/src/tests.rs` (`grounded_pure_*`, `grounded_coercion_*`) + [`docs/meta/grounded-propagation-design.md`](./grounded-propagation-design.md) | Operator owns trust in the upstream retrieval source. |

## 2. Moat — AI-native ergonomics + replay

| Claim | Runnable command | Evidence | Explicit non-scope |
|---|---|---|---|
| AI-native keywords are first-class language constructs | `corvid tour --topic language-keywords` | `crates/corvid-syntax/src/parser/tests.rs` + structural drift gate at `crates/corvid-syntax/tests/grammar_drift.rs` (slice 33J6) | Does not replace ordinary general-purpose code. |
| Replay is deterministic + auditable | `corvid replay <trace> --source <file>` | `effects-spec/14-replay.md` + `crates/corvid-cli/tests/bundle_verify.rs` | Receipts are observed evidence, not full formal verification. |
| Trace-aware evals run against committed trace corpora | `corvid eval` | `crates/corvid-types/src/lib.rs` + `effects-spec/12-verification.md` | Full eval-runner ergonomics ship in `corvid eval-drift` / `corvid eval-from-feedback` (AI-assisted; both are `Grounded<T>`-output helpers). |
| Replay quarantine for durable jobs | `corvid tour --topic replay-quarantine` | `crates/corvid-runtime/tests/replay_quarantine_corpus.rs` (4 adversarial + 4 positive/negative-control cases per side-effect surface, slice 35V2-P38-C-6) | Quarantine fails-closed on unknown side-effect class; future side-effect surfaces require new corpus coverage. |

## 3. Moat — adaptive routing + streaming + verification

(All shipped — [`inventions.md`](../reference/inventions.md) Proof Matrix rows for Typed Model Routing, Progressive Refinement, Ensemble Voting, Jurisdiction & Privacy Routing, Streaming Effects, Progressive Structured Streams, Typed Stream Resumption, Declarative Fan-Out / Fan-In, Proof-Carrying Dimension Registry, Adversarial Bypass Testing.)

The launch claim audit defers to the Proof Matrix for these 10 rows rather than re-listing them here — the matrix is the canonical source and the drift gate keeps it honest.

## 4. Production backend surface — Phases 36–41

Each of these claims maps to a shipped Phase entry whose phase-done checklist is ticked in [`ROADMAP.md`](../../ROADMAP.md) and a closing audit doc lives in [`docs/phases/phase-NN-audit-2026-05-17.md`](../phases/).

| Claim | Runnable command | Evidence | Explicit non-scope |
|---|---|---|---|
| HTTP server with typed routes, JSON encode/decode, env config (Phase 36) | `corvid build --target=server <file>` then `./target/release/<app>` | [`docs/phases/phase-36-backend-core.md`](../phases/phase-36-backend-core.md) + `crates/corvid-cli/tests/serve_smoke.rs` | First-impression-gap risks documented at [[project-phase-20l-closed]]. |
| HTTP approval queue: POST → 202 + approval id, reviewer transitions (slices `35V2-P42-E0-serve-5` + `serve-6`) | `corvid serve <file>` then POST to an approval-gated route → `202 + {"approval_id"}`, POST `/__approvals/<id>/approve` → re-executed result, POST `/__approvals/<id>/deny` → drop pending | `crates/corvid-cli/tests/serve_smoke.rs::approval_gated_post_answers_202_and_admin_endpoint_lists_the_pending_id` + `approval_transition_endpoints_approve_re_executes_and_deny_drops_pending` | Per-request reviewer auth (mTLS / OAuth / session) deferred to a Phase 39-shape slice; multi-step approval chains deferred; persistent approval DB deferred. |
| Persistence with typed records, migrations, and audit log (Phase 37) | `corvid migrate up`, `corvid migrate status`, `corvid migrate down` + `corvid audit <file>` | [`docs/phases/phase-37-persistence.md`](../phases/phase-37-persistence.md) + `std.db` integration tests | Production HA / sharding / read-replica routing is operator concern. |
| Durable jobs survive SIGKILL with replay quarantine (Phase 38) | `corvid jobs run` + `corvid jobs schedule add/list/recover` + `corvid jobs explain` | [`docs/phases/phase-38-audit-2026-05-17.md`](../phases/phase-38-audit-2026-05-17.md) + `t38l_d3_checkpoints_survive_unclean_shutdown` + `crates/corvid-runtime/tests/replay_quarantine_corpus.rs` | Distributed-jobs scheduling across nodes is operator concern. |
| Auth: sessions, API keys, per-tenant, per-role + approval contracts (Phase 39) | `corvid auth` + `corvid approvals` | [`docs/phases/phase-39-audit-2026-05-17.md`](../phases/phase-39-audit-2026-05-17.md) | Identity provider integration (OIDC / SAML) is host concern. |
| Observability: trace assertions, eval dashboards (Phase 40) | `corvid observe` + `corvid eval-drift` (AI-assisted, `Grounded<T>` output) + `corvid eval-from-feedback` | [`docs/phases/phase-40-audit-2026-05-17.md`](../phases/phase-40-audit-2026-05-17.md) | OTel exporter selection is operator concern; SLO definition is product concern. |
| Connectors: mock + replay + real provider modes share one typed surface (Phase 41) | `corvid connectors` + `CORVID_PROVIDER_LIVE=1` for real mode | [`docs/phases/phase-41-audit-2026-05-17.md`](../phases/phase-41-audit-2026-05-17.md) + `crates/corvid-runtime/tests/connector_drift_corpus.rs` | `connector ... grounded` source-syntax sugar deferred to post-v1.0 (`35V2-P41-I`). |
| `#[tool]` accepts struct params/returns at the JSON-wrapper boundary (slice `35V2-P42-G0-tools-3b`) | `cargo test -p corvid-macros --test expand` | `crates/corvid-macros/tests/expand.rs::struct_signature_tools_register_in_inventory_with_empty_symbol_marker` + `user_struct_signature_fns_still_callable_directly` + `scalar_signature_tools_keep_typed_wrapper_symbol` | Native-binary direct-call of a struct-signature tool intentionally fails to link (clean linker error rather than wrong-ABI miscompilation); cdylib targets work through the registry path. |

## 5. Per-app maturity — Phase 42 reference apps

All 5 apps closed via the `35V2-P42-D-LR-app-maturity-*` track (closed 2026-05-27/28). Each app commits a signed `apps/<name>/CLAIM.md` with the runnable command + the regeneration command in the header.

| Claim | CLAIM.md | Per-app audit doc | AI helpers |
|---|---|---|---|
| Personal Executive Agent: maturity bar closed | `apps/pea/CLAIM.md` | [`phase-42-pea-maturity-2026-05-27.md`](../phases/phase-42-pea-maturity-2026-05-27.md) | `corvid app boot-summary` + `corvid app adversarial-refresh` + `corvid app pr-describe` |
| Personal Knowledge Agent: maturity bar closed | `apps/pka/CLAIM.md` | [`phase-42-pka-maturity-2026-05-28.md`](../phases/phase-42-pka-maturity-2026-05-28.md) | `corvid app boot-summary` + `corvid app adversarial-refresh` + `corvid app pr-describe` |
| Finance Operations Agent: maturity bar closed | `apps/finance/CLAIM.md` | [`phase-42-finance-maturity-2026-05-28.md`](../phases/phase-42-finance-maturity-2026-05-28.md) | `corvid app boot-summary` + `corvid app adversarial-refresh` + `corvid app pr-describe` |
| Customer Support Agent: maturity bar closed | `apps/customer-support/CLAIM.md` | [`phase-42-customersupport-maturity-2026-05-28.md`](../phases/phase-42-customersupport-maturity-2026-05-28.md) | `corvid app boot-summary` + `corvid app adversarial-refresh` + `corvid app pr-describe` |
| Code Maintenance Agent: maturity bar closed | `apps/code-maintenance/CLAIM.md` | [`phase-42-codemaintenance-maturity-2026-05-28.md`](../phases/phase-42-codemaintenance-maturity-2026-05-28.md) | `corvid app boot-summary` + `corvid app adversarial-refresh` + `corvid app pr-describe` |

Per-app benchmark comparisons in `benches/comparisons/<app>.md` (governance line counts are real + machine-checked by `each_reference_app_has_a_benchmark_comparison_file`; baseline cells are `bounty-open` per the no-fabricated-numbers honesty rule).

## 6. Launch infrastructure — Phase 43

| Claim | Runnable command | Evidence (closing commit) | Explicit non-scope |
|---|---|---|---|
| `corvid deploy package` emits distroless image ≤ 80 MB + OCI labels + SPDX SBOM + HEALTHCHECK | `corvid deploy package <app> --out <dir>` | 43M (`a06f1fe`) + distroless slice (`f1aa59d`) | SBOM completeness audit (transitive native deps) — the registry row `deploy.sbom_completeness` records the depth limit honestly. |
| Deployment manifests for Compose / Fly / Render / K8s / systemd smoke-deploy in CI | `cargo test -p corvid-cli --test serve_smoke` + `app-deploy-smoke.yml` | 43C1-C3 + `35V2-P42-E-LR-app-deploy-smoke-ci` | kubeconform schema validation deferred (skipped to avoid network-fetch flakiness in CI). |
| Signed-attestation chain: deploy attestation = cdylib attestation envelope | `corvid claim --explain <cdylib>` references the same DSSE payload `corvid deploy package` emits | 43O (`7a2a42d`) — promotes `deploy.attestation_chain` to `RuntimeChecked` | Key-rotation procedure documented at [`docs/release-policy.md:87`](../release-policy.md). |
| Release channels (nightly / beta / stable) ship signed binaries + `SHA256SUMS.txt` rooted in a key-rotation policy doc | `corvid release build nightly --out <dir>` / `beta` / `stable` | 43D1-D2 + `docs/release-policy.md:87/108` + `30680a7` (43V — promotes `release.signed_artifact` to `RuntimeChecked` with 5 adversarial tests including MITM, payload-tampering, payload-type-replay, wrong-key, malformed-envelope) | Per-release key signing ceremony is operator concern. |
| Reproducible-build verification: second-host bit-identical signed artifact | `cargo test -p corvid-cli reproducible_build` + the `reproducible-build.yml` CI workflow | 43R (`0d2647c`) + host-path-prefix pinning (`e69fa85`) + codegen determinism (`85cf847`) + `C_RUNTIME_LIB_PATH` retirement (`3f77ec1`) | External reproducer in a foreign CI deferred to launch-readiness. |
| `corvid upgrade --check` reports any weakening guarantee before applying | `corvid upgrade check <app> --json` | 43Q (`14add6e`) — promotes `upgrade.claim_regression_check` to `RuntimeChecked` | Source-code migration auto-application has hand-review gates; semantics-changing migrations fail-closed. |
| `corvid claim audit` exits 0 with no aspirational wording | `corvid claim audit --json` + `corvid claim audit --explain-failures` | 43I (`e5c7320`) + 43T (`f3a8d0d`) | This audit doc is itself the input. |
| Live-binary introspection: signed `/__ops` snapshot | `curl http://prod/__ops > ops.json && corvid ops show --envelope-file ops.json --pubkey deploy.pub` | `35V2-P43-P-LR-ops-show` — promotes `ops.live_introspection_signed` to `RuntimeChecked` with 3 positive + 5 adversarial refs | Continuous polling / dashboard tooling is operator concern; the CLI verifies one envelope at a time. |
| Side-by-side benchmark archive | `corvid bench compare python|js` | 43U (`69f7453`) — `benches/comparisons/clone_to_deploy.md` | Baseline cells `bounty-open` per the no-fabricated-numbers rule. |

## 7. AI helpers shipped — purpose, grounding, and Grounded<T> contract

| Claim | What it does | Grounded source | Closing slice |
|---|---|---|---|
| `corvid jobs explain` is `Grounded<T>` | Typed classifier over `job audit-event trail`; sources every assertion to an audit-event id | `Grounded<T>` `sources` array names every event id consulted | `35V2-P38-G-LR-corvid-jobs-explain-helper` |
| `corvid release notes <prev> <new>` is `Grounded<T>` | Deterministic `git-log` + conventional-commit categorisation | `Grounded<T>` `sources` array names every commit id | `35V2-P43-T-LR-release-notes` |
| `corvid claim audit --explain-failures` is `Grounded<T>` | Typed `ClaimFindingKind` + `suggested_fix` back-references inventory line | `Grounded<T>` sources name the inventory row | `35V2-P43-T-LR-claim-audit-explain-failures` |
| `corvid app boot-summary` is `Grounded<T>` | Typed `BootSummary` over the app's ABI descriptor; every derived field carries a `BootSource` entry | `Grounded<T>` `sources` per derived field | `35V2-P42-H-LR-1-app-boot-summary` |
| `corvid app adversarial-refresh` is `Grounded<T>` | Typed walker emitting one `AdversarialSuggestion` per `(surface, threat)` pair | `Grounded<T>` `sources` back-reference the descriptor field | `35V2-P42-H-LR-2-app-adversarial-refresh` |
| `corvid app pr-describe` is `Grounded<T>` | Typed `PrDescription` with `Breaking` / `Additive` / `Informational` sections covering agents, tools, approvals, types, stores, claim guarantees | `Grounded<T>` `sources` back-reference the descriptor diff | `35V2-P42-H-LR-3-app-pr-describe` |
| `corvid eval-drift` is `Grounded<T>` | AI-assisted drift attribution across `(model / prompt / retrieval-index / input)` | `Grounded<T>` `sources` carry `(trace_id, span_id)` pairs | Phase 40 — `corvid eval-drift` |
| `corvid eval-from-feedback` is typed-fixture-shaped | AI-assisted eval fixture from `feedback JSON`, redacted via production policy | `corvid eval-from-feedback` emits a typed fixture file | Phase 40 |
| `corvid connector drift-narrator` is `Grounded<T>` | Typed narration over `(mock ↔ real)` drift report | `Grounded<T>` `sources` per change | Phase 41 — `connector.drift_narration_grounded` |

3 AI helpers stay filed under the Phase 43 umbrella `35V2-P43-T-LR-phase-43-ai-helpers` as genuinely LLM-shaped work pending the LLM-provider substrate slice (a post-v1.0 phase the audit doesn't claim ship): `corvid deploy tailor`, `corvid upgrade assist`, `corvid beta synthesize-feedback`.

## 8. Path-A launch-readiness status — honest gaps

Per the Path-A launch strategy in [`ROADMAP.md` L32-L42](../../ROADMAP.md), the following claims are **deferred-by-design** and remain blocked until the launch-readiness window opens in the final 2-4 weeks of Phase 43. The audit lists them so a public reader sees them as gaps rather than promises.

| Gap | Why it's blocked | When unblocked |
|---|---|---|
| `corvid-lang.org/benchmarks` benchmark page | **blocked: 33J4** — requires the website renderer in the `Micrurus-Ai/corvid-website` repo. The `benches/moat/RESULTS.md` + `benches/results/*/ratios.json` archives the page consumes ARE checked in. | Final 2 weeks of Phase 43 per Path A. |
| Launch blog post | **blocked: 33J5** — content + the website blog shell. Drafts not yet checked in. | Final 2 weeks of Phase 43 per Path A. |
| Launch GIF / video + announcement drafts | **blocked: 33L** — recording + external-reader review. | Final 2 weeks of Phase 43 per Path A. |
| Friends-and-family feedback closed | **blocked: 33M** (repositioned, Path A) — requires 5-10 hand-picked AI engineers building a small production-shape app on the v1.0 release candidate. | Final 4 weeks of Phase 43 per Path A. |
| `corvid deploy tailor` / `corvid upgrade assist` / `corvid beta synthesize-feedback` (3 of 5 Phase 43 AI helpers) | **blocked: LLM-provider-substrate phase** — genuinely LLM-shaped work pending a separate substrate; filed under the `35V2-P43-T-LR-phase-43-ai-helpers` umbrella. | Post-v1.0. |
| Browser-based cloud IDE (33J7c/d/e) | **non-scope** for v1.0 — Path B paused per user direction; runtime-split sub-slices 33J7b-3f/3g/3h also paused. | Post-v1.0 if Path A holds. |
| 17b-3 / 17b-4 / 17b-5 / 17b-6 RC-optimization passes | **non-scope** for v1.0 — Phase 17b explicitly conditional on "post-17b measurements show remaining allocation pressure justifies the complexity." | Post-launch if measurement justifies. |

## 9. Re-audit cadence

This doc is re-audited:

- After every Phase 35V2 LR slice closes (the closing commit MUST update relevant rows or the slice is incomplete).
- After every Phase 43 letter-slice closes (43M, 43O, 43Q, 43R, 43T, 43U, 43V each updated their row at close).
- Before the v1.0 cut, per launch criterion L49.
- Whenever a new genuinely-open slice ships that adds a public-facing claim — the audit-and-update slice that ticks it MUST update this doc in the same commit.

Mechanically enforced: `corvid claim audit` parses this doc + walks each command, refusing 0-exit on missing evidence or aspirational wording.
