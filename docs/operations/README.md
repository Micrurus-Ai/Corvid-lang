# Operations

Running Corvid in production: deployment, signing, observability,
incident response.

## Pages

- [Production checklist](production-checklist.md) — pre-deploy and
  post-deploy structured checklists.
- [Receipts and signed builds](receipts-and-signing.md) — DSSE-signed
  cdylib, separate-binary verifier, key rotation.
- [Maintainer runbooks](../maintainer-runbooks.md) — release
  checklist, security advisory process, CI gates, benchmarks,
  claim review, rollback (lives at the canonical public URL
  under `docs/`).
- [Developer production guide](../developer-production-guide.md)
  — the "ship Corvid in production" walk-through (also at the
  public URL under `docs/`).
- [Observability conformance](observability-conformance.md) — OTel
  span set, drift gates, exporter setup.
- [CI](ci.md) — recommended CI pipeline, drift gates, signed-claim
  coverage.

## See also

- [Security](../security/) — TCB, threat model, stability contract.
- [Guides: Observability](../guides/observability.md) — task-focused
  setup walkthrough.
