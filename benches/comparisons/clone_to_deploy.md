# Phase 43 — clone-to-production-shaped-deploy side-by-side

## Headline

For a developer cloning a working backend AI agent + bringing it up
to a deployable, signed, attested artifact, the Corvid implementation
is a single `corvid deploy package <app>` invocation that emits a
Dockerfile + OCI metadata + SPDX SBOM + DSSE signed build attestation
in one step. The equivalent FastAPI/LangChain + Next.js/Vercel
baselines require hand-assembling Dockerfile, SBOM tooling
(`syft`/`cargo-sbom`/`pip-licenses`), separate signing pipeline
(`cosign`, `sigstore`), separate health-check config, separate
attestation chain, and per-platform deployment manifests.

The governance-line delta below is a line-by-line count of the lines
that exist *only* for safety / attestation / SBOM / signing /
reproducibility — not feature lines. The reference workflow is the
Phase 42 Personal Executive Agent reaching a production-shaped
deploy on Fly.io.

## Reproduce

The Corvid implementation is:

```bash
corvid deploy package examples/backend/personal_executive_agent \
  --cdylib target/release/libpersonal_executive_agent.so \
  --out target/pea-package
```

That single command emits Dockerfile + oci-labels.json + env.schema.json
+ health.json + migrate.sh + startup-checks.md + sbom.spdx.json +
build-attestation.dsse.json + VERIFY.md.

The Python and TypeScript baselines below are *open for bounty
submission* — see
[`docs/internals/effect-spec/bounty.md`](../../docs/internals/effect-spec/bounty.md).
The numbers stay marked `bounty-open` until a submission lands.

## Side-by-side (sketch)

### Corvid (single command)

```bash
corvid deploy package examples/backend/personal_executive_agent \
  --cdylib target/release/libpersonal_executive_agent.so
```

Outputs:

| Artifact | What it covers |
|---|---|
| `Dockerfile` | distroless runtime base, multi-stage build, HEALTHCHECK directive, OCI labels |
| `oci-labels.json` | `org.opencontainers.image.title`, `image.source`, `dev.corvid.app`, `dev.corvid.package.source_sha256` |
| `env.schema.json` | typed env-var contract the runtime reads at boot |
| `health.json` | liveness/readiness probe definitions |
| `migrate.sh` | invocation wrapper for `corvid migrate up` against the production database |
| `startup-checks.md` | operator-facing pre-launch checklist |
| `sbom.spdx.json` | SPDX 2.3 JSON SBOM (app source + Corvid runtime) |
| `build-attestation.dsse.json` | DSSE envelope referencing the cdylib's claim attestation digest |
| `VERIFY.md` | step-by-step `cosign verify-blob` instructions |

Governance lines: **~0** (every line in the deploy package is a
line `corvid deploy package` generates, not a line the developer
maintains).

### Python (FastAPI + LangChain) — `bounty-open`

Hand-assembled per-app Dockerfile + `requirements.txt` +
`pre-commit` SBOM hook (`pip-licenses` or `syft`) + separate
`cosign sign-blob` step in CI + hand-written `healthcheck.py` +
hand-written attestation-chain glue + per-platform deployment
manifests (Compose, Fly.toml, K8s).

Estimated governance lines: **~150-300** per app (Dockerfile +
SBOM hook + signing pipeline + health check + attestation script
+ per-platform manifest scaffold). The number stays `bounty-open`
until a real-world FastAPI+LangChain PEA-equivalent is submitted
for measurement.

### TypeScript (Next.js + Vercel AI SDK) — `bounty-open`

Vercel handles a lot of the deploy story for the Vercel target,
which suppresses some governance-line cost for that platform —
but moving the same app to a non-Vercel target (Fly.io, Render,
self-hosted) reintroduces it. Estimated governance lines: **~80
on Vercel-only**, **~180-280** for portable deploy.

The number stays `bounty-open` until a real-world Next.js+Vercel
PEA-equivalent + a portable-deploy variant are submitted.

## Reproducibility

The `.github/workflows/reproducible-build.yml` workflow (slice 43R)
builds the Corvid CLI twice on Ubuntu 22.04 and asserts the two
binaries are SHA-256-identical. Python and TypeScript baselines
typically rely on lockfile-pinning + manual rebuild diffs — the
"two builds on the same host produce the same artifact" property
is achievable but is per-project glue rather than language-level
guarantee.

## Attestation chain

Corvid: the deploy attestation's payload includes the cdylib's
SHA-256, so the chain `claim --explain → cdylib bytes → deploy
attestation` cannot drift. Single `corvid claim audit` invocation
verifies the chain across the binary + the public claim
inventory.

Python / TypeScript: each project picks (or doesn't) its own
attestation pipeline. `cosign sign-blob` + `cosign attest-blob` +
`in-toto` predicates can compose into an equivalent chain but
require explicit per-project setup; the chain-drift adversarial
test is per-project test glue.

## What this benchmark proves and what it doesn't

**Proves:** Corvid's deploy story collapses cross-cutting safety +
attestation + SBOM + reproducibility concerns into a single
language-level command. The other stacks can match the
*capability* but pay governance-line cost per project.

**Does not prove:** the *cosmetic* parts of the deploy workflow
(image registry choice, CDN config, secrets management). Those
remain the operator's responsibility in every stack; Corvid does
not preempt them.

## Methodology + status

| Field | Value |
|---|---|
| Workflow under measurement | Phase 42 Personal Executive Agent → production-shaped deploy on Fly.io |
| Corvid status | Shipped (slices 43B, 43L, 43M, 43N, 43O, 43Q, 43R) |
| Python status | `bounty-open` |
| TypeScript status | `bounty-open` |
| Audit gate | The audit registry rows `deploy.reproducible_build`, `deploy.attestation_chain`, `deploy.sbom_completeness`, `upgrade.claim_regression_check` pin the Corvid-side guarantees this benchmark depends on |
