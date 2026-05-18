# Observability

The observability surface ships in v1.0 as the OTel-conformant
runtime + the `corvid observe` CLI + the `corvid eval` /
`corvid eval-drift` / `corvid eval-from-feedback` workflow. The
source-level `obs.log.info({...})` / `obs.span("name", {...})`
ergonomic surface is post-v1.0 — today every Corvid construct
already emits the lineage events the observability stack consumes;
agent code doesn't have to call a logging API to get observed.

## What ships today

Per the Phase 40 audit (`docs/phases/phase-40-audit-2026-05-17.md`):

- **Lineage events** — every prompt / tool / agent / approve /
  grounded-unwrap / budget / replay step emits a typed
  `LineageEvent` with stable `(trace_id, span_id, parent_span_id)`
  triples. Validated by
  `corvid_runtime::lineage::validate_lineage`.
- **OTel SDK export** —
  `crates/corvid-runtime/src/otel_sdk_export.rs` ships span
  emission through the standard `opentelemetry` +
  `opentelemetry-otlp` SDK. Span attributes carry
  `corvid.guarantee_id` / `corvid.cost_usd` /
  `corvid.approval_id` / `corvid.replay_key`. The in-process
  OTLP receiver test exercises the wire path; the docker-compose
  Jaeger harness is documented at
  `docs/operations/observability-conformance.md`.
- **Deterministic redaction** — replaying the same lineage event
  twice with the same `LineageRedactionPolicy` yields byte-
  identical output. Token-shaped values + SSN-pattern strings are
  removed; topology (trace_id, span_id, parent linkage) is
  preserved so observe / eval / OTel keep correlating after
  sensitive values are removed.
- **Contract-aware grouping** — `corvid observe show` groups
  incidents by `guarantee_id` / effect / budget / provenance /
  approval rule rather than by service.name, so an analyst's first
  pivot lands on the contract that broke.
- **Drift attribution** — `corvid eval-drift --explain` decomposes
  the difference between two trace runs into the four named
  dimensions (model_id / prompt_hash / retrieval_index_hash /
  input_fingerprint) plus a residual percentage.
- **Trace promotion to evals** — `corvid eval promote
  <trace.lineage.jsonl> --promote-out <DIR>` synthesises a typed
  eval fixture from a 'wrong answer' feedback record, redacting
  the matching lineage trace via the production redaction policy
  before writing the fixture.

## CLI

```sh
corvid observe list                          # local lineage runs + costs / failures / approvals / slowest spans
corvid observe show <run-id>                  # explain one run with contract-aware grouping
corvid observe drift <a.trace> <b.trace>      # compare two traces (or directories) for production drift
corvid observe explain <run-id>               # AI-assisted root-cause (RAG-grounded over the typed trace)
corvid observe cost-optimise --trace-dir <d>  # AI-assisted cost optimisation across recorded runs
```

The `--json` flag is accepted on every subcommand and emits the
machine-readable form the docs-site + downstream tooling consume.

## Trace + eval lifecycle

```sh
# 1. Run an agent and record its trace.
corvid run examples/backend/personal_executive_agent/src/main.cor \
    --trace-out /var/lib/corvid/traces/

# 2. Inspect the trace.
corvid observe show <run-id> --trace-dir /var/lib/corvid/traces/

# 3. Compare two traces (e.g. before/after a prompt change).
corvid observe drift /var/lib/corvid/traces/pre/ /var/lib/corvid/traces/post/

# 4. Promote a failing trace into an eval fixture.
corvid eval promote /var/lib/corvid/traces/<failed>.lineage.jsonl \
    --promote-out tests/evals/regressions/

# 5. Run the eval set during CI.
corvid eval tests/evals/regressions/ --source examples/backend/personal_executive_agent/src/main.cor
```

## OTel export configuration

Configure via environment variables on the running agent:

```sh
export CORVID_OTEL_ENDPOINT=http://otel-collector:4317
export CORVID_OTEL_SERVICE_NAME=my-app
export CORVID_OTEL_SAMPLE_RATE=1.0
```

The docker-compose Jaeger conformance harness (documented at
`docs/operations/observability-conformance.md`) exercises the
full wire path end-to-end against a real OTel collector.

## Redaction

The runtime applies a built-in redaction policy to every lineage
event before it leaves the process. Token-shaped values
(`Bearer <hex>`, SSN-like `\d{3}-\d{2}-\d{4}`) are replaced with
shape-preserving placeholders; the (trace_id, span_id, parent
linkage) topology is preserved so a redacted trace still
correlates against the eval store + OTel.

Adversarial test:
`crates/corvid-runtime/src/lineage_redact.rs::redaction_removes_obvious_secrets_from_serialized_lineage`
seeds `"Bearer sk-live-123 for 123-45-6789"` into a span and
asserts the SSN does not appear in the redacted JSON.

The source-level `obs.log.info({...})` API that would let agent
code emit arbitrary structured logs (with the redaction policy
applied to user-supplied fields) is post-v1.0 — today the
lineage events the runtime emits automatically are the
authoritative record.

## Drift attribution + cost optimisation

`corvid observe drift --explain` decomposes drift into:

- `model_fingerprint` — did the model id / version change?
- `prompt_hash` — did the rendered prompt change?
- `retrieval_index_hash` — did the retrieved context change?
- `input_fingerprint` — did the user input shape change?
- `residual` — unattributable portion

`corvid observe cost-optimise --trace-dir <d>` aggregates cost-
by-event-name across a directory of recorded runs, identifies
the top-N cost centres, and proposes typed suggestions (cache /
skip-pre-validate / model-swap). Each suggestion carries
`sources` linking back to the supporting events.

## Operator runbooks

Per-app operator runbooks live under each reference app's
`ops/runbook.md` (`examples/backend/<app>/ops/runbook.md`). The
v1.0 maturity bar (≥1500 lines per app, covering setup / secrets
/ migrations / backups / logs / metrics / incident response /
rollback) is filed as per-app launch-readiness slices
(`35V2-P42-D-LR-app-maturity-{PEA,PKA,Finance,CustomerSupport,CodeMaintenance}`).

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| Lineage IDs stable + parented across backends | `observability.lineage_completeness` | RuntimeChecked | `crates/corvid-runtime/src/lineage.rs` |
| OTel conformance (attribute shape + wire path) | `observability.otel_conformance` | RuntimeChecked | `crates/corvid-runtime/src/otel_sdk_export.rs` |
| Redaction is deterministic + topology-preserving | `observability.redaction_determinism` | RuntimeChecked | `crates/corvid-runtime/src/lineage_redact.rs` |
| Incidents group by contract id (not service.name) | `observability.contract_aware_grouping` | RuntimeChecked | `crates/corvid-runtime/src/lineage_incidents.rs` |
| Drift attributable into named dimensions | `eval.drift_attribution` | RuntimeChecked | `crates/corvid-cli/src/observe_helpers_cmd/eval_drift.rs` |
| Promoted evals carry signed lineage | `eval.promotion_signed_lineage` | RuntimeChecked | `crates/corvid-cli/src/observe_helpers_cmd/eval_from_feedback.rs` |
| Review-queue ranked by cost-of-being-wrong | `review_queue.cost_of_being_wrong_ranking` | OutOfScope | gated on `35V2-P40-C-LR-review-queue-ranking-cli` |
