# Phase 43 implementation plan — 2026-05-18

Pre-phase chat closure for Phase 43 (Packaging, deployment, release,
market readiness). Path A is locked (silent build → v1.0). Sub-slice
plan + dependency graph + pitch sentence + audit-and-correct vs new-
implementation split, all in one place.

**Slice id:** `43K`.

## v1.0 launch pitch — LOCKED 2026-05-18

> *Build the same production AI app you'd build in Python — auth,
> jobs, persistence, deploy — but with the safety guarantees
> compiled into the binary instead of audited by humans after the
> fact.*

This sentence anchors every Phase 43 scope decision. The "compiled
into the binary instead of audited by humans" half is what Phase 43
makes defensible to a skeptic:

- `corvid deploy package` emits a signed cdylib whose claim
  manifest is the same DSSE envelope the build attestation references.
- `corvid claim audit` exits 0 only when every README / website /
  launch-page claim points at a runnable command or committed
  artifact.
- `corvid upgrade --check` refuses an upgrade that would remove or
  weaken any registered guarantee.
- The reproducible-build CI workflow makes "the binary you run is
  the binary I built from this commit" a verifiable property, not a
  trust statement.
- `corvid ops show <prod-url>` lets an operator read the live
  binary's signed claim manifest + cost-since-start + approvals-
  pending — no hidden runtime state.

Anything Phase 43 ships that doesn't strengthen one of those five
surfaces is out of scope for the phase.

## State of Phase 43 as of the spot-check (2026-05-17)

Slice checklist: **16 of 19 sub-slices closed** (`43A-43G + 43I-43J +
sub-slices`). 3 open are `43H` beta-program, repositioned per Path
A to a 5-10 friends-and-family round in the final 4 weeks. No new
slice-level work needed there.

Phase-done checklist (12 items): the round-2 audit. Breakdown:

| Bucket | Count |
|---|---|
| Effectively shipped (verification only) | 4 — HEALTHCHECK in Dockerfile, OCI labels, release sign + SHA256SUMS, `corvid claim audit` exit-0 |
| Partial — needs extension | 3 — `corvid deploy package` (missing distroless + SBOM); `corvid upgrade --check` (missing claim-regression); signed-attestation chain (missing cdylib digest reference) |
| Not shipped — real implementation | 5 — `corvid ops show`, reproducible-build CI, deploy smoke-deploy CI, 5 AI helpers, `clone_to_deploy.md` benchmark |
| Operational gate (Path A) | 1 — beta program → friends-and-family in 33M repositioning |

7 `deploy.*` / `release.*` / `upgrade.*` / `ops.*` / `claim.*`
registry rows are **none-present** today; they need to land before
the implementation surfaces they describe ship.

## Slice plan with explicit dependencies

```
43K (this doc, this commit)
 │
 ├──→ 43L  Registry rows + presence sentinel (OutOfScope placeholders)
 │          Unblocks every later commit that references the ids.
 │
 ├──→ 43M  SBOM generation in `corvid deploy package`
 │   │      Adds SPDX `sbom.spdx.json` as a package artifact.
 │   │      Promotes `deploy.sbom_completeness` (43V).
 │   │
 │   └──→ 43R  Reproducible-build CI workflow
 │              Needs SBOM to compare. Builds twice on different
 │              runners, diffs the binary + SBOM, asserts bit-
 │              identical.
 │              Promotes `deploy.reproducible_build` (43V).
 │
 ├──→ 43N  Switch Dockerfile to distroless + size-budget check
 │          Independent of M/R/etc. Validates ≤80 MB runtime image.
 │
 ├──→ 43O  Signed-attestation chain
 │          `corvid deploy package`'s DSSE envelope includes the
 │          cdylib's claim attestation digest. Verifies the chain
 │          cannot drift.
 │          Promotes `deploy.attestation_chain` (43V).
 │
 ├──→ 43P  `corvid ops show <prod-url>` CLI + HTTP endpoint
 │          New CLI subcommand. Phase 36-generated axum server gets
 │          a `/__ops` endpoint that returns the signed claim
 │          manifest + cost-since-start + approvals-pending.
 │          Promotes `ops.live_introspection_signed` (43V).
 │
 ├──→ 43Q  `corvid upgrade --check` extended with claim-regression
 │          Compares the upgrade target's claim manifest against
 │          the current binary's. Refuses if any guarantee id is
 │          removed or downgraded.
 │          Promotes `upgrade.claim_regression_check` (43V).
 │
 ├──→ 43S  kind cluster + Fly/Render smoke-deploy CI matrix
 │          Operational gate. Provider credentials in GitHub
 │          secrets. Runs after every Phase 43 implementation
 │          commit.
 │
 ├──→ 43T  5 AI helpers
 │          release-note generator (generative)
 │          deploy-target tailor (agentic)
 │          migration assistant (agentic)
 │          beta-feedback synthesizer (agentic, fed by 33M
 │          repositioned friends-and-family round)
 │          final claim-audit narrator (adversarial)
 │
 ├──→ 43U  `benches/comparisons/clone_to_deploy.md` benchmark
 │          Side-by-side against FastAPI/LangChain + Next.js/Vercel
 │          on clone-to-production-shaped-deploy time.
 │
 ├──→ 43V  Promote registry rows from OutOfScope → Static/RuntimeChecked
 │          Each promotion ticks when its underlying surface from
 │          M/N/O/P/Q/R/S above ships. Lives as a single rolling
 │          slice that gets multiple commits over the implementation
 │          run.
 │
 ├──→ 43W  Launch-readiness tail interleaved
 │          33 filings from P38-P42:
 │            - 7 guide rewrites (interleaved with Phase 43 docs work)
 │            - P38 corvid-jobs-explain helper, P39 approvals-helpers,
 │              P41 connector-helpers — fold into 43T
 │            - P42 per-app maturity (5 apps × 1500-line runbook +
 │              evals + adversarial tests + CLAIM.md + benchmark
 │              + AI helpers)
 │            - P41 grounded-connector returns (depends on post-v1.0
 │              syntax sugar; folds to post-v1.0)
 │            - P38 cross-layer replay-quarantine wiring (multi-day
 │              integration work; folds in alongside P)
 │            - P38 loop-bounds enforcement hook
 │            - P39 CSRF middleware + session-rotation hook + role-
 │              coverage reachability + structured-scope model +
 │              batch-data-class equivalence
 │            - P40 review-queue ranking CLI
 │            - P41 connector-drift narration + live-provider CI matrix
 │            - 33M repositioned as 5-10 friends-and-family round
 │            - 33J4 benchmark page + 33J5 blog shell + 33L launch
 │              materials (final 2 weeks)
 │
 └──→ 43X  Phase 43 closeout + v1.0 cut
            Every box in the v1.0 launch criteria ticks; final
            claim audit re-runs exit-0; tag v1.0.
```

**No strict order across L/M/N/O/P/Q/T/U** (independent surfaces).
Strict order: L before V (rows must exist to promote); M before R
(SBOM before reproducible-build comparison).

## Audit-and-correct vs new-implementation split

| Category | Slices | Estimate |
|---|---|---|
| Audit-and-correct (P38-P42 pattern) | 43L (registry rows + sentinel), 43V (rolling promotions) | ~1 day |
| New implementation — CLI / runtime | 43M, 43N, 43O, 43P, 43Q | ~7-10 days |
| New implementation — CI / operational | 43R, 43S | ~3-4 days |
| New implementation — AI / docs / benchmark | 43T, 43U | ~5-7 days |
| Launch-readiness tail (interleaved) | 43W | ~3-4 weeks (parallel where possible) |
| v1.0 cut | 43X | ~1 day |

**Realistic total for Phase 43 plus launch-readiness tail: 6-8 weeks of focused work to v1.0 cut.**

That includes the 33 P38-P42 launch-readiness filings interleaved
(many are docs-shaped and don't block Phase 43 code work). The 33M
friends-and-family round needs ~2 calendar weeks of real-world dev
feedback time on top, so wall-clock to v1.0 is closer to **8-10
weeks** even if engineering finishes in 6-8.

## Phase-done criteria for `43X` v1.0 cut

Every box in the existing v1.0 launch criteria (from the Path A
ROADMAP section) must tick:

- [ ] Every Phase 37-43 closed per phase-done criteria.
- [ ] Every Phase 42 reference app demoably ships, runs in
  production-shape, and deploys via Phase 43 packaging on at
  least one supported target.
- [ ] Every cdylib claim id introduced in Phases 37-43 is wired
  into the signed-claim coverage gate.
- [ ] Launch claim audit re-run after Phase 43 closes; zero
  aspirational wording survives.
- [ ] Bilateral verifier green across production-backend surface.
- [ ] Friends-and-family round (5-10 devs) feedback closed as
  code/docs/tests/explicit non-scope.
- [ ] 33J4 benchmark page + 33J5 blog shell + 33L launch GIF +
  announcement drafts shipped on the website in the final 2 weeks.

## Validation discipline carried forward

Same as Path A:

- Every slice ships its own commit on `main`.
- `cargo check --workspace` clean between every commit.
- `cargo run -q -p corvid-cli -- verify --corpus tests/corpus` exit
  1 (only the two deliberate fixtures diverge).
- Each slice that adds a new contract id ships its row + tests
  together (no shipped-but-not-registered drift, learning from
  P38/P39 OutOfScope-promotion pattern).
- Each slice that touches user-facing docs runs through the
  `docs_drift_gate` sentinel (no aspirational syntax in docs;
  learning from P38-E surfaced gap).

Pre-phase chat formally closes with this commit. Implementation
opens.
