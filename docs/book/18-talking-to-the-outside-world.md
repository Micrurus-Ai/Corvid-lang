# Talking to the outside world

## Goal

Build a complete HTTP → JSON → SQLite pipeline in pure Corvid. Zero
Python glue. Zero host-language plumbing. Every load-bearing safety
property — SSRF block, structural SQL injection-resistance,
parameter-bound writes, replay quarantine, opaque handles — holds
end-to-end at the language level.

This chapter weaves together the three executing I/O surfaces shipped
in Phase 33S + 33R5b:

- **`std/http`** — `http_get` / `http_post_json` (Phase 33S2)
- **`std/json`** — opaque path + typed-decoder convention (Phase 33R5b)
- **`std/db`** — `db_open` / `db_query` / `db_execute` (Phase 33S3)

Each surface ships its own reference doc and structural safety story.
This chapter shows how they compose.

## Step 1 — Make a project

```sh
corvid new outside-world
cd outside-world
```

The `corvid new` scaffold writes a `corvid.toml` with `[io] root = "."`
and `[http] allow = []` — both executing-I/O security boundaries are
explicit and visible from day one. Open `corvid.toml` and widen the
HTTP allowlist to the host you'll call:

```toml
[io]
root = "."

[http]
allow = ["api.example.com"]
```

The `[io] root` boundary also confines `db_open` paths — SQLite is
structurally as narrow as `io_write_text`. No separate `[db]`
allowlist exists; the boundary is unified.

## Step 2 — Write the pipeline

Open `src/main.cor`:

```corvid
effect json_decode_eff:
    reversible: true

type User:
    id: Int
    email: String

import "./std/http" use http_get
import "./std/db" use db_open, db_execute, db_query, db_param_int, db_param_text

tool decode_user_from_json(text: String) -> Result<User, String> uses json_decode_eff

agent ingest_user(url: String, db_path: String) -> Result<Int, String>:
    # 1. HTTP GET. The URL host must appear in `[http] allow`.
    #    The always-on SSRF block refuses RFC1918 / loopback / link-local
    #    regardless of allowlist contents.
    response = http_get(url)

    # 2. Typed-decoder JSON. The `decode_<X>_from_json` name pattern
    #    + `Result<X, String>` return type together trigger the
    #    runtime's generic decode path: serde_json::from_str +
    #    json_to_value against the declared User type. Shape
    #    mismatches surface as `Result::Err`, never panics.
    user = decode_user_from_json(response.body)?

    # 3. SQLite write. `db_open` resolves the path through the same
    #    `[io] root` boundary the file-I/O tools enforce. `db_execute`
    #    parameter-binds via `rusqlite::params_from_iter` — there is
    #    no SQL interpolation path; the typechecker's `List<DbParam>`
    #    signature forces every value through the typed constructors.
    handle = db_open(db_path)
    db_execute(handle, "CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, email TEXT NOT NULL)", [])
    db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(user.id), db_param_text(user.email)])

    # 4. SQLite read-back. Parameterised SELECT through the same
    #    path. The DbResult envelope's `rows_affected` is 0 for a
    #    SELECT (the field is meaningful only for INSERT/UPDATE/DELETE);
    #    indexing `rows[0]` succeeds because the row exists.
    rows = db_query(handle, "SELECT id FROM users WHERE id = ?", [db_param_int(user.id)])
    return Ok(rows[0].rows_affected)

agent main() -> Result<Int, String>:
    return ingest_user("http://api.example.com/users/1", ":memory:")
```

## Step 3 — Notice what's NOT in the project

```
outside-world/
├── corvid.toml         # security boundaries declared
└── src/
    └── main.cor        # the pipeline
```

No `tools.py`. No `requirements.txt`. No glue layer. The three
executing surfaces (HTTP, JSON, SQLite) all run through the Corvid
interpreter against real reqwest, real serde_json, and real rusqlite.
Your Corvid code IS the program.

This is the load-bearing claim of the 33R5/33S umbrella: a v1.0 Corvid
program that consumes a JSON API and writes to a database needs ZERO
host-language glue. Other languages put their stdlib batteries here;
Corvid does too.

## Step 4 — Run it

```sh
corvid run src/main.cor
```

`main()` returns `Ok(0)` — the SELECT envelope's `rows_affected` field
(0 for read paths). The row exists; the program reached the read-back
step without any of the safety boundaries firing.

## Step 5 — Trigger each safety boundary, watch them fire

The pipeline above is the happy path. Each of the four executing-I/O
guarantees is testable by deliberately violating it.

### SSRF block (always on)

Change the URL to `http://127.0.0.1:8080/users/1` and re-run. The HTTP
dispatch refuses with a diagnostic naming the structural SSRF property —
not the allowlist. Even if you add `127.0.0.1` to `[http] allow`, the
structural block still fires. The block is the security floor; the
allowlist is the layer on top.

### `[http] allow` allowlist (fail-closed)

Change the URL to `http://api.attacker.com/users/1` and re-run. The
HTTP dispatch refuses with a diagnostic naming the missing allowlist
entry and the `CORVID_HTTP_ALLOW` env override pathway. Empty
allowlists fail-close on every executing HTTP call — the security
boundary is visible from day one.

### `[io] root` confinement (reused by SQLite)

Change the DB path to `"../../etc/users.db"` and re-run. The `db_open`
dispatch refuses at the `IoToolPolicy::resolve` boundary — the same
boundary the file-I/O tools enforce. SQLite paths ARE file paths; no
separate `[db]` allowlist exists.

### Structural SQL injection-resistance

Replace `db_param_text(user.email)` with `db_param_text("'; DROP TABLE
users; --")` and re-run. The pipeline still works. The `users` table
survives. The stored email is the EXACT verbatim metacharacter string —
the typechecker's `List<DbParam>` signature + the runtime's
`params_from_iter` binding path together prevent SQL interpolation
structurally, not by escaping.

### Typed-decoder JSON shape safety

Change the API response (or simulate by editing the URL to point at a
broken endpoint) so `id` is a string. The typed-decoder dispatch
returns `Result::Err("JSON shape mismatch in `decode_user_from_json`:
...")`. The `?` propagates it; the agent returns `Err`. No runtime
panic.

## Step 6 — Replay it

Every executing-I/O call records a deterministic trace:

```sh
corvid trace list
corvid replay <trace-id>
```

During Substitute-mode replay:

- HTTP calls refuse with `QuarantineViolation { surface: "http", .. }`
  regardless of allowlist.
- `db_execute` calls refuse with `QuarantineViolation { surface: "db",
  .. }` regardless of SQL contents.
- JSON parse + build run identically to live (deterministic and
  process-internal — no escape to block).
- `db_query` passes through (reads don't escape the process).

The network and the database are provably untouched during replay. The
JSON layer runs identically, so the typed-decoder convention produces
the same `User` value regardless of mode.

## Step 7 — Sign it (optional)

`corvid build --sign` accepts a descriptor declaring the
load-bearing structural claims this pipeline rests on:

- `io_source.http_ssrf_structural_block`
- `io_source.http_allowlist_enforcement`
- `io_source.http_quarantine_on_replay`
- `io_source.fs_path_confinement` (reused by `db_open`)
- `io_source.sqlite_parameter_binding_only`
- `io_source.sqlite_write_quarantine_on_replay`
- `io_source.sqlite_read_passthrough_on_replay`
- `json.parse_safety_no_panic`
- `json.field_type_safety_at_access_boundary`

A signed cdylib carries these in its claim manifest; a host that loads
the binary can audit them against its own policy.

## What you just shipped

A complete HTTP-fetch → JSON-decode → SQLite-write pipeline with zero
Python glue, end-to-end structural safety, deterministic replay, and a
signing path that audits the load-bearing claims.

The three executing surfaces compose:

- HTTP's SSRF + allowlist gates the URL.
- JSON's typed-decoder converts the response into a typed `User` struct.
- SQLite's parameter-binding writes the row without SQL interpolation.
- `?` propagation routes errors through the standard `Result<_, String>`
  envelope.
- `corvid.toml`'s `[io] root` and `[http] allow` declare the
  process-wide boundaries.

This is what "batteries included" means for Corvid: not "we wrap a JSON
parser." It's "the language structurally prevents an entire class of
bugs at every layer of the pipeline, and the binary you ship carries
the proof."

## Related references

- [`stdlib/http.md`](../reference/stdlib/http.md) — SSRF block,
  `[http] allow`, replay quarantine.
- [`stdlib/json.md`](../reference/stdlib/json.md) — opaque + typed-
  decoder shapes, parse-safety, field-type-safety.
- [`stdlib/db.md`](../reference/stdlib/db.md) — `[io] root` reuse,
  structural parameter-binding-only, write-quarantine on replay.
- [`core-semantics.md`](../reference/core-semantics.md) — the full
  guarantee registry including all the rows the signing claim above
  references.
- `corvid tour --topic file-io | http-client | sqlite | json` —
  the runnable demos for each surface in isolation.
