# Phase 35V — Pre-Launch Verification Round

Comprehensive reconciliation of every `[x]` slice in the approaching-
launch surface. Modeled on the 2026-04-29 audit that found four
phase-done bullets in Phases 38–41 structurally absent (multi-worker
job runner, real JWT verification, OTel SDK conformance, real-mode
connector CLI), and on the Phase 20m verifier-correction pattern
that closes drift discovered by an independent verification round.

The 2026-04-29 audit only happened because one was overdue. Phase
35V exists so the next audit doesn't happen *during* launch.

## Why this phase exists

Phase 35 is the v1.0 launch gate. Every public claim Corvid will
make at launch traces through Phase 35's 14 slices (registry,
diagnostic tagging, contract-list CLI, spec generation, test cross-
references, ABI fuzz, source fuzz, bilateral verifier, claim-
explain, sign-refusal, security model, README alignment, CI gate,
claim-coverage extension). Every Phase 35 slice is currently
ticked `[x]`. None has had an independent verification pass.

The same was true of Phases 38, 39, and 41 before the 2026-04-29
audit. Those phases were ticked too. The audit found:

- **Phase 38**: multi-worker job runner missing; SIGKILL-mid-step
  crash-recovery test missing; 4-concurrent-worker idempotency
  test missing; DST-aware cron test missing.
- **Phase 39**: JWT verification was contract-shape only (no JWKS
  fetch, no `kid` resolution, no signature verification); top-level
  `corvid auth` / `corvid approvals` CLI absent.
- **Phase 41**: connector real-mode CLI surface absent.

Audit-correction tracks were filed (38K/38M/39K/39L/41K/41L/41M)
but their own completeness has not been independently re-verified.
And Phase 36's audit-correction slices (36K/36L/36M) shipped
during the same period.

The launch gate cannot be defended publicly until each of these is
verified clean by an independent pass that treats the optimistic
`[x]` as a *claim to disprove*, not a fact to trust.

## The verifier-correction pattern, applied to the launch surface

Phase 20m formalised this pattern for the 20l first-impression
gap repair: same external reviewer (or a fresh one) re-tests the
post-fix HEAD against the same methodology and produces a scorecard.
Corrections phase addresses verifier-confirmed corrections only.
Adjacent gaps surfaced during verification get filed as separate
follow-ups.

Phase 35V is the same shape applied to a wider surface: every
phase-done claim in the approaching-launch surface gets adversarial
re-verification, and every drift gets a corrective commit before
the phase closes.

## Three tracks, sequential

Discoveries in earlier tracks may shift the work in later ones, so
tracks run **strictly sequential**, not parallel.

### Track 1 — Phase 35 verification (the launch gate)

One verification slice per Phase 35 entry. Each slice's job:
treat the `[x]` as a claim to disprove. Run shipped behavior
against the stated criteria. If the slice passes verification,
add a sentinel regression test that pins the verified property
going forward. If the slice fails, file a corrective commit that
closes the drift — track the correction within Phase 35V (35V-T1-X
corrects the drift; Phase 35 itself does not reopen).

| Slice | Verifies | Independent check |
|---|---|---|
| 35V-T1-A | 35-A registry | Walk every `GUARANTEE_REGISTRY` row. Each `id`, `kind`, `class`, `phase`, `description`, `positive_test_refs`, `adversarial_test_refs` is well-formed. Each test ref resolves to a real `fn` in the workspace (compile-and-find). No row references a deleted test. |
| 35V-T1-B | 35-B diag tagging | Walk every contract-enforcing diagnostic site in resolve / typecheck / IR-lower / codegen / runtime. Confirm each carries a `guarantee_id`. Verify the build-time lint via mutation: temporarily strip a tag, confirm CI rejects. |
| 35V-T1-C | 35-C contract list | `corvid contract list` JSON output equals the registry programmatically (parse + compare, not visual). Human-readable output renders cleanly for every `kind` / `class` combination without panic. |
| 35V-T1-D | 35-D spec generation | Regenerate `docs/core-semantics.md` via the xtask. Bit-compare against committed file. Mutate the registry, verify CI fails on drift. Verify the drift-detection test exists and runs. |
| 35V-T1-E | 35-E test cross-refs | Every `Static` guarantee has ≥1 positive + ≥1 adversarial test ref. Each ref resolves to a real `fn` declaration. Adversarial: empty adversarial coverage on a Static row rejects at build time. |
| 35V-T1-F | 35-F ABI fuzz corpus | Corpus exists at the documented location. Mutant count is ≥100 per gate. Every mutant is rejected with the documented exit code. Benign mutations round-trip. The fuzz run actually exits with the expected status; not stubbed. |
| 35V-T1-G | 35-G source fuzz corpus | Corpus exists. AST mutators cover all four documented attack classes (`@approve` re-export bypass, effect under-reporting, `Grounded<T>` provenance loss, import aliasing). Each mutated source fails typecheck with the right `guarantee_id` from 35-B. |
| 35V-T1-H | 35-H bilateral verifier | `corvid-abi-verify` binary's dep tree does NOT transitively include the main pipeline's typechecker. Disagreement triggers build rejection (negative test). The independence claim isn't decorative. |
| 35V-T1-I | 35-I claim --explain | Output stable byte-for-byte across re-runs on the same source. Output references registry rows that exist and match the embedded descriptor. |
| 35V-T1-J | 35-J sign-refusal | Adversarial: declare an unregistered contract pattern in a source file; `corvid build --sign` rejects with the right diagnostic id. Verify the rejection path actually runs (not skipped behind a feature flag). |
| 35V-T1-K | 35-K security model | `docs/security-model.md` exists. TCB diagram references real components by file path. Threat model items each map to a registry row OR an explicit non-goal. No claims that overshoot what 35-H/35-I/35-J actually do. |
| 35V-T1-L | 35-L README alignment | Every README launch claim has a runnable command. Extract claims by hand-walk; trace each to a `corvid` command or test. Aspirational wording flags. |
| 35V-T1-M | 35-M CI gate | Read `.github/workflows/*.yml`. Verify the fuzz corpus + bilateral verifier + spec drift checks actually run on every push. Verify the workflow doesn't skip them on certain branches. |
| 35V-T1-N | 35-N claim coverage extension | All promoted rows present in registry. `validate_signed_claim_coverage` walks `Decl::Schedule` and `Decl::Server`. Adversarial test (signed build refuses unregistered Decl::Schedule target) actually exists and runs. |

### Track 2 — Audit-correction completeness (38/39/41/36)

The 2026-04-29 audit filed corrective slices for Phases 38, 39,
41. Phase 36's own audit-correction slices (36K/36L/36M) shipped
during the same period. None has had an independent re-verify.

| Slice | Verifies | Independent check |
|---|---|---|
| 35V-T2-A | 36-K real HTTP runtime | Hand-rolled request-line parser is gone. Production HTTP runtime/parser handles HTTP/1.1 edge cases (chunked transfer, trailers, malformed headers, header injection). Edge-case tests exist and run. |
| 35V-T2-B | 36-L middleware pipeline | Auth, rate-limit, tracing, CORS, compression, request logging, effect-aware policy middleware run in declared order. The "declared order" claim is verified by an integration test that asserts ordering. |
| 35V-T2-C | 36-M shutdown/timeout tests | Graceful shutdown, request timeout, body-limit, handler-isolation behavior is covered by integration tests. Each test produces a deterministic outcome (not flaky). |
| 35V-T2-D | 38-K multi-worker job runner | Multi-worker runner exists. ≥2 workers consume from the queue. Lease-stealing on worker death is exercised. |
| 35V-T2-E | 38-K SIGKILL crash-recovery | Test that `SIGKILL`s a worker mid-step and asserts byte-exact resume with no double-spend / double-side-effect. The test actually does the SIGKILL, not a mock. |
| 35V-T2-F | 38-K idempotency under concurrency | 4-concurrent-worker idempotency test exists. Verifies the same job-key never produces duplicate side-effects under contention. |
| 35V-T2-G | 38-M DST-aware cron | DST-aware cron test exists and exercises both spring-forward and fall-back. Cron firing is deterministic across DST boundaries. |
| 35V-T2-H | 39-K real JWT verification | JWKS fetch happens against a real key endpoint (mock + real provider). `kid` resolution chooses the right key. Signature verification rejects forged JWTs. Issuer-URL prefix + alg name + claim presence are *additional* checks, not the only ones. |
| 35V-T2-I | 39-L corvid auth/approvals CLI | `corvid auth` and `corvid approvals` exist as top-level subcommands. The tenant-scoped queue surface they document is wired (not just a JobsCommand::Approvals alias). |
| 35V-T2-J | 41-K connector real-mode CLI | `corvid connectors` exists. Real-mode connector flow exercises live OAuth (or documented test-double); not a stub returning canned data. |
| 35V-T2-K | 41-L connector contract drift | Mock ≡ replay ≡ real shared typed surface holds across connectors. Mutating the typed surface in mock mode breaks real-mode tests too. |
| 35V-T2-L | 41-M connector approval bypass | Connector's approval requirements survive when called through the connector wrapper (no bypass via direct tool call). |

### Track 3 — Closer commits for Phase 35 + Phase 36

After Tracks 1 + 2 close any drift, Phases 35 and 36 close formally
with the same ceremony Phase 20n got.

| Slice | What lands |
|---|---|
| 35V-T3-A | Phase 35 closer | Write `docs/phase-35-defensible-core.md` if absent; add closing audit covering all 14 slices + any drift Track 1 found. Tick `✅ closed` in ROADMAP. Append Phase 35 learnings to `learnings.md`. Write `memory/project_phase_35_closed.md`. Add MEMORY.md pointer. |
| 35V-T3-B | Phase 36 closer | Mirror of T3-A for Phase 36. Includes the audit-correction work Track 2 verified. |
| 35V-T3-C | Phase 38/39/41 audit-correction completeness re-confirmation | Update each phase's existing audit-correction note in ROADMAP with re-verification status. Phases 38/39/41 don't get formal closers in this phase (those are wider surfaces); the audit-correction columns get updated to "verified clean by 35V Track 2 on 2026-MM-DD." |
| 35V-T3-D | Phase 35V closer | Standard closer ritual: `✅ closed` marker on the phase header, closing audit appended to this doc, learnings entry, memory record `project_phase_35V_closed.md`. |

## Sequencing rules

Per CLAUDE.md "When splitting" and "Pre-phase chat mandatory":

- **Tracks run strictly sequential.** Track 2 doesn't start until
  every Track 1 slice has either passed verification or been
  closed via correction. Track 3 doesn't start until every Track 2
  slice has either passed or been closed.
- **One slice = one feature.** Each verification slice is its own
  commit, even when it passes (the sentinel regression test that
  pins the property is still a real artifact worth its own
  commit).
- **Validation gate between every commit:** `cargo check
  --workspace` + targeted `cargo test -p <crate>` + `cargo run
  -q -p corvid-cli -- verify --corpus tests/corpus` (Windows
  whoami linker baseline tolerated).
- **Push before starting the next slice.** No batching local.
- **Pre-phase chat per slice when drift is found.** Track 1's
  verification slices that find clean state can chain quietly;
  any slice that finds drift triggers a pre-phase chat on the
  corrective approach before code lands.
- **No autonomous chaining across tracks.** End of Track 1 →
  pre-phase chat on Track 2 scope. End of Track 2 → pre-phase
  chat on Track 3 closer ceremony.

## Drift-found vs no-drift handling

Every verification slice produces one of two outcomes:

- **Clean signal.** Verification confirms the slice's claim
  matches shipped behavior. The slice's commit adds a *sentinel
  regression test* that pins the verified property — this is the
  test that catches future drift if the property silently
  regresses. The commit message documents both the verification
  method and the property the sentinel pins.
- **Drift signal.** Verification finds the claim is wrong, partial,
  or stubbed. The slice's commit *closes the drift* — adds the
  missing implementation, fixes the wrong test, etc. — and the
  same commit adds the sentinel regression test that catches the
  drift mode going forward. Phase 35 itself does NOT reopen; the
  drift is corrected within Phase 35V.

This convention keeps the audit's blast radius bounded and avoids
the "audit reopens prior phases" pattern that would let drift
propagate unbounded across the ROADMAP.

## Phase-done criteria

- [ ] Every Track 1 slice (35V-T1-A through 35V-T1-N) lands with
  either a clean-signal sentinel test OR a drift-correction commit.
- [ ] Every Track 2 slice (35V-T2-A through 35V-T2-L) lands with
  the same shape.
- [ ] Track 3 closers land for Phase 35 and Phase 36, with phase
  docs (writing them if absent), learnings entries, memory records,
  ROADMAP `✅ closed` markers, and MEMORY.md pointers.
- [ ] Closing audit recorded in this document with per-slice
  status (verified-clean / drift-found-and-closed) and a summary
  of any cross-slice patterns discovered.
- [ ] `learnings.md` updated per slice that finds drift; the
  cross-slice rollup at end of phase records the pre-launch-audit
  pattern itself as a reusable shape for future launch-readiness
  rounds.
- [ ] ROADMAP.md Phase 35V entry checkboxes ticked, `✅ closed`
  marker added.
- [ ] Memory record `project_phase_35V_closed.md` summarises
  every drift found, the verification methodology, and the
  pattern's reusability for future audit rounds.

## Out of scope (explicit non-goals)

- **Forward engineering on Phases 37+.** Persistence track stays
  paused until Phase 35V closes.
- **Phase 33 launch polish (33J/33L/33M).** Website + WASM
  playground showcase a launch claim; that work waits until the
  claim is verified clean.
- **The whoami `__imp_GetUserNameExW` linker fix from Phase 20n.**
  File as a separate one-commit slice after Phase 35V closes.
  Does not gate launch.
- **Reopening Phase 35 itself.** Drift discovered in Phase 35
  slices is corrected within Phase 35V's own slice list, not by
  reopening Phase 35.
- **Bug-bounty program, third-party audit contract, formal
  launch comms.** Those belong to the final market-launch phase,
  not to this phase.

## What this phase does NOT promise

- **It does not eliminate the possibility of post-launch drift.**
  External reviewers can still find issues. What this phase
  guarantees is that the *current* claim has been independently
  re-verified once before launch.
- **It does not replace the ongoing CI gate.** The sentinel tests
  added by each verification slice catch the specific drift modes
  found; the CI workflow from 35-M continues to run on every push.
- **It does not perform formal verification of the type system.**
  Per Phase 35's own non-scope, formal mechanized proofs are
  post-v1.0 research. This phase verifies that the *engineering
  claims* shipped in Phase 35 are honest, not that the underlying
  language semantics are mechanically proven.

## Estimated scope

26 verification slices across three tracks plus 4 closer slices =
**~30 slices**. Realistic timeline: several weeks. Most slices are
small (verify-then-pin) but some (Track 1 fuzz corpus integrity,
Track 2 multi-worker runner crash recovery) may surface real drift
that fans out into corrective work.

The audit is sized to be paid in engineering cost rather than
reputation cost. Doing it after launch — when external eyes find
the drift — is the same engineering cost paid in worse currency.

## Pre-phase chat checklist

Before any verification slice starts, confirm:

1. **Phase 35V scope** — Track 1 + Track 2 + Track 3 as defined
   above. Adjustments welcome but should land before slice 35V-T1-A.
2. **Naming** — "Phase 35V" is the working name. Open to "Phase
   35.5", "Phase 35-audit", or other.
3. **Slice-by-slice pre-phase chat for Track 2 and Track 3.**
   Track 1's per-slice verification can chain through clean-signal
   slices. Track 2 + Track 3 slices each get their own pre-phase
   chat because drift discovery is more likely there.
4. **Clean-signal vs drift-signal evidence.** Each slice's commit
   message explicitly labels which outcome occurred and what
   sentinel test pins the property going forward.
