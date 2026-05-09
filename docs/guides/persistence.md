# Persistence

## What `std.db` gives you

- SQLite + Postgres connection, query, transaction, and prepared-
  statement primitives with typed row decode.
- A migration system with checked-in SQL migrations, drift detection,
  checksum validation, and CI-safe dry runs.
- An audit-log schema for AI actions: who/what/why, prompt version,
  model, tool call, approval state, cost, trace id, replay key.
- Encrypted token/credential storage for connector tokens.

## Schema and migrations

```sh
corvid migrate status              # show applied/pending/drifted
corvid migrate up [--dry-run]      # apply pending migrations
corvid migrate down [--dry-run]    # roll back the last migration
```

Migration files live in `migrations/` with a `NNNN_name.up.sql` /
`NNNN_name.down.sql` pair:

```
migrations/
├── 0001_users.up.sql
├── 0001_users.down.sql
├── 0002_audit_log.up.sql
└── 0002_audit_log.down.sql
```

The migration state store records applied migrations + checksum.
`corvid migrate status` flags drift if a migration's recorded
checksum doesn't match the file on disk.

## Connecting

```corvid
import "@stdlib/db" as db

@budget($0.001)
agent fetch_user(id: String) -> Option<User> uses db_read_effect:
    let conn = db.connect("postgres://localhost/myapp")?
    let row = conn.query_one("select id, email from users where id = $1", [id])?
    match row:
        Some(r) -> Some(User { id: r.get("id"), email: r.get("email") })
        None    -> None
```

The connection is pooled. `db_read_effect` is a stdlib-provided effect
that carries `data: external_input` and a configurable cost.

## Transactions

```corvid
agent transfer(from_id: String, to_id: String, amount: Cents) -> Result<Unit, String> uses db_write_effect:
    let conn = db.connect(...)
    conn.transaction(fn (tx) -> {
        tx.execute("update accounts set balance = balance - $1 where id = $2", [amount, from_id])?
        tx.execute("update accounts set balance = balance + $1 where id = $2", [amount, to_id])?
        return Ok(Unit)
    })
```

Transactions auto-rollback on `Err`. Nested transactions are rejected
at compile time.

## Typed row decode

```corvid
struct User:
    id: String
    email: String
    created_at: Timestamp

let users: List<User> = conn.query_typed::<User>(
    "select id, email, created_at from users limit 10",
    []
)?
```

The compiler generates the per-struct decoder. A row that doesn't
match the struct shape produces a typed `Err(DecodeError)` with the
column-name and expected-type annotation, not a runtime panic.

## Audit-log schema

```sh
corvid migrate apply --include-stdlib-audit
```

Adds the canonical audit-log table:

```sql
create table audit_log (
    id ulid primary key,
    occurred_at timestamptz not null default now(),
    actor text not null,
    action text not null,
    prompt_version text,
    model text,
    tool_name text,
    approval_state text,
    cost_cents int,
    trace_id text,
    replay_key text,
    metadata jsonb
);
```

Use `db.audit_log.write(...)` from agents:

```corvid
db.audit_log.write({
    actor: "system",
    action: "refund_issued",
    cost_cents: 5000,
    trace_id: trace.current_id(),
    metadata: { customer_id, amount }
})
```

## Encrypted tokens

```corvid
let token = db.tokens.get_encrypted("gmail_oauth", customer_id)?
let access_token = token.decrypt()?       # decrypt with host key
```

Token values never appear in traces, error messages, or logs. The
runtime redacts them automatically. `corvid doctor` validates that
the host encryption key is present and well-formed without printing it.
