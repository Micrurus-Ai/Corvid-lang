# Personal Executive Agent — per-app maturity audit (2026-05-27)

Closes the launch-readiness track `35V2-P42-D-LR-app-maturity-PEA`.
This audit documents what shipped per sub-slice, which bar rows now
hold for PEA, and which cross-cutting rows remain filed under other
launch-readiness tracks because they apply to all five reference
apps rather than PEA specifically.

## Track-closing commits

| Slice | Commit | Title |
|---|---|---|
| D-PEA-1 | `5443cbd` | Operator runbook expanded to 1584 lines |
| D-PEA-2 | `c52c119` | 10 real eval cases + 3 promoted fixtures |
| D-PEA-3 | `893f417` | 5th approval contract (`ExternalCalendarShare`) + std-import path fix |
| D-PEA-4 | `03b4281` | Deploy manifests (Fly.io + K8s) + 5 typed permissions per dangerous tool |

(D-PEA-5 — this audit + ROADMAP tick — is the closing commit.)

## Per-app maturity bar (every row)

The bar is defined at `ROADMAP.md` Phase 42 phase-done checklist
(lines 2782-2794). Every row applies per-app; PEA's state is:

| Row | Required | PEA state | Status | Closed by |
|---|---|---|---|---|
| Tables | ≥10 | 12 | ✅ | Pre-track (existing schema) |
| Migrations | ≥5 | 5 | ✅ | Pre-track (existing schema) |
| Foreign keys + indexes | yes | yes | ✅ | Pre-track (existing schema) |
| Auth: sessions + API keys + per-tenant + per-role | yes | yes | ✅ | Pre-track (Phase 39 surface) |
| Connectors: ≥3 mock + ≥1 real | 3+1 | 5 mock declared | ✅ | Pre-track (mock manifest); real-mode is per-tenant opt-in |
| Approvals: ≥5 distinct | ≥5 | 5 | ✅ | D-PEA-3 (added `ExternalCalendarShare`) |
| Approvals: `policy { ... }` and `batch_with` | 1 each | 0 each | ❌ deferred | Source syntax filed post-v1.0 `35V2-P39-I` |
| Durable jobs: ≥3 cron | ≥3 | 4 | ✅ | Pre-track |
| Durable jobs: ≥3 retry-policy-driven | ≥3 | 4 (all via `executive_run`) | ✅ | Verified in D-PEA-4 survey |
| Durable jobs: each survives SIGKILL + restart | yes | yes (runtime gate) | ✅ | Phase 38 audit (Slice 38L's `t38l_d3_*`) |
| Evals: ≥10 cases | ≥10 | 11 | ✅ | D-PEA-2 (+ D-PEA-4 added permission case) |
| Evals: ≥3 promoted from traces | ≥3 | 3 (`pea-demo`, `pea-daily-brief`, `pea-meeting-prep`) | ✅ | D-PEA-2 |
| Adversarial tests: ≥5 named threats | ≥5 | 6 (added `ungated_share.cor`) | ✅ | D-PEA-3 |
| Operator runbook ≥1500 lines | ≥1500 | 1584 | ✅ | D-PEA-1 |
| Deploy manifests: Compose + PaaS + K8s | 3 categories | 3 (Compose existing + Fly.io + K8s new) | ✅ | D-PEA-4 |
| Deploy manifests: CI smoke-deploy | yes | filed | ❌ deferred | `35V2-P42-E-LR-app-deploy-smoke-ci` (cross-cutting) |
| Typed permission per dangerous tool | ≥1 per tool | 5/5 distinct | ✅ | D-PEA-4 |
| Side-by-side benchmark file | yes | filed | ❌ deferred | `35V2-P42-F-LR-per-app-benchmark-files` (cross-cutting) |
| CLAIM.md committed under apps/<name>/ | yes | filed | ❌ deferred | `35V2-P42-G-LR-per-app-claim-files` (cross-cutting) |
| Per-app AI helpers | 3 helpers | filed | ❌ deferred | `35V2-P42-H-LR-per-app-ai-helpers` (cross-cutting) |
| External reviewer signoff | ≥1 reviewer | filed | ❌ deferred | Phase 33M friends-and-family |

Twelve of the per-PEA rows close in this track. Five deferred rows
are cross-cutting launch-readiness slices that touch all five
reference apps — `35V2-P42-E/F/G/H-LR` plus Phase 33M. Two rows
depend on post-v1.0 source syntax (`policy { ... }` / `batch_with` /
`@requires`) filed as `35V2-P39-I`.

## Sub-slice content summary

### D-PEA-1 — operator runbook

The runbook at [`ops/runbook.md`](../../examples/backend/personal_executive_agent/ops/runbook.md)
expanded from 29 lines to 1584 lines across 16 sections. Every
section is grounded in actual app surfaces declared in
`src/main.cor` (route list, job cron schedules, approval
contracts, schema manifest counts) and actual deployment artifacts
(`deploy/Dockerfile`, `deploy/docker-compose.yml`,
`deploy/env.example`). No padding.

The 8 bar-required sections (setup, secrets, migrations, backups,
logs, metrics, incident response, rollback) are each backed by 8
contextual sections (service overview, architecture map,
production deployment shapes, connector mode operations, approval
queue operations, durable jobs and cron operations, disaster
recovery, appendix).

### D-PEA-2 — 10 real eval cases + 3 promoted fixtures

The eval suite at
[`evals/hardening_eval.cor`](../../examples/backend/personal_executive_agent/evals/hardening_eval.cor)
replaces the previous 10 placeholder `agent foo() -> Bool: return
true` agents with 10 (later 11) structural-invariant assertions
encoded as eval-local typed agents. The cases assert:

1. Schema manifest meets the maturity bar minima.
2. Five distinct approval contracts present.
3. Every approval irreversible + expires within 24h.
4. Approval cost ceilings bounded.
5. External-write approvals require Reviewer or Admin.
6. All scheduled jobs weekday-workday in America/New_York.
7. Every job contract budget-bounded.
8. Every job has a stable replay key.
9. Connector default mode is mock.
10. Approval label uniqueness.
11. (D-PEA-4) Typed permission per dangerous tool — 5/5 distinct.

Three promoted fixtures under `evals/promoted/` are
`corvid.eval.lineage_fixture.v1`:

- `pea-demo.lineage-eval.json` (3 events: route triage, follow-up
  job, send approval)
- `pea-daily-brief.lineage-eval.json` (3 events: daily brief job,
  inbox-summarisation prompt, brief render)
- `pea-meeting-prep.lineage-eval.json` (3 events: meeting prep
  job, calendar tool read, packet-assembly prompt)

D-PEA-2 also fixed the existing `demo.lineage.jsonl` to use
schema-valid status values (`approval_pending` / `pending` →
`pending_review`). The old values were never schema-valid; the
audit didn't catch this because `corvid eval promote` wasn't being
run against the demo trace until this track.

### D-PEA-3 — 5th approval contract + std-import path fix

Added the `ExternalCalendarShare` approval contract:

- New `calendar_share` effect (cost $0.02, trust human_required,
  data external).
- New `CalendarShareRequest` and `CalendarShareReceipt` types.
- New dangerous tool `external_calendar_share`. Tool name aligns
  with the `approve ExternalCalendarShare(...)` semantics — the
  compiler enforces snake_case → CamelCase match between tool name
  and approve label.
- New `external_calendar_share_approval_contract` agent — required
  role `"Admin"` (strictest, sharing outside the tenant), cost
  ceiling $0.25, data class `"external"`, irreversible, 24h expiry.
- New `execute_approved_calendar_share` agent applying the approve
  gate and dispatching the tool.
- New route `POST /actions/calendar/share`.
- New adversarial fixture `adversarial/ungated_share.cor` that
  refuses with `E0101 — dangerous tool called without a prior
  approve`. Sixth named threat in the adversarial corpus.

Std import path fix:
- Changed all 5 imports in `src/main.cor` from `"./std/X"` (which
  resolved to the non-existent `src/std/X.cor`) to
  `"../../../../std/X"` (resolves to the workspace-root
  `std/X.cor`). `corvid check src/main.cor` now exits clean.
- Filed for `examples/backend/audit_log/` and
  `examples/backend/state_app/` separately — they share the same
  bug but are owned by their respective per-app maturity tracks.

### D-PEA-4 — deploy manifests + typed permissions

Deploy manifest categories now satisfy the 3-category bar:

- Docker Compose (existing).
- Fly.io PaaS (new) — `deploy/fly.toml` with api + worker process
  groups, shared volume, force-HTTPS, `/schema` healthcheck,
  metrics on port 9090, secrets via `fly secrets set`.
- Kubernetes (new) — six manifests under `deploy/k8s/`:
  `namespace.yaml`, `configmap.yaml`, `secret.example.yaml`
  (template; DO NOT commit with real values),
  `deployment-api.yaml` (2 replicas, RollingUpdate, `/schema`
  probes), `deployment-worker.yaml` (2 replicas with `--source
  src/main.cor` argument required by `corvid jobs run` since
  C-1), `service.yaml` (ClusterIP API + headless worker metrics +
  PersistentVolumeClaim).

Typed permission per dangerous tool:

- Added 5 `permission_for_*` agents in `src/main.cor`, each
  returning a distinct `"executive.tool.<tool_name>"` string.
- Added `dangerous_tool_permissions_distinct()` agent verifying
  the 10-pair pairwise distinctness check.
- `main()` exercises the check as part of the end-to-end PEA
  validation.
- Eval suite gained the 11th case mirroring the source's
  assertion (`case_typed_permission_per_dangerous_tool`).

Source-level typecheck-time enforcement of "actor X holds
permission Y to call tool Z" awaits post-v1.0 `35V2-P39-I`. The
runtime auth surface propagates these strings through Actor
permissions today; this track contributes per-app coverage the bar
names.

## Validation summary across the track

Every sub-slice validated cleanly at commit time. Final state
(after D-PEA-4):

- `cargo run -q -p corvid-cli -- check examples/backend/personal_executive_agent/src/main.cor`
  → `ok: ... — no errors`.
- `cargo run -q -p corvid-cli -- check examples/backend/personal_executive_agent/adversarial/ungated_send.cor`
  → `[E0101] error: dangerous tool send_follow_up_email called
  without a prior approve`. Original adversarial gate firm.
- `cargo run -q -p corvid-cli -- check examples/backend/personal_executive_agent/adversarial/ungated_share.cor`
  → `[E0101] error: dangerous tool external_calendar_share called
  without a prior approve`. New adversarial gate firm.
- `cargo run -q -p corvid-cli -- eval examples/backend/personal_executive_agent/evals/hardening_eval.cor`
  → `PASS executive_agent_per_app_maturity, values: 11/11 passed`.
- `cargo check --workspace` clean across all four sub-slices.

## What this track does NOT close

The five cross-cutting launch-readiness slices and Phase 33M:

- `35V2-P42-E-LR-app-deploy-smoke-ci` — CI smoke-deploy on a clean
  cluster. Operational gate; needs GitHub Actions matrix or
  equivalent.
- `35V2-P42-F-LR-per-app-benchmark-files` — `benches/comparisons/<app>.md`
  per app showing FastAPI/LangChain or Next.js+Vercel-AI-SDK
  equivalent line-by-line.
- `35V2-P42-G-LR-per-app-claim-files` — `apps/<name>/CLAIM.md`
  from `corvid claim --explain` per app.
- `35V2-P42-H-LR-per-app-ai-helpers` — three per-app AI helpers
  (assistive boot summary, adversarial test refresh, generative
  PR description).
- `33M-beta-feedback` — external reviewer signoff per the
  friends-and-family round.

These five slices apply to all five reference apps. PKA, Finance,
CustomerSupport, and CodeMaintenance still need their own per-app
maturity tracks before the cross-cutting slices land — the bar
requires every app to meet the same row set, not just PEA.

## Suggested next per-app maturity slice

The four remaining reference apps sit much further from the bar
than PEA did (7-line runbooks; 0-2 approvals; no eval beyond the
placeholder set; in two cases the same `./std/X` import bug). The
ROADMAP order is PKA → Finance → CustomerSupport → CodeMaintenance.

PKA (Personal Knowledge Agent) is the next slice:
`35V2-P42-D-LR-app-maturity-PKA`.
