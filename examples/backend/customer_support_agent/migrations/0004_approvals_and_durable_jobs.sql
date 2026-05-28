-- Approvals + durable jobs + trace lineage for the Customer Support Agent.
-- Slice 35V2-P42-D-LR-app-maturity-CustomerSupport-1.
--
-- Approvals gate the dangerous support operations. SendSupportReply +
-- IssueSupportRefund already ship; D-CS-2 adds three more
-- (EscalateTicket, CloseTicket, ApplyAccountCredit), each
-- developer-authored with its own role and reversibility. Durable jobs
-- + checkpoints support the 3 cron jobs (sla_breach_scan,
-- nightly_csat_rollup, policy_reindex). Trace lineage is the storage
-- backend for the per-job JSONL trace files.

CREATE TABLE approvals (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    requester_actor_id TEXT NOT NULL,
    contract_id TEXT NOT NULL,
    contract_version TEXT NOT NULL,
    contract_action TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    required_role TEXT NOT NULL,
    max_cost_usd REAL NOT NULL CHECK (max_cost_usd >= 0),
    data_class TEXT NOT NULL,
    irreversible INTEGER NOT NULL CHECK (irreversible IN (0, 1)),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms > 0),
    decided_at_ms INTEGER NOT NULL DEFAULT 0,
    decided_by_actor_id TEXT NOT NULL DEFAULT '',
    decision TEXT NOT NULL DEFAULT 'pending'
        CHECK (decision IN ('pending', 'approved', 'denied', 'expired')),
    decision_reason TEXT NOT NULL DEFAULT '',
    trace_id TEXT NOT NULL,
    replay_key TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_approvals_tenant_id ON approvals(tenant_id);
CREATE INDEX idx_approvals_decision ON approvals(decision);
CREATE INDEX idx_approvals_contract_action ON approvals(contract_action);

CREATE TABLE audit_events (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    event_kind TEXT NOT NULL,
    status_before TEXT NOT NULL,
    status_after TEXT NOT NULL,
    reason TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_audit_events_tenant_id ON audit_events(tenant_id);
CREATE INDEX idx_audit_events_target ON audit_events(target_kind, target_id);

CREATE TABLE queue_jobs (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    task TEXT NOT NULL,
    payload TEXT NOT NULL,
    input_schema TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_retries INTEGER NOT NULL CHECK (max_retries >= 0),
    budget_usd REAL NOT NULL CHECK (budget_usd >= 0),
    effect_summary TEXT NOT NULL DEFAULT '',
    replay_key TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    output_kind TEXT NOT NULL DEFAULT '',
    output_fingerprint TEXT NOT NULL DEFAULT '',
    failure_kind TEXT NOT NULL DEFAULT '',
    failure_fingerprint TEXT NOT NULL DEFAULT '',
    next_run_ms INTEGER NOT NULL DEFAULT 0,
    lease_owner TEXT NOT NULL DEFAULT '',
    lease_expires_ms INTEGER NOT NULL DEFAULT 0,
    approval_id TEXT NOT NULL DEFAULT '',
    approval_expires_ms INTEGER NOT NULL DEFAULT 0,
    approval_reason TEXT NOT NULL DEFAULT '',
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL,
    UNIQUE (tenant_id, idempotency_key)
);
CREATE INDEX idx_queue_jobs_status ON queue_jobs(status);
CREATE INDEX idx_queue_jobs_lease ON queue_jobs(lease_owner, lease_expires_ms);
CREATE INDEX idx_queue_jobs_next_run ON queue_jobs(next_run_ms);

CREATE TABLE queue_job_checkpoints (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES queue_jobs(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    payload TEXT NOT NULL,
    payload_fingerprint TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    UNIQUE (job_id, sequence)
);
CREATE INDEX idx_checkpoints_job_id ON queue_job_checkpoints(job_id);

CREATE TABLE trace_lineage (
    trace_id TEXT NOT NULL,
    span_id TEXT NOT NULL,
    parent_span_id TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL DEFAULT '',
    request_id TEXT NOT NULL DEFAULT '',
    replay_key TEXT NOT NULL DEFAULT '',
    idempotency_key TEXT NOT NULL DEFAULT '',
    guarantee_id TEXT NOT NULL DEFAULT '',
    approval_id TEXT NOT NULL DEFAULT '',
    cost_usd REAL NOT NULL DEFAULT 0,
    started_ms INTEGER NOT NULL,
    ended_ms INTEGER NOT NULL,
    model_id TEXT NOT NULL DEFAULT '',
    model_fingerprint TEXT NOT NULL DEFAULT '',
    redaction_policy_hash TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (trace_id, span_id)
);
CREATE INDEX idx_trace_lineage_tenant_id ON trace_lineage(tenant_id);
CREATE INDEX idx_trace_lineage_kind ON trace_lineage(kind);
