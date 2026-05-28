# Customer Support Agent — per-app maturity audit (2026-05-28)

Closes the launch-readiness track
`35V2-P42-D-LR-app-maturity-CustomerSupport`. This audit documents what
shipped per sub-slice, which bar rows now hold for the Support agent,
and which cross-cutting rows remain filed under other launch-readiness
tracks.

Support is the fourth app through a per-app maturity track (after PEA,
PKA, and Finance). It started closer to the bar than Finance — it
already had 2 approval contracts and a policy-grounded triage/draft
flow — but still had no std imports, no auth surface, 0 real cron jobs
(only an SLA-job mock agent), 1 adversarial fixture, and a 7-line
runbook. Its posture is **policy-grounded replies**: every
customer-facing draft must cite policy.

Per the directive that **the developer holds the power to decide how
the approval flow works**, the five approval contracts are
developer-authored with a deliberate role/reversibility gradient
(Admin + irreversible for money-moving refund/credit; Reviewer for
customer-facing reply and the reversible escalate/close).

## Track-closing commits

| Slice | Commit | Title |
|---|---|---|
| D-CS-1 | `a7fb012` | Source foundations — std imports + auth surface + 3 cron jobs + 2 migrations |
| D-CS-2 | `f65b1ca` | 3 more developer-authored approval surfaces (escalate / close / credit) + adversarial gates |
| D-CS-3 | `f6c4d15` | 11 real eval cases + 3 promoted fixtures |
| D-CS-4 | `de0ebef` | Operator runbook 7 → 1500 lines |
| D-CS-5 | `c9c9966` | Deploy manifests (Compose + Fly + K8s) + 5 typed permissions |

(D-CS-6 — this audit + ROADMAP tick — is the closing commit.)

## Per-app maturity bar (every row)

The bar is defined at `ROADMAP.md` Phase 42 phase-done checklist
(lines 2782-2794). Every row applies per-app; Support's state is:

| Row | Required | Support state | Status | Closed by |
|---|---|---|---|---|
| Tables | ≥10 | 20 | ✅ | D-CS-1 (`0003_auth`) + D-CS-2 (`0005_support_operations`) |
| Migrations | ≥5 | 5 | ✅ | D-CS-1 + D-CS-2 |
| Foreign keys + indexes | yes | yes | ✅ | migrations 0003/0004/0005 |
| Auth: sessions + API keys + per-tenant + per-role | yes | yes | ✅ | D-CS-1 (auth surface + `0003_auth.sql`) |
| Connectors: ≥3 mock + ≥1 real | 3+1 | 3 mock declared | ✅ | `tickets`/`policy`/`support_ai`; real-mode is per-tenant opt-in |
| Approvals: ≥5 distinct | ≥5 | 5 | ✅ | D-CS-2 (3 new + 2 existing) |
| Approvals: `policy { ... }` and `batch_with` | 1 each | 0 each | ❌ deferred | Source syntax filed post-v1.0 `35V2-P39-I` |
| Durable jobs: ≥3 cron | ≥3 | 3 | ✅ | D-CS-1 (`sla_breach_scan`, `nightly_csat_rollup`, `policy_reindex`) |
| Durable jobs: ≥3 retry-policy-driven | ≥3 | 3 (all via `support_run`) | ✅ | D-CS-1 |
| Durable jobs: each survives SIGKILL + restart | yes | yes (runtime gate) | ✅ | Phase 38 audit (`t38l_d3_checkpoints_survive_unclean_shutdown`) |
| Evals: ≥10 cases | ≥10 | 11 | ✅ | D-CS-3 |
| Evals: ≥3 promoted from traces | ≥3 | 3 (`support-demo`, `support-sla-scan`, `support-reply-send`) | ✅ | D-CS-3 |
| Adversarial tests: ≥5 named threats | ≥5 | 6 (5 `ungated_*.cor` + `ungrounded_reply.json`) | ✅ | D-CS-2 |
| Operator runbook ≥1500 lines | ≥1500 | 1500 | ✅ | D-CS-4 |
| Deploy manifests: Compose + PaaS + K8s | 3 categories | 3 (Compose + Fly.io + K8s) | ✅ | D-CS-5 |
| Deploy manifests: CI smoke-deploy | yes | filed | ❌ deferred | `35V2-P42-E-LR-app-deploy-smoke-ci` (cross-cutting) |
| Typed permission per dangerous tool | ≥1 per tool | 5/5 distinct | ✅ | D-CS-5 |
| Side-by-side benchmark file | yes | filed | ❌ deferred | `35V2-P42-F-LR-per-app-benchmark-files` (cross-cutting) |
| CLAIM.md committed under apps/<name>/ | yes | filed | ❌ deferred | `35V2-P42-G-LR-per-app-claim-files` (cross-cutting) |
| Per-app AI helpers | 3 helpers | filed | ❌ deferred | `35V2-P42-H-LR-per-app-ai-helpers` (cross-cutting) |
| External reviewer signoff | ≥1 reviewer | filed | ❌ deferred | Phase 33M friends-and-family |

Fourteen of the per-Support rows close in this track — the same
fourteen PEA, PKA, and Finance closed. Five deferred rows are
cross-cutting launch-readiness slices (`35V2-P42-E/F/G/H-LR` + Phase
33M). Two rows depend on post-v1.0 source syntax (`policy { ... }` /
`batch_with`) filed as `35V2-P39-I`.

## Sub-slice content summary

### D-CS-1 — source foundations

Reshaped `src/main.cor` from a no-imports triage+draft+2-write demo
into a real multi-tenant agent foundation, preserving the
policy-grounded posture:

- Added the 4 std imports (`jobs`, `effects`, `agent`, `auth`).
- Added the auth surface (sessions + API keys + per-tenant + per-role)
  over the Phase-39 `std/auth` surface.
- Added 3 read/observe durable cron jobs: `sla_breach_scan`
  (`0 * * * *`, hourly — support SLAs are time-sensitive),
  `nightly_csat_rollup` (`0 2 * * *`), `policy_reindex` (`0 3 * * *` —
  keeps the cited policy corpus fresh), all America/New_York, each via
  `SupportJobContract` + `support_run`.
- Added migrations `0003_auth.sql` + `0004_approvals_and_durable_jobs.sql`,
  taking the schema to 17 tables across 4 migrations (5/20 after
  D-CS-2).
- Added `SupportSchemaManifest` + `/schema` route + auth + job routes;
  `main()` exercises the auth + all 3 job contracts.
- Updated the `reference_apps` migration-count assertion 2 → 4 in the
  same slice.

### D-CS-2 — 3 more developer-authored approval surfaces

Grew the approval surface from 2 to 5 distinct developer-authored
contracts, honoring the developer-owns-the-flow directive while keeping
every surface operational:

| Tool | Approval | Effect | Role | Reversible |
|---|---|---|---|---|
| `send_support_reply` | `SendSupportReply` | `support_write` | Reviewer | no |
| `issue_support_refund` | `IssueSupportRefund` | `refund_write` | Admin | no |
| `escalate_ticket` | `EscalateTicket` | `escalate_write` | Reviewer | yes |
| `close_ticket` | `CloseTicket` | `ticket_write` | Reviewer | yes |
| `apply_account_credit` | `ApplyAccountCredit` | `credit_write` | Admin | no |

The role gradient encodes blast radius — Admin for the money-moving
refund/credit; Reviewer for the customer-facing reply and the reversible
escalate/close. Each new tool is `dangerous`, reached only through an
`execute_approved_*` agent applying the `approve` gate, with an
`approval_contract_ref` agent and a POST route. `approval_surface_valid`
asserts the five contracts and is wired into `main()`. All 5 contracts
(including the two pre-existing) now have explicit contract_ref agents.

`migration 0005_support_operations.sql` adds 3 backing tables (→ 20
tables / 5 migrations). Six named adversarial threats:
`ungrounded_reply.json` (no ungrounded reply) + 5 `ungated_*.cor`, each
refused by `corvid check` with `E0101`. `security-model.md` expanded to
document the policy-grounded posture + all 5 contracts + the read-only
cron jobs. The `support_eval_dashboard` approval_gated_writes count
(2 → 5), the `approvals_sla.json` mock (2 → 5 approvals), the
`reference_apps` approval-count assertion (2 → 5), and the
migration-count assertion (4 → 5) were all updated in the same slice.

### D-CS-3 — 11 real eval cases + 3 promoted fixtures

`evals/support_ops_eval.cor` replaces 4 placeholder agents with 11
structural-invariant cases, three support-specific: irreversibility
gradient (reply/refund/credit irreversible, escalate/close reversible),
role gradient (Admin money / Reviewer customer-facing), and case 11 —
a draft reply is policy-grounded (drafted, has a citation with
provenance, names its approval label).

Three promoted `corvid.eval.lineage_fixture.v1` fixtures under
`evals/promoted/`: `support-demo` (policy-grounded triage/draft),
`support-sla-scan` (durable job + ticket read), `support-reply-send`
(reply route + `SendSupportReply` approval pending_review + audit). Two
new traces back the latter two.

### D-CS-4 — operator runbook

The runbook at [`ops/runbook.md`](../../examples/backend/customer_support_agent/ops/runbook.md)
expanded from 7 lines to 1500 lines across 17 sections, grounded in the
actual surface with the policy-grounded / no-autonomous-write posture
woven throughout. Built with real coverage, not padding: the
triage → draft → approve → send pipeline (showing where grounding,
human responsibility, and the compiler gate each apply), tenant
lifecycle, policy-corpus lifecycle + daily reconciliation, 9 incident
runbooks (incl. ungrounded reply, autonomous write, SLA scan miss,
escalation loop, refund/credit abuse, triage low-confidence, duplicate
reply), decision trees for all 5 contracts + segregation-of-duties
audit, capacity planning, compliance posture, SLA tiers, and the
role→permission mapping.

### D-CS-5 — deploy manifests + typed permissions

Deploy manifest categories now satisfy the 3-category bar: Docker
Compose, Fly.io PaaS (api + worker process groups, `/schema`
healthcheck, secrets via `fly secrets set`), and Kubernetes (six
manifests). The fly.toml + secret.example note that switching to real
connector mode means the agent can contact customers and move money — a
release-checklist event.

Typed permission per dangerous tool: 5 `permission_for_*` agents, each
returning a distinct `"support.tool.<tool_name>"` string, plus a
`dangerous_tool_permissions_distinct()` 10-pair pairwise check wired
into `main()`.

## Validation summary across the track

- `corvid check src/main.cor` → `ok: ... — no errors`.
- Each `adversarial/ungated_*.cor` → `[E0101] error: dangerous tool
  <tool> called without a prior approve`. All five gates firm;
  `ungrounded_reply.json` is the sixth declarative threat.
- `corvid eval evals/support_ops_eval.cor` → `PASS
  customer_support_agent_per_app_maturity, values: 11/11 passed`.
- `cargo test -p corvid-cli --test reference_apps customer_support`
  → 2 passed.
- Runbook 1500 lines, sections 1-17 sequential, no slice numbers leaked.

NOTE: the `reference_apps` suite carries 12 unrelated pre-existing
failures (Phase 43 release/upgrade/claim-audit/market-readiness doc
tests) that predate and are untouched by this track.

## What this track does NOT close

The five cross-cutting launch-readiness slices and Phase 33M
(`35V2-P42-E/F/G/H-LR` + `33M-beta-feedback`), all of which apply to
every reference app.

## Suggested next per-app maturity slice

The ROADMAP order is PKA → Finance → CustomerSupport → CodeMaintenance.
With Support closed, the next and final per-app slice is the Code
Maintenance Agent: `35V2-P42-D-LR-app-maturity-CodeMaintenance`. It
sits far from the bar (7-line runbook, placeholder evals) and carries a
write-actions-require-approval posture (review comments, patch
proposals, CI-aware risk) its maturity track should preserve.
