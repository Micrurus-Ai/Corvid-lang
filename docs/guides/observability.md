# Observability

> **What runs today, and what does not yet.** `corvid run` and a compiled
> binary record every run as a trace, `target/trace/<run_id>.jsonl`. That
> trace is the record replay reads, and the tools on this page read it
> too: `corvid observe`, `corvid trace lineage` and `corvid eval promote`
> derive a run's lineage from its trace. The run is the root, and each
> agent call that did something is an `agent` step holding the steps it
> made, so the tree shows which agent did what. Each table
> operation is a `db` step showing how long it waited for a connection or
> a turn to write and how long it ran (`waited=0.031ms ran=0.241ms`;
> `corvid.db.wait_us` and `corvid.db.statement_us` when exported). Each
> other tool is a `tool` step, each model call
> a `prompt` step with its model, tokens and cost, each approval an
> `approval` step, and each HTTP attempt
> sits under its tool. A lineage file (`*.lineage.jsonl`) is still read as
> written.
>
> Not built yet:
> - `corvid serve` records no trace.
> - In compiled native code, calls from one agent to another are not nested:
>   a native run's steps sit directly under the run.
> - A running program exports nothing to OpenTelemetry. The
>   `CORVID_OTEL_*` variables and a `corvid run --trace-out` flag do not
>   exist.
>
> The plan for each is [lineage from real runs](../proposals/lineage-from-runs.md).

The observability surface is the OTel-conformant lineage model, the
`corvid observe` CLI, and the `corvid eval`, `corvid eval-drift` and
`corvid eval-from-feedback` workflow. The source-level
`obs.log.info({...})` and `obs.span("name", {...})` surface comes later.

## What ships today

Per the Phase 40 audit (`docs/phases/phase-40-audit-2026-05-17.md`):

- **Lineage events.** A typed `LineageEvent` model for prompt, tool,
  agent, approval, database, retry and error steps, with stable
  `(trace_id, span_id, parent_span_id)` triples. Validated by
  `corvid_runtime::lineage::validate_lineage`. A run's lineage is derived
  from its trace by `corvid_runtime::lineage_from_trace`, the one
  projection every reader calls.
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
# 1. Run an agent; its trace is recorded under the project's target/trace/.
cd examples/backend/personal_executive_agent
corvid run src/main.cor

# 2. Inspect the run: its lineage is derived from the trace.
corvid observe list
corvid trace lineage <run-id>
corvid observe show <run-id>

# 3. Compare two sets of runs (e.g. before/after a prompt change).
corvid observe drift traces/before/ traces/after/

# 4. Promote a failing run into an eval fixture.
corvid eval promote target/trace/<failed-run-id>.jsonl \
    --promote-out tests/evals/regressions/

# 5. Run the eval set during CI.
corvid eval tests/evals/regressions/ --source src/main.cor
```

## OTel export configuration

A running program does not export to OpenTelemetry yet. No
configuration exists, and no `CORVID_OTEL_*` variable is read. The
export turns lineage events into OTLP/HTTP JSON, and its wire path is
tested against an in-process receiver. The docker-compose Jaeger harness
in `docs/operations/observability-conformance.md` exercises the same
path against a real collector. Exporting a served program's traces will
be declared in its source, not switched on by a variable, because it
sends what users did to another system. See
[lineage from real runs](../proposals/lineage-from-runs.md).

## Redaction

The redaction policy applies to every lineage event before it is
exported. Token-shaped values
(`Bearer <hex>`, SSN-like `\d{3}-\d{2}-\d{4}`) are replaced with
shape-preserving placeholders; the (trace_id, span_id, parent
linkage) topology is preserved so a redacted trace still
correlates against the eval store + OTel.

Adversarial test:
`crates/corvid-runtime/src/lineage_redact.rs::redaction_removes_obvious_secrets_from_serialized_lineage`
seeds `"Bearer sk-live-123 for 123-45-6789"` into a span and
asserts the SSN does not appear in the redacted JSON.

A source-level `obs.log.info({...})` API, letting agent code emit its
own structured logs with the redaction policy applied, comes later.
Today a run's authoritative record is its trace.

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
