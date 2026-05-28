# Code Maintenance Agent — Operator Runbook

This runbook is the operational source of truth for running the Code
Maintenance Agent backend in development, staging, and production. The
Code Maintenance agent is a reference Corvid application: it ingests
repository metadata, triages issues with CI-aware risk labels, drafts
review comments and patch proposals, and executes code-maintenance
operations a human has approved (review comment, patch proposal, PR
open, PR merge, release tag) — each behind a typed, developer-authored
approval contract.

**The defining constraints: every write requires approval, and risk
triage is CI-grounded.** No tool merges a PR, tags a release, or even
posts a comment without a human approving the typed contract. And a
risk label is not a guess — a high-severity regression label is
grounded in a failed `CiSignal`; the triage contract fails without it.

Every procedure below is grounded in surfaces the app actually ships.
The schema manifest at [`src/main.cor`](../src/main.cor) declares the
canonical counts (5 migrations / 21 tables / 3 connectors / 3 durable
jobs / 5 approval contracts / writes-require-approval) and `corvid serve` exposes the routes that drive each procedure.

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

### What the Code Maintenance Agent does

The Code Maintenance Agent runs three durable jobs on a cron and exposes
a typed HTTP surface for the work a code-maintenance team's assistant
performs:

- `hourly_ci_signal_scan` (cron `0 * * * *` America/New_York, hourly) —
  polls CI for failing jobs and produces `CodeRiskLabel`s grounded in
  the `CiSignal`. CI is time-sensitive, so this runs hourly. It labels
  risk; it does not open a PR, merge, or comment.
- `nightly_repo_reindex` (cron `0 2 * * *` America/New_York) — rescans
  registered repositories and refreshes the tree hashes that anchor
  issue/PR provenance. Observational.
- `weekly_stale_issue_sweep` (cron `0 6 * * 1` America/New_York) — flags
  issues with no activity past a staleness window so a human can decide
  whether to close or escalate. It flags; it does not close.

Every code write enters one of five approval contracts. The flow is the
developer's choice — role and reversibility differ per surface on
purpose:

| Approval label | Tool | Effect | Target type | Required role | Cost ceiling | Reversible |
|---|---|---|---|---|---|---|
| `PostReviewComment` | `post_review_comment` | `code_write` | `issue` | Reviewer | $0.05 | yes (delete) |
| `CreatePatchProposal` | `create_patch_proposal` | `code_write` | `repo` | Reviewer | $0.05 | yes (proposal) |
| `OpenPullRequest` | `open_pull_request` | `pr_write` | `repo` | Reviewer | $0.05 | yes (close PR) |
| `MergePullRequest` | `merge_pull_request` | `merge_write` | `repo` | Admin | $0.25 | no (mainline) |
| `TagRelease` | `tag_release` | `release_write` | `repo` | Admin | $0.25 | no (published) |

The role gradient encodes blast radius: the two Admin contracts change
the mainline or publish a release (irreversible); the three Reviewer
contracts are reversible proposals (comment, patch, PR).

### What the Code Maintenance Agent does NOT do

- **It does not write to a repository without approval.** Every
  `code_write`, `pr_write`, `merge_write`, and `release_write` is
  `dangerous` and the compiler rejects callers that lack a matching
  `approve <Label>(...)` boundary. Drafting a patch is allowed;
  *merging* it without approval does not compile.
- **It does not invent risk labels.** A high-severity regression label
  is grounded in a failed `CiSignal`; the triage contract
  (`code_triage_valid`) requires `ci.status == "failed"` and a
  confidence at/above the floor. The agent labels from CI evidence.
- **It does not merge or tag autonomously.** The three cron jobs scan
  CI, reindex repos, and flag stale issues — none of them opens a PR,
  merges, or tags. Those are human-approved actions only.
- **It does not talk to real provider APIs in the default mode.** The
  default `CORVID_CONNECTOR_MODE=mock` keeps every connector offline
  with deterministic fixtures.
- **It does not commit raw proprietary source.** The DB and fixtures
  hold fingerprints (patch, comment, tree, log), never raw code.

### Service-level objectives

- Availability: 99.9 % monthly for `GET /issues/triage` and
  `/repos/ingest`; 99.5 % for `POST /comments/*`, `/patches/*`,
  `/pull-requests/*`, `/releases/*` (lower because approval-gated).
- Latency (p99): `/issues/triage` < 1500 ms (CI read + repo read +
  model). Approved writes are async through the durable-job pool — see
  §15.
- Grounding integrity: 100 % of high-severity risk labels are grounded
  in a failed CI signal. Any ungrounded high-severity label is a Sev-2.
- Approval integrity: 100 % of executed code writes have a matching
  `approved` row in `approvals`. Any merge/tag without it is a Sev-1.

---

## 2. Architecture map

### Process layout

The Code Maintenance agent runs as a single Corvid server binary plus a
SQLite or Postgres backing store. The binary is built from `src/main.cor`
via `corvid build --target=server`. In production it is wrapped in a
distroless OCI image; `corvid serve` serves all HTTP routes and a `corvid jobs run` worker process runs the durable-job pool, the scheduler, the OTLP exporter, and the metrics
endpoint.

```
+---------------------------+
|   corvid jobs run         |
|   (durable job pool,      |  <-- hourly_ci_signal_scan,
|    in-process)            |       nightly_repo_reindex,
+---------------------------+       weekly_stale_issue_sweep
            |
            v
+---------------------------+
|   corvid runtime          |  <-- typed effects, approvals,
|   (HTTP routes,           |       CI-aware triage, replay
|    scheduler,             |       quarantine
|    OTLP exporter,         |
|    /metrics)              |
+---------------------------+
            |
   +--------+--------+
   |        |        |
   v        v        v
+------+ +------+ +-----------+
| DB   | | CI   | | Git host  |
| (5   | | host | | (real     |
|  migs| | (CI  | |  mode:    |
| / 21 | | runs)| |  PR/merge/|
| tbls)| |      | |  tag)     |
+------+ +------+ +-----------+
```

### Data classes

Every effect in the Code Maintenance agent carries the `code` data
class. The compiler refuses to cross from read/observe to a repository
write without an explicit approval. Operationally:

- **Read/observe** — repo metadata, CI signals, issue triage. Effects
  `repo_read`, `ci_read`, `code_ai`. Never writes to a repository.
- **Repository write** — anything that changes a repo or its published
  state. Effects `code_write` (comment/patch), `pr_write`, `merge_write`,
  `release_write`. Each requires its typed approval contract.

There is no separate `proprietary_code` data class because raw code is
never stored — only fingerprints. The grounding evidence (CI signals,
tree hashes) and the write artifacts (patches, comments) are all
fingerprinted.

### Storage surfaces

- **Database** (5 migrations, 21 tables):
  - Code domain (`0001`/`0002`/`0005`, 9 tables): `code_repositories`,
    `code_issues`, `code_ci_signals`, `code_risk_labels`,
    `code_write_plans`, `code_approval_audits`, `code_pull_requests`,
    `code_merges`, `code_releases`.
  - Auth (`0003`, 7 tables): `tenants`, `users`, `roles`, `user_roles`,
    `sessions`, `api_keys`, `permissions`.
  - Approvals + jobs + lineage (`0004`, 5 tables): `approvals`,
    `audit_events`, `queue_jobs`, `queue_job_checkpoints`,
    `trace_lineage`.
- **CI host** (real mode only): the CI system the agent polls for
  failing jobs. Read-only via the CI connector.
- **Git host** (real mode only): the git provider (GitHub, GitLab) where
  approved comments/patches/PRs/merges/tags land. Reached only through
  the approval-gated write tools.

### Connector layout

The Code Maintenance agent's three connectors are:

- `repo_connector` (effect: `repo_read`) — reads repository metadata,
  issues, and tree hashes from the git host.
- `ci_connector` (effect: `ci_read`) — reads CI run status and failing
  jobs from the CI host.
- `code_ai_connector` (effect: `code_ai`) — runs the bounded
  triage/draft model. Bounded trust: it labels risk from CI evidence
  and drafts review comments/patches; it does not merge or tag.

The write effects (`code_write`, `pr_write`, `merge_write`,
`release_write`) route through the runtime's `HttpRuntime` to the git
host; they are approval-gated and fail closed without approval.

### The triage → draft → approve → write pipeline

The core flow is four stages, and the CI-grounding + approval guarantees
live at specific points:

1. **Triage** (`GET /issues/triage`, effects `repo_read`, `ci_read`,
   `code_ai`). Reads the issue, reads the CI signal, and produces a
   `CodeRiskLabel`. The label's severity is grounded in the CI status —
   `code_triage_valid` requires `ci.status == "failed"` for a
   high-severity regression. CI grounding starts here.
2. **Draft** (`GET /writes/plan/mock`, same effects). Produces a
   `CodeWritePlan` — a review comment fingerprint and a patch
   fingerprint, both `writes_gated`. The plan is a proposal; nothing has
   touched the repo yet.
3. **Approve** (`corvid approvals approve`, or the queue UI). A human
   approves the relevant contract: a `Reviewer` for comment/patch/PR, an
   `Admin` for merge/release. This is where a human takes responsibility
   for the repository write.
4. **Write** (`POST /comments/post`, `/patches/propose`,
   `/pull-requests/{open,merge}`, `/releases/tag` — each gated by its
   `approve <Label>`). Only an approved write reaches the git host. The
   compiler guarantees the dangerous tool cannot be called without the
   `approve` boundary.

The invariant that matters operationally: **CI grounding is enforced at
stage 1 (triage), human responsibility at stage 3 (approve), and the
compiler gate at stage 4 (write).** The highest-blast-radius writes
(merge, release) additionally require Admin role and a CI-green check at
approval time (§13). A failure at any stage stops the pipeline rather
than writing to a repository on weak evidence or without a human.

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
cd examples/backend/code_maintenance_agent
export CORVID_APP_ENV=local              # local | staging | production
export CORVID_CONNECTOR_MODE=mock        # default; keeps everything offline
export CORVID_DATABASE_URL=sqlite:target/code.db
export CORVID_REQUIRE_APPROVALS=true     # default; fail closed on every dangerous tool
corvid check src/main.cor
corvid migrate --database-url=$CORVID_DATABASE_URL --dir=migrations
corvid seeds load seeds/demo.sql
corvid serve src/main.cor --listen 127.0.0.1:8089
```

If everything is wired correctly, `corvid serve` exposes the routes on
port 8089. `GET /config` returns the
`CodeConfig("code_maintenance_agent", "mock", true)` envelope — note
`writes_require_approval = true`.

### Smoke test the local boot

In a second shell:

```
curl -s http://127.0.0.1:8089/schema | jq
curl -s http://127.0.0.1:8089/issues/triage/mock | jq '.ci.status, .severity, .confidence'
curl -s http://127.0.0.1:8089/writes/plan/mock | jq '.writes_gated, .approval_count'
curl -s http://127.0.0.1:8089/jobs/hourly-ci-signal-scan/mock | jq '.contract.job_kind'
```

Expected: `ci.status` is `failed`, `severity` is `high`, `confidence` ≥
0.8, `writes_gated` is `true`. If `writes_gated` is ever `false` or a
high-severity label appears with `ci.status != failed`, *do not deploy*
— a core invariant has regressed.

### Run the typed eval suite

```
corvid eval evals/write_approval_eval.cor
```

Must exit 0 with `values: 11/11 passed`. The suite covers the maturity
bar minima, the five approval contracts, the role/reversibility
gradient, the three cron schedules, job bounding, and the CI-grounded
triage invariant (case 11).

### Confirm the adversarial gates

```
corvid check adversarial/ungated_post_review_comment.cor    # → E0101
corvid check adversarial/ungated_create_patch_proposal.cor  # → E0101
corvid check adversarial/ungated_open_pull_request.cor      # → E0101
corvid check adversarial/ungated_merge_pull_request.cor     # → E0101
corvid check adversarial/ungated_tag_release.cor            # → E0101
```

Each must exit `1` with `E0101 — dangerous tool called without a prior
approve`. The declarative `adversarial/raw_patch_committed.json` is the
sixth named threat (no raw proprietary code committed).

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

- **Edge** — TLS termination, request authentication, rate limit,
  request id injection. The agent does not bundle an edge proxy; use
  `nginx`, `envoy`, or the cloud provider's L7 LB.
- **App** — N replicas behind the edge. Each runs the HTTP server,
  durable-job pool, and scheduler. The scheduler uses a per-job advisory
  lock so only one replica runs each cron tick.
- **Data** — Postgres primary + read replica.

### Fly.io reference

`deploy/fly.toml` defines the canonical Fly.io deployment. Key
parameters:

- `app = "code-maintenance-agent"`
- Primary region: `iad`.
- VM size: `shared-cpu-1x`, 512 MB.
- Auto-scaling: 1 to 3 replicas; HTTP service on internal port `8080`.
- `[env]`: `CORVID_CONNECTOR_MODE = "mock"` by default. Production
  unsets this to `real` only after the operator has verified the git
  host + CI host credentials are configured (§5) AND the
  writes-require-approval posture has been re-confirmed in the release
  checklist.

Deploy:

```
flyctl deploy --config deploy/fly.toml
flyctl logs --app code-maintenance-agent
flyctl ssh console --app code-maintenance-agent --command "corvid ops show"
```

Archive each production `corvid ops show` snapshot under
`ops/snapshots/<YYYY-MM-DD>.json`.

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
kubectl -n code rollout status deploy/code-api
```

### Docker Compose (single-host)

```
cd examples/backend/code_maintenance_agent
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml exec code corvid ops show
```

### Boot sequence

1. Read env. If any `CORVID_*` variable is malformed, exit non-zero.
2. Connect to the DB. Five-attempt exponential backoff, then exit
   non-zero so the orchestrator restarts the pod.
3. Run migrations 0001 → 0005 (idempotent — see §6).
4. If `CORVID_CONNECTOR_MODE=real`, verify git host + CI host
   credentials are present; fail closed if not (an agent that can merge
   to mainline must never boot into real mode without verified write
   credentials).
5. Start the OTLP exporter and `/metrics` endpoint.
6. Start the durable-job pool; resume expired-lease jobs as `retryable`.
7. Start the scheduler; compute next-fire timestamps for the 3 crons.
8. Bind the HTTP listener.

If steps 1–4 fail, the binary exits and the orchestrator restarts it.

---

## 5. Secrets management

### Inventory

The Code Maintenance agent stores four classes of secrets:

- **Database credentials** — Postgres URL or SQLite path. Read at boot;
  never logged.
- **Git-host credentials** — the token / GitHub App key that posts
  comments, opens/merges PRs, and tags releases. Stored only in
  `connector_tokens` (encrypted with the connector-token key), never in
  env. This is the highest-value secret — it can merge to mainline.
- **CI-host credentials** — read-only token for polling CI. Same storage
  rules.
- **Connector-token key + auth secrets** — `CORVID_CONNECTOR_TOKEN_KEY`
  (AES-256), `CORVID_API_KEY_PEPPER`, `CORVID_SESSION_SIGNING_KEY`,
  `CORVID_CSRF_SECRET`. Injected from the secret manager at boot.

### Where to store them

- **Production:** secret manager (Vault, AWS SM, GCP SM, K8s Secret).
  Never in env files committed to git, never in `configmap.yaml`.
- **Staging:** same, in a separate secret namespace.
- **Local:** a developer-private `.env.local` (gitignored). Never check
  in real git-host credentials; use mock connectors.

### Rotation

- **DB credentials** — rotate quarterly via mint-new-user → update
  secret → roll deploy → drop old user.
- **Git-host credentials** — rotate per provider policy; scope the token
  to the minimum (a GitHub App with only the needed permissions, not a
  personal access token with full repo scope). Treat a suspected leak as
  a Sev-1 (it can merge/tag) and rotate immediately, which pauses
  real-mode writes until the new credential is verified.
- **CI-host credentials** — rotate per provider policy; read-only, so
  lower risk than the git-host token.
- **Connector-token key** — rotate annually or on suspected compromise
  via `corvid auth keys rotate --kind connector-token` under a
  maintenance window.
- **API-key pepper / session signing / CSRF secret** — pepper rotation
  invalidates API keys, session signing rotates every 30 days with a
  15-minute grace, CSRF secret rotates immediately.

### What never gets logged

- Plaintext git-host / CI-host credentials.
- Raw patch or comment bodies, or raw source (only fingerprints).
- CI log content (only the log fingerprint).
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

- `0001_ingestion_triage.sql` — `code_repositories`, `code_issues`,
  `code_ci_signals`, `code_risk_labels`.
- `0002_writes_approvals.sql` — `code_write_plans`,
  `code_approval_audits`.
- `0003_auth.sql` — `tenants`, `users`, `roles`, `user_roles`,
  `sessions`, `api_keys`, `permissions`.
- `0004_approvals_and_durable_jobs.sql` — `approvals`, `audit_events`,
  `queue_jobs`, `queue_job_checkpoints`, `trace_lineage`.
- `0005_code_operations.sql` — backing tables for the three new write
  surfaces: `code_pull_requests`, `code_merges`, `code_releases`.

### Detecting drift

```
corvid migrate --check --database-url=$CORVID_DATABASE_URL --dir=migrations
```

Compares the DB's `schema_version` to the migration directory. Non-zero
exit on any mismatch. `corvid ops show` includes the schema version and
each migration's file hash; archive it per release.

### Rollback

Code Maintenance migrations are **forward-only**. There is no `migrate
down`. To revert: restore the DB from the latest backup before the bad
migration (§7), roll back the binary, write a fix-forward migration.
Forward-only protects the `approvals` + `audit_events` + operation
tables that record every merge and release.

---

## 7. Backups — what, where, how often

### What gets backed up

- **DB**: full snapshot + WAL ship every 15 min (Postgres) or hot-copy
  every hour (SQLite).
- **Approvals + audit_events + the operation tables**: included in the
  DB backup. `code_merges` and `code_releases` are the record of every
  mainline change and published release — never lose them.
- **Repository content**: NOT backed up by this agent — it lives in the
  git host, which is the system of record. The agent stores only
  fingerprints and metadata.

### Where

- DB backups: cloud-provider managed object storage in a separate
  account with cross-region replication and an immutable retention lock.

### How often

- DB: 15-min WAL ship, hourly base backup, daily verified restore.
- Verified-restore drill: every Monday 09:00 UTC, staging restores the
  most recent prod backup and runs the smoke suite (§3). Failure is a
  Sev-2.

### Retention

- DB backups: governed by the org's source-control retention policy.
  `code_merges` / `code_releases` / `audit_events` follow the longer of
  the engineering-audit and compliance windows.
- `approvals` + `audit_events` + operation tables: retained for the full
  window even after tenant offboarding, subject to legal hold.

---

## 8. Logs and traces

### Log streams

- **App log** — structured JSON to stdout. One line per request, one per
  job state transition, one per error.
- **Trace log** — OTLP-equivalent JSONL under
  `target/traces/<date>/<trace_id>.lineage.jsonl`, schema
  `corvid.trace.lineage.v1`.
- **Audit log** — written to `audit_events` on every approval
  approve/deny and every executed code write. Never logged to stdout.

### What every trace must include

- `kind` ∈ `{ route, job, agent, prompt, tool, approval, db, retry,
  error, eval, review }`.
- `status` ∈ `{ ok, failed, denied, pending_review, replayed, redacted }`.
- `replay_key` — populated for every durable-job and approval span.
- `effect_ids` — the typed effects exercised; empty for read-only spans.
- `redaction_policy_hash` — must match the runtime's active policy.

### Audit event kinds

The `audit_events.event_kind` column is the code-maintenance compliance
vocabulary:

| event_kind | When | Joins to |
|---|---|---|
| `approval.request` | a write intent is created | `approvals.id` |
| `approval.approve` | an approver approves | `approvals.id`, `decided_by_actor_id` |
| `approval.deny` | an approver denies | `approvals.id`, `decision_reason` |
| `comment.post` | an approved review comment is posted | `approvals.id` |
| `patch.propose` | an approved patch proposal is created | `approvals.id` |
| `pr.open` | an approved PR is opened | `code_pull_requests.id` |
| `pr.merge` | an approved PR is merged | `code_merges.id` |
| `release.tag` | an approved release is tagged | `code_releases.id` |
| `schedule.disable` / `schedule.enable` | a cron schedule is toggled | job kind |

Every repository-write event_kind must have a preceding
`approval.approve` with a matching `approval_id`.

### Where to look

| Symptom | First log to read |
|---|---|
| `POST /pull-requests/merge` returns 403 | `audit_events` for the matching `approval_id`, then the `kind=approval` span |
| A high-severity label with passing CI | the `kind=agent` triage span; check the `CiSignal` status and the risk `confidence` |
| `hourly_ci_signal_scan` did not run | latest `queue_jobs` row with `task=hourly_ci_signal_scan`, then the `kind=job` span |
| A merge happened unexpectedly | `audit_events` for the `approval:merge_pr:*` row and its `approval.approve` event |
| Repo tree hashes look stale | the `nightly_repo_reindex` job trace |

### Promoting a trace to an eval fixture

```
corvid eval promote target/traces/<date>/<trace_id>.lineage.jsonl \
    --promote-out evals/promoted
```

The agent ships three promoted fixtures under
[`evals/promoted/`](../evals/promoted/) (code-demo, code-ci-scan,
code-merge-pr); every regression replays them.

---

## 9. Metrics and alerting

### What we export

`/metrics` exposes Prometheus-format counters and histograms:

- `code_http_requests_total{route,status}` — counter.
- `code_http_request_duration_seconds{route}` — histogram.
- `code_job_runs_total{kind,status}` — counter.
- `code_job_duration_seconds{kind}` — histogram.
- `code_approval_decisions_total{label,decision}` — counter.
- `code_approval_pending_age_seconds{label}` — gauge.
- `code_merges_total` — counter.
- `code_releases_total` — counter.
- `code_ungrounded_risk_labels_total` — counter (must stay at 0; a
  high-severity label without a failed CI signal).
- `code_autonomous_write_attempts_total` — counter (must stay at 0).
- `code_redaction_policy_mismatch_total` — counter (must stay at 0).
- `code_replay_quarantine_violations_total{surface}` — counter (must
  stay at 0 outside intentional fuzz tests).
- `code_open_issues` — gauge.

### Alerts

| Alert | Condition | Severity | First action |
|---|---|---|---|
| `code_ungrounded_risk_labels_total > 0` | any | Sev-2 | Freeze deploys; pull the triage trace; a high-severity label appeared without a failed CI signal |
| `code_autonomous_write_attempts_total > 0` | any | Sev-1 | Page security; a write reached a tool without an approved row |
| `code_redaction_policy_mismatch_total > 0` | any | Sev-2 | Page on-call; freeze deploys until the policy hash reconciles |
| `code_replay_quarantine_violations_total > 0` | any non-test | Sev-1 | Page security; pull the trace for the violating surface |
| `code_approval_pending_age_seconds{label="MergePullRequest"} > 3600` | pending > 1 h | Sev-3 | Page the admin on-call to review the pending merge |
| `code_approval_pending_age_seconds{label="TagRelease"} > 7200` | pending > 2 h | Sev-3 | Page the admin on-call |
| `code_job_runs_total{kind="hourly_ci_signal_scan",status="failed"} >= 2` | 2 in a row | Sev-2 | The CI scanner is the time-sensitive job; rerun and investigate |
| `code_merges_total` spike | > 3σ over 7-day baseline | Sev-2 | Possible runaway merge flow; freeze the merge route, audit recent approvals |

### Where alerts go

- Sev-1 → pager + security channel + incident channel.
- Sev-2 → pager + operations channel.
- Sev-3 → operations channel.

---

## 10. Incident response — diagnose and recover

### Common incidents

#### A. `POST /pull-requests/merge` (or another write) returns 403

**Diagnose:**

1. Pull the trace: `grep -r <request_id> target/traces/`.
2. Find the `kind=approval` span; `status` is `denied` or `pending_review`.
3. Check `audit_events` for the matching `approval_id`. The denial
   `reason` names which policy fired (expired, wrong role, cost ceiling).

**Recover:**

- Wrong role: the requester needs `Admin` (merge/tag) or `Reviewer`
  (comment/patch/PR); have them request via the tenant admin.
- Expired approval: re-issue; the contract has an `expires_ms`.
- Cost ceiling exceeded: file a contract change request, do not raise
  the ceiling in place.

#### B. A high-severity risk label without a failed CI signal

This is the code agent's signature grounding failure and a Sev-2.

**Diagnose:**

1. `code_ungrounded_risk_labels_total` incremented, or a human reported
   a high-severity label whose CI is green.
2. Pull the `kind=agent` triage span. Check the `CiSignal` status and
   the risk `confidence`.
3. Determine whether the cause is (a) the triage model overstated
   severity without CI evidence, or (b) a code change that produced a
   label without checking the CI signal.

**Recover:**

- Model overstatement: tighten the triage prompt so a high-severity
  label requires a failed CI signal; add an eval reproducing the
  ungrounded label.
- Code change: revert it. The CI-grounding requirement is a hard
  constraint — `code_triage_valid` must fail a high-severity label that
  is not CI-grounded, not ship it.

#### C. Suspected autonomous write (merge/tag without approval)

A write reached a tool without an `approved` row. The compiler enforces
the `approve` gate, so a violation means a defect or a bypassed binary.

**Diagnose:**

1. `code_autonomous_write_attempts_total` incremented.
2. `SELECT * FROM audit_events WHERE event_kind IN ('pr.merge',
   'release.tag') AND approval_id NOT IN (SELECT id FROM approvals WHERE
   decision = 'approved');` — any row is a confirmed bypass.
3. Confirm the deployed binary hash matches a `corvid check`-clean build.

**Recover:**

- Sev-1. Page security. Freeze the merge/tag routes at the edge.
- If a merge landed on mainline, revert it with a new commit (a merge is
  irreversible — the revert is a forward operation, not an undo).
- If a release was tagged, follow the release-rollback runbook for the
  affected repo.
- Confirm binary provenance; redeploy from a verified build.

#### D. `hourly_ci_signal_scan` job stuck or missed

**Diagnose:**

1. `SELECT * FROM queue_jobs WHERE task = 'hourly_ci_signal_scan' ORDER
   BY created_ms DESC LIMIT 5;`
2. `running` with expired `lease_expires_ms` → the replica crashed.
   `retryable` → awaiting next retry.

**Recover:**

- The scanner is hourly; a missed run means risk labels lag CI by up to
  an hour. Force a run: `corvid jobs run --kind=hourly_ci_signal_scan
  --tenant=<id> --window=business_hour`.
- Persistent failure: read the latest `queue_job_checkpoints` row; the
  `failure_fingerprint` names the cause. Common: the CI connector token
  expired (§12).

#### E. Merge conflict / failed merge

An approved `MergePullRequest` failed because the branch no longer merges
cleanly.

**Diagnose:**

1. The git host returns a conflict; the `code_merges` row stays in a
   non-`merged` status with a failure note.
2. The approval was valid — the failure is downstream at the git host.

**Recover:**

- A merge conflict is not an agent failure — re-base the branch (a human
  action) and re-request the `MergePullRequest` approval. The approval
  does not auto-retry a conflicted merge.
- Do not force-merge to resolve a conflict; that bypasses the human
  judgment the approval represents.

#### F. Replay quarantine violation

Replay quarantine refuses live connector / git-host surfaces during a
Substitute-mode replay.

**Diagnose:**

1. `RuntimeError::QuarantineViolation { surface, detail }` is logged with
   the calling span's `replay_key`.
2. Almost always a missing `@replayable` on a downstream call or a
   non-deterministic input not captured in the replay key.

**Recover:**

- Make the agent replayable and re-run.
- Until fixed, do not promote any fixture exercising that path.

#### G. Flaky CI producing false-positive risk labels

`hourly_ci_signal_scan` labeled a batch of issues high-severity from a
flaky CI job that fails intermittently.

**Diagnose:**

1. Inspect the `code_risk_labels` whose `ci_signal_id` points at the
   same `failing_job`.
2. Check the CI host: does the job fail intermittently on reruns? A
   flaky job produces `failed` signals that are not real regressions.

**Recover:**

- The labels are technically CI-grounded (the CI did fail), so this is
  not an ungrounded-label incident — it is a CI-quality problem. Sev-3.
- The labels do not write anything; they inform a human. The fix is to
  quarantine the flaky CI job upstream, not to hand-edit risk labels.
- If flaky jobs are common, raise the confidence floor or require N
  consecutive failures before a high-severity label — a triage-model
  change with its own eval.

#### H. Release rollback needed after a bad tag

A `TagRelease` was approved and published, then the release proved bad.

**Diagnose:**

1. The `code_releases` row records the tag and commit SHA with its
   `approval_id`.
2. A published tag is irreversible — downstreams may already have
   fetched it.

**Recover:**

- Do not delete the tag (downstreams may depend on it). Follow the
  repo's release-rollback runbook: publish a new patch release that
  reverts the bad change, or yank the release in the package registry if
  the ecosystem supports it.
- The agent's role ends at "tagged with approval"; the rollback is a
  human-driven forward operation. File a new `TagRelease` approval for
  the corrected release.
- Review why the bad release passed the `TagRelease` decision tree (§13)
  — was CI green on a stale base? Tighten the check.

---

## 11. Rollback procedures

### Rolling back the binary

```
flyctl releases list --app code-maintenance-agent
flyctl deploy --image registry.fly.io/code-maintenance-agent:<prior-tag>
```

or in Kubernetes:

```
kubectl rollout undo deployment/code-api -n code
kubectl rollout status deployment/code-api -n code
```

Verify by running the smoke (§3); the `corvid ops show` snapshot must
match the archived snapshot for the prior release.

### Rolling back a migration

Forward-only (§6). Rollback = restore DB from backup, roll back the
binary, fix-forward.

### Rolling back an approval contract

Approval contracts are typed and live in the binary; rolling back the
binary rolls back the contract. The contract's `version` field is
checked at every approval request and refuses mismatches. Roll the
binary, not the contract.

### Rolling back a connector mode switch

If `CORVID_CONNECTOR_MODE` was flipped from `mock` to `real` in error,
flip it back (re-read on every connector call, immediate). Then audit
`audit_events` for any `pr.merge` / `release.tag` / `comment.post` /
`pr.open` rows in the window the flag was live — a real-mode flip could
have written to a repository.

---

## 12. Connector mode operations

### Modes

The three connectors honor `CORVID_CONNECTOR_MODE`:

- `mock` (default) — deterministic fixtures, no network, no repository
  writes. Used by `corvid eval`, `corvid tour`, smoke suites.
- `real` — real provider calls. Requires valid git host + CI host tokens
  per tenant (§5).
- `record` — proxies to `real` but writes the raw response to a fixture
  under `target/recordings/`.
- `replay` — reads from `target/recordings/` instead of the provider.

### Switching modes

```
export CORVID_CONNECTOR_MODE=real
corvid ops show | jq '.connector_mode'   # must print "real"
```

Switching to `real` is a release-checklist event: it means the agent can
write to repositories — comment, open/merge PRs, tag releases. Confirm
git host + CI host credentials, re-verify the writes-require-approval
posture, and log the change in `corvid ops show` before serving traffic.

### Per-tenant connector tokens

```
corvid connectors token put --tenant=<id> --connector=repo --token-file=<path>
corvid connectors token put --tenant=<id> --connector=ci --token-file=<path>
corvid connectors token list --tenant=<id>
corvid connectors token revoke --tenant=<id> --connector=repo
```

Tokens are encrypted at rest with the connector-token key (§5). Revoking
is immediate — the next write fails closed. Scope the git-host token to
the minimum permissions (a GitHub App with only the needed scopes).

### When a connector token expires

The connector raises a typed error. The job retry policy
(`exponential_jitter`, 5 attempts) absorbs transient failures; a refresh
failure that survives all retries lands in the dead-letter queue. Mint a
new token and rerun; the replay key makes the rerun idempotent.

### CI signal freshness

The `ci_connector` polls CI hourly via `hourly_ci_signal_scan`. If a
critical regression lands mid-hour, an operator can force a scan rather
than wait for the next tick: `corvid jobs run
--kind=hourly_ci_signal_scan --tenant=<id> --window=business_hour`.
Risk labels are only as fresh as the last scan, so a stuck scanner means
stale risk labels (§10 incident D).

### CI signal lifecycle

The CI signal is the grounding evidence for every high-severity risk
label; treat it as a first-class operational input.

- **Source of truth.** CI runs live in the CI host (GitHub Actions,
  GitLab CI, Buildkite). The agent ingests a snapshot per scan and
  fingerprints the log. The `log_fingerprint` on `code_ci_signals` points
  back to the CI run, so an auditor can trace any high-severity label to
  the exact failing run that grounds it.
- **Freshness window.** A risk label is only as current as the CI signal
  that grounds it. The hourly scan means a label can be up to ~1 h behind
  the latest CI run; for fast-moving repos, consider a 15-minute scan in
  production.
- **Webhook ingestion.** If the CI host emits completion webhooks, route
  them to an ingestion handler that records a `code_ci_signals` row
  immediately rather than waiting for the next poll. This tightens the
  triage-to-evidence latency for critical regressions.
- **Flaky-signal handling.** A flaky CI job produces `failed` signals
  that are real (the CI did fail) but not regressions (§10 incident G).
  Operators can configure a confidence floor or an N-consecutive-failures
  rule before a high-severity label fires.

### Daily reconciliation cadence

Run a daily reconciliation (a cron outside the agent or a future durable
job) that:

1. Pulls the git host's merge + tag activity for the prior day.
2. Matches each merge/tag to an `approvals` row by idempotency key.
3. Emits a reconciliation report: matched, git-host-only (incident C),
   app-only (failed write to re-run).

The reconciliation report is a compliance artifact; archive it alongside
the `corvid ops show` snapshots. For a code agent, the git-host-only
case is the critical one — a merge or tag the git host shows but the app
DB did not authorize is an unauthorized write.

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
the approval is `pending` and not expired, and the approver is
same-tenant. It writes `approvals.decision = 'approved'` plus an
`audit_events` row (`event_kind = 'approval.approve'`) and a trace span.

### Per-contract considerations

The flow is the developer's choice; each contract reflects an explicit
decision:

- **PostReviewComment** — Reviewer, $0.05, reversible. Confirm the
  comment is constructive and references the issue; a comment can be
  deleted if wrong.
- **CreatePatchProposal** — Reviewer, $0.05, reversible. Confirm the
  patch fingerprint matches the reviewed diff; a proposal is not a
  merge.
- **OpenPullRequest** — Reviewer, $0.05, reversible. Confirm the branch
  and patch; a PR can be closed.
- **MergePullRequest** — Admin, $0.25, irreversible. Confirm CI is green
  on the PR head, required reviews are in, and the merge strategy is
  correct. Merging to mainline cannot be cleanly undone.
- **TagRelease** — Admin, $0.25, irreversible. Confirm the commit SHA is
  the intended release point and the tag follows the versioning scheme.
  A published tag is consumed by downstreams immediately.

### Decision tree — `MergePullRequest`

1. Is the requester an `Admin`? No → deny.
2. Is CI green on the PR head (not the stale base)? No → deny; the agent
   does not merge a red PR.
3. Are the required human reviews in place? No → deny.
4. Is the merge strategy (squash / merge / rebase) the repo's policy?
   No → deny.
5. Is the approver different from the requester (segregation of duties)?
   No → deny.
6. All yes → approve. The merge runs through the durable-job pool with
   the approval id attached.

### Decision tree — `TagRelease`

1. Is the requester an `Admin`? No → deny.
2. Is the commit SHA an ancestor of the release branch tip? No → deny.
3. Does the tag follow the versioning scheme and not collide with an
   existing tag? No → deny.
4. Is the approver different from the requester? No → deny.
5. All yes → approve. A tag is irreversible once downstreams fetch it.

### Decision tree — `OpenPullRequest`

1. Is the requester a `Reviewer` (or `Admin`)? No → deny.
2. Does the branch fingerprint match a real branch from an approved
   patch? No → deny.
3. Is the target repo the requester's tenant's repo? No → deny.
4. All yes → approve. A PR is reversible (close it), so the bar is lower.

### Segregation of duties

For the two Admin mainline contracts (`MergePullRequest`, `TagRelease`),
the requester and approver should differ. The periodic audit query flags
violations:

```
SELECT id, contract_action FROM approvals
WHERE requester_actor_id = decided_by_actor_id
  AND decision = 'approved'
  AND required_role = 'Admin';
```

Any row returned is a segregation-of-duties violation and a Sev-2
compliance finding — especially important for merges and releases.

### Pending queue SLOs

- `PostReviewComment` / `CreatePatchProposal` / `OpenPullRequest` —
  pending > 4 h is a Sev-3 page.
- `MergePullRequest` — pending > 1 h is a Sev-3 page (a stale PR may
  drift from CI-green).
- `TagRelease` — pending > 2 h is a Sev-3 page.

---

## 14. Tenant lifecycle operations

The Code Maintenance agent is multi-tenant: every repo, issue, CI
signal, risk label, approval, and audit event carries a `tenant_id`.
Onboarding and offboarding touch the most tables at once.

### Onboarding a tenant

1. **Create the tenant row.** `corvid tenants create --id=<id>
   --name=<display>`. Foreign-key anchor for everything else.
2. **Create roles and the first admin.** Every tenant needs at least one
   `Admin` (merge/tag approvals) and one `Reviewer` (comment/patch/PR).
   `corvid auth role grant --tenant=<id> --actor=<actor> --role=Admin`.
3. **Register the repo + CI connectors** (real mode) or rely on mock
   fixtures (default). Scope the git-host token to the minimum.
4. **Run the first repo reindex.** `corvid jobs run
   --kind=nightly_repo_reindex --tenant=<id> --repo=<repo>` to populate
   `code_repositories` and tree hashes.
5. **Confirm CI grounding.** `GET /issues/triage` for the tenant returns
   a label grounded in a `CiSignal` before serving real triage.

### Offboarding a tenant

Offboarding is a hard delete gated by a legal-hold check and the
retention requirement on the audit trail.

1. **Check for a legal hold.** Active hold → STOP.
2. **Revoke all sessions and API keys.** `corvid auth revoke-all
   --tenant=<id>`.
3. **Disable the tenant's schedules** so no job re-creates rows
   mid-delete.
4. **Export if contractually required** — final repo/audit export.
5. **Hard delete the operational data.** `corvid tenants delete
   --tenant=<id> --confirm` cascades through `code_*`, `sessions`,
   `api_keys`, `user_roles`.
6. **Retain the audit trail.** `approvals` + `audit_events` + the
   operation tables (incl. `code_merges`, `code_releases`) are NOT
   deleted — retained for the full window subject to legal hold. The
   delete tombstones the tenant row but preserves the merge/release
   history.

### Verifying tenant isolation

```
corvid tenants verify-isolation --tenant=<id>
```

Asserts no `code_*` row for tenant A references a parent row owned by
tenant B, and that no PR, merge, or release crosses a tenant boundary.
A failure is a Sev-1 cross-tenant code-access leak.

---

## 15. Durable jobs and cron operations

### The three jobs

| Kind | Cron | Tenant scope | Effects | Approval | Budget |
|---|---|---|---|---|---|
| `hourly_ci_signal_scan` | `0 * * * *` America/New_York (hourly) | per tenant per window | `ci_read` | none | $0.50 |
| `nightly_repo_reindex` | `0 2 * * *` America/New_York | per tenant per repo | `repo_read` | none | $0.50 |
| `weekly_stale_issue_sweep` | `0 6 * * 1` America/New_York | per tenant per window | `repo_read` | none | $0.50 |

None of the three carries a code-write effect — they scan CI, reindex
repos, and flag stale issues. Comments, patches, PRs, merges, and tags
only happen on the typed `POST` write routes, which are approval-gated by
construction. The scheduler can wake the agent up, but it can never merge
a PR or tag a release.

### Job SLOs

- `hourly_ci_signal_scan` p99: 2 min for a tenant with up to ~100 active
  repos. This is the time-sensitive job — a miss means risk labels lag
  CI, so its failure alert is Sev-2.
- `nightly_repo_reindex` p99: 5 min per tenant for ~100 repos.
- `weekly_stale_issue_sweep` p99: 5 min per tenant.

### Manual triggers

```
corvid jobs run --kind=hourly_ci_signal_scan --tenant=tenant-1 --window=business_hour
corvid jobs run --kind=nightly_repo_reindex --tenant=tenant-1 --repo=org/app
corvid jobs run --kind=weekly_stale_issue_sweep --tenant=tenant-1 --window=business_week
```

The `replay_key` is `kind:tenant:scope` and is the durable-job
idempotency key. Two manual triggers with the same arguments coalesce
into one queued job.

### Retry policy

All three jobs use `exponential_jitter`, 5 attempts, base 1 s, cap 10
min, dead-letter `code_maintenance_agent.dead_letter`. Dead-lettered
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

### Why no job merges or tags

It is worth stating plainly: the three durable jobs scan/reindex/flag
only. There is deliberately no "auto-merge" or "auto-release" job. Any
automation that wrote to a repository would have to call a `dangerous`
write tool, which the compiler refuses outside an `approve` boundary —
and that boundary requires a human decision. So the scheduler can label
risk, but it can never merge a PR or publish a release.

---

## 16. Disaster recovery

### Catastrophic DB loss

1. **Stop traffic.** Take the deploy out of the load balancer.
2. **Restore the latest backup** to a new DB instance (§7). Verify with
   `corvid migrate --check`.
3. **Reconcile executed writes.** Read `audit_events` from the backup for
   `pr.merge` / `release.tag` / `comment.post` / `pr.open` rows;
   cross-check against the git host's own record for the data-loss window
   (15 min). The git host is the source of truth for what actually landed
   in a repository; any git-host action the restored DB does not record
   must be re-recorded by hand with a clear note.
4. **Point the deploy at the restored DB** and roll.
5. **Run the smoke suite (§3) + the eval suite.**
6. **Rebuild repo tree hashes** via `nightly_repo_reindex` for every
   tenant (the index is rebuildable from the git host).
7. **Write the post-incident report** for the compliance trail.

### Audit redundancy

For regulated tenants, enable the audit-event forwarder writing a copy of
every `audit_events` row to an append-only object-storage log under
`s3://<bucket>/audit/<tenant>/<date>.jsonl`. Recommended for the
code-maintenance audit trail (it records every merge and release).

### Git-host divergence

If the app DB and the git host disagree about merges/releases, **the git
host wins for what actually landed in a repository**; the app DB wins for
intent and approval. Reconcile by matching each git-host merge/tag to an
`approvals` row by `approval_id`; a git-host merge/tag without a matching
approved row is incident C (unauthorized write).

### Loss of the connector-token key

The connector-token key encrypts git-host + CI-host tokens. If lost,
those tokens are unrecoverable but the DB and audit trail are intact.
Recovery: rotate the key (§5), force every tenant to re-mint connector
tokens. Real-mode writes are paused until done.

### RPO / RTO targets

- **RPO**: 15 min (WAL ship interval).
- **RTO**: 1 h for a regional DB failure (cross-region replica promote);
  4 h for catastrophic DB loss (restore + write reconciliation + repo
  reindex).

---

## 17. Appendix — reference data

### Schema manifest

`CodeSchemaManifest("code_maintenance_agent", 5, 21, 3, 3, 5, "mock",
true)`:

- 5 migrations: `0001_ingestion_triage`, `0002_writes_approvals`,
  `0003_auth`, `0004_approvals_and_durable_jobs`, `0005_code_operations`.
- 21 tables: see §2.
- 3 connectors: `repo_connector`, `ci_connector`, `code_ai_connector`.
- 3 durable jobs: `hourly_ci_signal_scan`, `nightly_repo_reindex`,
  `weekly_stale_issue_sweep`.
- 5 approval contracts: `PostReviewComment`, `CreatePatchProposal`,
  `OpenPullRequest`, `MergePullRequest`, `TagRelease`.
- Default mode: `mock`. Writes require approval: `true`.

### Capacity planning

Per tenant unless noted. Code Maintenance is sensitive to repo count and
CI volume.

| Tenant size | Repos | CI runs/day | `hourly_ci_signal_scan` | `nightly_repo_reindex` | Action |
|---|---|---|---|---|---|
| Small | < 10 | < 100 | < 30 s | < 1 min | Single replica, `shared-cpu-1x` |
| Medium | 10 – 100 | 100 – 2k | < 2 min | < 5 min | Default; 1-3 replicas |
| Large | 100 – 500 | 2k – 20k | 2 – 5 min | 5 – 15 min | Postgres read replica for the triage route |
| XL | > 500 | > 20k | multi-step | > 15 min | Shard the CI scan by repo group; move triage to its own worker pool |

Other limits:

- **Approval throughput** — DB-backed; the bottleneck is human review
  latency, not the queue. Watch `code_approval_pending_age_seconds`,
  especially for `MergePullRequest`.
- **DB sizing** — `code_approval_audits` + `audit_events` + the
  operation tables grow append-only and are retained for the engineering
  -audit window. Budget storage for the full window.
- **CI scan** — read-bound on the CI host. Stays under SLO up to ~20k CI
  runs/day; past that, partition the scan by repo group.

### Compliance posture

- **CI grounding.** A high-severity risk label is grounded in a failed
  CI signal; the triage contract fails without it. An ungrounded label
  (§10 incident B) is a quality + trust issue routed to the eng lead.
- **Audit immutability.** `approvals`, `audit_events`, and the operation
  tables (incl. `code_merges`, `code_releases`) are append-only and
  retained for the engineering-audit window. Offboarding tombstones the
  tenant but never deletes the merge/release history (§14).
- **Segregation of duties.** Requester ≠ approver for the Admin mainline
  contracts (merge, release) (§13).
- **No autonomous repository writes.** The three cron jobs cannot merge
  or tag — every repo write requires a human approval the compiler
  enforces (§15).
- **No raw source committed.** The DB and fixtures hold fingerprints
  (patch, comment, tree, log), not proprietary code.

### Role → permission mapping (reference)

| Role | Can approve | Cannot approve |
|---|---|---|
| `Reviewer` | `PostReviewComment`, `CreatePatchProposal`, `OpenPullRequest` | merges, release tags |
| `Admin` | all five (incl. `MergePullRequest`, `TagRelease`) | — |

An `Admin` can approve everything a `Reviewer` can; the mainline
contracts (merge, release) are Admin-only. The auth surface propagates
one typed permission string per dangerous tool
(`code.tool.post_review_comment`, `code.tool.create_patch_proposal`,
`code.tool.open_pull_request`, `code.tool.merge_pull_request`,
`code.tool.tag_release`) through the `Actor` permissions, so a
permission check can gate each tool independently of role — a finer
grain than the role gate alone. The five strings are distinct by
construction; an audit can confirm no two tools share a permission.

### Risk severity reference

The triage model assigns a severity grounded in the CI signal and other
evidence. Operators tune these thresholds per tenant; the demo assumes:

| Severity | CI evidence | Confidence floor | Typical response |
|---|---|---|---|
| `high` | a failing CI job on the issue's commit | ≥ 0.80 | draft a patch + open a PR (approval-gated) |
| `medium` | a failing CI job on a related path | ≥ 0.70 | draft a review comment (approval-gated) |
| `low` | no CI failure, heuristic signal only | ≥ 0.60 | flag for human triage; no write drafted |

A `high` label without a failing CI job is the ungrounded-label incident
(§10 incident B) and must fail `code_triage_valid` rather than ship. The
`low` tier never drafts a write — it only flags — because there is no CI
evidence to ground a repository change.

### Effect catalog

| Effect | Cost | Trust | Data class | Used by |
|---|---|---|---|---|
| `repo_read` | $0.01 | workspace | code | triage, repo reindex, stale sweep |
| `ci_read` | $0.01 | workspace | code | triage, CI signal scan |
| `code_ai` | $0.08 | bounded | code | triage, write plan |
| `code_write` | $0.02 | human_required | code | `post_review_comment`, `create_patch_proposal` |
| `pr_write` | $0.02 | human_required | code | `open_pull_request` |
| `merge_write` | $0.03 | human_required | code | `merge_pull_request` |
| `release_write` | $0.03 | human_required | code | `tag_release` |

### Route catalog

| Method | Route | Returns | Effects | Approval |
|---|---|---|---|---|
| GET | `/config` | `CodeConfig` | none | none |
| GET | `/schema` | `CodeSchemaManifest` | none | none |
| GET | `/repos/ingest/mock` | `RepoSnapshot` | `repo_read` | none |
| GET | `/issues/triage/mock` | `CodeRiskLabel` | `repo_read`, `ci_read`, `code_ai` | none |
| GET | `/writes/plan/mock` | `CodeWritePlan` | `repo_read`, `ci_read`, `code_ai` | none |
| POST | `/comments/post` | `ReviewCommentReceipt` | `code_write` | `PostReviewComment` |
| POST | `/patches/propose` | `PatchProposalReceipt` | `code_write` | `CreatePatchProposal` |
| POST | `/pull-requests/open` | `OpenPullRequestReceipt` | `pr_write` | `OpenPullRequest` |
| POST | `/pull-requests/merge` | `MergePullRequestReceipt` | `merge_write` | `MergePullRequest` |
| POST | `/releases/tag` | `TagReleaseReceipt` | `release_write` | `TagRelease` |
| POST | `/auth/session/login` | `LoginResponse` | none | none |
| POST | `/auth/api-key/login` | `ApiKeyLoginResponse` | none | none |
| GET | `/auth/status` | `AuthStatusResponse` | none | none |
| GET | `/auth/api-key/status` | `AuthStatusResponse` | none | none |
| GET | `/jobs/hourly-ci-signal-scan/mock` | `CodeJobRun` | `ci_read` | none |
| GET | `/jobs/nightly-repo-reindex/mock` | `CodeJobRun` | `repo_read` | none |
| GET | `/jobs/weekly-stale-issue-sweep/mock` | `CodeJobRun` | `repo_read` | none |

### Adversarial corpus

Six named threats under [`adversarial/`](../adversarial/):

- `ungated_post_review_comment.cor` — calls `post_review_comment` without
  `approve PostReviewComment(...)`.
- `ungated_create_patch_proposal.cor` — calls `create_patch_proposal`
  without `approve CreatePatchProposal(...)`.
- `ungated_open_pull_request.cor` — calls `open_pull_request` without
  `approve OpenPullRequest(...)`.
- `ungated_merge_pull_request.cor` — calls `merge_pull_request` without
  `approve MergePullRequest(...)`.
- `ungated_tag_release.cor` — calls `tag_release` without
  `approve TagRelease(...)`.
- `raw_patch_committed.json` — the declarative no-raw-code threat: a
  patch/comment must be committed as a fingerprint, not raw source.

The five `.cor` fixtures are refused by `corvid check` with `E0101`. Any
green build on these is a Sev-1 — the approval gate is the foundation of
the agent's no-autonomous-write claim.

### Approval contract reference

| Label | Role | Ceiling | Reversible | Reason |
|---|---|---|---|---|
| `PostReviewComment` | Reviewer | $0.05 | yes | delete comment |
| `CreatePatchProposal` | Reviewer | $0.05 | yes | proposal, not merge |
| `OpenPullRequest` | Reviewer | $0.05 | yes | close PR |
| `MergePullRequest` | Admin | $0.25 | no | changes mainline |
| `TagRelease` | Admin | $0.25 | no | published tag |

### Worked example: a failing CI run to a merged fix

End to end, the way the surfaces compose for a real regression:

1. `hourly_ci_signal_scan` polls CI and records a `code_ci_signals` row:
   `status=failed`, `failing_job=unit-tests`, fingerprinted log.
2. The next `GET /issues/triage` for the affected issue grounds a
   `CodeRiskLabel` in that failed signal: `severity=high`,
   `confidence=0.87`, `replay_key=code:triage:issue-1`. The label is
   CI-grounded (the triage contract required the failed signal).
3. `GET /writes/plan/mock` drafts a `CodeWritePlan`: a review-comment
   fingerprint and a patch fingerprint, `writes_gated=true`. Nothing has
   touched the repo.
4. A `Reviewer` approves `CreatePatchProposal` and `OpenPullRequest`; the
   patch becomes a PR. Each write went through its `approve` boundary and
   wrote a `code_pull_requests` row + `audit_events` entries.
5. CI goes green on the PR head. An `Admin` works the
   `MergePullRequest` decision tree (§13): Admin role, CI green on the
   head, required reviews in, correct merge strategy, approver ≠
   requester. They approve.
6. The merge runs through the durable-job pool with the approval id
   attached, writing a `code_merges` row (`event_kind=pr.merge`) and the
   merge commit SHA. The mainline changed — irreversibly — but only
   after a human's explicit, audited decision.

At no point did automation merge or tag on its own: the scheduler
produced evidence (the CI signal), the model drafted a proposal, and a
human took each irreversible step behind a typed approval the compiler
enforced. That is the whole posture in one flow.

### Promoted eval fixtures

Three promoted fixtures under [`evals/promoted/`](../evals/promoted/):

- `code-demo.lineage-eval.json` — CI-aware triage.
- `code-ci-scan.lineage-eval.json` — `hourly_ci_signal_scan` durable job
  + CI read.
- `code-merge-pr.lineage-eval.json` — merge route + `MergePullRequest`
  approval (pending_review) + audit.

### Environment variable reference

| Variable | Default | Purpose |
|---|---|---|
| `CORVID_APP_ENV` | `local` | Environment (local / staging / production) |
| `CORVID_CONNECTOR_MODE` | `mock` | Connector mode (mock / replay / real / record) |
| `CORVID_REQUIRE_APPROVALS` | `true` | If true, every dangerous tool fails closed without approval |
| `CORVID_DATABASE_URL` | `sqlite:target/code.db` | DB connection string |
| `CORVID_CONNECTOR_TOKEN_KEY` | — | AES-256 key for connector-token encryption |
| `CORVID_API_KEY_PEPPER` | — | Argon2id pepper for API-key hashing |
| `CORVID_SESSION_SIGNING_KEY` | — | Session signing key (30-day rotation) |
| `CORVID_CSRF_SECRET` | — | CSRF double-submit secret |
| `CORVID_OTLP_ENDPOINT` | — | OTLP exporter target |
| `CORVID_METRICS_LISTEN` | `0.0.0.0:9090` | Prometheus `/metrics` bind |
| `CORVID_TRACE_DIR` | `target/traces` | Trace JSONL output directory |
| `RUST_LOG` | `info` | Log filter |

### Glossary

- **CI-grounded triage** — a risk label whose severity is justified by a
  `CiSignal`; a high-severity label requires `ci.status == "failed"`.
- **`CodeRiskLabel`** — the triage output: issue + CI signal + category +
  severity + confidence + replay key.
- **`CodeWritePlan`** — the drafted proposal (review-comment + patch
  fingerprints) before any approval; `writes_gated` marks that the
  writes need approval.
- **Dangerous tool** — a tool with a `human_required` write effect; the
  compiler refuses to call it outside an `approve <Label>` boundary.
- **Reversible vs irreversible** — comment/patch/PR are reversible
  proposals (delete / close); merge/tag change the mainline or publish
  and are irreversible.
- **Replay key** — the durable-job idempotency key, `kind:tenant:scope`;
  also what the replay quarantine uses to locate a per-job trace.
- **Mock / replay / real / record** — the four connector modes; `mock`
  is the default and the only one that runs offline with deterministic
  fixtures.
- **Segregation of duties** — the requester of a merge/tag approval must
  differ from its approver; enforced by audit query, not by the runtime.
- **Approval contract** — a typed, developer-authored declaration of who
  may approve a write (role), at what cost ceiling, whether it is
  irreversible, and when it expires. Versioned; a binary refuses an
  approval issued against a mismatched contract version.
- **`corvid ops show`** — the signed, dated runtime snapshot (schema
  version, migration hashes, connector mode, manifest counts); archive
  one per release as the audit/compliance baseline.
- **Dead-letter queue** — `code_maintenance_agent.dead_letter`, where a
  job lands after exhausting its 5 retries; inspect with `corvid jobs
  list --dead-letter` and replay from the last checkpoint.
- **Replay quarantine** — the runtime guarantee that a Substitute-mode
  replay never reaches a live connector / git host; a breach raises
  `RuntimeError::QuarantineViolation`.

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
