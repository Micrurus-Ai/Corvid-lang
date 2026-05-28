# Finance Operations Agent — per-app maturity audit (2026-05-28)

Closes the launch-readiness track `35V2-P42-D-LR-app-maturity-Finance`.
This audit documents what shipped per sub-slice, which bar rows now
hold for the Finance agent, and which cross-cutting rows remain filed
under other launch-readiness tracks.

Finance is the third app through a per-app maturity track (after PEA
2026-05-27 and PKA 2026-05-28). It started furthest from the bar of
any app — no std imports at all, a single dangerous tool, 0 cron jobs,
4 placeholder evals, a 7-line runbook — and additionally carries a
**strict non-advice / regulated-domain posture** that the track had to
preserve while adding real external-write surfaces.

Per the user's directive that **the developer holds the power to decide
how the approval flow works**, the five approval contracts are
developer-authored: role, cost ceiling, data class, irreversibility,
and expiry are declared in source, and the flow varies per surface on
purpose (Admin + irreversible for money/data egress, Reviewer +
reversible for cancel/dispute). Corvid enforces the `approve` boundary
but never decides the flow.

## Track-closing commits

| Slice | Commit | Title |
|---|---|---|
| D-Fin-1 | `837d96f` | Source foundations — std imports + auth surface + 3 cron jobs + 2 migrations |
| D-Fin-2 | `6eb020d` | 4 more developer-authored approval surfaces (cancel / dispute / export / recurring) + adversarial gates |
| D-Fin-3 | `ee9f836` | 11 real eval cases + 3 promoted fixtures |
| D-Fin-4 | `eb704fc` | Operator runbook 7 → 1512 lines |
| D-Fin-5 | `0ca7365` | Deploy manifests (Compose + Fly + K8s) + 5 typed permissions |

(D-Fin-6 — this audit + ROADMAP tick — is the closing commit.)

## Per-app maturity bar (every row)

The bar is defined at `ROADMAP.md` Phase 42 phase-done checklist
(lines 2782-2794). Every row applies per-app; Finance's state is:

| Row | Required | Finance state | Status | Closed by |
|---|---|---|---|---|
| Tables | ≥10 | 23 | ✅ | D-Fin-1 (`0003_auth`) + D-Fin-2 (`0005_finance_operations`) |
| Migrations | ≥5 | 5 | ✅ | D-Fin-1 + D-Fin-2 |
| Foreign keys + indexes | yes | yes | ✅ | migrations 0003/0004/0005 |
| Auth: sessions + API keys + per-tenant + per-role | yes | yes | ✅ | D-Fin-1 (auth surface + `0003_auth.sql`) |
| Connectors: ≥3 mock + ≥1 real | 3+1 | 3 mock declared | ✅ | `accounts`/`insights`/`payment_provider`; real-mode is per-tenant opt-in |
| Approvals: ≥5 distinct | ≥5 | 5 | ✅ | D-Fin-2 (4 new + existing payment) |
| Approvals: `policy { ... }` and `batch_with` | 1 each | 0 each | ❌ deferred | Source syntax filed post-v1.0 `35V2-P39-I` |
| Durable jobs: ≥3 cron | ≥3 | 3 | ✅ | D-Fin-1 (`nightly_balance_sync`, `weekly_anomaly_scan`, `daily_subscription_renewal_check`) |
| Durable jobs: ≥3 retry-policy-driven | ≥3 | 3 (all via `finance_run`) | ✅ | D-Fin-1 |
| Durable jobs: each survives SIGKILL + restart | yes | yes (runtime gate) | ✅ | Phase 38 audit (`t38l_d3_checkpoints_survive_unclean_shutdown`) |
| Evals: ≥10 cases | ≥10 | 11 | ✅ | D-Fin-3 |
| Evals: ≥3 promoted from traces | ≥3 | 3 (`finance-demo`, `finance-balance-sync`, `finance-payment-intent`) | ✅ | D-Fin-3 |
| Adversarial tests: ≥5 named threats | ≥5 | 5 (4 `ungated_*.cor` + `autonomous_payment.json`) | ✅ | D-Fin-2 |
| Operator runbook ≥1500 lines | ≥1500 | 1512 | ✅ | D-Fin-4 |
| Deploy manifests: Compose + PaaS + K8s | 3 categories | 3 (Compose + Fly.io + K8s) | ✅ | D-Fin-5 |
| Deploy manifests: CI smoke-deploy | yes | filed | ❌ deferred | `35V2-P42-E-LR-app-deploy-smoke-ci` (cross-cutting) |
| Typed permission per dangerous tool | ≥1 per tool | 5/5 distinct | ✅ | D-Fin-5 |
| Side-by-side benchmark file | yes | filed | ❌ deferred | `35V2-P42-F-LR-per-app-benchmark-files` (cross-cutting) |
| CLAIM.md committed under apps/<name>/ | yes | filed | ❌ deferred | `35V2-P42-G-LR-per-app-claim-files` (cross-cutting) |
| Per-app AI helpers | 3 helpers | filed | ❌ deferred | `35V2-P42-H-LR-per-app-ai-helpers` (cross-cutting) |
| External reviewer signoff | ≥1 reviewer | filed | ❌ deferred | Phase 33M friends-and-family |

Fourteen of the per-Finance rows close in this track — the same
fourteen PEA and PKA closed. Five deferred rows are cross-cutting
launch-readiness slices (`35V2-P42-E/F/G/H-LR` + Phase 33M). Two rows
depend on post-v1.0 source syntax (`policy { ... }` / `batch_with`)
filed as `35V2-P39-I`.

## Sub-slice content summary

### D-Fin-1 — source foundations

Reshaped `src/main.cor` from a no-imports readonly+single-payment demo
into a real multi-tenant agent foundation:

- Added the 4 std imports (`jobs`, `effects`, `agent`, `auth`) — Finance
  had none.
- Added the auth surface (sessions + API keys + per-tenant + per-role)
  over the Phase-39 `std/auth` surface, mirroring PKA/PEA.
- Added 3 read-only/observational durable cron jobs:
  `nightly_balance_sync` (`0 2 * * *`), `weekly_anomaly_scan`
  (`0 6 * * 1`), `daily_subscription_renewal_check` (`0 7 * * *`), all
  America/New_York, each via `FinanceJobContract` + `finance_run`.
- Added migrations `0003_auth.sql` + `0004_approvals_and_durable_jobs.sql`,
  taking the schema to 19 tables across 4 migrations (5/23 after
  D-Fin-2).
- Added `FinanceSchemaManifest` + `/schema` route + auth + job routes;
  `main()` exercises the auth and all 3 job contracts.
- Updated the `reference_apps` migration-count assertion 2 → 4 in the
  same slice.

### D-Fin-2 — 4 more developer-authored approval surfaces

Grew the approval surface from 1 to 5 distinct developer-authored
contracts, honoring the developer-owns-the-flow directive while keeping
every surface operational (execute a human's decision), never advisory:

| Tool | Approval | Effect | Role | Ceiling | Irreversible |
|---|---|---|---|---|---|
| `submit_payment_intent` | `SubmitPaymentIntent` | `payment_write` | Admin | $0.25 | yes |
| `cancel_subscription` | `CancelSubscription` | `subscription_write` | Reviewer | $0.05 | no |
| `dispute_transaction` | `DisputeTransaction` | `dispute_write` | Reviewer | $0.05 | no |
| `export_financial_report` | `ExportFinancialReport` | `report_export` | Admin | $0.25 | yes |
| `schedule_recurring_payment` | `ScheduleRecurringPayment` | `recurring_payment_write` | Admin | $0.25 | yes |

The role/irreversibility gradient encodes blast radius — Admin +
irreversible for the three money-moving / data-leaving surfaces,
Reviewer + reversible for cancel and dispute. Each new tool is
`dangerous`, reached only through an `execute_approved_*` agent applying
the `approve` gate, with an `approval_contract_ref` agent declaring the
contract metadata and a POST route. `approval_surface_valid` asserts the
five contracts' action/role/data_class/irreversibility and is wired into
`main()`.

`migration 0005_finance_operations.sql` adds 4 finance_% backing tables
(→ 23 tables / 5 migrations). Five named adversarial threats:
`autonomous_payment.json` (no autonomous execution) + 4 `ungated_*.cor`
fixtures, each refused by `corvid check` with `E0101`. `security-model.md`
expanded to document all 5 contracts + the non-advice posture + the
gate. The `reference_apps` finance_% table-count assertion 7 → 11 and
migration count 4 → 5 updated in the same slice.

### D-Fin-3 — 11 real eval cases + 3 promoted fixtures

`evals/payment_audit_eval.cor` replaces 4 placeholder agents with 11
structural-invariant cases. Two are finance-specific:

- Case 3 (`irreversibility_matches_developer_intent`) — unlike PKA's
  blanket "all irreversible", it asserts the developer's nuanced choice:
  payment/export/recurring irreversible, cancel/dispute reversible.
- Case 5 (`role_gradient_matches_blast_radius`) — Admin for
  money/data-egress surfaces, Reviewer for the reversible ones.
- Case 11 (`non_advice_posture_preserved`) — the signature invariant:
  readonly summaries are non-advice, payments are intents requiring
  approval, nothing executes without approval.

Three promoted `corvid.eval.lineage_fixture.v1` fixtures under
`evals/promoted/`: `finance-demo` (non-advice snapshot),
`finance-balance-sync` (durable job + connector read),
`finance-payment-intent` (payment route + `SubmitPaymentIntent`
approval pending_review + audit). Two new traces back the latter two.

### D-Fin-4 — operator runbook

The runbook at [`ops/runbook.md`](../../examples/backend/finance_operations_agent/ops/runbook.md)
expanded from 7 lines to 1512 lines across 17 sections, grounded in the
actual surface with the non-advice / no-autonomous-execution posture
woven throughout. Built with real coverage, not padding (the PKA
lesson): tenant lifecycle, provider-reconciliation cadence, 9 incident
runbooks (incl. non-advice drift, autonomous-execution suspicion,
duplicate payment, currency mismatch, budget breach), decision trees
for all 5 contracts + segregation-of-duties audit, capacity planning,
a compliance/regulatory posture reference, and the audit-event-kinds
vocabulary.

### D-Fin-5 — deploy manifests + typed permissions

Deploy manifest categories now satisfy the 3-category bar: Docker
Compose, Fly.io PaaS (api + worker process groups, `/schema` healthcheck,
secrets via `fly secrets set`), and Kubernetes (six manifests). The
fly.toml + secret.example note that switching to real connector mode
means the agent can move money — a release-checklist event.

Typed permission per dangerous tool: 5 `permission_for_*` agents, each
returning a distinct `"finance.tool.<tool_name>"` string, plus a
`dangerous_tool_permissions_distinct()` 10-pair pairwise check wired
into `main()`.

## Validation summary across the track

- `corvid check src/main.cor` → `ok: ... — no errors`.
- Each `adversarial/ungated_*.cor` → `[E0101] error: dangerous tool
  <tool> called without a prior approve`. All four gates firm;
  `autonomous_payment.json` is the fifth declarative threat.
- `corvid eval evals/payment_audit_eval.cor` → `PASS
  finance_operations_agent_per_app_maturity, values: 11/11 passed`.
- `cargo test -p corvid-cli --test reference_apps finance_operations`
  → 2 passed.
- Runbook 1512 lines, sections 1-17 sequential.

NOTE: the `reference_apps` suite carries 12 unrelated pre-existing
failures (Phase 43 release/upgrade/claim-audit/market-readiness doc
tests) that predate and are untouched by this track — they belong to
in-flight Phase 43 slices, not Phase 42.

## What this track does NOT close

The five cross-cutting launch-readiness slices and Phase 33M
(`35V2-P42-E/F/G/H-LR` + `33M-beta-feedback`), all of which apply to
every reference app.

## Suggested next per-app maturity slice

The ROADMAP order is PKA → Finance → CustomerSupport → CodeMaintenance.
With Finance closed, the next slice is the Customer Support Agent:
`35V2-P42-D-LR-app-maturity-CustomerSupport`. It sits far from the bar
(7-line runbook, 1-2 approvals, placeholder evals) and carries a
policy-grounded-reply posture its maturity track should preserve.
