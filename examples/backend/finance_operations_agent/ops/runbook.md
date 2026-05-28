# Finance Operations Agent — Operator Runbook

This runbook is the operational source of truth for running the Finance
Operations Agent backend in development, staging, and production. The
Finance agent is a reference Corvid application: it aggregates read-only
financial data (accounts, budgets, subscriptions, anomalies), surfaces
operational summaries, and executes financial operations a human has
approved (payment intents, subscription cancellations, transaction
disputes, report exports, recurring payments) — each behind a typed,
developer-authored approval contract.

**The defining constraint: this agent never gives regulated financial
advice.** Every surface is "do the operation a human decided", never
"recommend what to do". The summaries describe; they do not prescribe.
There is no tool that recommends an allocation, predicts a return, or
tells a user what to buy or sell. That is an explicit, compiler-shaped
non-goal, not a disclaimer.

Every procedure below is grounded in surfaces the app actually ships.
The schema manifest at [`src/main.cor`](../src/main.cor) declares the
canonical counts (5 migrations / 23 tables / 3 connectors / 3 durable
jobs / 5 approval contracts / non-advice) and `corvid serve` exposes the routes that drive each procedure.

## Table of contents

1. [Service overview](#1-service-overview)
2. [Architecture map](#2-architecture-map)
3. [Setup — local development](#3-setup--local-development)
4. [Setup — staging and production deployment](#4-setup--staging-and-production-deployment)
5. [Secrets management](#5-secrets-management)
6. [Migrations — apply, drift, rollback](#6-migrations--apply-drift-rollback)
7. [Backups — what, where, how often](#7-backups--what-where-how-often)
8. [Logs and traces](#8-logs-and-traces)
9. [Metrics and alerting](#9-metrics-and-alerting)
10. [Incident response — diagnose and recover](#10-incident-response--diagnose-and-recover)
11. [Rollback procedures](#11-rollback-procedures)
12. [Connector mode operations](#12-connector-mode-operations)
13. [Approval queue operations](#13-approval-queue-operations)
14. [Tenant lifecycle operations](#14-tenant-lifecycle-operations)
15. [Durable jobs and cron operations](#15-durable-jobs-and-cron-operations)
16. [Disaster recovery](#16-disaster-recovery)
17. [Appendix — reference data](#17-appendix--reference-data)

---

## 1. Service overview

### What the Finance Operations Agent does

The Finance Operations Agent runs three durable jobs on a daily/weekly
cron and exposes a typed HTTP surface for the operational work a finance
team's assistant performs:

- `nightly_balance_sync` (cron `0 2 * * *` America/New_York) — re-pulls
  account balances and recent transactions from the registered
  account connectors, writes the refreshed `finance_accounts` rows, and
  recomputes budget spend. Read-only with respect to the outside world;
  it never moves money.
- `weekly_anomaly_scan` (cron `0 6 * * 1` America/New_York) — scans the
  week's transactions for spend anomalies, writing descriptive
  `finance_anomalies` rows with a confidence score and an explanation
  fingerprint. The explanation describes what looks unusual; it does
  not recommend an action.
- `daily_subscription_renewal_check` (cron `0 7 * * *` America/New_York)
  — surfaces upcoming subscription renewals as `finance_reminders` so a
  human can decide whether to keep or cancel. It reminds; it does not
  cancel.

Every financial write enters one of five approval contracts. The flow
is the developer's choice — role, ceiling, and irreversibility differ
per surface on purpose:

| Approval label | Tool | Effect | Target type | Required role | Cost ceiling | Irreversible |
|---|---|---|---|---|---|---|
| `SubmitPaymentIntent` | `submit_payment_intent` | `payment_write` | `payee` | Admin | $0.25 | yes |
| `CancelSubscription` | `cancel_subscription` | `subscription_write` | `subscription` | Reviewer | $0.05 | no |
| `DisputeTransaction` | `dispute_transaction` | `dispute_write` | `transaction` | Reviewer | $0.05 | no |
| `ExportFinancialReport` | `export_financial_report` | `report_export` | `export_target` | Admin | $0.25 | yes |
| `ScheduleRecurringPayment` | `schedule_recurring_payment` | `recurring_payment_write` | `payee` | Admin | $0.25 | yes |

The role gradient encodes blast radius: the three Admin contracts move
money or send financial data outside the tenant; the two Reviewer
contracts are reversible operational actions (you can re-subscribe or
withdraw a dispute).

### What the Finance Operations Agent does NOT do

This list is load-bearing — it is the non-advice posture made explicit:

- **It does not give financial advice.** No tool recommends an
  investment, predicts a return, suggests an allocation, or tells a
  user what to buy, sell, or hold. The anomaly explanations are
  descriptive ("this charge is 3x the category median"), never
  prescriptive ("you should cancel this").
- **It does not move money autonomously.** Every `payment_write`,
  `subscription_write`, `dispute_write`, `recurring_payment_write`, and
  `report_export` is `dangerous` and the compiler rejects any caller
  that lacks a matching `approve <Label>(...)` boundary. Creating a
  payment *intent* is allowed; *executing* it without approval does not
  compile.
- **It does not talk to real provider APIs in the default mode.** The
  default `CORVID_CONNECTOR_MODE=mock` keeps every connector offline
  with deterministic fixtures so `corvid eval` and `corvid replay` stay
  reproducible.
- **It does not store raw account numbers, card PANs, or full
  statements in the database.** The DB holds fingerprints, cent
  amounts, and provenance ids. Raw statement bytes (for an approved
  export) live transiently in the export pipeline, never at rest in the
  app DB.
- **It does not decide regulated outcomes.** Credit decisions, tax
  determinations, and suitability assessments are explicit non-goals;
  the security model names them.

### Service-level objectives

- Availability: 99.9 % monthly for `GET /readonly/*`; 99.5 % for `POST
  /payments/*`, `/subscriptions/*`, `/transactions/*`, `/reports/*`
  (lower because they are approval-gated).
- Latency (p99): `/readonly/snapshot` < 800 ms, `/payments/intents`
  (intent creation, not execution) < 500 ms. Approved executions are
  async through the durable-job pool — see §15.
- Non-advice integrity: 100 % of served summaries carry `non_advice =
  true`. Any surface that returns advisory content is a Sev-1.
- Approval integrity: 100 % of executed financial writes have a matching
  `approved` row in `approvals` with a co-sign where the contract
  requires one. Any execution without it is a Sev-1.

---

## 2. Architecture map

### Process layout

The Finance agent runs as a single Corvid server binary plus a SQLite
or Postgres backing store. The app is served by `corvid serve` (the interpreter-backed HTTP server). In production the binary is wrapped in
a distroless OCI image; the same binary serves all HTTP routes, runs
the durable-job pool, the scheduler, the OTLP exporter, and the metrics
endpoint.

```
+---------------------------+
|   corvid jobs run         |
|   (durable job pool,      |  <-- nightly_balance_sync,
|    in-process)            |       weekly_anomaly_scan,
+---------------------------+       daily_subscription_renewal_check
            |
            v
+---------------------------+
|   corvid runtime          |  <-- typed effects, approvals,
|   (HTTP routes,           |       non-advice posture, replay
|    scheduler,             |       quarantine
|    OTLP exporter,         |
|    /metrics)              |
+---------------------------+
            |
   +--------+--------+
   |                 |
   v                 v
+------+        +-----------------+
| DB   |        | Payment         |
| (5   |        | provider        |
|  migs|        | (real mode only,|
| / 23 |        |  approval-gated)|
| tbls)|        +-----------------+
+------+
```

### Data classes

Every effect in the Finance agent carries the `financial` data class.
The compiler refuses to cross from internal-financial to
external-financial (money or data leaving the tenant) without an
explicit approval. Operationally the distinction is:

- **Internal-financial** — balances, budgets, subscriptions, anomaly
  explanations. Effects `finance_read`, `finance_ai`. Never leaves the
  tenant partition; never moves money.
- **External-financial** — anything that moves money or sends financial
  data outside the tenant. Effects `payment_write`,
  `subscription_write`, `dispute_write`, `recurring_payment_write`,
  `report_export`. Each requires its typed approval contract.

Note there is no `advisory` data class because there is no advisory
surface. If a future change ever needs one, that is a security-model
amendment and a new approval contract, not a quiet addition.

### Storage surfaces

- **Database** (5 migrations, 23 tables):
  - Finance domain (`0001`/`0002`/`0005`, 11 tables):
    `finance_accounts`, `finance_budgets`, `finance_subscriptions`,
    `finance_reminders`, `finance_anomalies`, `finance_payment_intents`,
    `finance_audit_records`, `finance_subscription_cancellations`,
    `finance_transaction_disputes`, `finance_report_exports`,
    `finance_recurring_payments`.
  - Auth (`0003`, 7 tables): `tenants`, `users`, `roles`, `user_roles`,
    `sessions`, `api_keys`, `permissions`.
  - Approvals + jobs + lineage (`0004`, 5 tables): `approvals`,
    `audit_events`, `queue_jobs`, `queue_job_checkpoints`,
    `trace_lineage`.
- **Payment provider** (real mode only): the external system that
  actually moves money. Reached only through the approval-gated write
  tools. In mock mode it is a deterministic fixture.
- **Export destination** (real mode only): object storage or an SFTP
  target for an approved `ExportFinancialReport`. The financial data
  that leaves the tenant boundary lands here.

### Connector layout

The Finance agent's three connectors are:

- `accounts_connector` (effect: `finance_read`) — reads balances,
  transactions, budgets, and subscriptions from the account providers
  (bank aggregator, card provider, or manual CSV import).
- `insights_connector` (effect: `finance_ai`) — runs the bounded
  anomaly-detection / explanation model. Bounded trust: it produces
  descriptive explanations, never advice.
- `payment_provider_connector` (effects: `payment_write`,
  `subscription_write`, `dispute_write`, `recurring_payment_write`) —
  executes approved financial operations against the payment provider.

`report_export` routes through the runtime's `IoRuntime` to the export
destination; it is *not* a connector and has no mock mode — it fails
closed when approval is missing.

---

## 3. Setup — local development

### Prerequisites

- Corvid toolchain installed (`corvid --version` reports a tag in the
  `35V2` series).
- SQLite 3.40+ (default) or Postgres 14+ (set `CORVID_DATABASE_URL` to a
  `postgres://` URL).
- A writable `target/` directory in the workspace root.

### First-time local boot

```
# from the repo root
cd examples/backend/finance_operations_agent
export CORVID_APP_ENV=local              # local | staging | production
export CORVID_CONNECTOR_MODE=mock        # default; keeps everything offline
export CORVID_DATABASE_URL=sqlite:target/finance.db
export CORVID_REQUIRE_APPROVALS=true     # default; fail closed on every dangerous tool
corvid check src/main.cor
corvid migrate --database-url=$CORVID_DATABASE_URL --dir=migrations
corvid seeds load seeds/demo.sql
corvid serve src/main.cor --listen 127.0.0.1:8087
```

If everything is wired correctly, `corvid serve` exposes the routes on
port 8087. `GET /config` returns the
`FinanceConfig("finance_operations_agent", "mock", false, true)`
envelope — note `regulated_advice = false`.

### Smoke test the local boot

In a second shell:

```
curl -s http://127.0.0.1:8087/schema | jq
curl -s http://127.0.0.1:8087/readonly/snapshot/mock | jq '.readonly, .non_advice'
curl -s http://127.0.0.1:8087/payments/intents/mock | jq '.execute_without_approval, .non_advice'
curl -s http://127.0.0.1:8087/jobs/nightly-balance-sync/mock | jq '.contract.job_kind'
curl -s http://127.0.0.1:8087/jobs/weekly-anomaly-scan/mock | jq '.contract.job_kind'
```

Expected: `readonly` and `non_advice` are `true`,
`execute_without_approval` is `false`, the job kinds match. If
`execute_without_approval` is ever `true` or `non_advice` is `false`,
*do not deploy* — a core safety invariant has regressed.

### Run the typed eval suite

```
corvid eval evals/payment_audit_eval.cor
```

Must exit 0 with `values: 11/11 passed`. The suite covers the maturity
bar minima, the five approval contracts, the role/irreversibility
gradient, the three cron schedules, job bounding, and the non-advice
posture (case 11).

### Confirm the adversarial gates

```
corvid check adversarial/ungated_cancel_subscription.cor      # → E0101
corvid check adversarial/ungated_dispute_transaction.cor      # → E0101
corvid check adversarial/ungated_export_financial_report.cor  # → E0101
corvid check adversarial/ungated_schedule_recurring_payment.cor  # → E0101
```

Each must exit `1` with `E0101 — dangerous tool called without a prior
approve`. The declarative `adversarial/autonomous_payment.json` is the
fifth named threat (no autonomous payment execution).

### Promote a new fixture

```
corvid eval promote traces/<trace>.lineage.jsonl --promote-out evals/promoted
```

The promotion writes a `corvid.eval.lineage_fixture.v1` record. The
record is checked into git so the next release replays it
deterministically.

---

## 4. Setup — staging and production deployment

### Topology

The reference deployment is three tiers:

- **Edge** — TLS termination, request authentication (mTLS or
  signed-bearer), rate limit, request id injection. The Finance agent
  does not bundle an edge proxy; use `nginx`, `envoy`, or the cloud
  provider's L7 LB.
- **App** — N replicas of the binary behind the edge. Each replica runs
  the HTTP server, durable-job pool, and scheduler. The scheduler uses
  a per-job advisory lock so only one replica runs each cron tick.
- **Data** — Postgres primary + read replica. There is no object-store
  requirement unless `ExportFinancialReport` is enabled in real mode,
  in which case the export destination is a separate, tenant-owned
  bucket or SFTP target.

### Fly.io reference

`deploy/fly.toml` defines the canonical Fly.io deployment. Key
parameters:

- `app = "finance-operations-agent"`
- Primary region: `iad`.
- VM size: `shared-cpu-1x`, 512 MB (the agent is light — no local
  embedding model).
- Auto-scaling: 1 to 3 replicas; HTTP service on internal port `8080`.
- `[env]`: `CORVID_CONNECTOR_MODE = "mock"` by default. Production
  unsets this to `real` only after the operator has verified
  payment-provider credentials are configured (see §5) AND the
  non-advice posture has been re-confirmed in the release checklist.

Deploy:

```
flyctl deploy --config deploy/fly.toml
flyctl logs --app finance-operations-agent
flyctl ssh console --app finance-operations-agent --command "corvid ops show"
```

The `corvid ops show` snapshot is signed and dated; archive each
production snapshot under `ops/snapshots/<YYYY-MM-DD>.json` so audits
can diff a release against the deployed surface. For a finance agent
this archive is also part of the compliance trail.

### Kubernetes reference

`deploy/k8s/` defines the canonical Kubernetes deployment in six files:
`namespace.yaml`, `configmap.yaml`, `secret.example.yaml` (template),
`service.yaml` (ClusterIP API + headless worker metrics + PVC),
`deployment-api.yaml` (2 replicas, RollingUpdate, `/schema` probes),
`deployment-worker.yaml` (durable job pool).

Deploy:

```
kubectl apply -f deploy/k8s/namespace.yaml
kubectl apply -f deploy/k8s/configmap.yaml
kubectl apply -f deploy/k8s/secret.example.yaml   # after editing real values
kubectl apply -f deploy/k8s/service.yaml
kubectl apply -f deploy/k8s/deployment-api.yaml
kubectl apply -f deploy/k8s/deployment-worker.yaml
kubectl -n finance rollout status deploy/finance-api
```

### Docker Compose (single-host)

For staging or evaluation hosts:

```
cd examples/backend/finance_operations_agent
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml exec finance corvid ops show
```

### Boot sequence

Every replica follows the same boot sequence:

1. Read env. If any `CORVID_*` variable is malformed, exit non-zero.
2. Connect to the DB. Five-attempt exponential backoff (2/4/8/16/32 s),
   then exit non-zero so the orchestrator restarts the pod.
3. Run migrations 0001 → 0005 (idempotent — see §6).
4. If `CORVID_CONNECTOR_MODE=real`, verify payment-provider credentials
   are present and valid; fail closed if not (a finance agent must
   never boot into real mode without verified write credentials).
5. Start the OTLP exporter and `/metrics` endpoint.
6. Start the durable-job pool. Resume any jobs left `running` whose
   lease expired; mark them `retryable`.
7. Start the scheduler. Compute next-fire timestamps for the three
   cron schedules.
8. Bind the HTTP listener.

If steps 1–4 fail, the binary exits and the orchestrator restarts it.

---

## 5. Secrets management

### Inventory

The Finance agent stores four classes of secrets:

- **Database credentials** — Postgres URL with username + password, or
  the SQLite file path. Read at boot; never logged.
- **Payment-provider credentials** — API keys / OAuth tokens for the
  payment provider that executes approved writes. Stored *only* in the
  `connector_tokens` table (encrypted with the connector-token key) and
  never in env.
- **Export-destination credentials** — bucket / SFTP credentials for an
  approved `ExportFinancialReport`. Same storage rules.
- **Connector-token key + auth secrets** — `CORVID_CONNECTOR_TOKEN_KEY`
  (AES-256, encrypts connector tokens at rest), `CORVID_API_KEY_PEPPER`
  (Argon2id pepper for API-key hashing), `CORVID_SESSION_SIGNING_KEY`,
  and `CORVID_CSRF_SECRET`. Injected from the secret manager at boot.

### Where to store them

- **Production:** secret manager (Vault, AWS SM, GCP SM, K8s Secret).
  Never in env files committed to git, never in `configmap.yaml`, never
  in shell history.
- **Staging:** same, in a separate secret namespace so a staging
  compromise cannot leak prod payment credentials.
- **Local:** a developer-private `.env.local` (gitignored). Never check
  in real payment credentials; use mock connectors.

### Rotation

- **DB credentials** — rotate quarterly. The Postgres user has only the
  `finance_app` role; rotate by minting a new user, updating the
  secret, rolling the deployment, then dropping the old user.
- **Payment-provider credentials** — rotate per the provider's policy,
  at minimum quarterly. A finance agent's payment credentials are the
  highest-value secret; treat a suspected leak as a Sev-1 and rotate
  immediately, which also pauses all real-mode writes until the new
  credential is verified.
- **Connector-token key** — rotate annually or on suspected compromise
  via `corvid auth keys rotate --kind connector-token`. Runs under a
  maintenance window because the HTTP listener pauses for the duration.
- **API-key pepper / session signing / CSRF secret** — standard
  cadence: pepper rotation invalidates existing API keys (coordinate
  with tenants), session signing rotates every 30 days with a 15-minute
  grace, CSRF secret rotates immediately with old tokens rejected.

### What never gets logged

- Plaintext payment-provider credentials or account numbers.
- Raw transaction detail or full statements (only fingerprints and cent
  amounts).
- Approval payloads beyond the `approval_id` and the target type.
- Anomaly explanation text (only the explanation fingerprint).

The redaction policy is enforced by the runtime's
`redaction_policy_hash` on every trace span. A span missing that field
or not matching the active policy is dropped and an alert fires (§9).

---

## 6. Migrations — apply, drift, rollback

### Applying migrations

```
corvid migrate --database-url=$CORVID_DATABASE_URL --dir=migrations
```

Migrations are idempotent: every migration begins with the
schema-version check. Re-running an applied migration is a no-op.

### The five migrations

- `0001_readonly_finance.sql` — read-only domain: `finance_accounts`,
  `finance_budgets`, `finance_subscriptions`, `finance_reminders`,
  `finance_anomalies`.
- `0002_payment_intents.sql` — `finance_payment_intents`,
  `finance_audit_records`.
- `0003_auth.sql` — `tenants`, `users`, `roles`, `user_roles`,
  `sessions`, `api_keys`, `permissions`.
- `0004_approvals_and_durable_jobs.sql` — `approvals`, `audit_events`,
  `queue_jobs`, `queue_job_checkpoints`, `trace_lineage`.
- `0005_finance_operations.sql` — backing tables for the four
  operational write surfaces: `finance_subscription_cancellations`,
  `finance_transaction_disputes`, `finance_report_exports`,
  `finance_recurring_payments`.

### Detecting drift

```
corvid migrate --check --database-url=$CORVID_DATABASE_URL --dir=migrations
```

Compares the DB's `schema_version` table to the migration directory.
Non-zero exit on any mismatch. `corvid ops show` includes the schema
version and each migration's file hash; archive it per release.

### Rollback

Finance migrations are **forward-only**. There is no `migrate down`. To
revert to a prior schema:

1. Restore the DB from the latest backup taken before the bad migration
   (§7).
2. Roll back the deploy to the matching binary version.
3. File the regression and write a fix-forward migration.

Forward-only is doubly important for a finance agent: a down-migration
that dropped `approvals` or `audit_events` rows would destroy the
compliance trail. Restore-and-fix-forward preserves it.

---

## 7. Backups — what, where, how often

### What gets backed up

- **DB**: full snapshot + WAL ship every 15 min (Postgres) or hot-copy
  every hour (SQLite).
- **Approvals + audit_events + the four operation tables**: included in
  the DB backup. These are the canonical record of every executed
  financial operation and must never be lost.
- **Export destination**: if real-mode exports are enabled, the export
  bucket has its own versioning + retention managed by the tenant.

### Where

- DB backups: cloud-provider managed object storage in a *separate
  account* with cross-region replication and an immutable retention
  lock. For a finance agent the lock window matches the regulatory
  retention requirement (commonly 7 years for financial records;
  default the lock to your jurisdiction's requirement).

### How often

- DB: 15-min WAL ship, hourly base backup, daily verified restore.
- Verified-restore drill: every Monday 09:00 UTC, staging restores the
  most recent prod backup and runs the smoke suite (§3). Failure is a
  Sev-2.

### Retention

- DB backups: governed by the financial-records retention requirement
  (default 7 years; never shorter than the regulatory minimum).
- `approvals` + `audit_events` + the operation tables: retained for the
  full regulatory window even after tenant offboarding, subject to
  legal hold.

---

## 8. Logs and traces

### Log streams

- **App log** — structured JSON to stdout. One line per request, one
  per job state transition, one per error.
- **Trace log** — OTLP-equivalent JSONL under
  `target/traces/<date>/<trace_id>.lineage.jsonl`, schema
  `corvid.trace.lineage.v1`. Every span carries `trace_id`, `span_id`,
  `kind`, `status`, `actor_id`, `tenant_id`, `replay_key`,
  `idempotency_key`, `guarantee_id`, `effect_ids`, `approval_id`,
  `data_classes`, `cost_usd`, `confidence`, `model_id`,
  `model_fingerprint`, `input_fingerprint`, `output_fingerprint`,
  `redaction_policy_hash`.
- **Audit log** — written to `audit_events` on every approval
  approve/deny and every executed financial write. Never logged to
  stdout. This is the compliance trail.

### What every trace must include

- `kind` ∈ `{ route, job, agent, prompt, tool, approval, db, retry,
  error, eval, review }`.
- `status` ∈ `{ ok, failed, denied, pending_review, replayed,
  redacted }`.
- `replay_key` — populated for every durable-job and approval span.
- `effect_ids` — the typed effects exercised; empty for read-only
  observation spans.
- `redaction_policy_hash` — must match the runtime's active policy.

### Where to look

| Symptom | First log to read |
|---|---|
| `POST /payments/intents/submit` returns 403 | `audit_events` for the matching `approval_id`, then the `kind=approval` span |
| `nightly_balance_sync` did not run | latest `queue_jobs` row with `task=nightly_balance_sync`, then the `kind=job` span |
| A summary looks like advice | the `kind=agent` span for the readonly snapshot; check `non_advice` on the output and the explanation fingerprint |
| Anomaly false positive | the `weekly_anomaly_scan` job trace; the anomaly's `confidence` and `explanation_fingerprint` |
| Payment executed unexpectedly | `audit_events` for the `approval:submit_payment:*` row and its `approval.approve` event |

### Audit event kinds

The `audit_events.event_kind` column is the compliance vocabulary. The
finance agent emits:

| event_kind | When | Joins to |
|---|---|---|
| `approval.request` | a write intent is created | `approvals.id` |
| `approval.approve` | an approver approves | `approvals.id`, `decided_by_actor_id` |
| `approval.deny` | an approver denies | `approvals.id`, `decision_reason` |
| `payment.execute` | an approved payment is submitted to the provider | `approvals.id` |
| `payment.settled` | a settlement webhook/poll confirms | `approvals.id` |
| `subscription.cancel` | an approved cancellation executes | `finance_subscription_cancellations.id` |
| `transaction.dispute` | an approved dispute is filed | `finance_transaction_disputes.id` |
| `report.export` | an approved export is delivered | `finance_report_exports.id` |
| `recurring.schedule` | a recurring payment is scheduled | `finance_recurring_payments.id` |
| `schedule.disable` / `schedule.enable` | a cron schedule is toggled | job kind |

Every money-moving or data-leaving event_kind must have a preceding
`approval.approve` with a matching `approval_id`. A reconciliation query
that finds an execution event without its approval is incident C.

### Promoting a trace to an eval fixture

```
corvid eval promote target/traces/<date>/<trace_id>.lineage.jsonl \
    --promote-out evals/promoted
```

The Finance agent ships three promoted fixtures under
[`evals/promoted/`](../evals/promoted/) (finance-demo,
finance-balance-sync, finance-payment-intent); every regression replays
them.

---

## 9. Metrics and alerting

### What we export

`/metrics` exposes Prometheus-format counters and histograms:

- `finance_http_requests_total{route,status}` — counter.
- `finance_http_request_duration_seconds{route}` — histogram.
- `finance_job_runs_total{kind,status}` — counter.
- `finance_job_duration_seconds{kind}` — histogram.
- `finance_approval_decisions_total{label,decision}` — counter.
- `finance_approval_pending_age_seconds{label}` — gauge.
- `finance_payments_executed_total{currency}` — counter.
- `finance_payments_executed_cents{currency}` — counter (running sum).
- `finance_non_advice_violations_total` — counter (must stay at 0).
- `finance_autonomous_execution_attempts_total` — counter (must stay at
  0; an execution that reached a write tool without an approved row).
- `finance_redaction_policy_mismatch_total` — counter (must stay at 0).
- `finance_replay_quarantine_violations_total{surface}` — counter (must
  stay at 0 outside intentional fuzz tests).

### Alerts

| Alert | Condition | Severity | First action |
|---|---|---|---|
| `finance_non_advice_violations_total > 0` | any | Sev-1 | Freeze deploys; pull the offending summary's trace; the agent crossed into advice |
| `finance_autonomous_execution_attempts_total > 0` | any | Sev-1 | Page security; a write reached a tool without an approved row |
| `finance_redaction_policy_mismatch_total > 0` | any | Sev-2 | Page on-call; freeze deploys until the policy hash reconciles |
| `finance_replay_quarantine_violations_total > 0` | any non-test | Sev-1 | Page security; pull the trace for the violating surface |
| `finance_approval_pending_age_seconds{label="SubmitPaymentIntent"} > 3600` | pending > 1 h | Sev-3 | Page the admin on-call to review the pending payment |
| `finance_approval_pending_age_seconds{label="ExportFinancialReport"} > 7200` | pending > 2 h | Sev-3 | Page the admin on-call |
| `finance_job_runs_total{kind="nightly_balance_sync",status="failed"} >= 2` | 2 in a row | Sev-3 | Inspect the latest `queue_jobs` row; re-enqueue after fix |
| `finance_payments_executed_cents` spike | > 3σ over 7-day baseline | Sev-2 | Possible runaway approval flow; freeze the payment route, audit recent approvals |

### Where alerts go

- Sev-1 → pager + security channel + incident channel.
- Sev-2 → pager + operations channel.
- Sev-3 → operations channel.

---

## 10. Incident response — diagnose and recover

### Common incidents

#### A. `POST /payments/intents/submit` (or another write) returns 403

**Diagnose:**

1. Pull the trace: `grep -r <request_id> target/traces/`.
2. Find the `kind=approval` span; `status` is `denied` or
   `pending_review`.
3. Check `audit_events` for the matching `approval_id`. The denial
   `reason` names which policy fired (expired, wrong role, missing
   co-sign, cost ceiling).

**Recover:**

- Wrong role: the requester needs `Admin` (payment/export/recurring) or
  `Reviewer` (cancel/dispute); have them request via the tenant admin.
- Expired approval: re-issue; the contract has an `expires_ms`.
- Cost ceiling exceeded: do not raise the ceiling in place — file a
  contract change request. The ceiling is a typed part of the contract.

#### B. A served summary crossed into advice (non-advice violation)

This is the finance agent's signature failure mode and a Sev-1.

**Diagnose:**

1. `finance_non_advice_violations_total` incremented, or a human
   reported advisory content.
2. Pull the `kind=agent` span for the summary. Check the output
   fingerprint and the anomaly `explanation_fingerprint`.
3. Determine whether the advice came from (a) a model explanation that
   drifted prescriptive, or (b) a code change that added an advisory
   field.

**Recover:**

- Freeze deploys immediately.
- If a model explanation drifted: tighten the `insights_connector`
  prompt to descriptive-only, add an eval case that reproduces the
  drift, and do not unfreeze until red→green.
- If a code change added an advisory field: revert it. The non-advice
  posture is a hard constraint, not a tunable.
- Notify compliance per the security model — an advice violation may be
  a regulatory reportable event.

#### C. Suspected autonomous payment execution

A write reached a payment tool without an `approved` row. This should
be impossible (the compiler enforces the `approve` gate), so a
violation means a defect or a bypassed binary.

**Diagnose:**

1. `finance_autonomous_execution_attempts_total` incremented.
2. `SELECT * FROM audit_events WHERE event_kind = 'payment.execute'
   AND approval_id NOT IN (SELECT id FROM approvals WHERE decision =
   'approved');` — any row is a confirmed bypass.
3. Confirm the deployed binary hash matches a release built from
   `corvid check`-clean source (a hand-patched binary could bypass the
   gate the source enforces).

**Recover:**

- Sev-1. Page security. Freeze all payment routes
  (`corvid jobs schedule disable` for any payment-driving job; pull the
  `/payments/*` routes at the edge).
- Reverse the unauthorized payment through the provider if still
  reversible; file a dispute if not.
- Confirm the binary provenance; redeploy from a verified build.
- Notify compliance.

#### D. `nightly_balance_sync` job stuck

**Diagnose:**

1. `SELECT * FROM queue_jobs WHERE task = 'nightly_balance_sync' ORDER
   BY created_ms DESC LIMIT 5;`
2. `running` with an expired `lease_expires_ms` → the replica that took
   the lease crashed. `retryable` → already awaiting the next retry.

**Recover:**

- Expired lease: the scheduler picks it up next tick; force with
  `corvid jobs run --kind=nightly_balance_sync --tenant=<id>
  --provider=<provider>` if needed.
- Persistent failure: read the latest `queue_job_checkpoints` row; the
  `failure_fingerprint` names the cause. Common: the account connector
  token expired (§12) — mint a new token and rerun. The replay key
  makes the rerun idempotent.

#### E. Anomaly scan false positive storm

`weekly_anomaly_scan` flagged a large batch of normal transactions.

**Diagnose:**

1. Inspect the anomalies' `confidence` distribution; a storm usually
   shows a cluster just over the 0.80 floor.
2. Check whether a category's spend baseline shifted (e.g., an annual
   renewal posted, moving the median).

**Recover:**

- Anomalies are descriptive, not actionable — a false-positive storm
  does not move money, so it is Sev-3, not Sev-1.
- Re-run `weekly_anomaly_scan` after the baseline stabilises.
- If the floor is systematically wrong, that is a model-tuning change
  with its own eval; do not hand-edit anomaly rows.

#### F. Replay quarantine violation

Replay quarantine refuses live connector / payment / IO surfaces during
a Substitute-mode replay.

**Diagnose:**

1. `RuntimeError::QuarantineViolation { surface, detail }` is logged
   with the calling span's `replay_key`.
2. Almost always a missing `@replayable` on a downstream call or a
   non-deterministic input not captured in the replay key.

**Recover:**

- Make the agent replayable and re-run.
- Until fixed, do not promote any fixture exercising that path.

#### G. Duplicate payment executed

The same `SubmitPaymentIntent` resulted in two provider transactions.

**Diagnose:**

1. Query the provider ledger for two transactions with the same payee
   and amount in a short window.
2. Match both to `audit_events`. If both carry the same `approval_id`,
   the idempotency key was not honored on retry; if they carry
   different `approval_id`s, two approvals were issued for the same
   intent.

**Recover:**

- The durable-job pool keys executions by the approval's replay key, so
  a same-`approval_id` duplicate indicates a provider-side retry that
  the provider did not dedupe. Reverse the duplicate through the
  provider.
- Two different approvals for the same intent is an approval-queue
  hygiene problem — the first approval should have moved the intent out
  of `pending`. Audit the queue and add a guard that refuses a second
  approval against an already-approved intent.

#### H. Currency mismatch on a payment intent

A payment intent's currency does not match the source account's
currency.

**Diagnose:**

1. The intent's `currency` vs the `finance_accounts.currency` for the
   `source_account_id`.
2. Check whether the amount was interpreted in the wrong currency's
   minor units (cents vs yen have no decimal).

**Recover:**

- The agent does not perform FX — it never converts currencies, because
  an implied conversion rate would be a financial decision the agent is
  not allowed to make. A currency mismatch must be denied at approval,
  not silently converted.
- If a mismatched intent reached approval, deny it and require the
  requester to resubmit in the account's currency, or to use an account
  in the intended currency.

#### I. Budget breach surfaced by balance sync

`nightly_balance_sync` recomputed a budget's `spent_cents` above its
`monthly_limit_cents`.

**Diagnose:**

1. The budget row's `status` moved to `over` (or `watch` near the
   limit).
2. This is informational — a budget breach is a fact the agent
   surfaces, not an action it takes.

**Recover:**

- There is nothing for the agent to "recover" — it does not enforce
  budgets by blocking spend (that would be an autonomous financial
  control the non-advice posture forbids). It surfaces the breach as a
  reminder for a human.
- If an operator wants the breach to trigger an action (e.g., cancel a
  subscription), that action goes through the normal `CancelSubscription`
  approval — the agent never auto-acts on a budget breach.

---

## 11. Rollback procedures

### Rolling back the binary

```
flyctl releases list --app finance-operations-agent
flyctl deploy --image registry.fly.io/finance-operations-agent:<prior-tag>
```

or in Kubernetes:

```
kubectl rollout undo deployment/finance-api -n finance
kubectl rollout status deployment/finance-api -n finance
```

Verify by running the smoke (§3) against production; the `corvid ops
show` snapshot must match the archived snapshot for the prior release.

### Rolling back a migration

Forward-only (§6). Rollback = restore DB from backup, roll back the
binary, fix-forward.

### Rolling back an approval contract

Approval contracts are typed and live in the binary; rolling back the
binary rolls back the contract. The contract's `version` field is
checked at every approval request and refuses mismatches — so an
in-flight approval issued against `v1` will not be honored by a binary
that expects `v2`, and vice versa. Roll the binary, not the contract.

### Rolling back a connector mode switch

If `CORVID_CONNECTOR_MODE` was flipped from `mock` to `real` in error,
flip it back. The runtime re-reads mode on every connector call, so the
change is immediate. For a finance agent, immediately audit
`audit_events` for any `payment.execute` / `report.export` rows in the
window the flag was live — a real-mode flip could have moved money.

---

## 12. Connector mode operations

### Modes

The three connectors honor `CORVID_CONNECTOR_MODE`:

- `mock` (default) — deterministic fixtures, no network, no money
  movement. Used by `corvid eval`, `corvid tour`, smoke suites, and any
  environment where reproducibility matters.
- `real` — real provider calls. Requires valid payment-provider and
  account-connector tokens per tenant (§5).
- `record` — proxies to `real` but writes the raw response to a fixture
  under `target/recordings/`. Used to capture a new replay fixture.
- `replay` — reads from `target/recordings/` instead of the provider.

### Switching modes

```
export CORVID_CONNECTOR_MODE=real
corvid ops show | jq '.connector_mode'   # must print "real"
```

For a finance agent, switching to `real` is a release-checklist event,
not a casual toggle: it means the agent can move money. Confirm
payment-provider credentials are present, the non-advice posture is
re-verified, and the change is logged in `corvid ops show` before
serving traffic.

### Per-tenant connector tokens

```
corvid connectors token put --tenant=<id> --connector=payment_provider \
    --token-file=<path>
corvid connectors token list --tenant=<id>
corvid connectors token revoke --tenant=<id> --connector=payment_provider
```

Tokens are encrypted at rest with the connector-token key (§5).
Revoking is immediate — the next write fails closed.

### When a connector token expires

The connector raises a typed error. The job retry policy
(`exponential_jitter`, 5 attempts) absorbs transient failures; a
refresh failure that survives all retries lands in the dead-letter
queue. Mint a new token and rerun; the replay key makes the rerun
idempotent.

### Payment-provider integration notes

The `payment_provider_connector` executes approved writes in real mode.
Three operational properties matter:

- **Idempotency.** Every execution sends the approval's replay key as
  the provider idempotency key. If the durable-job pool retries an
  execution (lease expiry, transient 5xx), the provider must dedupe on
  that key. Confirm the provider honors idempotency keys before
  enabling real mode — without it, a retry could double-pay (§10
  incident G).
- **Asynchronous settlement.** A provider `accepted` is not a provider
  `settled`. The agent records the intent → approval → execution chain;
  settlement confirmation arrives later via the provider's webhook or a
  reconciliation poll. Do not treat `payment.execute` in `audit_events`
  as proof of settlement — treat it as proof of authorized submission.
- **Webhook reconciliation.** If the provider sends settlement
  webhooks, route them to a reconciliation handler that matches each
  webhook to an `approvals` row by the idempotency key and records a
  `payment.settled` audit event. A settled payment with no matching
  approved row is incident C (unauthorized execution); an approved
  payment with no settlement after the provider's SLA is a failed
  execution to re-run.

### Daily reconciliation cadence

Run a daily reconciliation (operationally, a cron outside the agent or
a future durable job) that:

1. Pulls the provider ledger for the prior day.
2. Matches each provider transaction to an `approvals` row by
   idempotency key.
3. Emits a reconciliation report: matched, provider-only (incident C),
   app-only (failed execution to re-run).

The reconciliation report is itself a compliance artifact; archive it
alongside the `corvid ops show` snapshots.

---

## 13. Approval queue operations

### Inspecting the queue

```
corvid approvals list --pending
corvid approvals show --id=<approval_id>
corvid approvals show --id=<approval_id> --include-trace
```

The queue lives in `approvals`. Every row carries `contract_id`,
`contract_version`, `contract_action`, `target_kind`, `target_id`,
`required_role`, `max_cost_usd`, `data_class`, `irreversible`,
`expires_at_ms`, plus the `approval_id` that joins to `audit_events`
and `trace_lineage`.

### Approving / denying

```
corvid approvals approve --id=<approval_id> --as=<actor_id> --note=<text>
corvid approvals deny --id=<approval_id> --as=<actor_id> --reason=<text>
```

A successful approve checks the approver's role matches `required_role`,
the approval is `pending` and not expired, and the approver is from the
same tenant. It writes `approvals.decision = 'approved'` plus an
`audit_events` row (`event_kind = 'approval.approve'`) and a trace span.

### Per-contract considerations

The flow is the developer's choice; each contract below reflects an
explicit decision about role, ceiling, and reversibility:

- **SubmitPaymentIntent** — Admin, $0.25 ceiling, irreversible (money
  leaves). Verify the payee fingerprint matches an expected payee and
  the amount is within the account's available balance.
- **CancelSubscription** — Reviewer, $0.05, reversible. Verify the
  subscription id belongs to the tenant; cancellation is recoverable
  (re-subscribe), so the bar is lower.
- **DisputeTransaction** — Reviewer, $0.05, reversible. Verify the
  transaction fingerprint and the dispute reason; a dispute can be
  withdrawn.
- **ExportFinancialReport** — Admin, $0.25, irreversible (financial
  data leaves the tenant). Verify the export destination matches the
  tenant's data-handling agreement and the redaction policy hash is
  present.
- **ScheduleRecurringPayment** — Admin, $0.25, irreversible (commits
  future money movement). Verify the cadence and the per-cycle amount;
  a recurring schedule is a standing authorization, so scrutinize it
  more than a one-off payment.

### Decision tree — `SubmitPaymentIntent`

1. Is the requester an `Admin`? No → deny.
2. Does the payee fingerprint match a known/expected payee? No → deny
   and ask for verification.
3. Is the amount within the source account's available balance? No →
   deny.
4. Is the amount within the contract ceiling and any per-tenant payment
   policy? No → deny.
5. All yes → approve. The execution runs through the durable-job pool
   with the approval id attached.

### Decision tree — `ExportFinancialReport`

1. Is the requester an `Admin`? No → deny.
2. Does the export destination match the tenant's data-handling
   agreement (bucket owner, region, encryption, retention)? No → deny.
3. Is a redaction policy hash present and current? No → deny.
4. All yes → approve. Financial data leaving the tenant is irreversible
   once delivered, so any doubt resolves as a deny.

### Decision tree — `ScheduleRecurringPayment`

1. Is the requester an `Admin`? No → deny.
2. Is the cadence and per-cycle amount explicitly stated and bounded?
   No → deny.
3. Is there a stop condition or end date? No → deny and ask for one; a
   standing authorization without an end is a liability.
4. All yes → approve, and set a calendar review of the schedule.

### Decision tree — `CancelSubscription`

1. Is the requester a `Reviewer` (or `Admin`)? No → deny.
2. Does the subscription id belong to the requester's tenant? No →
   deny.
3. Is the subscription currently active (cancelling an already-cancelled
   subscription is a no-op, not an error, but flag duplicate requests)?
4. All yes → approve. Cancellation is reversible (re-subscribe), so the
   bar is intentionally lower than the Admin contracts. Record the
   `reason_fingerprint` for the audit trail.

### Decision tree — `DisputeTransaction`

1. Is the requester a `Reviewer` (or `Admin`)? No → deny.
2. Does the transaction fingerprint resolve to a real transaction on an
   account the tenant owns? No → deny.
3. Is the dispute reason one the payment provider accepts (not a
   free-text fishing expedition)? No → deny and ask for a valid reason
   code.
4. Is the transaction within the provider's dispute window? No → deny;
   an out-of-window dispute will be rejected downstream anyway.
5. All yes → approve. A dispute is withdrawable, so it is a Reviewer
   contract, but a frivolous dispute can damage the provider
   relationship — scrutinize the reason.

### Segregation of duties

For the two Admin contracts that move money or send data
(`SubmitPaymentIntent`, `ExportFinancialReport`,
`ScheduleRecurringPayment`), the requester and the approver should not
be the same actor. The runtime records both `requester_actor_id` and
`decided_by_actor_id` on the `approvals` row; a periodic audit query
flags any row where they match:

```
SELECT id, contract_action FROM approvals
WHERE requester_actor_id = decided_by_actor_id
  AND decision = 'approved'
  AND required_role = 'Admin';
```

Any row returned is a segregation-of-duties violation and a Sev-2
compliance finding.

---

## 14. Tenant lifecycle operations

The Finance agent is multi-tenant: every account, budget, subscription,
anomaly, payment intent, approval, and audit event carries a
`tenant_id`. Onboarding and offboarding touch the most tables at once,
so they get their own playbook. For a finance agent, offboarding also
intersects the long regulatory retention window.

### Onboarding a tenant

1. **Create the tenant row.** `corvid tenants create --id=<id>
   --name=<display>`. Foreign-key anchor for everything else.
2. **Create roles and the first admin.** Every tenant needs at least
   one `Admin` (payment/export/recurring approvals) and one `Reviewer`
   (cancel/dispute). `corvid auth role grant --tenant=<id>
   --actor=<actor> --role=Admin`.
3. **Register account connectors.** `corvid connectors token put
   --tenant=<id> --connector=accounts ...` (real mode) or rely on the
   mock fixtures (default).
4. **Run the first balance sync.** `corvid jobs run
   --kind=nightly_balance_sync --tenant=<id> --provider=<provider>` to
   populate `finance_accounts`.
5. **Confirm the non-advice posture.** `GET /readonly/snapshot` for the
   tenant returns `non_advice = true` before any summary is served.

### Offboarding a tenant

Offboarding is a hard delete gated by a legal-hold check AND the
regulatory retention requirement on the audit trail.

1. **Check for a legal hold.** `corvid tenants hold status
   --tenant=<id>`. Active hold → STOP.
2. **Revoke all sessions and API keys.** `corvid auth revoke-all
   --tenant=<id>`.
3. **Disable the tenant's schedules** so no job re-creates rows
   mid-delete.
4. **Export if contractually required** — a final
   `ExportFinancialReport` (Admin approval), not an ad-hoc dump.
5. **Hard delete the operational data.** `corvid tenants delete
   --tenant=<id> --confirm` cascades through `finance_*`, `sessions`,
   `api_keys`, `user_roles`.
6. **Retain the audit trail.** `approvals` + `audit_events` + the four
   operation tables are NOT deleted — they are retained for the full
   regulatory window (default 7 years), subject to legal hold. The
   delete tombstones the tenant row but preserves the immutable
   financial-operation history.

### Verifying tenant isolation

```
corvid tenants verify-isolation --tenant=<id>
```

Asserts no `finance_*` row for tenant A references a parent row owned
by tenant B, and that no payment, export, or recurring schedule crosses
a tenant boundary. A failure is a Sev-1 cross-tenant financial leak.

---

## 15. Durable jobs and cron operations

### The three jobs

| Kind | Cron | Tenant scope | Effects | Approval | Budget |
|---|---|---|---|---|---|
| `nightly_balance_sync` | `0 2 * * *` America/New_York | per tenant per provider | `finance_read` | none | $0.50 |
| `weekly_anomaly_scan` | `0 6 * * 1` America/New_York | per tenant per window | `finance_read`, `finance_ai` | none | $0.50 |
| `daily_subscription_renewal_check` | `0 7 * * *` America/New_York | per tenant per day | `finance_read` | none | $0.50 |

None of the three carries a financial-write effect — they read,
observe, and remind. Money only moves on the typed `POST` write routes,
which are approval-gated by construction. This separation is the heart
of the non-advice, no-autonomous-execution posture: the scheduled
automation can never move money.

### Job SLOs

- `nightly_balance_sync` p99: 3 min for a tenant with up to ~50
  accounts. Partition by provider if a tenant grows past that.
- `weekly_anomaly_scan` p99: 5 min for a week of transactions per
  tenant.
- `daily_subscription_renewal_check` p99: 30 s per tenant.

### Manual triggers

```
corvid jobs run --kind=nightly_balance_sync --tenant=tenant-1 --provider=mock_bank
corvid jobs run --kind=weekly_anomaly_scan --tenant=tenant-1 --window=business_week
corvid jobs run --kind=daily_subscription_renewal_check --tenant=tenant-1 --day=2026-05-28
```

The `replay_key` is `kind:tenant:scope` and is the durable-job
idempotency key. Two manual triggers with the same arguments coalesce
into one queued job.

### Retry policy

All three jobs use `exponential_jitter`, 5 attempts, base 1 s, cap 10
min, dead-letter `finance_operations_agent.dead_letter`. Dead-lettered
jobs:

```
corvid jobs list --dead-letter
corvid jobs replay --id=<job_id>   # replays from the last checkpoint
```

### Checkpoints

Long-running jobs write checkpoints to `queue_job_checkpoints` after
every batch. If a replica crashes mid-job, the lease expires and another
replica resumes from the last checkpoint. Each batch is idempotent.

### Scheduler ownership

The scheduler uses a per-job-kind advisory lock in the DB so only one
replica fires each cron tick. If the locked replica disappears, the DB
releases the lock and the next tick is fired by whichever replica wins
the next acquisition.

### Disabling a schedule

```
corvid jobs schedule disable --kind=<kind>
corvid jobs schedule enable --kind=<kind>
```

Disabling pauses the schedule but does not affect in-flight jobs. The
`audit_events` row for the disable is required for compliance; the CLI
writes one automatically.

### Why no job moves money

It is worth stating plainly as an operational invariant: the three
durable jobs are read/observe/remind only. There is deliberately no
"auto-pay" or "auto-cancel" job. Any automation that moved money would
have to call a `dangerous` write tool, which the compiler refuses
outside an `approve` boundary — and an `approve` boundary requires a
human decision. So the scheduler can wake the agent up, but it can
never authorize a payment.

---

## 16. Disaster recovery

### Catastrophic DB loss

1. **Stop traffic.** Take the deploy out of the load balancer.
2. **Restore the latest backup** to a new DB instance (§7). Verify with
   `corvid migrate --check`.
3. **Reconcile executed payments.** Read `audit_events` from the backup
   for `payment.execute` / `report.export` rows; cross-check against the
   payment provider's own ledger for the data-loss window (the WAL ship
   interval, 15 min). Any payment the provider executed that the
   restored DB does not record must be re-recorded by hand with a clear
   note — the provider's ledger is the source of truth for money that
   actually moved.
4. **Point the deploy at the restored DB** and roll.
5. **Run the smoke suite (§3) + the eval suite.**
6. **Write the post-incident report**, including the exact reconciliation
   window, for the compliance trail.

### Audit redundancy

For regulated tenants, enable the audit-event forwarder which writes a
copy of every `audit_events` row to an append-only object-storage log
under `s3://<bucket>/audit/<tenant>/<date>.jsonl`. For a finance agent
this is strongly recommended, not optional — the audit trail is a
regulatory artifact and should survive a primary-DB loss independently.

### Payment-provider divergence

If the app DB and the payment provider disagree about what was paid,
**the provider's ledger wins for money that moved**; the app DB wins
for intent and approval. Reconcile by:

1. Pulling the provider's transaction ledger for the window.
2. Matching each provider transaction to an `approvals` row by
   `approval_id` (carried in the payment metadata).
3. Any provider transaction without a matching approved row is incident
   C (autonomous/unauthorized execution).
4. Any approved row without a provider transaction is a failed
   execution — re-run through the durable-job pool (idempotent on the
   replay key).

### Loss of the connector-token key

The connector-token key encrypts payment-provider and account tokens.
If lost, those tokens are unrecoverable but the DB and audit trail are
intact. Recovery: rotate the key (§5), force every tenant to re-mint
their connector tokens. Real-mode writes are paused until done. Test
the recovery quarterly in staging.

### RPO / RTO targets

- **RPO**: 15 min (WAL ship interval).
- **RTO**: 1 h for a regional DB failure (cross-region replica
  promote); 4 h for catastrophic DB loss (restore + payment
  reconciliation).

---

## 17. Appendix — reference data

### Schema manifest

`FinanceSchemaManifest("finance_operations_agent", 5, 23, 3, 3, 5,
"mock", true)`:

- 5 migrations: `0001_readonly_finance`, `0002_payment_intents`,
  `0003_auth`, `0004_approvals_and_durable_jobs`,
  `0005_finance_operations`.
- 23 tables: see §2.
- 3 connectors: `accounts_connector`, `insights_connector`,
  `payment_provider_connector`.
- 3 durable jobs: `nightly_balance_sync`, `weekly_anomaly_scan`,
  `daily_subscription_renewal_check`.
- 5 approval contracts: `SubmitPaymentIntent`, `CancelSubscription`,
  `DisputeTransaction`, `ExportFinancialReport`,
  `ScheduleRecurringPayment`.
- Default mode: `mock`. Non-advice: `true`.

### Capacity planning

Per tenant unless noted. Finance is light on compute (no embedding
model) but sensitive to transaction volume and approval throughput.

| Tenant size | Accounts | Monthly transactions | `nightly_balance_sync` | `weekly_anomaly_scan` | Action |
|---|---|---|---|---|---|
| Small | < 10 | < 1k | < 30 s | < 1 min | Single replica, `shared-cpu-1x` |
| Medium | 10 – 50 | 1k – 20k | < 3 min | < 5 min | Default; 1-3 replicas |
| Large | 50 – 200 | 20k – 200k | 3 – 10 min | 5 – 20 min | Partition balance sync by provider; Postgres read replica for the snapshot route |
| XL | > 200 | > 200k | multi-step | > 20 min | Shard providers across worker replicas; move anomaly scan to its own worker pool |

Other limits:

- **Approval throughput** — the queue is DB-backed; a single Postgres
  primary handles thousands of pending approvals comfortably. The
  bottleneck is human review latency, not the queue.
- **DB sizing** — `finance_audit_records` + `audit_events` + the four
  operation tables grow append-only and are retained for the
  regulatory window (7 years). Budget for this: at 200k
  transactions/month with ~1 % triggering an approval, that is ~24k
  approval rows/year/tenant. Plan storage for the full retention window
  up front; never prune within it.
- **Anomaly scan** — CPU-bound on the `insights_connector` model.
  Stays under SLO up to ~200k transactions/week/tenant; past that,
  partition the scan by account.

### Compliance and regulatory posture

A finance agent has compliance obligations a general app does not. The
operationally relevant ones:

- **Non-advice line.** The agent describes, it does not advise. A
  violation (§10 incident B) may be a regulatory reportable event;
  route it to compliance, not just engineering.
- **Audit immutability.** `approvals`, `audit_events`, and the four
  operation tables are append-only and retained for the regulatory
  window. Offboarding tombstones the tenant but never deletes the
  financial-operation history (§14).
- **Segregation of duties.** Requester ≠ approver for the Admin
  money/data contracts (§13). The periodic audit query flags
  violations.
- **Reconciliation source of truth.** For money that actually moved,
  the payment provider's ledger is authoritative; the app DB is
  authoritative for intent and approval (§16).
- **No autonomous execution.** The three scheduled jobs cannot move
  money — they read, observe, and remind. Every money movement requires
  a human approval the compiler enforces (§15). This is the structural
  guarantee behind the no-autonomous-execution claim, not a policy that
  could be toggled off.

These are operational facts an auditor can verify from the trace log,
the `approvals` table, and the source — not promises in a doc.

### Effect catalog

| Effect | Cost | Trust | Data class | Used by |
|---|---|---|---|---|
| `finance_read` | $0.01 | readonly | financial | snapshot, all 3 jobs |
| `finance_ai` | $0.05 | bounded | financial | snapshot, anomaly scan |
| `payment_write` | $0.02 | human_required | financial | `submit_payment_intent` |
| `subscription_write` | $0.01 | human_required | financial | `cancel_subscription` |
| `dispute_write` | $0.01 | human_required | financial | `dispute_transaction` |
| `report_export` | $0.05 | human_required | financial | `export_financial_report` |
| `recurring_payment_write` | $0.03 | human_required | financial | `schedule_recurring_payment` |

### Route catalog

| Method | Route | Returns | Effects | Approval |
|---|---|---|---|---|
| GET | `/config` | `FinanceConfig` | none | none |
| GET | `/schema` | `FinanceSchemaManifest` | none | none |
| GET | `/readonly/snapshot/mock` | `FinanceReadonlySnapshot` | `finance_read`, `finance_ai` | none |
| GET | `/payments/intents/mock` | `FinanceApprovalAuditSurface` | none | none |
| POST | `/payments/intents/submit` | `PaymentIntentReceipt` | `payment_write` | `SubmitPaymentIntent` |
| POST | `/subscriptions/cancel` | `CancelSubscriptionReceipt` | `subscription_write` | `CancelSubscription` |
| POST | `/transactions/dispute` | `DisputeTransactionReceipt` | `dispute_write` | `DisputeTransaction` |
| POST | `/reports/export` | `ExportFinancialReportReceipt` | `report_export` | `ExportFinancialReport` |
| POST | `/payments/recurring/schedule` | `ScheduleRecurringPaymentReceipt` | `recurring_payment_write` | `ScheduleRecurringPayment` |
| POST | `/auth/session/login` | `LoginResponse` | none | none |
| POST | `/auth/api-key/login` | `ApiKeyLoginResponse` | none | none |
| GET | `/auth/status` | `AuthStatusResponse` | none | none |
| GET | `/auth/api-key/status` | `AuthStatusResponse` | none | none |
| GET | `/jobs/nightly-balance-sync/mock` | `FinanceJobRun` | `finance_read` | none |
| GET | `/jobs/weekly-anomaly-scan/mock` | `FinanceJobRun` | `finance_read`, `finance_ai` | none |
| GET | `/jobs/daily-subscription-renewal-check/mock` | `FinanceJobRun` | `finance_read` | none |

### Adversarial corpus

Five named threats under [`adversarial/`](../adversarial/):

- `ungated_cancel_subscription.cor` — calls `cancel_subscription`
  without `approve CancelSubscription(...)`.
- `ungated_dispute_transaction.cor` — calls `dispute_transaction`
  without `approve DisputeTransaction(...)`.
- `ungated_export_financial_report.cor` — calls
  `export_financial_report` without `approve ExportFinancialReport(...)`.
- `ungated_schedule_recurring_payment.cor` — calls
  `schedule_recurring_payment` without
  `approve ScheduleRecurringPayment(...)`.
- `autonomous_payment.json` — the declarative no-autonomous-execution
  threat: the app may create payment intents but may not execute
  payments without approval.

The four `.cor` fixtures are refused by `corvid check` with `E0101`.
Any green build on these fixtures is a Sev-1 — the compiler-enforced
approval gate is the foundation of the agent's no-autonomous-execution
claim.

### Approval contract reference

| Label | Role | Ceiling | Irreversible | Reason |
|---|---|---|---|---|
| `SubmitPaymentIntent` | Admin | $0.25 | yes | money leaves |
| `CancelSubscription` | Reviewer | $0.05 | no | re-subscribable |
| `DisputeTransaction` | Reviewer | $0.05 | no | withdrawable |
| `ExportFinancialReport` | Admin | $0.25 | yes | financial data leaves tenant |
| `ScheduleRecurringPayment` | Admin | $0.25 | yes | standing money authorization |

### Promoted eval fixtures

Three promoted fixtures under [`evals/promoted/`](../evals/promoted/):

- `finance-demo.lineage-eval.json` — read-only non-advice snapshot.
- `finance-balance-sync.lineage-eval.json` — `nightly_balance_sync`
  durable job + connector read.
- `finance-payment-intent.lineage-eval.json` — payment route +
  `SubmitPaymentIntent` approval (pending_review) + audit.

### Environment variable reference

| Variable | Default | Purpose |
|---|---|---|
| `CORVID_APP_ENV` | `local` | Environment (local / staging / production) |
| `CORVID_CONNECTOR_MODE` | `mock` | Connector mode (mock / replay / real / record) |
| `CORVID_REQUIRE_APPROVALS` | `true` | If true, every dangerous tool fails closed without approval |
| `CORVID_DATABASE_URL` | `sqlite:target/finance.db` | DB connection string |
| `CORVID_CONNECTOR_TOKEN_KEY` | — | AES-256 key for connector-token encryption |
| `CORVID_API_KEY_PEPPER` | — | Argon2id pepper for API-key hashing |
| `CORVID_SESSION_SIGNING_KEY` | — | Session signing key (30-day rotation) |
| `CORVID_CSRF_SECRET` | — | CSRF double-submit secret |
| `CORVID_OTLP_ENDPOINT` | — | OTLP exporter target |
| `CORVID_METRICS_LISTEN` | `0.0.0.0:9090` | Prometheus `/metrics` bind |
| `CORVID_TRACE_DIR` | `target/traces` | Trace JSONL output directory |
| `RUST_LOG` | `info` | Log filter |

### Source map

- App source: [`src/main.cor`](../src/main.cor)
- Migrations: [`migrations/`](../migrations/)
- Seeds: [`seeds/`](../seeds/)
- Evals: [`evals/`](../evals/)
- Promoted fixtures: [`evals/promoted/`](../evals/promoted/)
- Traces: [`traces/`](../traces/)
- Adversarial: [`adversarial/`](../adversarial/)
- Deploy: [`deploy/`](../deploy/)
- Security model: [`../security-model.md`](../security-model.md)
