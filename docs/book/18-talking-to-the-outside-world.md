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

## Read configuration from the environment


Import `env_get` from `./std/env` in a source file beside the vendored
`std/` directory. It runs through the interpreter with the default CLI.
The following function supplies a default when the variable is absent.
A variable set to the empty string keeps that value.

```corvid-fragment
import "./std/env" use env_get

agent mode() -> Result<String, String>:
    setting = env_get("APP_MODE")?
    value = match setting:
        Some(value) -> value
        None -> "local"
    return Ok(value)
```

`env_get(name: String) -> Result<Option<String>, String>` returns an error
for blank names, names containing `=` or NUL, and values the runtime cannot
read as Unicode.

## Build a file path

Use the host's path rules when combining a directory and a filename. A
relative child is appended to the base. A fully absolute child replaces it.

```corvid-fragment
import "./std/path" use path_join, path_filename, path_extension
import "./std/io" use io_read_text

agent read_report(directory: String) -> Result<String, String>:
    report = path_join(directory, "report.json")
    file = io_read_text(report)?
    return Ok(file.contents)
```

For this report, `path_filename(report)` returns `Some("report.json")` and
`path_extension(report)` returns `Some("json")`. A leading dot alone, as in
`.profile`, supplies no extension. `path_parent(report)` returns an optional
parent directory. A single relative filename has the empty relative directory
as its parent. `path_is_absolute(report)` tells whether the path is absolute
on the current host. All five operations work on paths before files exist.
The default CLI runs these tools in the interpreter.

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
    #    regardless of allowlist contents. Every executing tool returns
    #    `Result` — a policy refusal or transport failure is an Err
    #    VALUE `?` propagates, never a crash. (An error HTTP STATUS
    #    like 404 is still `Ok` — inspect `response.status`.)
    response = http_get(url)?

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
    #    A confinement refusal or SQL error is an Err value, so `?`
    #    keeps the pipeline honest at every step.
    handle = db_open(db_path)?
    db_execute(handle, "CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, email TEXT NOT NULL)", [])?
    db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(user.id), db_param_text(user.email)])?

    # 4. SQLite read-back. Parameterised SELECT through the same
    #    path. The DbResult envelope's `rows_affected` is 0 for a
    #    SELECT (the field is meaningful only for INSERT/UPDATE/DELETE);
    #    indexing `rows[0]` succeeds because the row exists.
    rows = db_query(handle, "SELECT id FROM users WHERE id = ?", [db_param_int(user.id)])?
    return Ok(rows[0].rows_affected)

agent main() -> Result<Int, String>:
    return ingest_user("http://api.example.com/users/1", ":memory:")
```

## Step 3 — Notice what's NOT in the project

```text
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

Each refusal arrives as an `Err` **value** — the program keeps
control. `?` propagates it up here, but an agent that wants to retry,
fall back, or log can `match` on the Result instead. The boundary is
just as hard; the failure mode is honest.

### SSRF block (always on)

Change the URL to `http://127.0.0.1:8080/users/1` and re-run. `http_get`
returns `Err` with a diagnostic naming the structural SSRF property —
not the allowlist. Even if you add `127.0.0.1` to `[http] allow`, the
structural block still fires. The block is the security floor; the
allowlist is the layer on top.

### `[http] allow` allowlist (fail-closed)

Change the URL to `http://api.attacker.com/users/1` and re-run.
`http_get` returns `Err` with a diagnostic naming the missing allowlist
entry and the `CORVID_HTTP_ALLOW` env override pathway. Empty
allowlists fail-close on every executing HTTP call — the security
boundary is visible from day one.

### `[io] root` confinement (reused by SQLite)

Change the DB path to `"../../etc/users.db"` and re-run. `db_open`
returns `Err` at the `IoToolPolicy::resolve` boundary — the same
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
- Every executing tool returns `Result` — refusals and I/O failures are
  Err values, and `?` propagation routes them through the standard
  `Result<_, String>` envelope.
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

### Reading path and query parameters

A route can take an identifier from its path and a record from its query string.

```corvid
type Search:
    term: String
    limit: Int

type SearchReply:
    owner: String
    term: String
    limit: Int

server search_api:
    route GET "/owners/{id}/search" query Search -> json SearchReply:
        return SearchReply(path.id, query.term, query.limit)
```

Run `corvid serve src/main.cor --listen 127.0.0.1:8000` and request
`/owners/lee/search?term=caf%C3%A9+au+lait&limit=2`. The response contains
the owner `lee`, the text `café au lait` and the integer `2`. A missing
required query field, an invalid integer or malformed UTF-8 returns HTTP
400 with `invalid_query`.

### Reading SQLite rows into a Corvid record

`db_query_rows(handle, sql, params)` in `std/db` returns
`Result<List<JsonValue>, String>`. Each row is an object keyed by the
column names in the SELECT. Use SQL aliases when names would overlap.
Bind values with `db_param_text` or `db_param_int`. An empty result
is an empty list.

```corvid
import "./std/db" use db_open, db_query_rows, db_param_text
import "./std/json" use json_get_string

type Note:
    name: String
    text: String

agent load_note(name: String) -> Result<Note, String>:
    db = db_open("notes.sqlite")?
    rows = db_query_rows(db, "SELECT name, text FROM notes WHERE name = ?", [db_param_text(name)])?
    if rows.length() == 0:
        return Err("missing note")
    saved_name = json_get_string(rows[0], "name")?
    saved_text = json_get_string(rows[0], "text")?
    return Ok(Note(saved_name, saved_text))
```

With `root = "."` in the manifest's `[io]` section, both `corvid run` and
`corvid serve` open the database relative to the project. Put schema SQL
in `migrations/001_notes.sql` and run
`corvid migrate up --database notes.sqlite` from the project directory. If a statement fails during a batch of pending
migrations, fix the failed SQL and rerun the command. Earlier statements
in that batch are rolled back with it.

Use the same handle for `BEGIN IMMEDIATE`, changes and `COMMIT` via
`db_execute`. You can issue `ROLLBACK` explicitly. When the last
reference to a handle leaves scope, SQLite closes that connection and
rolls back an unfinished transaction. This includes an early return via `?`.

### Giving an outbound request a deadline

`corvid serve` reads the same `[http] allow` list as `corvid run`.
List each upstream host your service uses in `corvid.toml`. Configured
clients also check redirect destinations. Use a JSON builder when
posting user-provided text so it is encoded as a JSON string.

```corvid
import "./std/http" use http_post_json
import "./std/json" use json_object_new, json_object_set_string, json_object_finish

agent send_message(text: String) -> Result<Int, String>:
    message = json_object_set_string(json_object_new(), "text", text)
    response = http_post_json("https://api.example.com/messages", json_object_finish(message))?
    return Ok(response.status)

server relay:
    route POST "/messages" body String -> json Result<Int, String>:
        return try send_message(body) timeout 300
```

The timeout is in milliseconds per attempt. If the route's call exceeds
its deadline, the service responds with HTTP 504 and `handler_timeout`.
A caller disconnect also cancels an outstanding request. An upstream
operation may already have received the message before cancellation.

For stream expressions, the deadline covers startup through the first
item. After that, the consumer can continue reading. Releasing the final
consumer cancels the stream producer even while it is awaiting a tool.

### Serving and stopping an application

Start the server with `corvid serve --access-log` to write JSON access
records to stderr. For streams, elapsed time measures how long it took to
prepare the response headers. A route such as `/notes/{name}` appears as
the router template `/notes/:name` in the log, keeping note names private.

To stop the service, press Ctrl+C or send SIGTERM on Unix. New connections
stop and in-flight requests have up to 30 seconds to finish. A long-lived
stream still open after that deadline is disconnected.

## Search a persistent document collection

Import `rag_ingest` and `rag_search` from `std/rag`. Both return a
`Result`. Ingest accepts the SQLite index path, a document ID, a source
label, the text, and a maximum chunk length in characters. A second
successful ingest with the same ID replaces the whole document. An
embedding or database write error leaves the previous version searchable.

```corvid
import "./std/rag" use rag_ingest, rag_search, RagChunkEnvelope

agent store_note(text: String) -> Result<Int, String>:
    return rag_ingest("notes.sqlite", "travel", "notes/travel.txt", text, 400)

agent find_notes(question: String) -> Result<List<RagChunkEnvelope>, String>:
    return rag_search("notes.sqlite", question, 3)
```

Set `[io] root` in the project's `corvid.toml` to the directory where
the index belongs. By default, searches rank chunks by matching terms.
For embedding-based retrieval, configure a running Ollama embedding model:

```toml
[rag]
embedder = "ollama"
model = "nomic-embed-text"
endpoint = "http://127.0.0.1:11434"
```

Use the same embedding model for indexing and querying. Both `corvid run`
and `corvid serve` load this configuration. A hit carries its text,
document ID, source, provenance key, and starting and ending character
offsets. The end offset is exclusive. These fields let an application
show the passage alongside its source after reopening the index.

## Run a scoped browser session

`std/browser` lets a Corvid program operate a browser started specifically
for its run. Set `CORVID_BROWSER_URL` to the page the user selected. The
local driver uses Python with Playwright and an installed Chromium browser.
Set `CORVID_BROWSER_PYTHON` to that interpreter and `CORVID_BROWSER_CHANNEL`
if a particular installed browser channel is required.

```corvid
import "./std/browser" use browser_open_scoped, browser_click, browser_read_text, browser_capture, browser_close

agent update_page() -> String:
    browser_open_scoped(800, 600)
    browser_click("Update")
    status = browser_read_text("#status")
    browser_close()
    return status
```

`browser_open_scoped` takes viewport width and height. It uses the
starting page's scheme, host and port as the interaction scope. The driver
checks the active page and restricts navigation requests to that origin.
The page may load resources such as styles or images from other origins.
`browser_click` selects a button by its exact accessible name, and
`browser_read_text` reads a visible element. `browser_capture()` returns
the path to a PNG. Put the capture directory (`CORVID_BROWSER_EVIDENCE_DIR`)
inside the configured I/O root to pass a screenshot to `io_read_image`.
`browser_close()` ends the session. Opening again creates a fresh context.

## Calling your own Python tool

Run `corvid new --with-python-tools notes`, then `cd notes`. The project includes an `echo` tool declared in Corvid and implemented by an async Python function in `tools.py`. Run it with `corvid run src/python_tools_example.cor "Hello"`. The interpreter loads the decorated function and returns `"Hello"`. Edit the function and rerun the same command to try your own implementation.
