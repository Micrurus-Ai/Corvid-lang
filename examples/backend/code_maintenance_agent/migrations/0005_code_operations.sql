-- Backing tables for the three operational write surfaces D-CM-2 adds.
-- Slice 35V2-P42-D-LR-app-maturity-CodeMaintenance-2.
--
-- Each table records the result of executing a code-maintenance
-- operation a human approved through its typed contract
-- (OpenPullRequest, MergePullRequest, TagRelease). Review comments and
-- patch proposals already have their audit trail in
-- code_approval_audits; these rows join to approvals.id by approval_id.

CREATE TABLE code_pull_requests (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL REFERENCES code_repositories(id) ON DELETE CASCADE,
    branch_fingerprint TEXT NOT NULL,
    patch_fingerprint TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_pull_requests_tenant ON code_pull_requests(tenant_id);
CREATE INDEX idx_pull_requests_approval ON code_pull_requests(approval_id);

CREATE TABLE code_merges (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL REFERENCES code_repositories(id) ON DELETE CASCADE,
    pull_request_id TEXT NOT NULL REFERENCES code_pull_requests(id) ON DELETE CASCADE,
    merge_commit_sha TEXT NOT NULL,
    merge_strategy TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_merges_tenant ON code_merges(tenant_id);
CREATE INDEX idx_merges_approval ON code_merges(approval_id);

CREATE TABLE code_releases (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL REFERENCES code_repositories(id) ON DELETE CASCADE,
    tag_fingerprint TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_releases_tenant ON code_releases(tenant_id);
CREATE INDEX idx_releases_approval ON code_releases(approval_id);
