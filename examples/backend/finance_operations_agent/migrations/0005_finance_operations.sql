-- Backing tables for the four operational write surfaces D-Fin-2 adds.
-- Slice 35V2-P42-D-LR-app-maturity-Finance-2.
--
-- Each table records the result of executing a financial operation a
-- human approved through its typed contract (CancelSubscription,
-- DisputeTransaction, ExportFinancialReport, ScheduleRecurringPayment).
-- The agent never advises; these rows are the audit trail of executed
-- operations, joined to approvals.id by approval_id.

CREATE TABLE finance_subscription_cancellations (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    subscription_id TEXT NOT NULL REFERENCES finance_subscriptions(id) ON DELETE CASCADE,
    reason_fingerprint TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_sub_cancellations_tenant ON finance_subscription_cancellations(tenant_id);
CREATE INDEX idx_sub_cancellations_approval ON finance_subscription_cancellations(approval_id);

CREATE TABLE finance_transaction_disputes (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES finance_accounts(id) ON DELETE CASCADE,
    transaction_fingerprint TEXT NOT NULL,
    dispute_reason TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_disputes_tenant ON finance_transaction_disputes(tenant_id);
CREATE INDEX idx_disputes_approval ON finance_transaction_disputes(approval_id);

CREATE TABLE finance_report_exports (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    report_kind TEXT NOT NULL,
    export_destination TEXT NOT NULL,
    redaction_policy_hash TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_report_exports_tenant ON finance_report_exports(tenant_id);
CREATE INDEX idx_report_exports_approval ON finance_report_exports(approval_id);

CREATE TABLE finance_recurring_payments (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    source_account_id TEXT NOT NULL REFERENCES finance_accounts(id) ON DELETE CASCADE,
    payee_fingerprint TEXT NOT NULL,
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    currency TEXT NOT NULL,
    cadence TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_recurring_payments_tenant ON finance_recurring_payments(tenant_id);
CREATE INDEX idx_recurring_payments_approval ON finance_recurring_payments(approval_id);
