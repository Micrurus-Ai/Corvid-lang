# Personal Executive Agent — Operator Runbook

This runbook is the operational source of truth for running the Personal
Executive Agent backend in development, staging, and production. The PEA is
a reference Corvid application: it triages inbox threads, drafts replies,
schedules and prepares meetings, generates daily briefs, extracts tasks,
tracks follow-ups, and gates every external write behind a typed approval
contract.

Every procedure below is grounded in surfaces the app actually ships. The
schema manifest at [`src/main.cor`](../src/main.cor) declares the canonical
counts (5 migrations / 12 tables / 5 connectors / 4 durable jobs / 5
approval contracts) and `corvid run --target=server` exposes the routes
that drive each procedure.

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
14. [Durable jobs and cron operations](#14-durable-jobs-and-cron-operations)
15. [Disaster recovery](#15-disaster-recovery)
16. [Appendix — reference data](#16-appendix--reference-data)

---

## 1. Service overview

### What the Personal Executive Agent does

The Personal Executive Agent runs four durable jobs on a workday cron and
exposes a typed HTTP surface for every action a human executive's assistant
takes on their behalf:

- `daily_brief_job` (cron `0 7 * * 1-5` America/New_York) — prepares a
  morning brief covering high-priority inbox threads, the day's meetings,
  open follow-ups, and outstanding tasks.
- `meeting_prep_job` (cron `0 6 * * 1-5` America/New_York) — gathers
  context packets for upcoming calendar events: prior threads, relevant
  files, attendee summaries.
- `email_triage_job` (cron `*/10 8-18 * * 1-5` America/New_York) — classifies
  every new inbox thread with urgency, reply-needed, and task-extraction
  signals at a confidence floor of 0.80.
- `follow_up_job` (cron `*/15 8-18 * * 1-5` America/New_York) — drives the
  follow-up reminder loop and renders a draft outbound reply that is
  approval-gated before any send.

Every external write enters one of four approval contracts:

| Approval label | Tool | Effect | Required role | Cost ceiling |
|---|---|---|---|---|
| `SendFollowUpEmail` | `send_follow_up_email` | `send_email` | Reviewer | $0.25 |
| `EditCalendarEvent` | `edit_calendar_event` | `calendar_write` | Reviewer | $0.25 |
| `EditTaskItem` | `edit_task_item` | `task_write` | Reviewer | $0.10 |
| `SendChatMessage` | `send_chat_message` | `chat_write` | Reviewer | $0.10 |

A fifth approval contract is filed (per the maturity bar) for the
`ExternalCalendarShare` flow and ships in the same maturity track.

### What the Personal Executive Agent does not do

- Send email, edit calendars, write tasks, or post chat messages without
  human approval.
- Talk to real provider APIs in the default mode. The default
  `CORVID_CONNECTOR_MODE=mock` keeps every connector in offline mode with
  deterministic mock fixtures.
- Store passwords or raw provider tokens in process memory beyond the
  scope of a single connector call. Tokens live in the encrypted
  connector-token store managed by the runtime.
- Make autonomous decisions in finance, legal, or regulated domains. Those
  are explicit non-goals; the security model
  [`security-model.md`](../security-model.md) names them.

---

## 2. Architecture map

### Process layout

The PEA runs as a single Corvid server binary plus a SQLite or Postgres
backing store. The binary is built from `src/main.cor` via
`corvid build --target=server`. In production the binary is wrapped in a
distroless OCI image; the same binary serves all HTTP routes, runs the
durable-job pool, the scheduler, the OTLP exporter, and the metrics
endpoint.

```
+---------------------------+
|   corvid jobs run         |
|   (durable job pool,      |  <-- handles cron + on-demand jobs,
|    N tokio workers)       |      respects budget + retry policies
+------------+--------------+
             |
             | lease + execute via Runtime
             v
+---------------------------+      +-----------------------------+
|   corvid run server       |<---->|   SQLite or Postgres store  |
|   (HTTP routes from       |      |   - queue_jobs              |
|    main.cor)              |      |   - queue_job_checkpoints   |
+------+--------------------+      |   - approvals               |
       |                           |   - audit_events            |
       | trace events              |   - sessions / api_keys     |
       v                           |   - tenants / users         |
+---------------------------+      |   - connector_accounts      |
|   trace store + OTLP      |      |   - work_items / drafts     |
|   exporter                |      |   - calendar / briefs       |
+---------------------------+      +-----------------------------+
       |
       | Prometheus exposition
       v
+---------------------------+
|   /metrics (text)         |
+---------------------------+
```

### Routes

The HTTP surface declares 24 routes covering schema introspection, demo
flows, auth, approvals, and approval-gated execute actions. The full set is
in [`src/main.cor`](../src/main.cor); the operational subset operators care
about is:

- `GET /schema` — manifest with table/connector/job/approval counts. First
  health check after deployment.
- `GET /schema/tenant`, `GET /schema/user`, `GET /schema/inbox-thread`,
  `GET /schema/draft-reply`, `GET /schema/calendar-event`,
  `GET /schema/meeting-prep`, `GET /schema/daily-brief`,
  `GET /schema/task`, `GET /schema/follow-up` — typed schema probes; used
  by the deployment smoke test.
- `GET /inbox/triage/mock`, `GET /drafts/reply/mock`,
  `GET /calendar/schedule/mock`, `GET /jobs/durable/mock`,
  `GET /brief/daily/mock`, `GET /meeting-prep/mock`, `GET /follow-ups/mock`
  — deterministic mock flows. Used in CI integration tests + the live
  drift narrator.
- `POST /auth/session/login`, `POST /auth/api-key/login`,
  `GET /auth/status`, `GET /auth/api-key/status` — auth surface.
- `GET /approvals/follow-up`, `GET /approvals/surface` — approval queue
  surface for the reviewer UI.
- `POST /actions/follow-up/send`, `POST /actions/calendar/edit`,
  `POST /actions/task/edit`, `POST /actions/chat/send` — the four
  approval-gated execute actions. Every POST goes through an `approve`
  statement in source; the compiler rejected the binary when the gate was
  missing.

### Durable jobs

Four durable jobs run as the workday operational core. Each is
`@replayable`, carries a typed `ExecutiveJobContract`, declares a
deterministic budget ceiling, exponential-jitter retry with cap, and emits
a stable `replay_key`:

| Job | Cron | Budget | Max attempts | Effects | Approval-gated? |
|---|---|---|---|---|---|
| `daily_brief_job` | `0 7 * * 1-5` | $0.75 | 5 | `inbox_read`, `calendar_read`, `executive_llm` | No (read-only) |
| `meeting_prep_job` | `0 6 * * 1-5` | $0.75 | 5 | `inbox_read`, `calendar_read`, `executive_llm` | No (read-only) |
| `email_triage_job` | `*/10 8-18 * * 1-5` | $0.75 | 5 | `inbox_read`, `executive_llm`, `task_write` | No (task writes are internal) |
| `follow_up_job` | `*/15 8-18 * * 1-5` | $0.75 | 5 | `inbox_read`, `executive_llm`, `send_email`, `task_write` | Yes (`SendExecutiveFollowUp`) |

The follow-up job is the only one that *requires* an approval. Every
`@replayable` job in the PEA goes through the cross-layer replay
quarantine — see [`docs/phases/phase-38-replay-quarantine.md`](../../../../docs/phases/phase-38-replay-quarantine.md)
for the full quarantine surface.

### Connectors

Five mock connectors are wired by default (`email`, `calendar`, `tasks`,
`chat`, `files`). Each is declared in
[`connectors/mock_manifest.json`](../connectors/mock_manifest.json) with
`approval_required: true` and `replay_policy: quarantine_writes`.

Real provider mode binds:
- `email` → Gmail / Google Workspace
- `calendar` → Google Calendar (Outlook via M365 is filed)
- `tasks` → Linear (GitHub Issues is filed)
- `chat` → Slack
- `files` → local filesystem index (S3 / GCS post-v1.0)

### Tables (12)

The five migrations under
[`migrations/`](../migrations/) bring up 12 tables:

- `0001_identity.sql` — `tenants`, `users`, `roles`, `user_roles`
- `0002_connector_accounts.sql` — `connector_accounts`, `connector_tokens`
- `0003_work_items.sql` — `work_items`, `drafts`
- `0004_calendar_and_briefs.sql` — `calendar_events`, `meeting_prep_packets`,
  `daily_briefs`
- `0005_approvals_jobs_traces.sql` — `approvals`, `audit_events`,
  `queue_jobs`, `queue_job_checkpoints`, `trace_lineage`

(Some tables span multiple migration files; the count of 12 reflects the
schema manifest assertion in `executive_schema_manifest()`.)

---

## 3. Setup — local development

### Prerequisites

- Rust 1.83+ toolchain (matches the workspace `rust-toolchain.toml`).
- SQLite 3.40+ (`rusqlite` ships its own bundled SQLite; system one is
  used only for `sqlite3` interactive inspection).
- The `corvid` CLI built from this repo:
  ```bash
  cargo install --path crates/corvid-cli
  ```
  Or run via `cargo run -q -p corvid-cli --`.

No provider credentials are needed for local development — the default
`CORVID_CONNECTOR_MODE=mock` keeps every connector offline.

### Boot from a fresh clone

```bash
cd examples/backend/personal_executive_agent

# Stage the local env. The example writes to ./local-pea/ by default.
cp deploy/env.example .env.local
export $(grep -v '^#' .env.local | xargs)

# 1. Type-check the source against the workspace stdlib.
corvid check src/main.cor

# 2. Apply migrations to a fresh SQLite database.
mkdir -p data
corvid migrate up --dir migrations --state data/pea.sqlite

# 3. Load seed fixtures so the demo flows produce content.
sqlite3 data/pea.sqlite < seeds/demo.sql

# 4. Run the typed evals.
corvid eval evals/hardening_eval.cor

# 5. Boot the server.
corvid run --target=server src/main.cor

# 6. Smoke check in another shell.
curl http://127.0.0.1:8080/schema
curl http://127.0.0.1:8080/auth/status
curl http://127.0.0.1:8080/jobs/durable/mock
```

Expected: `GET /schema` returns
`{"service":"personal_executive_agent","migration_count":5,"table_count":12,"connector_count":5,"job_count":4,"approval_count":5,"default_mode":"mock"}`.
If anything differs from those counts, stop and inspect — the schema
manifest is a contract surface, not a status report.

### Workday simulation

To exercise the four durable jobs without waiting on cron, enqueue one of
each:

```bash
corvid jobs enqueue --state data/pea.sqlite --task daily_brief \
  --payload '["active_users", "business_day"]' --max-retries 5 \
  --budget-usd 0.75 --effect-summary "executive.daily_brief" \
  --replay-key "executive:daily_brief:active_users:business_day"

corvid jobs enqueue --state data/pea.sqlite --task email_triage \
  --payload '["active_users", "workday_window"]' --max-retries 5 \
  --budget-usd 0.75 --effect-summary "executive.email_triage" \
  --replay-key "executive:email_triage:active_users:workday_window"

corvid jobs run --source src/main.cor --state data/pea.sqlite \
  --workers 2 --lease-ttl-ms 60000 --max-runtime-ms 30000
```

Inspect outcomes:

```bash
corvid jobs inspect --state data/pea.sqlite <job-id>
corvid jobs explain --state data/pea.sqlite <job-id>
```

### Live drift narrator (operator-side connector check)

```bash
# Compare the shipped manifest against a recorded baseline.
corvid connectors check \
  --baseline connectors/mock_manifest.json \
  --observed connectors/mock_manifest.json --narrate
```

Returns zero drift. Use this when bumping connector versions or rotating
provider credentials to confirm the manifest still matches the wire shape.

### Adversarial corpus

Five named threats live under [`adversarial/`](../adversarial/):

- `missing_replay_key.json` — proves a job cannot be enqueued without a
  stable replay key.
- `provider_secret_in_source.json` — proves source-embedded secrets are
  refused at typecheck.
- `real_connector_default.json` — proves `mode=real` cannot be the default
  for any connector in any reference app.
- `ungated_send.cor` — proves a `send_follow_up_email(...)` call without a
  preceding `approve SendFollowUpEmail(...)` does not compile.
- `unredacted_trace.json` — proves the redaction rules in the connector
  manifest strip message bodies from emitted traces.

Run any one of them against the app's typecheck:

```bash
corvid check adversarial/ungated_send.cor
# expects: error[ApprovalMissing] — cdylib build would refuse to sign
```

---

## 4. Setup — staging and production deployment

### Container build

The app ships a multi-stage Dockerfile in
[`deploy/Dockerfile`](../deploy/Dockerfile). Build:

```bash
cd examples/backend/personal_executive_agent
docker build -t pea:$(git rev-parse --short HEAD) -f deploy/Dockerfile ../../..
```

The build context is the repo root because the Dockerfile copies the
`corvid` CLI build plus the workspace stdlib. The final stage is
distroless and includes only:

- The `corvid` binary
- The compiled `libpersonal_executive_agent.so` cdylib (signed)
- The migrations directory
- The seeds directory
- The `mock_manifest.json` connector manifest

Image size budget: ≤80 MB. Inspect:

```bash
docker images pea --format '{{.Repository}}:{{.Tag}} {{.Size}}'
```

### Docker Compose (single-host staging)

[`deploy/docker-compose.yml`](../deploy/docker-compose.yml) starts:

- The PEA service on port 8080
- An OTLP collector receiving `corvid.*` spans on port 4317
- A Prometheus scraper polling `/metrics` every 15s
- A Grafana instance pre-provisioned with the PEA dashboard

Bring up:

```bash
docker compose -f deploy/docker-compose.yml up -d
```

Bring down without losing state:

```bash
docker compose -f deploy/docker-compose.yml stop
```

Bring down and wipe state:

```bash
docker compose -f deploy/docker-compose.yml down -v
```

### Kubernetes (production)

A reference Kubernetes manifest set ships in `deploy/k8s/` (planned). The
shape: one Deployment for the HTTP service, one Deployment for the worker
pool (different replica count, different resource limits), one CronJob
template per scheduled task (for environments that prefer K8s-native cron
over Corvid's scheduler), a Secret containing the encrypted-token key + 
provider keys, a ConfigMap for `CORVID_CONNECTOR_MODE` and feature flags,
and a Service + Ingress for the HTTP surface.

Smoke deploy from a clean cluster:

```bash
kubectl apply -f deploy/k8s/namespace.yaml
kubectl apply -f deploy/k8s/configmap.yaml
kubectl apply -f deploy/k8s/secret.example.yaml  # template; fill before applying
kubectl apply -f deploy/k8s/deployment-api.yaml
kubectl apply -f deploy/k8s/deployment-worker.yaml
kubectl apply -f deploy/k8s/service.yaml

kubectl -n pea wait --for=condition=available deployment/pea-api --timeout=120s
kubectl -n pea wait --for=condition=available deployment/pea-worker --timeout=120s
kubectl -n pea exec deploy/pea-api -- curl -sf http://localhost:8080/schema
```

### Fly.io / Render (managed PaaS)

For Fly.io, the `fly.toml` template in `deploy/fly.toml` (planned) sets:

- Two service groups: `api` (HTTP, autoscale 1-3) and `worker` (jobs, fixed
  count 1 in dev, 2-4 in prod).
- A shared volume mount at `/data/pea` for the SQLite store + traces.
- An internal DNS entry for the OTLP collector if running co-located.

Deploy:

```bash
fly deploy --config deploy/fly.toml
```

Render follows the same shape with a `render.yaml` blueprint. The contract
both PaaS targets satisfy is: two services backed by one binary, shared
state on a persistent volume, secrets injected via the platform.

### First production boot — sequence of operations

1. Apply migrations against the production database. **Do not let the
   service auto-migrate.** Migrations are operator actions; the binary
   refuses to start if the schema version on disk is older than the
   binary's expected version.
   ```bash
   corvid migrate up --dir migrations --state $POSTGRES_DSN
   corvid migrate status --state $POSTGRES_DSN
   ```
2. Seed reference data ONLY if this is a fresh tenant. Otherwise skip.
   ```bash
   psql $POSTGRES_DSN -f seeds/demo.sql
   ```
3. Start the API replica set.
4. Start the worker replica set with `--source src/main.cor` pointing at
   the deployed source. `corvid jobs run` refuses to start without a
   source path.
5. Confirm OTLP exporter is reaching the collector by tailing the
   collector logs for the first `corvid.guarantee_id` attribute.
6. Confirm `/metrics` is scraped by Prometheus by checking the
   `corvid_jobs_succeeded_total` counter incrementing after the first
   workday cron fires.
7. Run the smoke evals against the production-shaped instance:
   ```bash
   corvid eval evals/hardening_eval.cor --state $POSTGRES_DSN
   ```

---

## 5. Secrets management

### Inventory of secrets

The PEA holds five categories of secret. Each must be rotatable
independently; none ever appears in source, traces, or logs.

| Secret | Lives in | Used by | Rotation cadence |
|---|---|---|---|
| Connector OAuth refresh tokens (Gmail, Calendar, Slack, Linear) | Encrypted connector-token store at `CORVID_CONNECTOR_TOKEN_KEY` | All `*_write` and `send_*` connectors in real mode | Per-tenant, on token revocation or 30 days |
| API key hash salt + Argon2id parameters | `CORVID_API_KEY_PEPPER` | `api_key_login` and `api_key_auth_status` agents | 90 days (with key invalidation on rotate) |
| Session signing secret | `CORVID_SESSION_SIGNING_KEY` | `session_login`, `session_active`, `auth_trace_for_session` | 30 days |
| CSRF double-submit secret | `CORVID_CSRF_SECRET` | The double-submit middleware shipped in slice `35V2-P39-C-LR` | 30 days |
| Cdylib signing key (the build secret) | Hardware token or HSM, NOT the running process | `corvid build --sign=...` at release time | Per-release-key policy |

The cdylib signing key is intentionally not in the runtime's reach. The
running PEA process consumes signed binaries; it does not produce them.

### Secret storage in development

A `deploy/env.example` template enumerates every environment variable the
binary reads. Copy to `.env.local` and edit. **Do not commit** `.env.local`
to source control — `.gitignore` excludes it by convention.

```dotenv
# deploy/env.example (excerpt)
CORVID_CONNECTOR_MODE=mock
CORVID_CONNECTOR_TOKEN_KEY=base64:dev-only-do-not-use-in-prod
CORVID_API_KEY_PEPPER=dev-only-do-not-use-in-prod
CORVID_SESSION_SIGNING_KEY=dev-only-do-not-use-in-prod
CORVID_CSRF_SECRET=dev-only-do-not-use-in-prod
CORVID_OTLP_ENDPOINT=http://localhost:4317
CORVID_METRICS_LISTEN=:9090
```

`corvid doctor` validates that every required key is set and rejects the
literal string `dev-only-do-not-use-in-prod` in any environment other than
`CORVID_ENV=development`. Running `corvid doctor` should be the first
thing CI does in any production deployment.

### Secret storage in production

Recommended: a managed secret store (AWS Secrets Manager, GCP Secret
Manager, HashiCorp Vault, K8s Secrets with KMS encryption at rest). The
deployment manifest reads the secret at boot via init container or
operator pattern and exposes it to the binary only via the environment.

Forbidden:
- Embedding secrets in container images.
- Embedding secrets in ConfigMaps (K8s) without KMS encryption.
- Logging environment variables on startup. `corvid doctor` never prints
  secret values, only the boolean `present` per key.

### Rotation procedure: CORVID_CONNECTOR_TOKEN_KEY

The connector token key encrypts every OAuth refresh token in
`connector_tokens`. Rotating it requires re-encrypting the table.

```bash
# 1. Generate a new key.
openssl rand -base64 32 > /tmp/new_key

# 2. Mint a re-keying envelope.
corvid auth keys rotate --kind connector-token \
  --from $(cat /current/key) \
  --to $(cat /tmp/new_key) \
  --state $POSTGRES_DSN

# 3. Update the deployment env (or secret store).
# 4. Roll the deployment with --strategy=RollingUpdate.
# 5. Verify with corvid doctor in a sample pod:
kubectl exec deploy/pea-api -- corvid doctor
```

`corvid auth keys rotate` does not delete the old key until the
re-encryption transaction has committed for every row. If the transaction
fails mid-way, the rollback restores the old ciphertext under the old key,
and the new key is discarded.

### Rotation procedure: CORVID_SESSION_SIGNING_KEY

Session secrets rotate by minting a new key and accepting both keys for a
grace period (15 minutes by default; sessions are bounded at 60 minutes,
so a 15-minute grace forces all old sessions to re-authenticate within an
hour).

```bash
corvid auth keys rotate --kind session \
  --from $(cat /current/session_key) \
  --to $(openssl rand -base64 32) \
  --grace 15m \
  --state $POSTGRES_DSN
```

Verify by tailing the OTLP `session_replay_attempt` span — old sessions
present pre-rotate tokens, hit the grace window, and refresh transparently.

### Rotation procedure: CORVID_CSRF_SECRET

CSRF tokens are short-lived (single submission). Rotation is immediate:
mint a new secret, deploy, and the next request derives the double-submit
binding under the new HMAC key. Old tokens are rejected on the safe side.

```bash
NEW=$(openssl rand -base64 32)
kubectl set env deploy/pea-api CORVID_CSRF_SECRET=$NEW
kubectl set env deploy/pea-worker CORVID_CSRF_SECRET=$NEW
kubectl rollout restart deploy/pea-api deploy/pea-worker
```

---

## 6. Migrations — apply, drift, rollback

### Apply forward

```bash
corvid migrate up --dir migrations --state $POSTGRES_DSN
```

Behavior:
- Migrations apply in lexicographic order (`0001_*` before `0002_*` …).
- Each migration runs inside a single transaction. On failure the
  transaction rolls back and the partial schema is gone.
- After each successful migration, the `corvid_migrations` bookkeeping
  table records the SHA-256 of the migration file content, the timestamp
  applied, and the operator user from `$USER`.

If a migration fails mid-flight, the runtime prints the failing
migration's path and the underlying SQL error. The schema is left at the
last successfully-applied migration; the partial migration is not
recorded.

### Status

```bash
corvid migrate status --dir migrations --state $POSTGRES_DSN
```

Prints one line per migration with `applied | pending | drifted`:

```
0001_identity.sql                  applied  sha256:7a3f...  applied:2026-04-29T07:00:12Z
0002_connector_accounts.sql        applied  sha256:b211...  applied:2026-04-29T07:00:13Z
0003_work_items.sql                applied  sha256:9c8e...  applied:2026-04-29T07:00:14Z
0004_calendar_and_briefs.sql       applied  sha256:1b6a...  applied:2026-04-29T07:00:15Z
0005_approvals_jobs_traces.sql     applied  sha256:48f2...  applied:2026-04-29T07:00:16Z
```

### Drift detection

If a migration file's content changes after it has been applied,
`corvid migrate status` flags it as `drifted` and exits non-zero. This is
a CI gate — drifted migrations are an error in source control, and the
binary refuses to start with a drifted schema.

Resolve drift by reverting the file change OR by deliberately re-creating
the schema (in non-production environments) via `corvid migrate reset`.
**Never edit an applied migration in production.** Forward-only.

### Dry run

```bash
corvid migrate up --dry-run --dir migrations --state $POSTGRES_DSN
```

Prints the SQL that would be executed without touching the database. Use
this in pre-deploy checks and PR reviews.

### Rollback (forward-only schema with explicit rollback files)

Corvid's migration tool supports a paired `0001_identity.up.sql` /
`0001_identity.down.sql` convention. The PEA's current migrations are
single-file `.sql`; pairing each with a `.down.sql` is filed as a
maturity-track follow-up. Until then, rollback uses application-level
data unwinding, not schema unwinding.

For staging, the practical procedure is:

```bash
# 1. Stop the service.
docker compose stop pea

# 2. Snapshot the current DB.
docker compose exec db pg_dump -Fc pea > /backup/pea-$(date +%s).dump

# 3. Restore the previous known-good dump.
docker compose exec db pg_restore -d pea < /backup/pea-2026-04-29.dump

# 4. Start the previous binary version.
docker compose up -d --force-recreate
```

For production, see [§11 Rollback procedures](#11-rollback-procedures) for
a step-by-step.

### Migration file conventions

Every migration file:
- Starts with a comment block naming the migration's purpose and the
  tables it creates or alters.
- Wraps every statement in a single transaction.
- Uses `IF NOT EXISTS` only for indexes, never for tables (deterministic
  table creation is required for drift detection).
- Names indexes after the column(s) they cover plus the table prefix
  (`idx_work_items_tenant_id`).
- Names foreign keys after the source-table column pair
  (`fk_drafts_work_item_id`).

---

## 7. Backups — what, where, how often

### What to back up

| Asset | Source | Frequency | Retention |
|---|---|---|---|
| Application database | SQLite file at `data/pea.sqlite` OR `pg_dump` of the production Postgres | Hourly snapshots, daily full | 30 days hot, 90 days cold |
| Trace store (lineage JSONL + queue checkpoints) | `data/traces/` directory + the `queue_job_checkpoints` table | Daily full | 7 days hot, 30 days cold |
| Connector tokens (encrypted) | `connector_tokens` table | Daily full (with key separately) | 90 days; key rotation invalidates old ciphertext |
| Eval fixtures | `evals/` directory in source control | Per-commit (via git) | Forever (source-controlled) |
| Approval audit events | `audit_events` table | Daily full | 7 years (regulatory minimum for financial-adjacent flows) |

### Backup strategy by deployment shape

**SQLite single-host (staging or self-hosted):**

```bash
# Hourly incremental: VACUUM INTO a snapshot file.
sqlite3 data/pea.sqlite "VACUUM INTO 'backups/pea-$(date +%Y%m%d%H).sqlite'"

# Daily full: copy + tar + ship to off-host storage.
tar czf backups/pea-$(date +%Y%m%d).tar.gz data/
aws s3 cp backups/pea-$(date +%Y%m%d).tar.gz s3://pea-backups/$(date +%Y/%m/)/
```

**Postgres-managed (RDS, Cloud SQL, Crunchy Bridge, etc.):**
The managed provider's daily snapshot + 5-minute PITR transaction log
covers the database. Add cron jobs for the file-based artifacts (trace
store + eval fixtures).

**Self-hosted Postgres:**

```bash
# Daily logical dump.
pg_dump -Fc pea > /backup/pea-$(date +%Y%m%d).dump
aws s3 cp /backup/pea-$(date +%Y%m%d).dump s3://pea-backups/$(date +%Y/%m/)/

# Continuous WAL archiving (postgresql.conf):
# archive_mode = on
# archive_command = 'aws s3 cp %p s3://pea-wal/%f'
# archive_timeout = 300
```

### Restore — full recovery

```bash
# 1. Stop the service.
kubectl scale deploy/pea-api --replicas=0
kubectl scale deploy/pea-worker --replicas=0

# 2. Restore the database snapshot.
pg_restore -d pea_restore /backup/pea-2026-05-26.dump
# Or for SQLite:
cp backups/pea-2026052612.sqlite data/pea.sqlite

# 3. Restore the trace store directory.
tar xzf backups/pea-traces-2026-05-26.tar.gz -C data/

# 4. Replay any in-flight queue jobs from checkpoints.
corvid jobs schedule recover --state $POSTGRES_DSN

# 5. Scale workloads back.
kubectl scale deploy/pea-api --replicas=2
kubectl scale deploy/pea-worker --replicas=2
```

### Restore — selective table

Sometimes a single table is corrupted (manual schema edit, application
bug, intern-with-shell incident). Restore one table without rolling back
the whole database:

```bash
# Extract just the work_items table from a dump.
pg_restore -t work_items -Fc /backup/pea-2026-05-26.dump > /tmp/work_items.sql

# Apply to a temp schema, then UPSERT into production.
psql -d pea_temp < /tmp/work_items.sql
psql -d pea -c "INSERT INTO work_items SELECT * FROM pea_temp.work_items ON CONFLICT (id) DO UPDATE SET …"
```

For an `audit_events` table — DO NOT delete or modify rows. Audit events
are append-only by policy. If a row is wrong, append a corrective event
referencing the bad row's `id` in the `reason` field; do not overwrite.

### Backup verification

Backups that aren't tested are not backups. The PEA includes a quarterly
restore drill procedure:

1. Spin up a fresh ephemeral Postgres on a non-prod host.
2. Restore the latest weekly dump.
3. Boot a PEA worker pointed at it.
4. Run `corvid eval evals/hardening_eval.cor --state <ephemeral-db>`.
5. Confirm the schema manifest endpoint returns the canonical counts
   (5 / 12 / 5 / 4 / 5).
6. Tear down.

Document the drill outcome (date, restore time, eval pass/fail) in
`docs/operations/backup-drill.md`. A drill that does not produce a
written outcome did not happen.

---

## 8. Logs and traces

### Where logs land

The PEA emits four log streams:

| Stream | Format | Destination | Retention |
|---|---|---|---|
| Structured runtime logs | JSON to stderr | Container stdout → log aggregator | 30 days |
| Per-job lineage traces | JSONL | `data/traces/jobs/<job-id>.jsonl` | 7 days hot, 90 days cold |
| OTLP spans | Protobuf | OTLP collector at `$CORVID_OTLP_ENDPOINT` | Whatever the OTel backend keeps |
| Audit events | SQL rows | `audit_events` table | 7 years |

### Per-job trace contents

Every `@replayable` job persists a JSONL trace alongside the queue's
SQLite checkpoints. The trace schema (defined in `corvid-trace-schema`)
captures:

- `SchemaHeader` — schema version, writer tier, source path, run id
- `RunStarted` — agent name + arguments
- `LlmCall` / `LlmResult` — model name, prompt fingerprint, response
- `ToolCall` / `ToolResult` — tool name, args, return value
- `ApprovalRequest` / `ApprovalResponse` / `ApprovalDecision` — approval
  contract, requester, decision
- `SeedRead` / `ClockRead` — every nondeterministic input the agent saw
- `RunCompleted` — final state with output value or error

The trace file is the input to `corvid jobs replay --job <id>`. During
replay every side-effect surface is quarantined (see
[`docs/phases/phase-38-replay-quarantine.md`](../../../../docs/phases/phase-38-replay-quarantine.md)),
so a leaked trace cannot trigger a real provider call.

### Redaction

Connector manifests declare per-field redaction rules. The PEA's mock
manifest redacts:

- Email message bodies (replaced with `sha256:<digest>` in traces)
- Calendar attendee email addresses (replaced with fingerprint)
- Task descriptions if they came from external systems (Linear, GitHub)
- Chat message bodies

The redaction adversarial test
[`adversarial/unredacted_trace.json`](../adversarial/unredacted_trace.json)
seeds a synthetic SSN and asserts it does not appear in any emitted trace.
Run it after any change to redaction rules.

### Searching traces

```bash
# Find the trace for one job.
corvid observe show --job-id <job-id>

# Filter traces by approval label.
corvid observe list --since 1h --approval-label SendFollowUpEmail

# Show the failing runs.
corvid observe list --status failed --since 24h
```

### Log levels and tuning

The runtime respects the `RUST_LOG` environment variable. Recommended
production setting:

```dotenv
RUST_LOG=info,corvid_runtime::queue=info,corvid_runtime::auth=warn,reqwest=warn
```

`debug` is appropriate during incident response. `trace` will fill the
log aggregator quickly — use only with a clear hypothesis.

---

## 9. Metrics and alerting

### Prometheus exposition

The runtime exposes a `/metrics` endpoint on the port set by
`$CORVID_METRICS_LISTEN` (default `:9090`). The metric naming follows
OpenMetrics conventions; every metric carries `service`, `tenant_id`
(where meaningful), and `agent_name` (where meaningful) labels.

### Counters worth alerting on

| Metric | What it means | Suggested alert |
|---|---|---|
| `corvid_jobs_failed_total{service="personal_executive_agent"}` | Jobs that reached terminal failure | `rate(...[5m]) > 0.1` (more than 1 failure / 50 jobs) |
| `corvid_jobs_dead_lettered_total` | Jobs exhausted retries | `increase(...[15m]) > 0` (page on any) |
| `corvid_approvals_pending{service="personal_executive_agent"}` | Open approvals awaiting reviewer | `> 50 for 30m` (reviewer queue backed up) |
| `corvid_approvals_expired_total` | Approvals that timed out without decision | `increase(...[1h]) > 5` |
| `corvid_quarantine_violations_total{surface="llm"}` | Replay-mode LLM call refused | `> 0` always pages (means something tried to leak) |
| `corvid_quarantine_violations_total{surface="http"}` | Replay-mode HTTP send refused | `> 0` always pages |
| `corvid_quarantine_violations_total{surface="store"}` | Replay-mode store write refused | `> 0` always pages |
| `corvid_quarantine_violations_total{surface="io"}` | Replay-mode file write refused | `> 0` always pages |
| `corvid_llm_cost_usd_sum` | Cumulative LLM spend | `rate(...[1h]) > $5 / hour for 4h` |
| `corvid_http_provider_failures_total{provider="gmail"}` | Gmail real-mode failures | `rate(...[5m]) > 0.05` |
| `corvid_http_provider_failures_total{provider="slack"}` | Slack real-mode failures | `rate(...[5m]) > 0.05` |
| `corvid_session_replay_attempt_total` | Sessions presenting pre-rotation tokens | `increase(...[5m]) > 100` (key rotation under attack) |
| `corvid_csrf_rejections_total` | CSRF double-submit failures | `increase(...[5m]) > 50` (possible CSRF attack OR client bug) |

### Histograms

| Metric | What to watch |
|---|---|
| `corvid_job_duration_seconds{job="daily_brief"}` | p99 < 90s (each daily brief should finish under 90s) |
| `corvid_job_duration_seconds{job="email_triage"}` | p99 < 5s per batch |
| `corvid_job_duration_seconds{job="follow_up"}` | p99 < 15s (approval-gated; most of this is HTTP) |
| `corvid_route_duration_seconds{route="/actions/follow-up/send"}` | p99 < 2s |
| `corvid_llm_call_duration_seconds{model="claude-opus-4-7"}` | p99 < 8s (per-call upper bound) |

### Dashboards

The Grafana provisioning bundle in `deploy/docker-compose.yml` includes
the PEA dashboard at `deploy/grafana/dashboards/pea.json`. Key panels:

- Workday job throughput (4 jobs × workday hours × users)
- Approval queue depth over time
- LLM spend per agent per day
- Provider latency by connector
- Quarantine violation rate (target: zero)

### Alert hygiene

Every alert routes to the on-call rotation through PagerDuty or
equivalent. Quarantine-violation alerts are P0 because they mean either
a code change opened a leak path or an attacker found one. Job-failure
alerts are P2 unless the dead-letter rate climbs above `1/hour`, at which
point they're P1.

---

## 10. Incident response — diagnose and recover

This section is the on-call playbook for the failure modes the PEA can
exhibit. Each procedure is a numbered sequence; do not skip steps without
a written justification.

### Common-failure decision tree

The most useful first question is "what is `corvid jobs explain <id>`
telling us?" — it's a typed classifier over the queue state + audit
events that classifies the operational position (pending / leased /
running / approval_wait / retry_wait / dead_lettered / succeeded /
failed / canceled / loop_stall_*) and renders position-specific
suggested-next-steps.

### Incident: Daily brief did not run at 07:00 ET

```bash
# 1. Check the worker pool is healthy.
kubectl get pods -n pea -l app=pea-worker
# Expected: at least one Running.

# 2. Check the cron manifest is loaded.
corvid jobs schedule list --state $POSTGRES_DSN | grep daily_brief
# Expected: one row with cron="0 7 * * 1-5", zone="America/New_York".

# 3. Check the worker actually polled at 07:00.
corvid observe list --since 1h --task daily_brief
# Expected: one job per active user, status=succeeded or running.

# 4. If no jobs visible, check the scheduler is firing.
kubectl logs -n pea -l app=pea-worker --since=2h | grep "schedule fired"

# 5. If schedule didn't fire, the most common cause is a DST transition
#    or a missed-fire policy. Check the recovery action:
corvid jobs schedule recover --state $POSTGRES_DSN --dry-run
# Expected: 0 missed schedules. Non-zero means catch-up needed.
```

### Incident: Approval queue is backed up

Signal: `corvid_approvals_pending > 50 for 30m`.

```bash
# 1. List the oldest pending approvals.
corvid approvals queue --tenant <id> --order=oldest --limit=20

# 2. For each: who is the assigned reviewer? Is the reviewer on call?
corvid approvals explain --tenant <id> <approval-id>

# 3. If the reviewer rotation is broken, batch-approve the
#    semantically-equivalent items (only if the policy allows):
corvid approvals batch --tenant <id> --label SendFollowUpEmail --since 4h

# 4. If the queue depth is growing faster than reviewers can handle,
#    page the human-operator-of-the-day. Don't auto-resolve.
```

### Incident: A connector is timing out

Signal: `corvid_http_provider_failures_total{provider="gmail"} rate > 5%`.

```bash
# 1. Confirm the provider's status page first.
# 2. Check the connector's rate-limit headers.
corvid connectors check --connector gmail --tenant <id> --narrate

# 3. Inspect the last successful exchange's headers.
corvid observe list --task email_triage --status succeeded --since 24h \
  --json | jq '.[0].spans[] | select(.kind=="http") | .response_headers'

# 4. If the provider has degraded but not failed, flip the connector
#    into mock mode for the affected tenant:
corvid connectors mode --tenant <id> --connector gmail --mode mock

# 5. File a status post and switch back when the provider recovers.
```

### Incident: Quarantine violation reported

Signal: `corvid_quarantine_violations_total{surface="llm"} > 0`.

This is a P0. Quarantine violations should NEVER fire in steady state.

```bash
# 1. Get the violation detail.
corvid observe list --since 5m --status quarantine_violation

# 2. The detail names the adapter, model, and prompt. Inspect the run.
corvid jobs explain --state $POSTGRES_DSN <job-id>

# 3. If this is during a replay: the trace and the agent body have
#    diverged. Either the trace is stale (re-record) or the agent
#    body changed (replay can't be run against the old trace).

# 4. If this is during a normal run: something installed the quarantine
#    layer incorrectly. Stop the worker. Inspect:
corvid jobs run --state $POSTGRES_DSN --source src/main.cor --dry-run

# 5. If you can't reproduce: kill the running worker and rotate
#    `CORVID_RUN_ID` in case of state corruption.
```

### Incident: Worker process panicked

Signal: pod restart loop, OOMKilled, or `panic` in stderr logs.

```bash
# 1. Last 200 lines of logs from the failing pod.
kubectl logs -n pea --tail=200 --previous deploy/pea-worker

# 2. Check the job that was running when the panic happened.
corvid jobs list --state $POSTGRES_DSN --status leased --since 5m

# 3. The runtime's lease will expire naturally (60s default). After
#    expiry, another worker re-leases and retries the job.

# 4. If the same job panics every worker: it's poisoned.
#    Quarantine it:
corvid jobs cancel --state $POSTGRES_DSN <job-id>
#    File an issue with the job trace attached.
```

### Incident: Migrations drifted in production

Signal: the binary refused to start with `migration drift detected`.

```bash
# 1. STOP. Do not edit the migration file to make the drift go away.
corvid migrate status --dir migrations --state $POSTGRES_DSN
# Lists drifted migrations with their committed vs computed SHA-256.

# 2. Inspect git history for the drifted file.
git log -p migrations/0003_work_items.sql

# 3. If the file was edited intentionally for a reason, revert the edit
#    and ship a new forward migration (0006_*) with the change.

# 4. If the file was edited by mistake, revert with git.

# 5. After reverting, confirm the binary starts.
```

### Incident: An LLM provider is degraded

```bash
# 1. corvid observe drift compares the most recent runs against a
#    baseline. Use it to confirm the degradation.
corvid observe drift --from <baseline-trace> --to <current-trace>

# 2. If the degradation is provider-side (Anthropic / OpenAI status page
#    confirms), enable the failover model catalog:
corvid model failover --enable --primary claude-opus-4-7 \
  --fallback claude-sonnet-4-6 --tenant <id>

# 3. If a cost spike accompanies the degradation, set a per-tenant
#    budget cap until the provider stabilises.
```

---

## 11. Rollback procedures

### When to roll back

Roll back if any of the following are true:

- A new release caused a P0 incident that can't be hotfixed within 30
  minutes.
- A quarantine violation is firing in steady state on the new release
  and was not on the previous release.
- The schema manifest is reporting different counts than the deployed
  binary expects (suggests a partial migration).
- Two consecutive replicas crashed on startup with the same panic.

### Pre-rollback checklist

```bash
# 1. Snapshot the current state for post-mortem.
kubectl get pods -n pea -o yaml > /tmp/pea-pods.yaml
kubectl logs -n pea --since=30m -l app=pea-api > /tmp/pea-api.log
kubectl logs -n pea --since=30m -l app=pea-worker > /tmp/pea-worker.log
corvid observe export --since 1h --out /tmp/pea-incident.tar.gz

# 2. Identify the previous good image tag.
kubectl describe deploy/pea-api | grep Image:
# Note the current tag and the previous one (visible in rollout history).
kubectl rollout history deploy/pea-api
```

### Binary-only rollback (schema unchanged)

```bash
# 1. Roll back the deployment.
kubectl rollout undo deploy/pea-api --to-revision=<previous>
kubectl rollout undo deploy/pea-worker --to-revision=<previous>

# 2. Wait for ready.
kubectl rollout status deploy/pea-api --timeout=180s
kubectl rollout status deploy/pea-worker --timeout=180s

# 3. Smoke check.
curl -sf https://pea.example.com/schema | jq '.migration_count'
```

### Binary + schema rollback

If the new release shipped a migration that the old binary cannot read,
you must roll back BOTH the binary and the schema. This requires a
database restore.

```bash
# 1. Stop everything.
kubectl scale deploy/pea-api --replicas=0
kubectl scale deploy/pea-worker --replicas=0

# 2. Restore the database to a point-in-time BEFORE the bad migration.
#    (Use the most recent dump that predates the bad release.)
pg_restore -d pea_restore /backup/pea-pre-bad-release.dump

# 3. Cut over.
# (Specific cutover depends on your DB topology — pg_restore into the
# live database is destructive; prefer a rename-swap if possible.)

# 4. Roll back the deployment.
kubectl rollout undo deploy/pea-api --to-revision=<previous>
kubectl rollout undo deploy/pea-worker --to-revision=<previous>

# 5. Re-replay any in-flight queue jobs from checkpoints that survived.
corvid jobs schedule recover --state $POSTGRES_DSN
```

### Rollback verification

After any rollback:

```bash
# 1. Schema manifest matches binary expectations.
curl -sf https://pea.example.com/schema | jq .
# Expected: migration_count=5, table_count=12, ...

# 2. Workers are draining the queue.
corvid jobs run-stats --state $POSTGRES_DSN --since 5m

# 3. Quarantine violations stopped.
# Wait 5 minutes. Then:
curl -sf https://pea.example.com/metrics | grep corvid_quarantine_violations_total

# 4. Run the eval suite.
corvid eval evals/hardening_eval.cor --state $POSTGRES_DSN
```

If any check fails, escalate to the engineering owner — do not try
another rollback layer without a written hypothesis.

### Post-rollback

- File a post-mortem within 24 hours per the team's PIR template.
- Attach `/tmp/pea-incident.tar.gz` and the two pod logs to the PIR.
- Add a regression test under
  [`adversarial/`](../adversarial/) that catches the same failure mode.
- The fix that didn't ship is re-attempted only after the regression test
  goes red in CI without it AND green with the fix.

---

## 12. Connector mode operations

### Mode semantics

Every connector runs in one of three modes:

| Mode | Behavior | Use case |
|---|---|---|
| `mock` | All operations return deterministic fixtures from `mocks/`. No network. Reproducible. | Default. Local dev. CI tests. Replay verification. |
| `replay` | Operations consume the next event from a recorded trace. Same shape as mock; trace-driven. | Diagnosing a past run. Eval promotion. |
| `real` | Operations hit live provider APIs. Requires credentials. | Production. Staging end-to-end tests. |

The default is `mock` — `CORVID_CONNECTOR_MODE=mock`. Real mode is opt-in
per environment AND per connector AND per tenant. The adversarial fixture
`adversarial/real_connector_default.json` proves no reference app can
default to real mode.

### Flip a tenant from mock to real

Pre-conditions:
- OAuth tokens for the connector are present in `connector_tokens`.
- The connector has a scope grant that matches the manifest.
- The provider is not in a status incident.

```bash
# 1. Confirm tokens are present.
corvid auth keys list --tenant <id> --kind connector-token --connector gmail

# 2. Confirm scopes match.
corvid connectors scopes-min --connector gmail --source src/main.cor

# 3. Flip the tenant.
corvid connectors mode --tenant <id> --connector gmail --mode real

# 4. Smoke a read-only operation.
corvid observe show --tenant <id> --route /inbox/triage/mock --status succeeded --since 5m
```

### Flip a tenant back to mock (emergency)

```bash
corvid connectors mode --tenant <id> --connector gmail --mode mock
```

Effect: every subsequent connector call returns the mock fixture instead
of hitting Gmail. In-flight jobs receive their next call in mock mode;
already-issued calls complete naturally.

### Flip the WHOLE environment to mock (declared incident)

```bash
kubectl set env deploy/pea-api CORVID_CONNECTOR_MODE=mock
kubectl set env deploy/pea-worker CORVID_CONNECTOR_MODE=mock
kubectl rollout restart deploy/pea-api deploy/pea-worker
```

This forces every connector to mock for every tenant, regardless of
per-tenant overrides. Use during a provider-wide outage when you want
the PEA to keep running but not attempt live calls.

### Webhook signature verification

For connectors that receive webhooks (Slack, GitHub, Linear), the
verifier lives in `corvid-connector-runtime`. Configure the shared
webhook secret per connector:

```bash
corvid connectors webhook secret --connector slack --tenant <id> \
  --secret-from-stdin < /tmp/slack-secret
```

Verify a webhook payload locally:

```bash
corvid connectors verify-webhook --connector slack \
  --sig "v0=$(cat /tmp/slack-sig)" --body @/tmp/slack-body.json
```

Returns `verified` or a typed `signature_forgery` / `replay_attack` /
`stale_timestamp` error.

---

## 13. Approval queue operations

### Queue model

Every dangerous tool call enters an `ApprovalQueueItem` carrying the
typed `ApprovalContractRef` (target, cost ceiling, data class,
irreversible flag, expiry, required role). The PEA defines four
contracts at build time:

- `SendFollowUpEmail` — Reviewer role, $0.25 cost ceiling, irreversible
- `EditCalendarEvent` — Reviewer role, $0.25, irreversible (external invites)
- `EditTaskItem` — Reviewer role, $0.10, reversible (Linear/GitHub edit)
- `SendChatMessage` — Reviewer role, $0.10, irreversible

A fifth (`ExternalCalendarShare`) is filed under the maturity track.

### List the queue

```bash
# All pending approvals for a tenant, oldest first.
corvid approvals queue --tenant <id> --order=oldest

# Filter by contract label.
corvid approvals queue --tenant <id> --label SendFollowUpEmail

# Show the structured payload for a reviewer UI.
corvid approvals queue --tenant <id> --format json | jq '.[0]'
```

### Approve, deny, expire

```bash
# Approve with audit reason.
corvid approvals decide --tenant <id> <approval-id> --decision approved \
  --reason "Matches the customer's prior reply tone"

# Deny.
corvid approvals decide --tenant <id> <approval-id> --decision denied \
  --reason "Recipient mismatch"

# Expire (operator-initiated). Approvals expire automatically when
# their `expires_at` is reached.
corvid approvals decide --tenant <id> <approval-id> --decision expired \
  --reason "Reviewer on leave; followup will recreate"
```

Every decision writes an `ApprovalAuditEnvelope` row carrying the actor,
status before, status after, reason, and trace id.

### Batch approval (semantic equivalence)

```bash
# Approve all SendFollowUpEmail items for tenant within last 4h.
corvid approvals batch --tenant <id> --label SendFollowUpEmail --since 4h
```

The batch endpoint applies one approval per item; it does not collapse
them into a single audit row. Each item gets its own audit envelope.

### Delegate

```bash
corvid approvals delegate <approval-id> --to <reviewer-actor-id> \
  --tenant <id> --reason "Domain handoff to compliance"
```

Delegation writes an audit event but does not change the approval's
required role; the delegate must satisfy the same role check.

### Reviewer queue staleness

If approvals are accumulating without being decided:

```bash
# How long have items been pending?
corvid approvals stats --tenant <id> --since 24h

# Who decided what?
corvid approvals export --tenant <id> --since 24h --by actor
```

Operational handoff: when no reviewer is available, the operator may
batch-deny with reason "no reviewer available" — but this is a P2
operational event that should be reported in the next standup.

### Approval explain (assistive helper)

```bash
corvid approvals explain --tenant <id> <approval-id>
```

Renders a typed reviewer summary with the operator facts (target,
expected cost, data touched, irreversibility), every audit-event
transition, optional loop usage, and position-specific
suggested-next-steps. The `sources` array carries every audit-event id
the explanation consulted — the `Grounded<T>` contract at the JSON layer.

---

## 14. Durable jobs and cron operations

### Job model

Every PEA job is a `@replayable` agent with a typed
`ExecutiveJobContract` declaring queue, max attempts, budget, retry
policy, idempotency key, and replay key. The four scheduled jobs are
listed in [§2 Architecture map](#2-architecture-map).

### Run the queue manually

```bash
corvid jobs run --source src/main.cor --state $POSTGRES_DSN \
  --workers 2 --lease-ttl-ms 60000 --idle-poll-ms 100 \
  --max-runtime-ms 0     # 0 = run until Ctrl-C
```

In production, the worker pool runs in its own deployment. Manual runs
are for debugging.

### Pause, drain, resume

```bash
# Stop accepting new lease grants. In-flight jobs complete.
corvid jobs pause --state $POSTGRES_DSN --reason "deploy window"

# Drain all active leases (lets in-flight jobs finish without restart).
corvid jobs drain --state $POSTGRES_DSN --reason "rolling deploy"

# Resume accepting leases.
corvid jobs resume --state $POSTGRES_DSN
```

### Retry, cancel, dead-letter

```bash
# Retry a failed or canceled job from its last checkpoint.
corvid jobs retry --state $POSTGRES_DSN <job-id>

# Cancel a job (terminal). The next checkpoint resume is no-op.
corvid jobs cancel --state $POSTGRES_DSN <job-id>

# Inspect the dead-letter queue.
corvid jobs dlq --state $POSTGRES_DSN
```

### Schedule manifest

```bash
# List every scheduled (cron-driven) job.
corvid jobs schedule list --state $POSTGRES_DSN

# Recover missed fires (e.g. after a maintenance window).
corvid jobs schedule recover --state $POSTGRES_DSN \
  --policy enqueue_one_bounded
```

The PEA's missed-fire policy is `fire_once_on_recovery` for the morning
jobs (daily brief, meeting prep) and `skip_missed` for the inner-loop
jobs (email triage, follow-up). Don't change these without thinking
through user impact — `fire_once` on email triage during a 6-hour outage
would enqueue ~36 catchup jobs per user.

### Job export-trace (full lineage)

```bash
corvid jobs export-trace --state $POSTGRES_DSN <job-id> --out /tmp/job.lineage.jsonl
```

The exported trace is suitable for `corvid replay`, eval promotion via
`corvid eval promote`, and post-mortem inspection.

### Loop limits

Every agent-backed job declares loop limits at enqueue time:

| Limit | PEA value |
|---|---|
| `max_steps` | 50 |
| `max_wall_ms` | 90000 (90 s) |
| `max_spend_usd` | 0.75 |
| `max_tool_calls` | 30 |

Exceeding any limit triggers an escalation: the job moves to
`loop_stall_<dimension>` status with the bound that was exceeded in the
failure fingerprint.

```bash
# Inspect current loop usage.
corvid jobs loop-usage --state $POSTGRES_DSN <job-id>

# Set/override limits per job (only for running jobs).
corvid jobs loop-limits set --state $POSTGRES_DSN <job-id> \
  --max-steps 75 --max-spend-usd 1.00
```

---

## 15. Disaster recovery

### Recovery objectives

| Objective | Target |
|---|---|
| RPO — recovery point objective | ≤ 1 hour (hourly DB snapshots) |
| RTO — recovery time objective | ≤ 30 minutes (single-host) / ≤ 60 minutes (full region) |

### Single-host failure (volume loss)

```bash
# 1. Provision a replacement host.
# 2. Install the deploy artifacts.
# 3. Restore the database (see §7 Backups).
# 4. Restore the trace store.
# 5. Boot the service.
# 6. Recover missed schedules.
corvid jobs schedule recover --state $POSTGRES_DSN
# 7. Smoke evals.
corvid eval evals/hardening_eval.cor --state $POSTGRES_DSN
```

### Region-wide outage

If the entire primary region is unavailable:

1. Promote the cross-region read replica to primary
   (provider-specific).
2. Update the application's `DATABASE_URL` to point at the new primary.
3. Roll the deployment.
4. Run the smoke eval suite.
5. Re-enable cross-region replication once the primary region is
   restored (now reversed — old primary becomes the replica).

### Connector-token store loss

If the connector-token store is lost OR the
`CORVID_CONNECTOR_TOKEN_KEY` is rotated without a re-encryption pass,
every tenant is logged out from every connector.

Recovery:
1. Flip every tenant to `mock` mode immediately to prevent retries from
   hammering broken provider auth.
2. Notify users that they need to re-authorize via the connector
   re-auth flow.
3. As tenants re-auth, the new tokens are stored under the new key and
   `mock` flips back to `real` automatically (if the tenant had real
   mode enabled before).

### Approval queue corruption

If `audit_events` is corrupted:
- DO NOT restore from a partial backup — it would create gaps in the
  audit trail that the regulatory framework forbids.
- Restore the entire audit table from the latest known-good backup.
- Re-issue affected approvals (they'll be replayed via the trace store)
  rather than mutating existing rows.

### Trace store loss

Traces are append-only and lossy by design (90-day cold retention).
Losing them up to RPO is acceptable; losing them entirely means losing
the ability to:
- Reproduce past runs via `corvid jobs replay`.
- Promote new evals from production data via `corvid eval promote`.
- Run drift attribution on the lost period.

If trace store is lost without backup, declare the loss in the next
post-mortem and re-evaluate retention policy.

---

## 16. Appendix — reference data

### Environment variable inventory

| Variable | Default | Required? |
|---|---|---|
| `CORVID_ENV` | `development` | Recommended in production (`production`) |
| `CORVID_CONNECTOR_MODE` | `mock` | Always |
| `CORVID_CONNECTOR_TOKEN_KEY` | — | Production yes |
| `CORVID_API_KEY_PEPPER` | — | Production yes |
| `CORVID_SESSION_SIGNING_KEY` | — | Production yes |
| `CORVID_CSRF_SECRET` | — | Production yes |
| `CORVID_OTLP_ENDPOINT` | unset | If OTLP export is desired |
| `CORVID_METRICS_LISTEN` | `:9090` | Recommended |
| `RUST_LOG` | `info` | Optional tuning |
| `DATABASE_URL` | `sqlite:./data/pea.sqlite` | Always |
| `ANTHROPIC_API_KEY` | — | Only when LLM real mode is wanted |
| `OPENAI_API_KEY` | — | Only when LLM real mode is wanted |

### Cron expressions

| Cron | Zone | Job |
|---|---|---|
| `0 7 * * 1-5` | America/New_York | `daily_brief_job` |
| `0 6 * * 1-5` | America/New_York | `meeting_prep_job` |
| `*/10 8-18 * * 1-5` | America/New_York | `email_triage_job` |
| `*/15 8-18 * * 1-5` | America/New_York | `follow_up_job` |

### Approval contract reference

| Label | Required role | Cost ceiling | Data class | Irreversible | Expires in |
|---|---|---|---|---|---|
| `SendFollowUpEmail` | Reviewer | $0.25 | private | true | 24h |
| `EditCalendarEvent` | Reviewer | $0.25 | private | true | 24h |
| `EditTaskItem` | Reviewer | $0.10 | private | true | 24h |
| `SendChatMessage` | Reviewer | $0.10 | private | true | 24h |
| `ExternalCalendarShare` (filed) | Admin | $0.25 | external | true | 24h |

### Adversarial corpus reference

| Fixture | What it proves |
|---|---|
| [`missing_replay_key.json`](../adversarial/missing_replay_key.json) | Jobs without a stable replay key cannot be enqueued |
| [`provider_secret_in_source.json`](../adversarial/provider_secret_in_source.json) | Source-embedded provider secrets are refused at typecheck |
| [`real_connector_default.json`](../adversarial/real_connector_default.json) | No reference app may default to `mode=real` |
| [`ungated_send.cor`](../adversarial/ungated_send.cor) | `send_follow_up_email` without preceding `approve SendFollowUpEmail` does not compile |
| [`unredacted_trace.json`](../adversarial/unredacted_trace.json) | Connector manifest redaction rules strip the named fields from emitted traces |

### Related documents

- Security model: [`security-model.md`](../security-model.md)
- Source: [`src/main.cor`](../src/main.cor)
- Mock connector manifest: [`connectors/mock_manifest.json`](../connectors/mock_manifest.json)
- Eval suite: [`evals/hardening_eval.cor`](../evals/hardening_eval.cor)
- Demo trace: [`traces/demo.lineage.jsonl`](../traces/demo.lineage.jsonl)
- Replay quarantine design: [`docs/phases/phase-38-replay-quarantine.md`](../../../../docs/phases/phase-38-replay-quarantine.md)
- Phase 39 audit (auth surface): [`docs/phases/phase-39-audit-2026-05-17.md`](../../../../docs/phases/phase-39-audit-2026-05-17.md)
- Phase 40 audit (observability surface): [`docs/phases/phase-40-audit-2026-05-17.md`](../../../../docs/phases/phase-40-audit-2026-05-17.md)
- Phase 42 audit (per-app maturity): [`docs/phases/phase-42-audit-2026-05-17.md`](../../../../docs/phases/phase-42-audit-2026-05-17.md)

### Non-goals (restated from §1)

- No external write without human approval.
- No real provider call by default.
- No password storage; only Argon2id hashes.
- No financial advice, legal advice, or medical advice.
- No autonomous action outside the four declared scheduled jobs and the
  approval-gated execute actions.
