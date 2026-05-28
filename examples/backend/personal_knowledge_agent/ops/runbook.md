# Personal Knowledge Agent — Operator Runbook

This runbook is the operational source of truth for running the Personal
Knowledge Agent backend in development, staging, and production. The PKA
is a reference Corvid application: it ingests heterogeneous source roots
into a tenant-scoped, citation-preserving index, answers questions with
typed `Grounded<T>` provenance, accepts reviewer feedback, and gates every
external write (chat post, email send, knowledge-base publish, corpus
export, cross-tenant index share) behind a typed approval contract.

Every procedure below is grounded in surfaces the app actually ships. The
schema manifest at [`src/main.cor`](../src/main.cor) declares the canonical
counts (5 migrations / 18 tables / 3 connectors / 3 durable jobs / 5
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

### What the Personal Knowledge Agent does

The Personal Knowledge Agent runs three durable jobs on a daily/weekly
cron and exposes a typed HTTP surface for every action a knowledge
worker's assistant takes on their behalf:

- `nightly_reindex` (cron `0 2 * * *` America/New_York) — rescans every
  registered source root for a tenant, applies the connector's
  content-hash diff, re-embeds changed chunks, and writes the new
  `KnowledgeChunk`/`KnowledgeEmbedding` rows under the tenant's index
  partition. Provenance IDs are preserved across versions.
- `weekly_feedback_batch` (cron `0 6 * * 1` America/New_York) — promotes
  accepted reviewer feedback into the eval corpus. Each promotion writes
  a `corvid.eval.lineage_fixture.v1` record that becomes part of
  `corvid eval` for the next release.
- `daily_provenance_audit` (cron `0 3 * * *` America/New_York) — verifies
  that every `KnowledgeAnswer` written in the past 24 hours has a
  `Grounded<T>` citation chain whose `provenance_id` resolves to a live
  `KnowledgeSource`. Any answer whose chain is broken is quarantined and
  the operator is paged.

Every external write enters one of five approval contracts:

| Approval label | Tool | Effect | Target type | Required role | Cost ceiling |
|---|---|---|---|---|---|
| `ShareAnswerToChat` | `share_answer_to_chat` | `chat_share` | `chat_channel` | Reviewer | $0.05 |
| `ShareAnswerViaEmail` | `share_answer_via_email` | `email_share` | `email_recipient` | Reviewer | $0.05 |
| `PublishAuthoritativeAnswer` | `publish_authoritative_answer` | `kb_publish` | `knowledge_base` | Reviewer | $0.10 |
| `ExportTenantCorpus` | `export_tenant_corpus` | `corpus_export` | `export_target` | Admin | $0.25 |
| `CrossTenantIndexShare` | `cross_tenant_index_share` | `cross_tenant_share` | `tenant_index_share` | Admin | $0.25 |

The first three are reviewer-level day-to-day publish flows. The last two
are admin-level boundary crossings (data leaves a tenant's blast radius)
and are gated by a stricter audit trail: every `ExportTenantCorpus` and
`CrossTenantIndexShare` event must be co-signed by a second admin within
24 hours or the receipt is auto-revoked by the `daily_provenance_audit`
job.

### What the Personal Knowledge Agent does not do

- Post to chat, send email, publish to a knowledge base, export a corpus,
  or share an index across tenants without human approval. Every one of
  those tools is `dangerous` and the compiler rejects callers that lack
  a matching `approve <Label>(...)` boundary.
- Talk to real provider APIs in the default mode. The default
  `CORVID_CONNECTOR_MODE=mock` keeps every connector in offline mode with
  deterministic mock fixtures so `corvid eval` and `corvid replay` stay
  reproducible.
- Store raw document bytes outside the tenant's storage partition. Bytes
  live in object storage keyed by content hash; the database holds only
  fingerprints, byte ranges, provenance IDs, and embedding metadata.
- Allow cross-tenant index reads or writes without an `Admin`-role
  approval and a co-signature trail. The `cross_tenant_share` effect is
  the only effect that can route a chunk across tenant partitions, and
  it is gated by the `CrossTenantIndexShare` contract.
- Decide policy for what is "authoritative" knowledge. The
  `PublishAuthoritativeAnswer` flow asks a human reviewer to assert
  authority; the agent never auto-promotes.

### Service-level objectives

- Availability: 99.9 % monthly for `GET /answer/*` and `GET /search/*`;
  99.5 % for `POST /actions/*` (lower because they are approval-gated).
- Latency (p99): `/search/*` < 600 ms, `/answer/*` < 1500 ms, `/ingest/*`
  < 5 s for a single source under 1 MB. Jobs are async — see §14 for
  durable job SLOs.
- Provenance correctness: 100 % of `KnowledgeAnswer` rows pass
  `daily_provenance_audit`. Any miss is an Sev-2 incident.

---

## 2. Architecture map

### Process layout

The PKA runs as a single Corvid server binary plus a SQLite or Postgres
backing store plus an object-storage bucket for raw document bytes. The
binary is built from `src/main.cor` via `corvid build --target=server`.
In production the binary is wrapped in a distroless OCI image; the same
binary serves all HTTP routes, runs the durable-job pool, the scheduler,
the OTLP exporter, and the metrics endpoint.

```
+---------------------------+
|   corvid jobs run         |
|   (durable job pool,      |  <-- handles cron + on-demand jobs,
|    in-process)            |       writes queue_jobs +
+---------------------------+       queue_job_checkpoints
            |
            v
+---------------------------+
|   corvid runtime          |  <-- typed effects, approvals,
|   (HTTP routes,           |       Grounded<T>, replay quarantine
|    scheduler,             |
|    OTLP exporter,         |
|    /metrics)              |
+---------------------------+
            |
   +--------+--------+
   |        |        |
   v        v        v
+------+ +-------+ +-----------+
| DB   | | Index | | Object    |
| (5   | | (vec  | | store     |
|  migs| |  per  | | (raw      |
| / 18 | | tenant| |  bytes)   |
| tbls)| | / root| |           |
+------+ +-------+ +-----------+
```

### Data classes

PKA processes three data classes. The compiler enforces that every
effect carries one and refuses to cross class boundaries without an
explicit approval:

- `private` — raw source bytes, embedding vectors, and any text reachable
  via a `Grounded<T>` citation. Lives only in the tenant's partition.
  Effects: `files_read`, `local_embed`, `index_write`.
- `bounded` — model prompts, model completions, redacted answer
  fingerprints. Lives in the LLM provider's TLS session and the trace
  log. Effects: `knowledge_llm`.
- `external` — anything that crosses the tenant boundary: a chat post,
  an email body, an authoritative KB publication, a corpus export, an
  index share. Lives in third-party systems forever once published.
  Effects: `chat_share`, `email_share`, `kb_publish`, `corpus_export`,
  `cross_tenant_share`.

### Storage surfaces

- **Database** (5 migrations, 18 tables): tenant + auth tables
  (`tenants`, `users`, `roles`, `user_roles`, `sessions`, `api_keys`,
  `permissions`), approval + audit tables (`approvals`, `audit_events`,
  `queue_jobs`, `queue_job_checkpoints`, `trace_lineage`), knowledge
  tables (`knowledge_sources`, `knowledge_documents`,
  `knowledge_chunks`, `knowledge_embeddings`, `knowledge_ingestion_jobs`,
  `knowledge_feedback`). The exact DDL ships in `migrations/0001_*.sql`
  through `migrations/0005_*.sql`.
- **Index**: a per-tenant, per-root vector index (FAISS or pgvector
  depending on the storage mode). The index is rebuildable from
  `knowledge_chunks` + `knowledge_embeddings` — it is not a primary
  store.
- **Object storage**: raw document bytes keyed by SHA-256 content hash.
  Versioning is preserved by content hash; deletion is a tenant-scoped
  operation and is always logged to `audit_events`.

### Connector layout

The PKA's three connectors are:

- `files_connector` (effect: `files_read`) — reads from local FS, S3,
  Google Drive, or SharePoint depending on the registered source root.
- `local_embed_connector` (effect: `local_embed`) — runs a local
  embedding model (default `bge-small-en`). Never makes a network call.
- `index_connector` (effect: `index_write`) — writes chunk/embedding
  rows under the tenant partition.

External-write effects (`chat_share`, `email_share`, `kb_publish`,
`corpus_export`, `cross_tenant_share`) route through the runtime's
`HttpRuntime` and `IoRuntime`; they are *not* connectors and have no
mock mode — they fail closed when approval is missing.

---

## 3. Setup — local development

### Prerequisites

- Corvid toolchain installed (`corvid --version` reports a tag in the
  `35V2` series).
- SQLite 3.40+ (default) or Postgres 14+ (if `CORVID_STORAGE_MODE=postgres`).
- A writable `target/` directory in the workspace root.
- ~2 GB of free disk for the local embedding model cache.

### First-time local boot

```
# from the repo root
cd examples/backend/personal_knowledge_agent
export CORVID_CONNECTOR_MODE=mock     # default; keeps everything offline
export CORVID_STORAGE_MODE=sqlite     # default; uses target/pka.db
export CORVID_LOCAL_ONLY=true         # default; refuses any network call
corvid check src/main.cor
corvid migrate --target=sqlite --dir=migrations
corvid seeds load seeds/demo.sql
corvid run --target=server --bind=127.0.0.1:8086
```

If everything is wired correctly, `corvid run` exposes the routes listed
in §1 on port 8086. `GET /config` should return the
`KnowledgeConfig("personal_knowledge_agent", "mock",
"sqlite+local_index", true, "target/traces")` envelope.

### Smoke test the local boot

In a second shell:

```
curl -s http://127.0.0.1:8086/schema | jq
curl -s http://127.0.0.1:8086/ingest/mock | jq '.provenance_preserved'
curl -s http://127.0.0.1:8086/search/mock | jq '.grounded'
curl -s http://127.0.0.1:8086/answer/mock | jq '.grounded, .citation_count'
curl -s http://127.0.0.1:8086/feedback/eval/mock | jq '.accepted, .promoted_to_eval'
curl -s http://127.0.0.1:8086/jobs/nightly-reindex/mock | jq '.contract.job_kind'
curl -s http://127.0.0.1:8086/jobs/daily-provenance-audit/mock | jq '.contract.job_kind'
```

Expected results: every response is `true` / `>0` / matches the job kind
name. If any returns `false` or a `null` field, *do not deploy* — the
build is broken and the smoke is the first defense.

### Run the typed eval suite

```
corvid eval evals/search_answer_eval.cor
corvid eval evals/hardening_eval.cor
```

Both must exit 0. The first eval covers the 11 grounded-search-answer
cases; the second covers approval gating and tenant isolation
adversarials.

### Promote a new fixture

```
corvid eval promote --case=<case-id> --in=traces/<trace>.lineage.jsonl \
    --out=evals/promoted/<case-id>.lineage-eval.json
```

The promotion writes a `corvid.eval.lineage_fixture.v1` record. The
record is checked into git so the next release replays it deterministically.

---

## 4. Setup — staging and production deployment

### Topology

The reference deployment is three tiers:

- **Edge** — TLS termination, request authentication (mTLS or
  signed-bearer), rate limit, request ID injection. The PKA does not
  bundle an edge proxy; use `nginx`, `envoy`, or the cloud provider's
  L7 LB.
- **App** — N replicas of the PKA binary behind the edge. Each replica
  runs the HTTP server, durable-job pool, and scheduler. The scheduler
  uses a per-job advisory lock so only one replica runs each cron tick
  at a time — no external coordinator is required.
- **Data** — Postgres primary + read replica + object-storage bucket +
  optional pgvector extension if running the index inline with the DB.

### Fly.io reference

`deploy/fly.toml` defines the canonical Fly.io deployment. Key
parameters:

- `app = "personal-knowledge-agent"`
- Primary region: `iad` (us-east); fallback: `sjc`.
- VM size: `shared-cpu-2x`, 2 GB RAM (enough for the local embedding
  model + a small index).
- Auto-scaling: 1 to 3 replicas; HTTP service on internal port `8086`,
  exposed on `443`.
- `[env]`: `CORVID_CONNECTOR_MODE = "mock"` by default. Production
  unsets this only after the operator has verified per-connector token
  storage is configured (see §5).

Deploy:

```
flyctl deploy --config deploy/fly.toml
flyctl logs --app personal-knowledge-agent
flyctl ssh console --app personal-knowledge-agent --command "corvid ops show"
```

The `corvid ops show` snapshot is signed and dated; archive each
production snapshot under `ops/snapshots/<YYYY-MM-DD>.json` so audits can
diff a release against the deployed surface.

### Kubernetes reference

`deploy/k8s/` defines the canonical Kubernetes deployment in 6 files:

- `deployment.yaml` — 2-replica `Deployment` with the PKA image,
  liveness probe on `/healthz`, readiness probe on `/readyz`.
- `service.yaml` — `ClusterIP` service exposing port `8086`.
- `ingress.yaml` — ingress class agnostic; TLS host required.
- `configmap.yaml` — non-secret env (mode, log level, OTLP endpoint).
- `secret.yaml` — placeholder Secret (do not check real secrets in).
- `hpa.yaml` — `HorizontalPodAutoscaler` keyed on CPU and request rate.

Deploy:

```
kubectl apply -f deploy/k8s/configmap.yaml
kubectl apply -f deploy/k8s/secret.yaml          # after editing real values
kubectl apply -f deploy/k8s/service.yaml
kubectl apply -f deploy/k8s/deployment.yaml
kubectl apply -f deploy/k8s/ingress.yaml
kubectl apply -f deploy/k8s/hpa.yaml
kubectl -n pka rollout status deploy/personal-knowledge-agent
```

### Docker Compose (single-host)

For staging or evaluation hosts:

```
cd examples/backend/personal_knowledge_agent
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml exec pka corvid ops show
```

The compose file boots three containers: `pka` (the app), `postgres`
(the DB), `minio` (object storage). The `pka` container's entrypoint
runs migrations before binding the HTTP listener; a non-zero migration
exit fails the boot loudly.

### Boot sequence

Every replica follows the same boot sequence:

1. Read env. If any `CORVID_*` variable is malformed, exit non-zero.
2. Connect to the DB. If the connection fails after 5 attempts (2 s, 4 s,
   8 s, 16 s, 32 s backoff), exit non-zero so the orchestrator restarts
   the pod.
3. Run migrations 0001 → 0005 (idempotent — see §6).
4. Open object-storage connection. Fail closed if not reachable.
5. Start the OTLP exporter and `/metrics` endpoint.
6. Start the durable-job pool. Resume any jobs left in `queue_jobs`
   with status `running` whose lease has expired; mark them `retryable`.
7. Start the scheduler. Compute next-fire timestamps for the three
   cron schedules.
8. Bind the HTTP listener.

If steps 1–4 fail, the binary exits and the orchestrator restarts it.
Steps 5–8 are logged but non-fatal — for example, a transient OTLP
outage does not stop the HTTP listener.

---

## 5. Secrets management

### Inventory

PKA stores four classes of secrets:

- **Database credentials** — Postgres URL with username + password, or
  the SQLite file path. Read at boot; never logged.
- **Object-storage credentials** — bucket name + access key + secret
  key. Read at boot; never logged.
- **Connector tokens** — per-tenant OAuth tokens for file sources
  (Google Drive, SharePoint, S3 IAM role ARN). Stored *only* in the
  `connector_tokens` table (encrypted with the master encryption key)
  and never in env.
- **Master encryption key** — 32-byte AES key used to encrypt
  connector tokens at rest. Stored in the secret manager (Vault, AWS
  Secrets Manager, Kubernetes Secret) and injected as
  `CORVID_MASTER_ENCRYPTION_KEY` env at boot.

### Where to store them

- **Production:** secret manager (Vault, AWS SM, GCP SM, K8s Secret).
  Never in env files committed to git, never in `configmap.yaml`,
  never in shell history. Use the IaC provider's templating to inject
  at deploy time.
- **Staging:** same as production but with a separate secret namespace
  so a staging compromise cannot leak prod tokens.
- **Local:** a developer-private `.env.local` (gitignored). Never check
  in real tokens; use mock connectors.

### Rotation

- **DB credentials** — rotate quarterly. The Postgres user has only
  the `pka_app` role; rotate by minting a new user, updating the
  secret, rolling the deployment, then dropping the old user.
- **Object-storage credentials** — rotate quarterly using the cloud
  provider's IAM key rotation flow.
- **Connector tokens** — refresh on the connector's own expiry. Treat a
  refresh failure as a connector-level failure (§12), not a service
  outage.
- **Master encryption key** — rotate annually or on suspected
  compromise. Rotation runs `corvid keys rotate
  --table=connector_tokens --old-key=<old> --new-key=<new>` which
  decrypts each token row with the old key and re-encrypts with the
  new. The rotation runs under a maintenance window because the
  HTTP listener is paused for the duration.

### What never gets logged

- Plaintext provider tokens.
- Plaintext document bytes (only fingerprints and provenance IDs).
- LLM prompt and completion bodies (only redacted prompt hashes).
- Approval payloads beyond the `approval_id` and the target type.

The redaction policy is enforced by the runtime's
`redaction_policy_hash` field on every trace span. If a span is missing
that field or it does not match the active policy, the span is dropped
and an operator alert fires (§9).

---

## 6. Migrations — apply, drift, rollback

### Applying migrations

```
corvid migrate --target=<sqlite|postgres> --dir=migrations
```

Migrations are idempotent: every migration begins with the schema-version
check from `0001_init.sql`'s `schema_version` table. Re-running a
migration that is already applied is a no-op.

### The five migrations

- `0001_init.sql` — base schema. Creates `tenants`, `users`, `roles`,
  `user_roles`, `permissions`, `schema_version`.
- `0002_knowledge.sql` — knowledge tables. Creates
  `knowledge_sources`, `knowledge_documents`, `knowledge_chunks`,
  `knowledge_embeddings`, `knowledge_ingestion_jobs`,
  `knowledge_feedback`.
- `0003_traces.sql` — trace lineage tables for OTLP-equivalent storage
  in-DB.
- `0004_auth.sql` — sessions, API keys, refresh tokens, audit_events
  for the auth surface.
- `0005_approvals_and_durable_jobs.sql` — `approvals`, `audit_events`
  (extended), `queue_jobs`, `queue_job_checkpoints`, `trace_lineage`
  (extended for approval correlation).

### Detecting drift

Drift = the DB schema diverges from `migrations/`. Every replica logs
its applied schema version on boot. The `corvid ops show` snapshot
includes the schema version and the file hash of every migration. To
detect drift:

```
corvid migrate --check --target=<sqlite|postgres> --dir=migrations
```

This command compares the DB's `schema_version` table to the
migration directory. Non-zero exit on any mismatch.

### Rollback

PKA migrations are **forward-only**. There is no `migrate down`. To
revert to a prior schema:

1. Restore the DB from the latest backup taken before the bad migration
   (see §7).
2. Roll back the deploy to the matching binary version.
3. File the regression and write a fix-forward migration.

Why forward-only: a typed migration that drops a column (e.g., during
the `0005` audit-event extension) cannot be safely reversed without
data loss. We prefer "restore + fix-forward" over "rollback the schema."

---

## 7. Backups — what, where, how often

### What gets backed up

- **DB**: full snapshot + WAL ship every 15 min (Postgres) or
  hot-copy every hour (SQLite).
- **Object storage**: enable bucket versioning + cross-region
  replication. Raw document bytes are content-addressed, so versioning
  is "free" — the same byte hash maps to the same object.
- **Index**: not backed up. The index is rebuildable from
  `knowledge_chunks` + `knowledge_embeddings` via `corvid jobs run
  --kind=rebuild_index --tenant=<id> --root=<id>`. Rebuilds take ~5 min
  for a 10k-source corpus.
- **Approvals + audit_events**: included in the DB backup. These rows
  are append-only and must never be lost — they are the canonical
  audit trail for every external write.

### Where

- DB backups: cloud-provider managed object storage in a *separate
  account* with cross-region replication and an immutable retention
  lock. The lock window is 90 days.
- Object-storage backups: native cross-region replication on the
  bucket. The replica bucket is read-only.

### How often

- DB: 15-min WAL ship, hourly base backup, daily verified restore.
- Object storage: continuous (replication).
- Verified-restore drill: every Monday at 09:00 UTC, the staging
  environment restores the most recent prod backup and runs the smoke
  suite from §3. Any failure is a Sev-2.

### Retention

- 90 days for DB backups.
- Indefinite for object storage (governed by data-retention policy per
  tenant; tenant offboarding triggers a hard delete with a documented
  legal hold check).
- 1 year for approval + audit_events rows even after tenant offboarding,
  subject to data-retention policy.

---

## 8. Logs and traces

### Log streams

- **App log** — structured JSON to stdout. One line per request, one
  line per job state transition, one line per error.
- **Trace log** — OTLP-equivalent JSONL written under
  `target/traces/<date>/<trace_id>.lineage.jsonl`. The schema is
  `corvid.trace.lineage.v1`. Every span carries `trace_id`, `span_id`,
  `kind`, `status`, `actor_id`, `tenant_id`, `replay_key`,
  `idempotency_key`, `guarantee_id`, `effect_ids`, `approval_id`,
  `data_classes`, `cost_usd`, `tokens_in`, `tokens_out`, `confidence`,
  `model_id`, `model_fingerprint`, `prompt_hash`,
  `retrieval_index_hash`, `input_fingerprint`, `output_fingerprint`,
  `redaction_policy_hash`.
- **Audit log** — written to `audit_events` table on every approval
  approve/deny, every external-write attempt, every cross-tenant
  share. Never logged to stdout.

### What every trace must include

Per the trace lineage schema:

- `trace_id` — stable across all spans of a request.
- `span_id` + `parent_span_id` — DAG structure.
- `kind` — one of `route`, `job`, `agent`, `prompt`, `tool`, `approval`,
  `db`, `retry`, `error`, `eval`, `review`.
- `status` — one of `ok`, `failed`, `denied`, `pending_review`,
  `replayed`, `redacted`.
- `replay_key` — populated for every replayable surface; required for
  durable-job spans and approval spans.
- `effect_ids` — the typed effects exercised by the span. Empty for
  observation-only spans.
- `redaction_policy_hash` — the hash of the active redaction policy.
  Must match the runtime's current policy hash.

### Where to look

| Symptom | First log to read |
|---|---|
| `POST /actions/*` returns 403 | `audit_events` for matching `approval_id`, then trace span with `kind=approval` |
| `nightly_reindex` did not run | `queue_jobs` for the latest row with `kind=nightly_reindex`, then trace span with `kind=job` |
| Answer missing citations | trace span with `kind=agent name=answer_with_provenance_*`, check `output_fingerprint` and citation count |
| `daily_provenance_audit` flagged a chain | the audit job's trace JSONL — every flagged `KnowledgeAnswer` is named with its `provenance_id` |
| Cross-tenant share suspicious | `audit_events` for the matching `approval:cross_tenant_share:*` row plus the trace span |

### Promoting a trace to an eval fixture

```
corvid eval promote --case=<case-id> \
    --in=target/traces/<date>/<trace_id>.lineage.jsonl \
    --out=evals/promoted/<case-id>.lineage-eval.json
```

The promotion writes a `corvid.eval.lineage_fixture.v1` record. PKA
ships three promoted fixtures in
[`evals/promoted/`](../evals/promoted/) (knowledge-demo,
knowledge-reindex, knowledge-cross-share); every regression replays
them.

---

## 9. Metrics and alerting

### What we export

`/metrics` exposes Prometheus-format counters and histograms:

- `pka_http_requests_total{route,status}` — counter.
- `pka_http_request_duration_seconds{route}` — histogram.
- `pka_job_runs_total{kind,status}` — counter.
- `pka_job_duration_seconds{kind}` — histogram.
- `pka_approval_decisions_total{label,decision}` — counter.
- `pka_approval_pending_age_seconds{label}` — gauge.
- `pka_provenance_audit_misses_total` — counter (must stay at 0).
- `pka_redaction_policy_mismatch_total` — counter (must stay at 0).
- `pka_replay_quarantine_violations_total{surface}` — counter (must
  stay at 0 outside of intentional fuzz tests).
- `pka_index_chunks{tenant,root}` — gauge.
- `pka_embedding_cache_hit_ratio` — gauge.

### Alerts

| Alert | Condition | Severity | First action |
|---|---|---|---|
| `pka_provenance_audit_misses_total > 0` | any miss in 24 h | Sev-2 | Quarantine the affected `KnowledgeAnswer`, page on-call |
| `pka_redaction_policy_mismatch_total > 0` | any mismatch | Sev-2 | Page on-call; freeze deploys until the policy hash is reconciled |
| `pka_replay_quarantine_violations_total > 0` | any non-test violation | Sev-1 | Page security on-call; pull the trace for the violating surface |
| `pka_approval_pending_age_seconds{label="CrossTenantIndexShare"} > 3600` | pending > 1 h | Sev-3 | Page the admin on-call to review the pending share |
| `pka_approval_pending_age_seconds{label="ExportTenantCorpus"} > 7200` | pending > 2 h | Sev-3 | Page the admin on-call |
| `pka_job_runs_total{kind="nightly_reindex",status="failed"} >= 2` | 2 failures in a row | Sev-3 | Inspect the latest `queue_jobs` row for the kind; re-enqueue manually after fix |
| `pka_job_runs_total{kind="daily_provenance_audit",status="failed"} >= 1` | any failure | Sev-2 | Audit cannot fail silently; rerun manually and investigate |
| `pka_http_request_duration_seconds{route="/answer/*"} p99 > 1.5s` | > 5 min sustained | Sev-3 | Check embedding-cache hit ratio; check DB query plan; consider scale-out |

### Where alerts go

- Sev-1 → pager + security channel + incident channel.
- Sev-2 → pager + operations channel.
- Sev-3 → operations channel.

The on-call rotation and channels are defined in the security model;
this runbook does not duplicate them.

---

## 10. Incident response — diagnose and recover

### Common incidents

#### A. `POST /actions/share/*` returns 403 unexpectedly

**Diagnose:**

1. Pull the trace for the request: `grep -r <request_id> target/traces/`.
2. Find the span with `kind=approval`. The `status` will be `denied` or
   `pending_review`.
3. Check `audit_events` for the matching `approval_id`. The denial
   `reason` column names which policy fired (expired, wrong role,
   missing co-sign, cost ceiling).

**Recover:**

- Wrong role: the requester needs an `Admin` or `Reviewer` role; have
  them request via the tenant admin.
- Expired approval: re-issue the approval request; the original
  contract has an `expires_ms` field.
- Cost ceiling exceeded: do not raise the ceiling in-place to push the
  action through; instead, file a contract change request — the
  ceiling is part of the typed approval and is reviewed at the contract
  level.

#### B. `daily_provenance_audit` flagged a `KnowledgeAnswer`

**Diagnose:**

1. The audit's trace JSONL names the flagged `provenance_id`.
2. Resolve the provenance ID in `knowledge_chunks` → `knowledge_documents`
   → `knowledge_sources`. The miss is where the chain breaks.
3. Common causes: the `KnowledgeSource` was deleted (offboarding), the
   `KnowledgeDocument` was re-indexed and the chunk ID changed, the
   embedding model rolled and the vector hash is stale.

**Recover:**

- Source deleted: this is correct; the answer should be quarantined. Mark
  the answer "stale" and notify the requester.
- Re-index orphaned: re-run `nightly_reindex` for the affected root,
  then re-issue the answer. Old answers in cache should be
  invalidated by `daily_provenance_audit`.
- Embedding model rolled: a model roll is a planned maintenance event
  that touches `pka_index_chunks` for every tenant. Roll only with a
  full re-index follow-up.

#### C. `nightly_reindex` job stuck

**Diagnose:**

1. `SELECT * FROM queue_jobs WHERE kind = 'nightly_reindex' ORDER BY
   started_at DESC LIMIT 5;`
2. If status is `running` and `lease_expires_at` is in the past, the
   replica that took the lease crashed.
3. If status is `retryable`, the job already raised and is awaiting the
   next retry per the exponential-jitter policy.

**Recover:**

- Expired lease: the scheduler will pick the job up at the next tick.
  No manual action needed; if it doesn't, run `corvid jobs run
  --kind=nightly_reindex --tenant=<id> --root=<id>` to force.
- Persistent failure: read the latest `queue_job_checkpoints` row for
  the job; the `error` column names the failure. Common: the source
  root is unreachable (connector token expired — §12), the index
  partition is full (extend storage), the embedding model crashed
  (restart the replica).

#### D. Cross-tenant share suspected

**Diagnose:**

1. `SELECT * FROM audit_events WHERE approval_id LIKE
   'approval:cross_tenant_share:%' ORDER BY occurred_at DESC LIMIT 50;`
2. For each event, confirm the co-sign trail: there must be a second
   `audit_events` row with `event_kind = 'approval.co_sign'` within 24 h
   of the original.
3. If a co-sign is missing, the receipt should already be revoked by
   `daily_provenance_audit`. Confirm in `audit_events` for an
   `event_kind = 'cross_tenant_share.revoked'` row.

**Recover:**

- Missing co-sign and revocation not yet fired: manually revoke. Then
  investigate why the audit job did not catch it (was the audit job
  itself failing — see incident B).
- Confirmed unauthorized share: page the security on-call. Pull all
  trace spans for the share's `trace_id`. Notify both tenants per
  the data-handling agreement.

#### E. Replay quarantine violation

Replay quarantine refuses live LLM, HTTP, store, or IO surfaces during
a Substitute-mode replay. A violation means a replay tried to call a
non-replayable surface.

**Diagnose:**

1. The `RuntimeError::QuarantineViolation { surface, detail }` is
   logged with the `replay_key` of the calling span.
2. Pull the source for the agent that raised. It is almost always a
   missing `@replayable` attribute on a downstream call, or a
   non-deterministic input not captured in the replay key.

**Recover:**

- Fix the agent to be replayable (add `@replayable`, capture the
  non-determinism in the replay key) and re-run the replay.
- Until fixed, do not promote any fixture that exercises that path —
  promotion would freeze the violation into the eval corpus.

---

## 11. Rollback procedures

### Rolling back the binary

```
flyctl releases list --app personal-knowledge-agent
flyctl deploy --image registry.fly.io/personal-knowledge-agent:<prior-tag>
```

or in Kubernetes:

```
kubectl rollout undo deployment/personal-knowledge-agent -n pka
kubectl rollout status deployment/personal-knowledge-agent -n pka
```

Verify the rollback by running the smoke from §3 against the production
host. The `corvid ops show` snapshot must match the snapshot archived
for the prior release (under `ops/snapshots/`).

### Rolling back a migration

PKA migrations are forward-only (§6). Rollback = restore DB from
backup taken before the bad migration, then roll back the binary, then
fix-forward.

### Rolling back an approval contract

Approval contracts are typed and live in the binary; rolling back the
binary rolls back the contract. There is no way to roll back a
contract without rolling back the binary — the contract's `version`
field is checked at every approval request and refuses mismatches.

### Rolling back a connector mode switch

If `CORVID_CONNECTOR_MODE` was flipped from `mock` to `live` in error,
flip it back. The runtime re-reads connector mode on every connector
call, so the change takes effect immediately — there is no need to
restart the binary. The trace span for any in-flight `live` call still
records the live attempt; review those spans for any external write
that escaped before the flip-back.

---

## 12. Connector mode operations

### Modes

PKA's three connectors (`files_connector`, `local_embed_connector`,
`index_connector`) honor `CORVID_CONNECTOR_MODE`:

- `mock` (default) — deterministic fixtures, no network. Used by
  `corvid eval`, `corvid tour`, smoke suites, and any environment
  where reproducibility matters more than fidelity.
- `live` — real provider calls (Google Drive, S3, SharePoint, local FS).
  Requires a valid connector token per tenant (§5).
- `record` — proxies to `live` but writes the raw response to a fixture
  file under `target/recordings/`. Used to capture a new replay
  fixture against a real provider.
- `replay` — reads from `target/recordings/` instead of the live
  provider. Used to replay a captured fixture deterministically in a
  test environment.

### Switching modes

```
# in the deploy environment
export CORVID_CONNECTOR_MODE=live
corvid ops show | jq '.connector_mode'   # must print "live"
```

Mode changes are *not* logged to `audit_events` (mode is a deploy-time
concern, not a per-request decision); they appear in `corvid ops show`
and the boot log. A mode change to `live` without per-tenant tokens
configured is a configuration error and causes all `files_read` calls
to fail closed.

### Per-tenant connector tokens

```
corvid connectors token put --tenant=<id> --connector=files \
    --token-file=<path>
corvid connectors token list --tenant=<id>
corvid connectors token revoke --tenant=<id> --connector=files
```

Tokens are encrypted at rest with the master encryption key (§5).
Revoking a token is immediate — the next `files_read` call for that
tenant fails closed.

### When a connector token expires

The `files_connector` raises a typed error. The job retry policy
(`exponential_jitter`, 5 attempts) absorbs transient errors; a refresh
failure that survives all retries lands in the dead-letter queue. The
on-call playbook is to mint a new token and rerun the job. The job's
`replay_key` makes a manual rerun idempotent.

---

## 13. Approval queue operations

### Inspecting the queue

```
corvid approvals list --pending
corvid approvals show --id=<approval_id>
corvid approvals show --id=<approval_id> --include-trace
```

The queue lives in the `approvals` table. Every row carries `label`,
`version`, `action`, `target`, `required_role`, `max_cost_usd`,
`data_class`, `irreversible`, `expires_ms`, plus the
`approval_id` that joins to `audit_events` and `trace_lineage`.

### Approving

```
corvid approvals approve --id=<approval_id> --as=<actor_id> --note=<text>
```

The CLI checks that:

- The approver's role matches `required_role`.
- The approval is `pending_review`, not expired, not revoked.
- The approver is from the same tenant as the request (except for
  `CrossTenantIndexShare`, which is a deliberate cross-tenant action
  and uses the *source* tenant's admin role).

A successful approve writes:

- `approvals.status = 'approved'`, sets `approved_by`, `approved_at`.
- An `audit_events` row with `event_kind = 'approval.approve'`.
- A trace span with `kind = approval`, `status = ok`.

### Denying

```
corvid approvals deny --id=<approval_id> --as=<actor_id> --reason=<text>
```

A deny is also logged to `audit_events` with `event_kind =
'approval.deny'`. The corresponding agent call returns the typed denial
to the caller; the caller cannot retry without filing a new approval.

### Per-contract considerations

- **ShareAnswerToChat** — Reviewer role. Inspect the channel ID and
  verify the channel is one the requester is a member of. Cost
  ceiling $0.05 covers the chat connector call.
- **ShareAnswerViaEmail** — Reviewer role. Inspect the recipient
  domain; verify the recipient is on the tenant's allowlist if the
  tenant has one. Cost ceiling $0.05.
- **PublishAuthoritativeAnswer** — Reviewer role. The answer becomes
  the "authoritative" entry for that question in the KB; deny if the
  answer's `Grounded<T>` chain is incomplete or its confidence is
  below the policy floor (default 0.85). Cost ceiling $0.10.
- **ExportTenantCorpus** — Admin role. Verify the export destination
  matches the data-handling agreement for that tenant (S3 bucket
  ownership, encryption, retention). Co-sign required within 24 h.
  Cost ceiling $0.25.
- **CrossTenantIndexShare** — Admin role. Verify a signed
  data-sharing agreement exists between the source and target
  tenants and that the target tenant's admin has accepted the share.
  Co-sign required within 24 h by the *target* tenant's admin. Cost
  ceiling $0.25. This is the highest-blast-radius contract and any
  doubt should resolve as a deny.

### Co-signature trail

`ExportTenantCorpus` and `CrossTenantIndexShare` require a second
approval within 24 h. The co-sign is recorded as a separate
`audit_events` row (`event_kind = 'approval.co_sign'`). The
`daily_provenance_audit` job revokes any receipt whose co-sign is
missing past the 24 h window — the revocation is itself logged.

### Pending queue SLOs

- `ShareAnswerToChat` / `ShareAnswerViaEmail` — pending more than 1 h
  is a Sev-3 page (requester likely waiting on a publish flow).
- `PublishAuthoritativeAnswer` — pending more than 4 h is a Sev-3 page.
- `ExportTenantCorpus` — pending more than 2 h is a Sev-3 page.
- `CrossTenantIndexShare` — pending more than 1 h is a Sev-3 page; the
  cross-tenant nature means a hung approval is suspicious.

---

## 14. Durable jobs and cron operations

### The three jobs

| Kind | Cron | Tenant scope | Effects | Approval | Budget |
|---|---|---|---|---|---|
| `nightly_reindex` | `0 2 * * *` America/New_York | per tenant per root | `files_read`, `local_embed`, `index_write` | none | $0.50 |
| `weekly_feedback_batch` | `0 6 * * 1` America/New_York | per tenant per window | `knowledge_llm` | none | $0.50 |
| `daily_provenance_audit` | `0 3 * * *` America/New_York | per tenant per day | `files_read` | none | $0.50 |

None of the three carries an external-write effect — they are all
internal jobs that read, embed, and verify. External writes only happen
on the typed `POST /actions/*` routes, which are approval-gated by
construction.

### Job SLOs

- `nightly_reindex` p99: 5 min for a 10k-source corpus per tenant per
  root. Partition by root if a single root grows past 50k sources.
- `weekly_feedback_batch` p99: 10 min for a 1k-feedback-row batch per
  tenant per window.
- `daily_provenance_audit` p99: 60 s for a 100k-answer-row scan per
  tenant per day. **If the audit exceeds 60 s, file an incident** —
  the audit is the integrity gate and slow audits leak risk.

### Manual triggers

```
corvid jobs run --kind=nightly_reindex --tenant=tenant-1 --root=notes
corvid jobs run --kind=weekly_feedback_batch --tenant=tenant-1 --window=business_week
corvid jobs run --kind=daily_provenance_audit --tenant=tenant-1 --day=2026-05-27
```

The `replay_key` is computed from `kind:tenant:scope` and is the
durable-job idempotency key. Two manual triggers with the same
arguments coalesce into one queued job — the second is a no-op.

### Retry policy

All three jobs use `exponential_jitter` with 5 attempts, base 1 s, cap
10 min. The dead-letter queue is named
`personal_knowledge_agent.dead_letter`. Dead-lettered jobs are visible
via:

```
corvid jobs list --dead-letter
corvid jobs replay --id=<job_id>   # replays from the last checkpoint
```

### Checkpoints

Long-running jobs (especially `nightly_reindex` on large corpora) write
checkpoints to `queue_job_checkpoints` after every batch. If a replica
crashes mid-job, the lease expires and another replica resumes from the
last checkpoint. The job kind is responsible for making each batch
idempotent.

### Scheduler ownership

The scheduler uses a per-job-kind advisory lock in the DB so only one
replica fires each cron tick. If the locked replica disappears, the
lock is released by the DB and the next tick is fired by whichever
replica wins the next lock acquisition.

### Disabling a schedule

```
corvid jobs schedule disable --kind=<kind>
corvid jobs schedule enable --kind=<kind>
```

Disabling pauses the schedule but does not affect in-flight jobs. The
`audit_events` row for the disable is required for compliance; the CLI
writes one automatically.

---

## 15. Disaster recovery

### Catastrophic DB loss

1. **Stop traffic.** Take the deploy out of the load balancer.
2. **Restore the latest backup** to a new DB instance (§7). Verify the
   restore by running `corvid migrate --check` against the restored
   instance.
3. **Replay any post-backup external writes** by reading
   `audit_events` from object-storage backups (if the audit table is
   in the same DB, you cannot — see "audit redundancy" below). For
   PKA, the canonical post-backup recovery path is to accept the
   data-loss window of the WAL ship interval (15 min for Postgres) and
   re-issue any missing approvals manually with a clear note.
4. **Point the deploy at the restored DB.** Roll the deploy.
5. **Run the smoke suite (§3) + the eval suite + a manual audit job
   for the day of loss.**
6. **Write the post-incident report.** Tenant impact is bounded by the
   WAL ship interval; document the exact window.

### Audit redundancy

`audit_events` lives in the same DB as `approvals` and
`queue_jobs`. For maximum redundancy in regulated tenants, enable the
audit-event forwarder which writes a copy of every `audit_events` row
to an append-only object-storage log under
`s3://<bucket>/audit/<tenant>/<date>.jsonl`. The forwarder is
configured per tenant via the `audit.forwarders` row in the tenants
config; see the security model.

### Catastrophic index loss

The index is rebuildable. To recover:

```
for tenant in $(corvid tenants list --format=ids); do
  for root in $(corvid sources list --tenant=$tenant --format=root-ids); do
    corvid jobs run --kind=rebuild_index --tenant=$tenant --root=$root
  done
done
```

A rebuild of a 10k-source corpus takes ~5 min. During the rebuild,
`/search/*` and `/answer/*` for that tenant return an explicit
`index_rebuilding` envelope rather than partial results.

### Catastrophic object-storage loss

Object storage holds raw document bytes. Loss means content is
recoverable only from the source connector (the user's Drive, S3, etc.).
Re-ingest:

```
for tenant in $(corvid tenants list --format=ids); do
  for root in $(corvid sources list --tenant=$tenant --format=root-ids); do
    corvid jobs run --kind=nightly_reindex --tenant=$tenant --root=$root
  done
done
```

Re-ingest is full, not incremental, because the content-hash diff
basis is gone. Plan for a multi-hour rebuild on large tenants.

### Loss of the master encryption key

The master encryption key encrypts connector tokens. If lost,
connector tokens are unrecoverable but the index, embeddings, and
audit trail are intact (they are not encrypted at the application
layer — they rely on DB and object-storage encryption at rest).

Recovery: rotate the key (§5), force every tenant to re-mint their
connector tokens. This is a multi-day operation across all tenants.
Test the recovery quarterly in staging.

### RPO / RTO targets

- **RPO** (recovery point objective): 15 min (WAL ship interval).
- **RTO** (recovery time objective): 1 h for a regional DB failure
  (cross-region replica promote); 4 h for catastrophic DB loss
  (restore from backup); 24 h for index rebuild from a cold start.

---

## 16. Appendix — reference data

### Schema manifest

`KnowledgeSchemaManifest("personal_knowledge_agent", 5, 18, 3, 3, 5,
"mock")`:

- 5 migrations: `0001_init`, `0002_knowledge`, `0003_traces`,
  `0004_auth`, `0005_approvals_and_durable_jobs`.
- 18 tables: see §2.
- 3 connectors: `files_connector`, `local_embed_connector`,
  `index_connector`.
- 3 durable jobs: `nightly_reindex`, `weekly_feedback_batch`,
  `daily_provenance_audit`.
- 5 approval contracts: `ShareAnswerToChat`, `ShareAnswerViaEmail`,
  `PublishAuthoritativeAnswer`, `ExportTenantCorpus`,
  `CrossTenantIndexShare`.
- Default mode: `mock`.

### Effect catalog

| Effect | Cost | Trust | Data class | Used by |
|---|---|---|---|---|
| `files_read` | $0.01 | local | private | ingest, reindex, audit |
| `local_embed` | $0.02 | local | private | ingest, reindex, search |
| `index_write` | $0.01 | local | private | ingest, reindex |
| `knowledge_llm` | $0.20 | bounded | private | feedback batch |
| `chat_share` | $0.01 | human_required | external | `share_answer_to_chat` |
| `email_share` | $0.02 | human_required | external | `share_answer_via_email` |
| `kb_publish` | $0.02 | human_required | external | `publish_authoritative_answer` |
| `corpus_export` | $0.05 | human_required | external | `export_tenant_corpus` |
| `cross_tenant_share` | $0.03 | human_required | external | `cross_tenant_index_share` |

### Route catalog

| Method | Route | Returns | Effects | Approval |
|---|---|---|---|---|
| GET | `/config` | `KnowledgeConfig` | none | none |
| GET | `/schema` | `KnowledgeSchemaManifest` | none | none |
| GET | `/ingest/mock` | `KnowledgeIngestionResult` | `files_read`, `local_embed`, `index_write` | none |
| GET | `/search/mock` | `KnowledgeSearchHit` | `files_read`, `local_embed` | none |
| GET | `/answer/mock` | `KnowledgeAnswer` | `files_read`, `local_embed` | none |
| GET | `/feedback/eval/mock` | `KnowledgeFeedbackEval` | none | none |
| POST | `/auth/session/login` | `LoginResponse` | none | none |
| POST | `/auth/api-key/login` | `ApiKeyLoginResponse` | none | none |
| GET | `/auth/status` | `AuthStatusResponse` | none | none |
| GET | `/auth/api-key/status` | `AuthStatusResponse` | none | none |
| GET | `/jobs/nightly-reindex/mock` | `KnowledgeJobRun` | `files_read`, `local_embed`, `index_write` | none |
| GET | `/jobs/weekly-feedback-batch/mock` | `KnowledgeJobRun` | `knowledge_llm` | none |
| GET | `/jobs/daily-provenance-audit/mock` | `KnowledgeJobRun` | `files_read` | none |
| POST | `/actions/share/chat` | `ShareAnswerToChatReceipt` | `chat_share` | `ShareAnswerToChat` |
| POST | `/actions/share/email` | `ShareAnswerViaEmailReceipt` | `email_share` | `ShareAnswerViaEmail` |
| POST | `/actions/publish/authoritative` | `PublishAuthoritativeAnswerReceipt` | `kb_publish` | `PublishAuthoritativeAnswer` |
| POST | `/actions/export/corpus` | `ExportTenantCorpusReceipt` | `corpus_export` | `ExportTenantCorpus` |
| POST | `/actions/share/cross-tenant` | `CrossTenantIndexShareReceipt` | `cross_tenant_share` | `CrossTenantIndexShare` |

### Adversarial corpus

PKA ships six adversarial fixtures under
[`adversarial/`](../adversarial/). The `corvid check` compiler refuses
every one with `E0101` (missing approval boundary):

- `ungated_share_chat.cor` — calls `share_answer_to_chat` without
  `approve ShareAnswerToChat(...)`.
- `ungated_share_email.cor` — calls `share_answer_via_email` without
  `approve ShareAnswerViaEmail(...)`.
- `ungated_publish_authoritative.cor` — calls
  `publish_authoritative_answer` without
  `approve PublishAuthoritativeAnswer(...)`.
- `ungated_export_corpus.cor` — calls `export_tenant_corpus` without
  `approve ExportTenantCorpus(...)`.
- `ungated_cross_tenant_share.cor` — calls `cross_tenant_index_share`
  without `approve CrossTenantIndexShare(...)`.
- `raw_text_committed.cor` — attempts to write raw document text to a
  log surface bypassing `redaction_policy_hash`.

The CI verify suite asserts each fixture exits `1` with `E0101`. Any
green build on these fixtures is a Sev-1 — the compiler-enforced
approval gate is the foundation of PKA's safety claims.

### Trace lineage schema reference

`corvid.trace.lineage.v1`:

- `kind ∈ { route, job, agent, prompt, tool, approval, db, retry,
  error, eval, review }`
- `status ∈ { ok, failed, denied, pending_review, replayed, redacted }`
- Required fields per span: `schema`, `trace_id`, `span_id`,
  `parent_span_id`, `kind`, `name`, `status`, `started_ms`,
  `ended_ms`, `tenant_id`, `actor_id`, `request_id`, `replay_key`,
  `idempotency_key`, `guarantee_id`, `effect_ids`, `approval_id`,
  `data_classes`, `cost_usd`, `tokens_in`, `tokens_out`, `confidence`,
  `latency_ms`, `model_id`, `model_fingerprint`, `prompt_hash`,
  `retrieval_index_hash`, `input_fingerprint`, `output_fingerprint`,
  `redaction_policy_hash`.

### Promoted eval fixtures

PKA ships three promoted fixtures under
[`evals/promoted/`](../evals/promoted/):

- `knowledge-demo.lineage-eval.json` — mock ingest + grounded search +
  answer with provenance.
- `knowledge-reindex.lineage-eval.json` — `nightly_reindex` durable-job
  end-to-end including queue checkpoint.
- `knowledge-cross-share.lineage-eval.json` — `CrossTenantIndexShare`
  approval, audit, co-sign trail.

### Environment variable reference

| Variable | Default | Purpose |
|---|---|---|
| `CORVID_CONNECTOR_MODE` | `mock` | Connector mode (mock/live/record/replay) |
| `CORVID_STORAGE_MODE` | `sqlite` | DB backend (sqlite/postgres) |
| `CORVID_LOCAL_ONLY` | `true` | If true, refuses any network call |
| `CORVID_DB_URL` | `target/pka.db` | DB connection string |
| `CORVID_OBJECT_STORE_URL` | `target/object-store` | Object-storage URL |
| `CORVID_MASTER_ENCRYPTION_KEY` | — | AES-256 key for connector-token encryption |
| `CORVID_OTLP_ENDPOINT` | — | OTLP exporter target |
| `CORVID_METRICS_BIND` | `0.0.0.0:9486` | Prometheus `/metrics` bind |
| `CORVID_TRACE_DIR` | `target/traces` | Trace JSONL output directory |
| `CORVID_REDACTION_POLICY_HASH` | computed at boot | Active redaction policy hash |

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
