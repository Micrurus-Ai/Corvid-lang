# `std.db` — Executing SQLite surface

> **Status:** Phase 33S3 (closed 2026-06-09). Three tools execute
> real SQLite operations against rusqlite; three RuntimeChecked
> guarantees + the existing `io_source.fs_path_confinement`
> guarantee (reused via `IoToolPolicy`) govern their behavior.
> Earlier slices shipped envelope types only; 33S3 promotes the
> SQLite path to executing. The Postgres path remains envelope-
> only — `std.db`'s Postgres types are still the boundary
> between your agent and a tool wrapper.

## Quick reference

```corvid
import "./std/db" use db_open, db_query, db_execute, db_param_int, db_param_text

agent record_user(email: String) -> Int:
    handle = db_open(":memory:")
    db_execute(handle, "CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT NOT NULL)", [])
    db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(1), db_param_text(email)])
    rows = db_query(handle, "SELECT id FROM users WHERE email = ?", [db_param_text(email)])
    return rows[0].rows_affected
```

When this agent runs through `corvid run`, the calls flow through
typed `DbParam` constructors and `rusqlite::params_from_iter` —
there is no string-interpolation path anywhere on the dispatch.

## The three tools

### `db_open(path: String) -> DbHandle uses db_egress_open`

Opens a SQLite connection at `path` and returns an opaque,
refcounted `DbHandle` the runtime uses to look up the connection
on subsequent `db_query` / `db_execute` calls. The path is
resolved through the project's `IoToolPolicy` — see the security
section below. The documented special case `":memory:"` skips
path resolution and opens an ephemeral in-memory database.

`db_egress_open` is reversible: opening a connection by itself
doesn't durably modify state (creating the file at the
configured path is the only side effect, treated as reversible
because the connection lifecycle is reverted when the runtime
drops).

### `db_query(handle: DbHandle, sql: String, params: List<DbParam>) -> List<DbResult> uses db_egress_read`

Runs a parameterised SELECT against the connection at `handle`.
The `params` list flows through the typed `DbParam` constructors
(below); the runtime binds via `rusqlite::params_from_iter` —
there is no SQL interpolation path. Returns one `DbResult`
envelope per row.

`db_egress_read` is reversible: reads don't modify state.

### `db_execute(handle: DbHandle, sql: String, params: List<DbParam>) -> DbResult uses db_egress_write`

Runs a parameterised INSERT / UPDATE / DELETE / DDL statement
against the connection at `handle`. Same parameter-binding path
as `db_query` — typed `DbParam`s threaded through
`params_from_iter`. Returns a `DbResult` envelope with
`rows_affected` populated.

`db_egress_write` is NOT reversible: writes durably change DB
state. The replay-quarantine guarantee below leverages this:
`db_execute` during Substitute-mode replay is refused with
`QuarantineViolation { surface: "db", .. }`.

## The `DbHandle` opaque type

`DbHandle` is an **opaque, refcounted** primitive type. The
typechecker carries it through agent and tool signatures, but
user code CANNOT construct a `DbHandle` directly:

- The Corvid surface type `DbHandle` has no field-construction
  shape (it's a primitive, not a struct).
- The VM's `Value::DbHandle(Arc<DbHandleInner>)` variant has no
  JSON marshalling path — `value_to_json` emits an opaque
  sentinel for trace-debug visibility only; `json_to_value`
  REFUSES to mint a handle from any JSON payload, including a
  payload that exactly matches the sentinel shape.
- The runtime's `DbHandleRegistry::open` is the SOLE allocator
  of valid handle ids. Anything that hands you back an
  `Arc<DbHandleInner>` had to go through that allocator.

The opacity is structural: there is no path in user code that
fabricates a `DbHandle`. That makes "you cannot fabricate a
SQLite connection in user code" a **load-bearing language
property** rather than a documentation claim.

The handle is refcounted via `Arc`. Multiple agents can hold
the same handle (clones share the same Arc + the same backing
`rusqlite::Connection`). The underlying connection lives until
the registry drops; a follow-up slice will wire a
runtime-callback closer for early release when the last
`Arc::drop` fires.

## The `DbParam` parameter type

```corvid
public type DbParam:
    name: String
    value_kind: String
    redacted: Bool
    int_value: Int
    float_value: Float
    string_value: String
    bool_value: Bool
```

The `value_kind` discriminator names which of the four value
fields carries the bound value. Use the typed constructors —
NEVER construct `DbParam` positionally with an arbitrary
`value_kind`:

```corvid
db_param_int(42)            # value_kind = "Int",    int_value    = 42
db_param_float(3.14)        # value_kind = "Float",  float_value  = 3.14
db_param_text("hello")      # value_kind = "String", string_value = "hello"
db_param_bool(true)         # value_kind = "Bool",   bool_value   = true
db_param_null()             # value_kind = "Null",   all fields zeroed
```

The runtime reads only the field matching `value_kind` and
threads it through `rusqlite::params_from_iter` as a typed
`DbValue`. There is no path that interpolates `string_value`
into the SQL string.

## Security model

### 1. Structural parameter-binding-only — **always on**

The load-bearing structural property: SQL injection is
prevented by the language's type system + the runtime's
binding path, not by escaping or sanitisation.

```corvid
# This is SAFE — the attack string is bound as TEXT data:
db_execute(handle, "INSERT INTO users(email) VALUES (?)",
    [db_param_text("'; DROP TABLE users; --")])
```

After this call:
- The `users` table still exists. No DROP fired.
- The stored email is the EXACT verbatim string
  `"'; DROP TABLE users; --"`.
- The string was bound through `rusqlite::params_from_iter` as
  a typed `DbValue::Text`, never parsed as SQL.

The structural argument: the `db_execute` tool's signature is
`(DbHandle, String, List<DbParam>) -> DbResult`. The
typechecker REJECTS any call that doesn't pass a `List<DbParam>`
as the third argument. The dispatch path
(`crates/corvid-vm/src/interp.rs::extract_db_params`) reads
each `DbParam`'s `value_kind` discriminator and picks the
matching typed value field; the runtime's
`DbHandleRegistry::execute` then runs
`params_from_iter(typed_values)`. There is no `format!` call
anywhere on the path that concatenates a parameter into the
SQL string.

### 2. `[io] root` path confinement — **reused from the file-I/O surface**

`db_open` resolves the supplied path through the project's
`IoToolPolicy::resolve` — the SAME boundary the io tools
(`io_read_text` / `io_write_text` / `io_list_dir`) enforce.
This is the deliberate design that makes `db_open` strictly
narrower than `io_write_text`:

- Paths that traverse out of `[io] root` are refused with a
  structured diagnostic naming the offending path AND the
  configured root.
- The documented special case `":memory:"` bypasses resolution
  because there is no filesystem path to confine.
- A program with no `[io] root` configured fails closed on
  every non-`:memory:` `db_open` call.

There is no separate `[db]` allowlist, no `[db] root`, no
`CORVID_DB_ROOT` env. SQLite paths ARE file paths;
duplicating the confinement boundary would invite drift
between the two surfaces. The existing
`io_source.fs_path_confinement` guarantee carries the property
for both `io_*` and `db_open`.

#### corvid.toml

```toml
[io]
root = "./data"     # restrict both io_* AND db_open to ./data
```

### 3. Replay write-quarantine — **always on during Substitute-mode replay**

When a Corvid program runs through `corvid replay <trace>`,
the runtime enters Substitute-mode and the
`DbHandleRegistry::quarantine_writes` flag is flipped.
`db_execute` then short-circuits with `QuarantineViolation
{ surface: "db", .. }` regardless of SQL contents — INSERTs,
UPDATEs, DELETEs, and DDL are all blocked. The database is
provably untouched during replay.

`db_query` reads pass through (SQLite reads don't escape the
process; the trace-substitution upper gate for db_query rows
lands in a follow-up slice once the trace schema carries
row events).

## Determinism

All three tools are non-deterministic. The checker rejects
calls inside a `@deterministic` body — this is the existing
`decl_replayability.rs::is_deterministic_compatible` rule
that treats every tool call as non-deterministic by
declaration kind. Programs that need both deterministic logic
AND SQLite access must factor the database calls into a
separate, non-deterministic agent.

## Guarantees

Three RuntimeChecked rows in the canonical guarantee registry
([`docs/reference/core-semantics.md`](../core-semantics.md)):

| id | class | enforces |
|---|---|---|
| `io_source.sqlite_parameter_binding_only` | RuntimeChecked | All SQL parameters bound via `rusqlite::params_from_iter` over typed `DbValue`s; no string-interpolation path exists. SQL injection prevented structurally. |
| `io_source.sqlite_write_quarantine_on_replay` | RuntimeChecked | `db_execute` during replay refuses with QuarantineViolation. Database provably untouched. |
| `io_source.sqlite_read_passthrough_on_replay` | RuntimeChecked | `db_query` not blocked by write-quarantine (reads pass through; trace-substitution upper gate lands in a follow-up slice). |

Plus the REUSED `io_source.fs_path_confinement` guarantee
(from 33S1c) — `db_open` routes through the same
`IoToolPolicy` boundary, so `db_open(path)` is structurally
as narrow as `io_write_text(path)`.

The `@deterministic`-rejection property is covered by the
existing `replay.deterministic_pure_path` guarantee — it
applies to all tool calls regardless of effect.

## Worked example — typed user store

```corvid
import "./std/db" use db_open, db_execute, db_query, db_param_int, db_param_text

agent open_users(path: String) -> DbHandle:
    handle = db_open(path)
    db_execute(handle, "CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE)", [])
    return handle

agent register_user(handle: DbHandle, id: Int, email: String) -> Int:
    result = db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(id), db_param_text(email)])
    return result.rows_affected

agent find_email(handle: DbHandle, id: Int) -> Int:
    rows = db_query(handle, "SELECT email FROM users WHERE id = ?", [db_param_int(id)])
    return rows[0].rows_affected
```

With `corvid.toml`:

```toml
[io]
root = "./data"
```

`open_users("./users.sqlite")` succeeds and resolves to
`./data/users.sqlite`. `open_users("../../etc/passwd")` is
refused at the `IoToolPolicy` boundary. `register_user(handle,
1, "alice")` inserts. `register_user(handle, 2, "'; DROP TABLE
users; --")` ALSO succeeds — the attack string is bound as
TEXT data, the table survives, and `find_email(handle, 2)`
returns the verbatim metacharacter string.

## Post-v1.0 — what's deliberately NOT in scope

- **Postgres executing surface.** 33S3 ships SQLite only. The
  Postgres types in `std/db.cor` (DbConnection,
  postgres_open, etc.) remain envelope-only — declare your
  Postgres tool in user code and reach the
  `corvid-runtime::PostgresDbRuntime` from a tool wrapper.
- **Trace substitution for db_query rows.** 33S3d ships read-
  passthrough; a follow-up slice will add the upper gate that
  substitutes recorded row events during replay.
- **Early Arc-drop registry-slot release.** The runtime
  callback closer that releases the registry slot when the
  last `Arc<DbHandleInner>` drops lands in a follow-up slice.
  Today connections live until the registry drops (i.e.,
  runtime drop).
- **cdylib codegen.** `DbHandle` is interpreter-only today
  (the codegen-cl backend emits a structured "interpreter-only
  in 33S3; future slice lands C-ABI opaque-pointer codegen"
  diagnostic). The interpreter tier is fully supported.

## Related references

- [`core-semantics.md`](../core-semantics.md) — full guarantee
  registry, including the three `io_source.sqlite_*` rows and
  the reused `io_source.fs_path_confinement` row.
- [`inventions.md`](../inventions.md) — invention proof matrix.
- `corvid tour --topic sqlite` — runnable demo (`:memory:`,
  runs offline; the topic source compiles through the
  driver's tour-compile gate).
- `crates/corvid-runtime/src/db.rs` — `DbHandleRegistry`
  source + inline guarantee anchors
  (`GUARANTEE_ID_IO_SOURCE_SQLITE_*`).
- `crates/corvid-runtime/src/runtime/llm_dispatch.rs` —
  `Runtime::db_open_tool` / `db_query_tool` / `db_execute_tool`
  typed-Value dispatch methods.
- `crates/corvid-vm/src/interp.rs` — `dispatch_stdlib_db_tool`
  interpreter routing branch + `extract_db_params` converter.
- `std/db.cor` — the tool declarations + typed parameter
  constructors + envelope types.
