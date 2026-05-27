-- Auth surface for the Personal Knowledge Agent.
-- Slice 35V2-P42-D-LR-app-maturity-PKA-1.
--
-- Mirrors the Phase 39 auth model: per-tenant identity, sessions
-- bound to a CSRF cookie, API keys with Argon2id-hashed material,
-- and a roles/permissions table that gates dangerous knowledge
-- writes (cross-tenant share, external publish, corpus export).

CREATE TABLE tenants (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    plan TEXT NOT NULL,
    region TEXT NOT NULL,
    data_retention_days INTEGER NOT NULL CHECK (data_retention_days > 0),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE users (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    display_name TEXT NOT NULL,
    timezone TEXT NOT NULL,
    role TEXT NOT NULL,
    preferences_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (tenant_id, email)
);

CREATE TABLE roles (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    UNIQUE (tenant_id, name)
);

CREATE TABLE user_roles (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    granted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, role_id)
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    csrf_binding_id TEXT NOT NULL,
    issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms > issued_at_ms),
    revoked_at_ms INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_tenant_id ON sessions(tenant_id);

CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,
    actor_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    scope TEXT NOT NULL,
    hash_algorithm TEXT NOT NULL DEFAULT 'argon2id',
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms > 0),
    revoked_at_ms INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_api_keys_actor_id ON api_keys(actor_id);
CREATE INDEX idx_api_keys_tenant_id ON api_keys(tenant_id);

CREATE TABLE permissions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    UNIQUE (tenant_id, name)
);
