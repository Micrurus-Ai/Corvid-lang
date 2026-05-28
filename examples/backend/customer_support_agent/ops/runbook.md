# Customer Support Agent — Operator Runbook

This runbook is the operational source of truth for running the Customer
Support Agent backend in development, staging, and production. The
Support agent is a reference Corvid application: it triages support
tickets, drafts policy-grounded replies, tracks SLA deadlines, and
executes support operations a human has approved (reply send, refund,
escalation, ticket close, account credit) — each behind a typed,
developer-authored approval contract.

**The defining constraint: every customer-facing reply is
policy-grounded.** A draft reply is not produced from the model's
imagination — it carries a `PolicyCitation` with a `provenance_id`, and
the triage/draft contract fails if the citation is missing. The agent
answers from policy, and a human approves the send.

Every procedure below is grounded in surfaces the app actually ships.
The schema manifest at [`src/main.cor`](../src/main.cor) declares the
canonical counts (5 migrations / 20 tables / 3 connectors / 3 durable
jobs / 5 approval contracts / policy-grounded) and `corvid run
--target=server` exposes the routes that drive each procedure.

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

### What the Customer Support Agent does

The Customer Support Agent runs three durable jobs on a cron and exposes
a typed HTTP surface for the work a support team's assistant performs:

- `sla_breach_scan` (cron `0 * * * *` America/New_York, hourly) — scans
  open tickets for ones approaching their SLA deadline and flags them.
  SLA work is time-sensitive, so this runs hourly rather than nightly.
  It flags; it does not reply, escalate, or close.
- `nightly_csat_rollup` (cron `0 2 * * *` America/New_York) — aggregates
  customer-satisfaction signals and eval-dashboard counts for the day.
  Observational; it produces a rollup, not an action.
- `policy_reindex` (cron `0 3 * * *` America/New_York) — refreshes the
  policy corpus that grounds every reply. Keeps `PolicyCitation`
  provenance current so a drafted reply cites live policy, not a stale
  snapshot.

Every support write enters one of five approval contracts. The flow is
the developer's choice — role and reversibility differ per surface on
purpose:

| Approval label | Tool | Effect | Target type | Required role | Cost ceiling | Reversible |
|---|---|---|---|---|---|---|
| `SendSupportReply` | `send_support_reply` | `support_write` | `ticket` | Reviewer | $0.05 | no (can't unsend) |
| `IssueSupportRefund` | `issue_support_refund` | `refund_write` | `ticket` | Admin | $0.25 | no (money leaves) |
| `EscalateTicket` | `escalate_ticket` | `escalate_write` | `ticket` | Reviewer | $0.05 | yes (de-escalate) |
| `CloseTicket` | `close_ticket` | `ticket_write` | `ticket` | Reviewer | $0.05 | yes (reopen) |
| `ApplyAccountCredit` | `apply_account_credit` | `credit_write` | `customer` | Admin | $0.25 | no (money-equivalent) |

The role gradient encodes blast radius: the two Admin contracts move
money (refund, goodwill credit); the three Reviewer contracts are
customer-facing or state changes, two of which are reversible.

### What the Customer Support Agent does NOT do

- **It does not reply without policy grounding.** Every `SupportDraftReply`
  carries a `PolicyCitation`; the draft contract fails without it. The
  agent does not improvise customer-facing answers.
- **It does not send a reply, issue a refund, escalate, close, or credit
  without human approval.** Every one of those tools is `dangerous` and
  the compiler rejects callers that lack a matching `approve <Label>(...)`
  boundary. Drafting a reply is allowed; *sending* it without approval
  does not compile.
- **It does not talk to real provider APIs in the default mode.** The
  default `CORVID_CONNECTOR_MODE=mock` keeps every connector offline with
  deterministic fixtures so `corvid eval` and `corvid replay` stay
  reproducible.
- **It does not store raw customer PII in the database.** The DB holds
  fingerprints (`customer_fingerprint`, `subject_fingerprint`,
  `body_fingerprint`) and provenance ids. Raw ticket bodies live in the
  ticketing provider, not at rest in the app DB.
- **It does not auto-resolve or auto-refund.** The three cron jobs scan,
  aggregate, and reindex — none of them sends, refunds, escalates, or
  closes. Those are human-approved actions only.

### Service-level objectives

- Availability: 99.9 % monthly for `GET /tickets/triage` and
  `/replies/draft`; 99.5 % for `POST /replies/send`, `/refunds/issue`,
  `/tickets/*`, `/credits/apply` (lower because approval-gated).
- Latency (p99): `/tickets/triage` < 1200 ms, `/replies/draft` < 1500 ms
  (both involve the policy search + model). Approved sends/refunds are
  async through the durable-job pool — see §15.
- Grounding integrity: 100 % of drafted replies carry a policy citation.
  Any ungrounded reply is a Sev-2.
- SLA integrity: `sla_breach_scan` runs every hour without a miss; a
  missed scan that lets a ticket breach undetected is a Sev-3.

---

## 2. Architecture map

### Process layout

The Support agent runs as a single Corvid server binary plus a SQLite or
Postgres backing store plus the policy corpus index. The binary is built
from `src/main.cor` via `corvid build --target=server`. In production it
is wrapped in a distroless OCI image; the same binary serves all HTTP
routes, runs the durable-job pool, the scheduler, the OTLP exporter, and
the metrics endpoint.

```
+---------------------------+
|   corvid jobs run         |
|   (durable job pool,      |  <-- sla_breach_scan (hourly),
|    in-process)            |       nightly_csat_rollup,
+---------------------------+       policy_reindex
            |
            v
+---------------------------+
|   corvid runtime          |  <-- typed effects, approvals,
|   (HTTP routes,           |       policy-grounded posture, replay
|    scheduler,             |       quarantine
|    OTLP exporter,         |
|    /metrics)              |
+---------------------------+
            |
   +--------+--------+
   |        |        |
   v        v        v
+------+ +--------+ +-----------+
| DB   | | Policy | | Ticketing |
| (5   | | corpus | | provider  |
|  migs| | index  | | (real     |
| / 20 | | (cite  | |  mode)    |
| tbls)| |  source| |           |
+------+ +--------+ +-----------+
```

### Data classes

The Support agent processes three data classes; the compiler enforces
that each effect carries one and refuses to cross boundaries without an
approval:

- `customer` — ticket content fingerprints, draft reply bodies, escalation
  and close state. Effects: `ticket_read`, `support_ai`, `support_write`,
  `escalate_write`, `ticket_write`.
- `internal` — the policy corpus the replies cite. Effect: `policy_search`.
- `financial` — refunds and account credits (money or money-equivalent).
  Effects: `refund_write`, `credit_write`.

### Storage surfaces

- **Database** (5 migrations, 20 tables):
  - Support domain (`0001`/`0002`/`0005`, 8 tables): `support_tickets`,
    `support_policy_citations`, `support_draft_replies`,
    `support_sla_jobs`, `support_approval_audits`, `support_escalations`,
    `support_ticket_closures`, `support_account_credits`.
  - Auth (`0003`, 7 tables): `tenants`, `users`, `roles`, `user_roles`,
    `sessions`, `api_keys`, `permissions`.
  - Approvals + jobs + lineage (`0004`, 5 tables): `approvals`,
    `audit_events`, `queue_jobs`, `queue_job_checkpoints`,
    `trace_lineage`.
- **Policy corpus index**: the searchable index of support policies that
  every reply cites. Rebuildable via `policy_reindex`. It is the
  grounding source — a stale or empty index is a grounding incident.
- **Ticketing provider** (real mode only): the external system holding
  raw ticket content. Reached read-only via the tickets connector;
  writes (reply send, status change) go through the approval-gated tools.

### Connector layout

The Support agent's three connectors are:

- `tickets_connector` (effect: `ticket_read`) — reads tickets from the
  ticketing provider (Zendesk, Intercom, or a CSV import).
- `policy_connector` (effect: `policy_search`) — searches the policy
  corpus and returns `PolicyCitation`s with provenance.
- `support_ai_connector` (effect: `support_ai`) — runs the bounded
  triage/draft model. Bounded trust: it drafts grounded replies, it does
  not invent policy.

The write effects (`support_write`, `refund_write`, `escalate_write`,
`ticket_write`, `credit_write`) route through the runtime's `HttpRuntime`
to the ticketing / billing providers; they are approval-gated and fail
closed without approval.

### The triage → draft → approve → send pipeline

The core customer-facing flow is four stages, and the grounding +
approval guarantees live at specific points in it:

1. **Triage** (`GET /tickets/triage`, effects `ticket_read`,
   `policy_search`, `support_ai`). Reads the ticket, searches the policy
   corpus, and classifies the ticket with a confidence. The triage
   already carries a `PolicyCitation` — grounding starts here, not at
   draft time.
2. **Draft** (`GET /replies/draft`, same effects). Produces a
   `SupportDraftReply` that inherits the triage's citation. The draft
   contract (`support_triage_contract_valid`) fails if the reply is not
   grounded, its citation has no provenance, or its approval label is
   wrong. An ungrounded draft never leaves this stage.
3. **Approve** (`corvid approvals approve`, or the queue UI). A human
   `Reviewer` checks the draft and its citation and approves the
   `SendSupportReply` contract. This is the point where a human takes
   responsibility for the customer-facing message.
4. **Send** (`POST /replies/send`, effect `support_write`, gated by
   `approve SendSupportReply`). Only an approved draft reaches the
   provider. The compiler guarantees `send_support_reply` cannot be
   called without the `approve` boundary.

The invariant that matters operationally: **grounding is enforced at
stage 2 (draft), human responsibility at stage 3 (approve), and the
compiler gate at stage 4 (send).** A failure at any stage stops the
pipeline rather than shipping an ungrounded or unapproved message. The
refund, escalate, close, and credit flows skip stages 1–2 (no draft) but
share stages 3–4 (approve + compiler-gated execute).

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
cd examples/backend/customer_support_agent
export CORVID_APP_ENV=local              # local | staging | production
export CORVID_CONNECTOR_MODE=mock        # default; keeps everything offline
export CORVID_DATABASE_URL=sqlite:target/support.db
export CORVID_REQUIRE_APPROVALS=true     # default; fail closed on every dangerous tool
corvid check src/main.cor
corvid migrate --database-url=$CORVID_DATABASE_URL --dir=migrations
corvid seeds load seeds/demo.sql
corvid run --target=server --bind=127.0.0.1:8088
```

If everything is wired correctly, `corvid run` exposes the routes on
port 8088. `GET /config` returns the
`SupportConfig("customer_support_agent", "mock", true)` envelope — note
`policy_required = true`.

### Smoke test the local boot

In a second shell:

```
curl -s http://127.0.0.1:8088/schema | jq
curl -s http://127.0.0.1:8088/tickets/triage/mock | jq '.grounded, .confidence'
curl -s http://127.0.0.1:8088/replies/draft/mock | jq '.grounded, .approval_label'
curl -s http://127.0.0.1:8088/sla/jobs/mock | jq '.replay_key'
curl -s http://127.0.0.1:8088/jobs/sla-breach-scan/mock | jq '.contract.job_kind'
curl -s http://127.0.0.1:8088/eval/dashboard/mock | jq '.approval_gated_writes'
```

Expected: `grounded` is `true`, `approval_label` is `SendSupportReply`,
`approval_gated_writes` is `5`. If `grounded` is ever `false` or a draft
lacks a citation, *do not deploy* — the policy-grounding invariant has
regressed.

### Run the typed eval suite

```
corvid eval evals/support_ops_eval.cor
```

Must exit 0 with `values: 11/11 passed`. The suite covers the maturity
bar minima, the five approval contracts, the role/reversibility
gradient, the three cron schedules, job bounding, and the policy-grounded
posture (case 11).

### Confirm the adversarial gates

```
corvid check adversarial/ungated_send_support_reply.cor      # → E0101
corvid check adversarial/ungated_issue_support_refund.cor    # → E0101
corvid check adversarial/ungated_escalate_ticket.cor         # → E0101
corvid check adversarial/ungated_close_ticket.cor            # → E0101
corvid check adversarial/ungated_apply_account_credit.cor    # → E0101
```

Each must exit `1` with `E0101 — dangerous tool called without a prior
approve`. The declarative `adversarial/ungrounded_reply.json` is the
sixth named threat (no ungrounded reply).

### Promote a new fixture

```
corvid eval promote traces/<trace>.lineage.jsonl --promote-out evals/promoted
```

The promotion writes a `corvid.eval.lineage_fixture.v1` record, checked
into git so the next release replays it deterministically.

---

## 4. Setup — staging and production deployment

### Topology

The reference deployment is three tiers:

- **Edge** — TLS termination, request authentication, rate limit, request
  id injection. The agent does not bundle an edge proxy; use `nginx`,
  `envoy`, or the cloud provider's L7 LB.
- **App** — N replicas behind the edge. Each runs the HTTP server,
  durable-job pool, and scheduler. The scheduler uses a per-job advisory
  lock so only one replica runs each cron tick.
- **Data** — Postgres primary + read replica + the policy corpus index.

### Fly.io reference

`deploy/fly.toml` defines the canonical Fly.io deployment. Key
parameters:

- `app = "customer-support-agent"`
- Primary region: `iad`.
- VM size: `shared-cpu-1x`, 512 MB.
- Auto-scaling: 1 to 3 replicas; HTTP service on internal port `8080`.
- `[env]`: `CORVID_CONNECTOR_MODE = "mock"` by default. Production unsets
  this to `real` only after the operator has verified the ticketing +
  billing provider credentials are configured (§5) AND the
  policy-grounding posture has been re-confirmed in the release
  checklist.

Deploy:

```
flyctl deploy --config deploy/fly.toml
flyctl logs --app customer-support-agent
flyctl ssh console --app customer-support-agent --command "corvid ops show"
```

Archive each production `corvid ops show` snapshot under
`ops/snapshots/<YYYY-MM-DD>.json` so audits can diff a release against
the deployed surface.

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
kubectl -n support rollout status deploy/support-api
```

### Docker Compose (single-host)

```
cd examples/backend/customer_support_agent
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml exec support corvid ops show
```

### Boot sequence

1. Read env. If any `CORVID_*` variable is malformed, exit non-zero.
2. Connect to the DB. Five-attempt exponential backoff, then exit
   non-zero so the orchestrator restarts the pod.
3. Run migrations 0001 → 0005 (idempotent — see §6).
4. If `CORVID_CONNECTOR_MODE=real`, verify ticketing + billing provider
   credentials are present; fail closed if not.
5. Confirm the policy corpus index is non-empty (a support agent that
   can't ground replies should not serve drafts); if empty, run
   `policy_reindex` before binding the listener.
6. Start the OTLP exporter and `/metrics` endpoint.
7. Start the durable-job pool; resume expired-lease jobs as `retryable`.
8. Start the scheduler; compute next-fire timestamps for the 3 crons.
9. Bind the HTTP listener.

If steps 1–5 fail, the binary exits and the orchestrator restarts it.

---

## 5. Secrets management

### Inventory

The Support agent stores four classes of secrets:

- **Database credentials** — Postgres URL or SQLite path. Read at boot;
  never logged.
- **Ticketing-provider credentials** — API token/OAuth for the ticketing
  system. Stored only in `connector_tokens` (encrypted with the
  connector-token key), never in env.
- **Billing-provider credentials** — for refunds and account credits.
  Same storage rules; this is the highest-value secret (it moves money).
- **Connector-token key + auth secrets** — `CORVID_CONNECTOR_TOKEN_KEY`
  (AES-256), `CORVID_API_KEY_PEPPER` (Argon2id pepper),
  `CORVID_SESSION_SIGNING_KEY`, `CORVID_CSRF_SECRET`. Injected from the
  secret manager at boot.

### Where to store them

- **Production:** secret manager (Vault, AWS SM, GCP SM, K8s Secret).
  Never in env files committed to git, never in `configmap.yaml`.
- **Staging:** same, in a separate secret namespace.
- **Local:** a developer-private `.env.local` (gitignored). Never check
  in real provider credentials; use mock connectors.

### Rotation

- **DB credentials** — rotate quarterly via mint-new-user → update secret
  → roll deploy → drop old user.
- **Ticketing-provider credentials** — rotate per provider policy.
- **Billing-provider credentials** — rotate quarterly at minimum; treat a
  suspected leak as a Sev-1 (it can move money) and rotate immediately,
  which pauses real-mode refunds/credits until the new credential is
  verified.
- **Connector-token key** — rotate annually or on suspected compromise
  via `corvid auth keys rotate --kind connector-token` under a
  maintenance window.
- **API-key pepper / session signing / CSRF secret** — pepper rotation
  invalidates API keys, session signing rotates every 30 days with a
  15-minute grace, CSRF secret rotates immediately.

### What never gets logged

- Plaintext provider credentials or customer PII.
- Raw ticket bodies or reply bodies (only fingerprints).
- Policy citation text (only the citation id + provenance id).
- Approval payloads beyond the `approval_id` and target type.

The redaction policy is enforced by the runtime's `redaction_policy_hash`
on every trace span; a span missing it or not matching the active policy
is dropped and an alert fires (§9).

---

## 6. Migrations — apply, drift, rollback

### Applying migrations

```
corvid migrate --database-url=$CORVID_DATABASE_URL --dir=migrations
```

Migrations are idempotent: every migration begins with the schema-version
check. Re-running an applied migration is a no-op.

### The five migrations

- `0001_support_triage.sql` — `support_tickets`, `support_policy_citations`,
  `support_draft_replies`.
- `0002_approvals_sla.sql` — `support_sla_jobs`, `support_approval_audits`.
- `0003_auth.sql` — `tenants`, `users`, `roles`, `user_roles`,
  `sessions`, `api_keys`, `permissions`.
- `0004_approvals_and_durable_jobs.sql` — `approvals`, `audit_events`,
  `queue_jobs`, `queue_job_checkpoints`, `trace_lineage`.
- `0005_support_operations.sql` — backing tables for the three new write
  surfaces: `support_escalations`, `support_ticket_closures`,
  `support_account_credits`.

### Detecting drift

```
corvid migrate --check --database-url=$CORVID_DATABASE_URL --dir=migrations
```

Compares the DB's `schema_version` to the migration directory. Non-zero
exit on any mismatch. `corvid ops show` includes the schema version and
each migration's file hash; archive it per release.

### Rollback

Support migrations are **forward-only**. There is no `migrate down`. To
revert: restore the DB from the latest backup before the bad migration
(§7), roll back the binary, write a fix-forward migration. Forward-only
protects the `approvals` + `audit_events` + operation tables that are the
support compliance trail.

---

## 7. Backups — what, where, how often

### What gets backed up

- **DB**: full snapshot + WAL ship every 15 min (Postgres) or hot-copy
  every hour (SQLite).
- **Approvals + audit_events + the operation tables**: included in the DB
  backup. These are the record of every executed support operation
  (reply, refund, escalation, close, credit).
- **Policy corpus index**: not backed up — rebuildable from the policy
  source via `policy_reindex`. The source policies themselves live in
  their system of record (a docs repo or CMS) and are backed up there.

### Where

- DB backups: cloud-provider managed object storage in a separate account
  with cross-region replication and an immutable retention lock. The lock
  window matches the customer-data retention policy.

### How often

- DB: 15-min WAL ship, hourly base backup, daily verified restore.
- Verified-restore drill: every Monday 09:00 UTC, staging restores the
  most recent prod backup and runs the smoke suite (§3). Failure is a
  Sev-2.

### Retention

- DB backups: governed by the customer-data retention policy; refunds and
  credits (financial records) follow the financial-records retention
  window, commonly longer.
- `approvals` + `audit_events` + operation tables: retained for the full
  retention window even after tenant offboarding, subject to legal hold.

---

## 8. Logs and traces

### Log streams

- **App log** — structured JSON to stdout. One line per request, one per
  job state transition, one per error.
- **Trace log** — OTLP-equivalent JSONL under
  `target/traces/<date>/<trace_id>.lineage.jsonl`, schema
  `corvid.trace.lineage.v1`.
- **Audit log** — written to `audit_events` on every approval
  approve/deny and every executed support write. Never logged to stdout.

### What every trace must include

- `kind` ∈ `{ route, job, agent, prompt, tool, approval, db, retry,
  error, eval, review }`.
- `status` ∈ `{ ok, failed, denied, pending_review, replayed, redacted }`.
- `replay_key` — populated for every durable-job and approval span.
- `effect_ids` — the typed effects exercised; empty for read-only spans.
- `redaction_policy_hash` — must match the runtime's active policy.

### Audit event kinds

The `audit_events.event_kind` column is the support compliance
vocabulary:

| event_kind | When | Joins to |
|---|---|---|
| `approval.request` | a write intent is created | `approvals.id` |
| `approval.approve` | an approver approves | `approvals.id`, `decided_by_actor_id` |
| `approval.deny` | an approver denies | `approvals.id`, `decision_reason` |
| `reply.send` | an approved reply is sent | `approvals.id` |
| `refund.issue` | an approved refund executes | `approvals.id` |
| `ticket.escalate` | an approved escalation executes | `support_escalations.id` |
| `ticket.close` | an approved closure executes | `support_ticket_closures.id` |
| `credit.apply` | an approved account credit executes | `support_account_credits.id` |
| `schedule.disable` / `schedule.enable` | a cron schedule is toggled | job kind |

Every customer-facing or money-moving event_kind must have a preceding
`approval.approve` with a matching `approval_id`.

### Where to look

| Symptom | First log to read |
|---|---|
| `POST /replies/send` returns 403 | `audit_events` for the matching `approval_id`, then the `kind=approval` span |
| A drafted reply has no citation | the `kind=agent` span for the draft; check the `PolicyCitation` provenance |
| `sla_breach_scan` did not run | latest `queue_jobs` row with `task=sla_breach_scan`, then the `kind=job` span |
| Refund executed unexpectedly | `audit_events` for the `approval:issue_refund:*` row and its `approval.approve` event |
| Policy citations look stale | the `policy_reindex` job trace; the policy corpus index hash |

### Promoting a trace to an eval fixture

```
corvid eval promote target/traces/<date>/<trace_id>.lineage.jsonl \
    --promote-out evals/promoted
```

The agent ships three promoted fixtures under
[`evals/promoted/`](../evals/promoted/) (support-demo, support-sla-scan,
support-reply-send); every regression replays them.

---

## 9. Metrics and alerting

### What we export

`/metrics` exposes Prometheus-format counters and histograms:

- `support_http_requests_total{route,status}` — counter.
- `support_http_request_duration_seconds{route}` — histogram.
- `support_job_runs_total{kind,status}` — counter.
- `support_job_duration_seconds{kind}` — histogram.
- `support_approval_decisions_total{label,decision}` — counter.
- `support_approval_pending_age_seconds{label}` — gauge.
- `support_replies_sent_total` — counter.
- `support_refunds_issued_cents` — counter (running sum).
- `support_credits_applied_cents` — counter (running sum).
- `support_ungrounded_replies_total` — counter (must stay at 0).
- `support_autonomous_write_attempts_total` — counter (must stay at 0).
- `support_redaction_policy_mismatch_total` — counter (must stay at 0).
- `support_replay_quarantine_violations_total{surface}` — counter (must
  stay at 0 outside intentional fuzz tests).
- `support_sla_breaches_total` — counter (tickets that breached SLA
  before being flagged).
- `support_tickets_open` — gauge.

### Alerts

| Alert | Condition | Severity | First action |
|---|---|---|---|
| `support_ungrounded_replies_total > 0` | any | Sev-2 | Freeze deploys; pull the draft's trace; a reply was produced without a policy citation |
| `support_autonomous_write_attempts_total > 0` | any | Sev-1 | Page security; a write reached a tool without an approved row |
| `support_redaction_policy_mismatch_total > 0` | any | Sev-2 | Page on-call; freeze deploys until the policy hash reconciles |
| `support_replay_quarantine_violations_total > 0` | any non-test | Sev-1 | Page security; pull the trace for the violating surface |
| `support_approval_pending_age_seconds{label="IssueSupportRefund"} > 3600` | pending > 1 h | Sev-3 | Page the admin on-call to review the pending refund |
| `support_job_runs_total{kind="sla_breach_scan",status="failed"} >= 2` | 2 in a row | Sev-2 | The SLA scanner is the time-sensitive job; rerun manually and investigate |
| `support_sla_breaches_total` increasing | any breach | Sev-3 | A ticket breached SLA before being flagged; check the scan cadence |
| `support_refunds_issued_cents` spike | > 3σ over 7-day baseline | Sev-2 | Possible refund abuse; freeze the refund route, audit recent approvals |

### Where alerts go

- Sev-1 → pager + security channel + incident channel.
- Sev-2 → pager + operations channel.
- Sev-3 → operations channel.

---

## 10. Incident response — diagnose and recover

### Common incidents

#### A. `POST /replies/send` (or another write) returns 403

**Diagnose:**

1. Pull the trace: `grep -r <request_id> target/traces/`.
2. Find the `kind=approval` span; `status` is `denied` or `pending_review`.
3. Check `audit_events` for the matching `approval_id`. The denial
   `reason` names which policy fired (expired, wrong role, cost ceiling).

**Recover:**

- Wrong role: the requester needs `Admin` (refund/credit) or `Reviewer`
  (reply/escalate/close); have them request via the tenant admin.
- Expired approval: re-issue; the contract has an `expires_ms`.
- Cost ceiling exceeded: file a contract change request, do not raise the
  ceiling in place.

#### B. A drafted reply was ungrounded (no policy citation)

This is the support agent's signature failure mode and a Sev-2.

**Diagnose:**

1. `support_ungrounded_replies_total` incremented, or a human reported a
   reply that cited no policy.
2. Pull the `kind=agent` span for the draft. Check whether the
   `PolicyCitation` provenance is empty.
3. Determine whether the cause is (a) an empty/stale policy corpus index
   (the search returned nothing to cite), or (b) a code change that
   produced a draft without requiring a citation.

**Recover:**

- Empty/stale index: run `policy_reindex` to refresh the corpus, then
  re-draft. If the index was empty at boot, that is a boot-check failure
  (step 5 of §4) — investigate why the listener bound with an empty
  index.
- Code change: revert it. The grounding requirement is a hard
  constraint; a draft without a citation must fail its contract, not ship.
- Add an eval case reproducing the ungrounded draft before unfreezing.

#### C. Suspected autonomous write (reply/refund without approval)

A write reached a tool without an `approved` row. The compiler enforces
the `approve` gate, so a violation means a defect or a bypassed binary.

**Diagnose:**

1. `support_autonomous_write_attempts_total` incremented.
2. `SELECT * FROM audit_events WHERE event_kind IN ('reply.send',
   'refund.issue', 'credit.apply') AND approval_id NOT IN (SELECT id FROM
   approvals WHERE decision = 'approved');` — any row is a confirmed
   bypass.
3. Confirm the deployed binary hash matches a `corvid check`-clean build.

**Recover:**

- Sev-1. Page security. Freeze the affected write routes at the edge.
- If a refund/credit executed, reverse it through the billing provider.
- Confirm binary provenance; redeploy from a verified build.
- Notify compliance.

#### D. `sla_breach_scan` job stuck or missed

**Diagnose:**

1. `SELECT * FROM queue_jobs WHERE task = 'sla_breach_scan' ORDER BY
   created_ms DESC LIMIT 5;`
2. `running` with expired `lease_expires_ms` → the replica crashed.
   `retryable` → awaiting next retry.
3. Check `support_sla_breaches_total` — a rise means tickets breached
   while the scanner was down.

**Recover:**

- The scanner is hourly and time-sensitive; a missed run risks an
  undetected breach. Force a run: `corvid jobs run --kind=sla_breach_scan
  --tenant=<id> --window=business_hour`.
- Persistent failure: read the latest `queue_job_checkpoints` row; the
  `failure_fingerprint` names the cause. Common: the tickets connector
  token expired (§12).

#### E. Escalation loop

A ticket bounces between escalation tiers repeatedly.

**Diagnose:**

1. `SELECT * FROM support_escalations WHERE ticket_id = '<id>' ORDER BY
   created_at;` — a loop shows alternating tiers in a short window.
2. Each escalation required an `EscalateTicket` approval, so the loop is
   human-driven, not autonomous.

**Recover:**

- Escalation is reversible (de-escalate), so this is Sev-3, not Sev-1.
- The fix is process, not code — the agent did exactly what humans
  approved. Flag the ticket for a supervisor to break the loop.

#### F. Refund/credit abuse pattern

`support_refunds_issued_cents` or `support_credits_applied_cents` spiked.

**Diagnose:**

1. Pull recent `approval.approve` events for `IssueSupportRefund` /
   `ApplyAccountCredit`.
2. Check whether one approver approved an unusual volume, or one customer
   fingerprint received repeated credits.

**Recover:**

- Freeze the refund/credit routes at the edge.
- Audit the approver's recent decisions; both are Admin contracts, so the
  blast radius is bounded to Admin actors.
- Apply the segregation-of-duties check (§13): requester ≠ approver.

#### G. Replay quarantine violation

Replay quarantine refuses live connector / provider surfaces during a
Substitute-mode replay.

**Diagnose:**

1. `RuntimeError::QuarantineViolation { surface, detail }` is logged with
   the calling span's `replay_key`.
2. Almost always a missing `@replayable` on a downstream call or a
   non-deterministic input not captured in the replay key.

**Recover:**

- Make the agent replayable and re-run.
- Until fixed, do not promote any fixture exercising that path.

#### H. Triage low-confidence / misclassification

A ticket was triaged into the wrong category, or the triage confidence
is below the floor (the demo floor is 0.80).

**Diagnose:**

1. Pull the `kind=agent` span for the triage; read `confidence` and the
   chosen `category`.
2. A confidence below the floor means the model was unsure — the triage
   should not be treated as authoritative.

**Recover:**

- Low confidence is not an error — it is a signal to route the ticket to
  a human triager rather than auto-drafting. The agent surfaces the
  confidence; it does not hide it. There is no autonomous action to undo.
- A systematic misclassification (a whole category triaging wrong) is a
  model-tuning change with its own eval; do not hand-edit triage rows.
- The reply drafted from a wrong triage still requires `SendSupportReply`
  approval — the reviewer is the backstop, and an off-base draft should
  be denied at the queue.

#### I. Duplicate reply sent to a customer

The same `SendSupportReply` produced two outbound messages.

**Diagnose:**

1. Check the ticketing provider for two outbound messages with the same
   body fingerprint in a short window.
2. Match both to `audit_events`. Same `approval_id` → a provider-side
   retry that did not dedupe on the idempotency key; different
   `approval_id`s → two approvals were issued for the same draft.

**Recover:**

- Same `approval_id`: the durable-job pool keys the send by the
  approval's replay key; a duplicate means the ticketing provider did not
  honor idempotency. Confirm the provider dedupes on the idempotency key
  before enabling real mode.
- Two approvals: an approval-queue hygiene problem — the first approval
  should have moved the draft out of `pending`. Add a guard that refuses
  a second approval against an already-sent draft, and apologize to the
  customer for the duplicate.

---

## 11. Rollback procedures

### Rolling back the binary

```
flyctl releases list --app customer-support-agent
flyctl deploy --image registry.fly.io/customer-support-agent:<prior-tag>
```

or in Kubernetes:

```
kubectl rollout undo deployment/support-api -n support
kubectl rollout status deployment/support-api -n support
```

Verify by running the smoke (§3); the `corvid ops show` snapshot must
match the archived snapshot for the prior release.

### Rolling back a migration

Forward-only (§6). Rollback = restore DB from backup, roll back the
binary, fix-forward.

### Rolling back an approval contract

Approval contracts are typed and live in the binary; rolling back the
binary rolls back the contract. The contract's `version` field is checked
at every approval request and refuses mismatches. Roll the binary, not
the contract.

### Rolling back a connector mode switch

If `CORVID_CONNECTOR_MODE` was flipped from `mock` to `real` in error,
flip it back (re-read on every connector call, immediate). Then audit
`audit_events` for any `reply.send` / `refund.issue` / `credit.apply`
rows in the window the flag was live — a real-mode flip could have sent
replies or moved money.

---

## 12. Connector mode operations

### Modes

The three connectors honor `CORVID_CONNECTOR_MODE`:

- `mock` (default) — deterministic fixtures, no network, no customer
  contact, no money movement. Used by `corvid eval`, `corvid tour`,
  smoke suites.
- `real` — real provider calls. Requires valid ticketing + billing
  provider tokens per tenant (§5).
- `record` — proxies to `real` but writes the raw response to a fixture
  under `target/recordings/`.
- `replay` — reads from `target/recordings/` instead of the provider.

### Switching modes

```
export CORVID_CONNECTOR_MODE=real
corvid ops show | jq '.connector_mode'   # must print "real"
```

Switching to `real` is a release-checklist event: it means the agent can
send customer-facing replies and move money. Confirm provider
credentials, re-verify the policy-grounding posture, and log the change
in `corvid ops show` before serving traffic.

### Per-tenant connector tokens

```
corvid connectors token put --tenant=<id> --connector=tickets --token-file=<path>
corvid connectors token put --tenant=<id> --connector=billing --token-file=<path>
corvid connectors token list --tenant=<id>
corvid connectors token revoke --tenant=<id> --connector=billing
```

Tokens are encrypted at rest with the connector-token key (§5). Revoking
is immediate — the next write fails closed.

### When a connector token expires

The connector raises a typed error. The job retry policy
(`exponential_jitter`, 5 attempts) absorbs transient failures; a refresh
failure that survives all retries lands in the dead-letter queue. Mint a
new token and rerun; the replay key makes the rerun idempotent.

### Keeping the policy corpus fresh

The `policy_connector` searches the policy corpus that grounds every
reply. The `policy_reindex` job refreshes it nightly. If support policies
change mid-day (a new refund window, a changed SLA), run `policy_reindex`
manually rather than waiting for the nightly tick — a stale corpus means
replies cite outdated policy, which is a grounding-quality issue even
though the citation provenance is still valid.

### Policy corpus lifecycle

The policy corpus is the grounding source; treat it as a first-class
operational asset, not a static file.

- **Source of truth.** Policies live in their system of record (a docs
  repo, CMS, or knowledge base), not in the support agent. The agent
  indexes a snapshot. The `provenance_id` on each `PolicyCitation` points
  back to the source policy + section, so an auditor can trace any reply
  to the exact policy text it cited.
- **Versioning.** Each `policy_reindex` records a corpus index hash. The
  `corvid ops show` snapshot includes it, so you can tell which policy
  version a release was grounding against. When a policy changes, the
  hash changes — that is the signal that replies drafted before the
  change cited the prior version.
- **Validation before promotion.** A new policy corpus should be indexed
  in staging and smoke-tested (`GET /replies/draft` returns `grounded =
  true` with a citation to the new policy) before promoting to
  production. A corpus that fails to index leaves the agent unable to
  ground replies — the boot check (§4 step 5) refuses to serve drafts
  against an empty index.
- **Stale-policy detection.** If the source policies move ahead of the
  indexed corpus, replies cite valid-but-outdated policy. Reconcile by
  comparing the source's last-modified timestamp to the last
  `policy_reindex` run; if the source is newer, reindex. Consider a
  source webhook that triggers `policy_reindex` on policy publication for
  tenants with frequently changing policies.
- **Per-tenant corpora.** Each tenant grounds against its own policy
  corpus (`policy_reindex --tenant=<id> --corpus=<name>`). A tenant's
  replies never cite another tenant's policies — `corvid tenants
  verify-isolation` asserts this.

### Daily reconciliation cadence

Run a daily reconciliation (a cron outside the agent or a future durable
job) that:

1. Pulls the ticketing + billing provider activity for the prior day.
2. Matches each outbound reply / refund / credit to an `approvals` row by
   idempotency key.
3. Emits a reconciliation report: matched, provider-only (incident C),
   app-only (failed execution to re-run).

The reconciliation report is a compliance artifact; archive it alongside
the `corvid ops show` snapshots.

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
`expires_at_ms`, plus the `approval_id` joining to `audit_events` and
`trace_lineage`.

### Approving / denying

```
corvid approvals approve --id=<approval_id> --as=<actor_id> --note=<text>
corvid approvals deny --id=<approval_id> --as=<actor_id> --reason=<text>
```

A successful approve checks the approver's role matches `required_role`,
the approval is `pending` and not expired, and the approver is same-tenant.
It writes `approvals.decision = 'approved'` plus an `audit_events` row
(`event_kind = 'approval.approve'`) and a trace span.

### Per-contract considerations

The flow is the developer's choice; each contract reflects an explicit
decision:

- **SendSupportReply** — Reviewer, $0.05, irreversible. Confirm the draft
  carries a `PolicyCitation` (an ungrounded draft should never reach the
  send queue). Verify the body fingerprint matches the reviewed draft.
- **IssueSupportRefund** — Admin, $0.25, irreversible. Verify the amount
  is within the original transaction and the refund policy window.
- **EscalateTicket** — Reviewer, $0.05, reversible. Verify the
  escalation tier and reason; de-escalation is possible if wrong.
- **CloseTicket** — Reviewer, $0.05, reversible. Verify the resolution;
  a closed ticket can be reopened.
- **ApplyAccountCredit** — Admin, $0.25, irreversible. Verify the credit
  amount and reason; a goodwill credit is money-equivalent and should
  follow the credit policy.

### Decision tree — `SendSupportReply`

1. Is the requester a `Reviewer` (or `Admin`)? No → deny.
2. Does the draft carry a `PolicyCitation` with a provenance id? No →
   deny; an ungrounded reply must never be sent.
3. Does the body fingerprint match the reviewed draft? No → deny (the
   draft changed after review).
4. All yes → approve. The send runs through the durable-job pool with the
   approval id attached.

### Decision tree — `IssueSupportRefund`

1. Is the requester an `Admin`? No → deny.
2. Is the refund amount within the original transaction amount? No → deny.
3. Is the request within the refund policy window? No → deny.
4. Is the approver different from the requester (segregation of duties)?
   No → deny.
5. All yes → approve.

### Decision tree — `ApplyAccountCredit`

1. Is the requester an `Admin`? No → deny.
2. Is the credit amount within the per-credit ceiling in the credit
   policy? No → deny.
3. Has this customer fingerprint received an unusual number of credits
   recently (abuse check)? Yes → escalate before approving.
4. Is the approver different from the requester? No → deny.
5. All yes → approve.

### Decision tree — `EscalateTicket`

1. Is the requester a `Reviewer` (or `Admin`)? No → deny.
2. Does the ticket belong to the requester's tenant? No → deny.
3. Is the escalation tier a valid next tier (not skipping or looping back
   to a tier the ticket already left)? No → deny and ask for the correct
   tier.
4. All yes → approve. Escalation is reversible (de-escalate), so the bar
   is intentionally lower; record the `reason_fingerprint`.

### Decision tree — `CloseTicket`

1. Is the requester a `Reviewer` (or `Admin`)? No → deny.
2. Does the ticket belong to the requester's tenant and is it currently
   open? No → deny (closing an already-closed ticket is a no-op; flag
   duplicate requests).
3. Is there a resolution fingerprint recorded? No → deny; a close
   without a resolution loses the audit story.
4. All yes → approve. Closure is reversible (reopen), so the bar is
   lower, but a premature close hurts CSAT — confirm the customer's
   issue is actually resolved.

### Segregation of duties

For the two Admin money contracts (`IssueSupportRefund`,
`ApplyAccountCredit`), the requester and approver should differ. The
periodic audit query flags violations:

```
SELECT id, contract_action FROM approvals
WHERE requester_actor_id = decided_by_actor_id
  AND decision = 'approved'
  AND required_role = 'Admin';
```

Any row returned is a segregation-of-duties violation and a Sev-2
compliance finding.

### Pending queue SLOs

- `SendSupportReply` — pending > 30 min is a Sev-3 page (the customer is
  waiting).
- `IssueSupportRefund` / `ApplyAccountCredit` — pending > 1 h is a Sev-3
  page.
- `EscalateTicket` — pending > 30 min is a Sev-3 page (escalations are
  time-sensitive).

---

## 14. Tenant lifecycle operations

The Support agent is multi-tenant: every ticket, citation, draft, SLA
job, approval, and audit event carries a `tenant_id`. Onboarding and
offboarding touch the most tables at once.

### Onboarding a tenant

1. **Create the tenant row.** `corvid tenants create --id=<id>
   --name=<display>`. Foreign-key anchor for everything else.
2. **Create roles and the first admin.** Every tenant needs at least one
   `Admin` (refund/credit approvals) and one `Reviewer` (reply/escalate/
   close). `corvid auth role grant --tenant=<id> --actor=<actor>
   --role=Admin`.
3. **Register the ticketing connector** (real mode) or rely on mock
   fixtures (default).
4. **Seed and index the policy corpus.** Load the tenant's support
   policies and run `policy_reindex --tenant=<id>` so replies can cite
   them. A tenant with no policy corpus cannot draft grounded replies.
5. **Confirm grounding.** `GET /replies/draft` for the tenant returns
   `grounded = true` with a citation before serving real traffic.

### Offboarding a tenant

Offboarding is a hard delete gated by a legal-hold check and the
retention requirement on the audit trail.

1. **Check for a legal hold.** Active hold → STOP.
2. **Revoke all sessions and API keys.** `corvid auth revoke-all
   --tenant=<id>`.
3. **Disable the tenant's schedules** so no job re-creates rows
   mid-delete.
4. **Export if contractually required** — final ticket/audit export.
5. **Hard delete the operational data.** `corvid tenants delete
   --tenant=<id> --confirm` cascades through `support_*`, `sessions`,
   `api_keys`, `user_roles`.
6. **Retain the audit trail.** `approvals` + `audit_events` + the
   operation tables are NOT deleted — retained for the full retention
   window subject to legal hold. The delete tombstones the tenant row but
   preserves the support-operation history.

### Verifying tenant isolation

```
corvid tenants verify-isolation --tenant=<id>
```

Asserts no `support_*` row for tenant A references a parent row owned by
tenant B, and that no reply, refund, or credit crosses a tenant boundary.
A failure is a Sev-1 cross-tenant customer-data leak.

---

## 15. Durable jobs and cron operations

### The three jobs

| Kind | Cron | Tenant scope | Effects | Approval | Budget |
|---|---|---|---|---|---|
| `sla_breach_scan` | `0 * * * *` America/New_York (hourly) | per tenant per window | `ticket_read` | none | $0.50 |
| `nightly_csat_rollup` | `0 2 * * *` America/New_York | per tenant per day | `ticket_read` | none | $0.50 |
| `policy_reindex` | `0 3 * * *` America/New_York | per tenant per corpus | `policy_search` | none | $0.50 |

None of the three carries a support-write effect — they scan, aggregate,
and reindex. Replies, refunds, escalations, closes, and credits only
happen on the typed `POST` write routes, which are approval-gated by
construction. The scheduler can wake the agent up, but it can never send
a reply or move money.

### Job SLOs

- `sla_breach_scan` p99: 2 min for a tenant with up to ~5k open tickets.
  This is the time-sensitive job — a miss risks an undetected SLA breach,
  so its failure alert is Sev-2 (stricter than the others).
- `nightly_csat_rollup` p99: 5 min per tenant per day.
- `policy_reindex` p99: 5 min for a corpus of up to ~10k policy
  documents.

### Manual triggers

```
corvid jobs run --kind=sla_breach_scan --tenant=tenant-1 --window=business_hour
corvid jobs run --kind=nightly_csat_rollup --tenant=tenant-1 --day=2026-05-28
corvid jobs run --kind=policy_reindex --tenant=tenant-1 --corpus=support_policies
```

The `replay_key` is `kind:tenant:scope` and is the durable-job
idempotency key. Two manual triggers with the same arguments coalesce
into one queued job.

### Retry policy

All three jobs use `exponential_jitter`, 5 attempts, base 1 s, cap 10
min, dead-letter `customer_support_agent.dead_letter`. Dead-lettered
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
writes one automatically. Be cautious disabling `sla_breach_scan` — with
it off, SLA breaches go undetected.

### Why no job sends a reply or moves money

It is worth stating plainly: the three durable jobs scan/aggregate/
reindex only. There is deliberately no "auto-reply" or "auto-refund"
job. Any automation that contacted a customer or moved money would have
to call a `dangerous` write tool, which the compiler refuses outside an
`approve` boundary — and that boundary requires a human decision.

---

## 16. Disaster recovery

### Catastrophic DB loss

1. **Stop traffic.** Take the deploy out of the load balancer.
2. **Restore the latest backup** to a new DB instance (§7). Verify with
   `corvid migrate --check`.
3. **Reconcile executed writes.** Read `audit_events` from the backup for
   `reply.send` / `refund.issue` / `credit.apply` rows; cross-check
   replies against the ticketing provider and refunds/credits against the
   billing provider's ledger for the data-loss window (15 min). Any
   provider action the restored DB does not record must be re-recorded by
   hand with a clear note — the provider is the source of truth for what
   actually reached the customer or moved money.
4. **Point the deploy at the restored DB** and roll.
5. **Run the smoke suite (§3) + the eval suite.**
6. **Rebuild the policy corpus index** via `policy_reindex` for every
   tenant (the index is not in the DB backup).
7. **Write the post-incident report** for the compliance trail.

### Audit redundancy

For regulated tenants, enable the audit-event forwarder writing a copy of
every `audit_events` row to an append-only object-storage log under
`s3://<bucket>/audit/<tenant>/<date>.jsonl`. Recommended for the support
audit trail (it records every customer contact and money movement).

### Policy corpus loss

The policy corpus index is rebuildable from the policy source (the docs
repo / CMS). Run `policy_reindex` per tenant. During the rebuild,
`/replies/draft` returns a `policy_index_rebuilding` envelope rather than
an ungrounded draft — the agent refuses to draft without a citable
corpus rather than improvise.

### Billing-provider divergence

If the app DB and the billing provider disagree about refunds/credits,
**the provider's ledger wins for money that moved**; the app DB wins for
intent and approval. Reconcile by matching each provider transaction to
an `approvals` row by `approval_id`; a provider transaction without a
matching approved row is incident C.

### Loss of the connector-token key

The connector-token key encrypts ticketing + billing tokens. If lost,
those tokens are unrecoverable but the DB and audit trail are intact.
Recovery: rotate the key (§5), force every tenant to re-mint connector
tokens. Real-mode writes are paused until done.

### RPO / RTO targets

- **RPO**: 15 min (WAL ship interval).
- **RTO**: 1 h for a regional DB failure (cross-region replica promote);
  4 h for catastrophic DB loss (restore + write reconciliation + policy
  reindex).

---

## 17. Appendix — reference data

### Schema manifest

`SupportSchemaManifest("customer_support_agent", 5, 20, 3, 3, 5, "mock",
true)`:

- 5 migrations: `0001_support_triage`, `0002_approvals_sla`,
  `0003_auth`, `0004_approvals_and_durable_jobs`,
  `0005_support_operations`.
- 20 tables: see §2.
- 3 connectors: `tickets_connector`, `policy_connector`,
  `support_ai_connector`.
- 3 durable jobs: `sla_breach_scan`, `nightly_csat_rollup`,
  `policy_reindex`.
- 5 approval contracts: `SendSupportReply`, `IssueSupportRefund`,
  `EscalateTicket`, `CloseTicket`, `ApplyAccountCredit`.
- Default mode: `mock`. Policy-grounded: `true`.

### Capacity planning

Per tenant unless noted. Support is sensitive to ticket volume and
approval throughput.

| Tenant size | Open tickets | Daily new tickets | `sla_breach_scan` | `policy_reindex` | Action |
|---|---|---|---|---|---|
| Small | < 100 | < 50 | < 10 s | < 30 s | Single replica, `shared-cpu-1x` |
| Medium | 100 – 1k | 50 – 500 | < 1 min | < 2 min | Default; 1-3 replicas |
| Large | 1k – 5k | 500 – 5k | 1 – 2 min | 2 – 5 min | Postgres read replica for the triage route |
| XL | > 5k | > 5k | multi-step | > 5 min | Shard the SLA scan by queue; move triage to its own worker pool |

Other limits:

- **Approval throughput** — DB-backed; the bottleneck is human review
  latency, not the queue. Watch `support_approval_pending_age_seconds`.
- **DB sizing** — `support_approval_audits` + `audit_events` + the
  operation tables grow append-only and are retained for the customer-
  data / financial-records window. Budget storage for the full window.
- **Policy corpus** — `policy_reindex` is CPU-bound on the
  `support_ai_connector` model. Stays under SLO up to ~10k policy docs;
  past that, partition the reindex.

### Compliance posture

- **Policy grounding.** Every customer-facing reply cites policy; the
  draft contract fails without a citation. An ungrounded reply (§10
  incident B) is a quality + compliance issue routed to the support lead.
- **Audit immutability.** `approvals`, `audit_events`, and the operation
  tables are append-only and retained for the regulatory window.
  Offboarding tombstones the tenant but never deletes the history (§14).
- **Segregation of duties.** Requester ≠ approver for the Admin money
  contracts (refund, credit) (§13).
- **No autonomous customer contact or money movement.** The three cron
  jobs cannot send a reply or issue a refund — every such action requires
  a human approval the compiler enforces (§15).
- **PII minimization.** Customer identifiers are fingerprints in the DB
  and fixtures; raw PII stays in the ticketing provider.

### SLA tiers (reference)

`sla_breach_scan` flags tickets approaching these windows. The windows
are operator policy, not hard-coded in the agent — tune them per tenant
contract. The defaults the demo assumes:

| Priority | First-response SLA | Resolution SLA | Scan flags at |
|---|---|---|---|
| `urgent` | 1 h | 8 h | 45 min / 6 h |
| `high` | 4 h | 24 h | 3 h / 20 h |
| `normal` | 8 h | 72 h | 6 h / 60 h |
| `low` | 24 h | 120 h | 20 h / 108 h |

The hourly scan cadence means a flag fires at most ~1 h before the
breach for the tightest tier; for `urgent` tickets, consider a 15-minute
scan in production rather than the hourly default.

### Role → permission mapping (reference)

| Role | Can approve | Cannot approve |
|---|---|---|
| `Reviewer` | `SendSupportReply`, `EscalateTicket`, `CloseTicket` | refunds, credits |
| `Admin` | all five (incl. `IssueSupportRefund`, `ApplyAccountCredit`) | — |

An `Admin` can approve everything a `Reviewer` can; the money contracts
are Admin-only. The auth surface propagates one typed permission string
per dangerous tool (`support.tool.send_support_reply`,
`support.tool.issue_support_refund`,
`support.tool.escalate_ticket`, `support.tool.close_ticket`,
`support.tool.apply_account_credit`) through the `Actor` permissions, so
a permission check can gate each tool independently of role — a finer
grain than the role gate alone. The five strings are distinct by
construction; an audit can confirm no two tools share a permission.

### Effect catalog

| Effect | Cost | Trust | Data class | Used by |
|---|---|---|---|---|
| `ticket_read` | $0.01 | workspace | customer | triage, SLA scan, CSAT rollup |
| `policy_search` | $0.01 | grounded | internal | triage, draft, policy reindex |
| `support_ai` | $0.06 | bounded | customer | triage, draft |
| `support_write` | $0.01 | human_required | customer | `send_support_reply` |
| `refund_write` | $0.02 | human_required | financial | `issue_support_refund` |
| `escalate_write` | $0.01 | human_required | customer | `escalate_ticket` |
| `ticket_write` | $0.01 | human_required | customer | `close_ticket` |
| `credit_write` | $0.02 | human_required | financial | `apply_account_credit` |

### Route catalog

| Method | Route | Returns | Effects | Approval |
|---|---|---|---|---|
| GET | `/config` | `SupportConfig` | none | none |
| GET | `/schema` | `SupportSchemaManifest` | none | none |
| GET | `/tickets/triage/mock` | `SupportTriage` | `ticket_read`, `policy_search`, `support_ai` | none |
| GET | `/replies/draft/mock` | `SupportDraftReply` | `ticket_read`, `policy_search`, `support_ai` | none |
| GET | `/sla/jobs/mock` | `SupportSlaJob` | none | none |
| GET | `/eval/dashboard/mock` | `SupportEvalDashboard` | none | none |
| POST | `/replies/send` | `SupportReplySendReceipt` | `support_write` | `SendSupportReply` |
| POST | `/refunds/issue` | `SupportRefundReceipt` | `refund_write` | `IssueSupportRefund` |
| POST | `/tickets/escalate` | `EscalateTicketReceipt` | `escalate_write` | `EscalateTicket` |
| POST | `/tickets/close` | `CloseTicketReceipt` | `ticket_write` | `CloseTicket` |
| POST | `/credits/apply` | `ApplyAccountCreditReceipt` | `credit_write` | `ApplyAccountCredit` |
| POST | `/auth/session/login` | `LoginResponse` | none | none |
| POST | `/auth/api-key/login` | `ApiKeyLoginResponse` | none | none |
| GET | `/auth/status` | `AuthStatusResponse` | none | none |
| GET | `/auth/api-key/status` | `AuthStatusResponse` | none | none |
| GET | `/jobs/sla-breach-scan/mock` | `SupportJobRun` | `ticket_read` | none |
| GET | `/jobs/nightly-csat-rollup/mock` | `SupportJobRun` | `ticket_read` | none |
| GET | `/jobs/policy-reindex/mock` | `SupportJobRun` | `policy_search` | none |

### Adversarial corpus

Six named threats under [`adversarial/`](../adversarial/):

- `ungated_send_support_reply.cor` — calls `send_support_reply` without
  `approve SendSupportReply(...)`.
- `ungated_issue_support_refund.cor` — calls `issue_support_refund`
  without `approve IssueSupportRefund(...)`.
- `ungated_escalate_ticket.cor` — calls `escalate_ticket` without
  `approve EscalateTicket(...)`.
- `ungated_close_ticket.cor` — calls `close_ticket` without
  `approve CloseTicket(...)`.
- `ungated_apply_account_credit.cor` — calls `apply_account_credit`
  without `approve ApplyAccountCredit(...)`.
- `ungrounded_reply.json` — the declarative grounding threat: a draft
  reply without a policy citation must be rejected.

The five `.cor` fixtures are refused by `corvid check` with `E0101`. Any
green build on these is a Sev-1 — the approval gate is the foundation of
the agent's no-autonomous-write claim.

### Approval contract reference

| Label | Role | Ceiling | Reversible | Reason |
|---|---|---|---|---|
| `SendSupportReply` | Reviewer | $0.05 | no | customer-facing, can't unsend |
| `IssueSupportRefund` | Admin | $0.25 | no | money leaves |
| `EscalateTicket` | Reviewer | $0.05 | yes | de-escalate |
| `CloseTicket` | Reviewer | $0.05 | yes | reopen |
| `ApplyAccountCredit` | Admin | $0.25 | no | money-equivalent |

### Promoted eval fixtures

Three promoted fixtures under [`evals/promoted/`](../evals/promoted/):

- `support-demo.lineage-eval.json` — policy-grounded triage/draft.
- `support-sla-scan.lineage-eval.json` — `sla_breach_scan` durable job +
  ticket read.
- `support-reply-send.lineage-eval.json` — reply route +
  `SendSupportReply` approval (pending_review) + audit.

### Environment variable reference

| Variable | Default | Purpose |
|---|---|---|
| `CORVID_APP_ENV` | `local` | Environment (local / staging / production) |
| `CORVID_CONNECTOR_MODE` | `mock` | Connector mode (mock / replay / real / record) |
| `CORVID_REQUIRE_APPROVALS` | `true` | If true, every dangerous tool fails closed without approval |
| `CORVID_DATABASE_URL` | `sqlite:target/support.db` | DB connection string |
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
