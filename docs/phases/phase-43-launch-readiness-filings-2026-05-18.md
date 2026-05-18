# Phase 43 launch-readiness filings — 2026-05-18

Cross-layer / operational Phase 43 slices that don't fit a
single-session implementation slot. Each filing records the
specific surface needed, the rough effort, and the registry row
that promotes when it lands.

## 43P-LR-corvid-ops-show-end-to-end

**Surface:** `corvid ops show <prod-url> [--key=<pubkey>]` CLI
subcommand + Phase 36 server-render addition that exposes a
`/__ops` HTTP endpoint returning a signed introspection payload.

**Payload schema** (proposed `corvid.ops.live.v1`):

```json
{
  "schema": "corvid.ops.live.v1",
  "claim_manifest_digest": "<sha256 of the cdylib's embedded claim attestation>",
  "cost_usd_since_start": 0.0,
  "approvals_pending": 0,
  "uptime_ms": 12345,
  "started_at": "<RFC3339>"
}
```

**Why launch-readiness:** Three independent pieces of work that
each need real implementation, none of which fits a single
session:

1. CLI subcommand with HTTP client (`reqwest`) + DSSE envelope
   verification using the binary's public key.
2. Phase 36 `server_render.rs` addition that emits a `/__ops`
   axum route + a `Live ops` middleware that maintains the
   counters (cost-since-start, approvals-pending, uptime).
3. The cost-since-start + approvals-pending state stores plumbed
   through the generated server's runtime.

**Promotes:** `ops.live_introspection_signed` row from
OutOfScope → RuntimeChecked when CLI + server endpoint + the
signature-match adversarial test all land together.

**Estimated effort:** ~3-5 days when picked up.

## 43S-LR-deploy-smoke-deploy-ci-matrix

**Surface:** CI workflow that smoke-deploys each Phase 42
reference app to:
- a kind (Kubernetes-in-Docker) cluster on every push
- a Fly.io or Render preview environment behind a manual gate

**Why launch-readiness:** Needs operational provider credentials
in GitHub secrets (Fly.io API token; Render service hook URL). I
can't provision those — the user does, then I land the workflow
that consumes them. The kind-cluster half can ship before the
PaaS half.

**Promotes:** None directly — it's CI infrastructure, not a
registry row. Phase-done item "Deployment manifests smoke-deploy
in CI" ticks when this lands.

**Estimated effort:** ~1-2 days for the workflow + ~1 day for
the per-app smoke-test fixtures.

## 43T-LR-phase-43-ai-helpers

**Surface:** Five Phase 43 AI helper subcommands:

| Helper | Pattern | Surface |
|---|---|---|
| `corvid release notes <prev-tag> <new-tag>` | generative | Markdown release notes synthesised from commit history + closed launch-readiness slices |
| `corvid deploy tailor <app> --target=<platform>` | agentic | Iteratively refines deploy manifests until smoke-deploy passes |
| `corvid upgrade assist <old-version> <new-version>` | agentic | Walks source diffs, proposes codemod fixes for breaking changes, holds human-review tag for non-mechanical cases |
| `corvid beta synthesize-feedback <round-dir>` | agentic | Aggregates friends-and-family feedback (33M repositioning) into categorised issue templates |
| `corvid claim audit --explain-failures` | adversarial | Narrates each failed claim with the specific evidence path + suggested fix |

**Why launch-readiness:** Each helper is a real ~1-day slice and
they share an infrastructure dependency (LLM-helper pattern that
the Phase 38/39/41 helper umbrellas also depend on). Filing them
together under one umbrella keeps the rollout coherent.

**Promotes:** None directly — none of the P43 registry rows
require an AI helper. The Phase 43 phase-done item "5 AI helpers
landed" ticks when this lands.

**Folds in:** Phase 38 `corvid jobs explain` (35V2-P38-G-LR);
Phase 39 `corvid approvals explain/policy-suggest`
(35V2-P39-G-LR/H-LR); Phase 41 connectors AI helpers
(35V2-P41-H-LR). All five Phase 43 helpers + the four phase-tail
helpers from P38/P39/P41 share the LLM-helper infrastructure
slice that lands first.

**Estimated effort:** ~5-7 days for all 9 AI helpers (P43's 5 +
P38/P39/P41's 4) once the shared infrastructure ships.

## Cross-cutting note

These three filings + the 33 P38-P42 launch-readiness filings
form the **launch-readiness tail** that `43W` tracks. Total
launch-readiness work to v1.0 cut: ~36 filings, ~3-4 weeks of
focused work running in parallel with the remaining
single-session Phase 43 slices (43Q upgrade-check + 43R
reproducible-build CI + 43U benchmark + 43V promotions).

Phase 43 implementation order updated:

1. ✅ 43L registry rows + sentinel (shipped)
2. ✅ 43M SBOM + promotion (shipped)
3. ✅ 43N distroless + Dockerfile sentinel (shipped)
4. ✅ 43O attestation chain + promotion (shipped)
5. ⏭️ 43P → launch-readiness (this filing)
6. 43Q upgrade --check claim-regression (next inline)
7. 43R reproducible-build CI
8. ⏭️ 43S → launch-readiness (this filing)
9. ⏭️ 43T → launch-readiness (this filing)
10. 43U clone_to_deploy benchmark
11. 43V rolling promotions + missing test pairs
12. 43W launch-readiness tail (parallel)
13. 43X v1.0 cut
