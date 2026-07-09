# Persistence

Persistence in v1.0 ships through three surfaces:

1. `corvid migrate` — the CLI that owns migration discovery,
   drift detection, applied-state tracking, and SQL execution.
2. `crates/corvid-runtime/src/db.rs` — the host-side runtime
   that holds SQLite + Postgres connections and exposes typed
   query / transaction primitives to embedding Rust code.
3. `agent` declarations + `tool` declarations — the Corvid
   source-level path. Agents call tools that wrap DB queries;
   the tool implementation lives on the host side. The
   source-level `db.query(...)` / `db.transaction(...)` ergonomic
   surface is post-v1.0 — today persistence enters Corvid through
   tool declarations.

## CLI: migrations

The shipped `corvid migrate` surface (verified against the
Phase 37 spot-check, 2026-05-17):

```sh
corvid migrate status \
    --dir       examples/backend/state_app/migrations \
    --state     /var/lib/corvid/migration_state \
    --database  /var/lib/corvid/state_app.sqlite
```

Reports applied / pending / drifted migrations + the CI-safe
mutation-intent field (`none` for status, `apply_pending` for up).

```sh
corvid migrate up \
    --dir       examples/backend/state_app/migrations \
    --state     /var/lib/corvid/migration_state \
    --database  /var/lib/corvid/state_app.sqlite
```

Executes pending SQL transactionally + records the applied state.

```sh
corvid migrate down \
    --dir       examples/backend/state_app/migrations \
    --state     /var/lib/corvid/migration_state \
    --database  /var/lib/corvid/state_app.sqlite
```

Executes a reviewed rollback SQL from `migrations/down/<name>.down.sql`.
Fails clearly when no rollback file exists — the CLI refuses to
guess. The "always-ship a `.down.sql` next to every `.up.sql`"
discipline is the operator's contract.

`--dry-run` is accepted on every subcommand and emits the same
report without mutating state.

## Migration file layout

```text
migrations/
├── 0001_core_state.sql        # forward migration
└── down/
    └── 0001_core_state.down.sql   # paired rollback
```

The `examples/backend/state_app/migrations/0001_core_state.sql`
fixture is the canonical reference: 6 tables (users / tasks /
approvals / traces / connector_tokens / agent_state) the audit-
log + approval-queue + connector-token surfaces all build on.

## Tool declarations: DB queries from Corvid source

Agents reach the database through `tool` declarations whose
implementation is a host-side Rust function. The Corvid source
side declares the typed interface; the host-side wires the
SQL.

```corvid
effect db_read_effect:
    cost: $0.001
    data: external_input

effect db_write_effect:
    cost: $0.001
    data: external_input

type UserRecord:
    id: String
    email: String

tool fetch_user(id: String) -> UserRecord uses db_read_effect
tool issue_refund(order_id: String) -> Nothing dangerous uses db_write_effect

agent process_refund(order_id: String) -> Nothing uses db_read_effect, db_write_effect:
    approve IssueRefund(order_id)
    issue_refund(order_id)
```

The host-side implementation of `fetch_user` + `issue_refund`
lives in your application's Rust code and calls the
`corvid_runtime::db` query / transaction primitives. The
typechecker enforces the effect row + the approve boundary; the
runtime enforces the actual SQL execution + transactional
semantics + audit-log writes.

## Audit-log schema

The canonical audit-log schema lives in the runtime's
production-ready migration set. Every approval transition writes
an audit event automatically — your agent doesn't need to call
`audit_log.write(...)` explicitly; the runtime intercepts
`approve` + dangerous tool calls.

To inspect:

```sh
corvid approvals inspect <id>            # one approval + its audit trail
corvid approvals export --tenant <t> --since <iso>   # full audit log
```

The exported JSONL is the source of truth a compliance review
consumes.

## Encrypted token storage

API keys + connector OAuth tokens are Argon2id-hashed at rest
(`crates/corvid-runtime/src/auth/api_keys.rs`). The runtime
exposes encrypted storage; tokens never appear in traces, error
messages, or logs (the redaction is enforced by
`observability.redaction_determinism`, see
`crates/corvid-runtime/src/lineage_redact.rs`). `corvid doctor`
validates that the host encryption key is present and well-formed
without printing it.

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| `corvid migrate up/down/status` execute against real SQLite + Postgres | `replay.deterministic_pure_path` (covers the migrate-as-deterministic property) | Static | `crates/corvid-cli/tests/migrate.rs` |
| API keys hashed at rest | `auth.api_key_at_rest_hashed` | RuntimeChecked | `crates/corvid-runtime/src/auth/api_keys.rs` |
| Audit-log redaction is deterministic | `observability.redaction_determinism` | RuntimeChecked | `crates/corvid-runtime/src/lineage_redact.rs` |
| Source-level `db.query(...)` / `db.transaction(...)` syntax | n/a | post-v1.0 | filed as ergonomic improvement; today persistence enters Corvid through tool declarations |
