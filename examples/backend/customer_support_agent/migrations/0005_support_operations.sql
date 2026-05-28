-- Backing tables for the three operational write surfaces D-CS-2 adds.
-- Slice 35V2-P42-D-LR-app-maturity-CustomerSupport-2.
--
-- Each table records the result of executing a support operation a
-- human approved through its typed contract (EscalateTicket,
-- CloseTicket, ApplyAccountCredit). Customer-facing replies and refunds
-- already have their audit trail in support_approval_audits; these
-- rows join to approvals.id by approval_id.

CREATE TABLE support_escalations (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    ticket_id TEXT NOT NULL REFERENCES support_tickets(id) ON DELETE CASCADE,
    escalation_tier TEXT NOT NULL,
    reason_fingerprint TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_support_escalations_tenant ON support_escalations(tenant_id);
CREATE INDEX idx_support_escalations_approval ON support_escalations(approval_id);

CREATE TABLE support_ticket_closures (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    ticket_id TEXT NOT NULL REFERENCES support_tickets(id) ON DELETE CASCADE,
    resolution_fingerprint TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_ticket_closures_tenant ON support_ticket_closures(tenant_id);
CREATE INDEX idx_ticket_closures_approval ON support_ticket_closures(approval_id);

CREATE TABLE support_account_credits (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    customer_fingerprint TEXT NOT NULL,
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    currency TEXT NOT NULL,
    reason_fingerprint TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_account_credits_tenant ON support_account_credits(tenant_id);
CREATE INDEX idx_account_credits_approval ON support_account_credits(approval_id);
