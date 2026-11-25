# B09: Typed persistence

Status: P1 shipped 2026-11-24; later slices in progress. Backend and AI syntax; no frontend grammar
is affected.

## Problem

A Corvid program that stores data today writes SQL in strings and reads
rows back as `JsonValue`:

```corvid
rows = db_query_rows(db, "SELECT shelf, title FROM shelves WHERE owner = ?", [db_param_text(user)])?
```

Renaming the `title` column breaks this program at run time, not at
compile time. Every example that edits shared data also writes its own
revision guard (`UPDATE ... SET revision = revision + 1 WHERE ... AND
revision = ?`) and its own conflict variant. The runtime already
implements revision-checked writes and a conflict error for `store`
declarations, but no Corvid source reaches them, and migrations and
the executing database tools support SQLite only.

## What a developer writes

```corvid
database library:
    engine: sqlite
    location: env("LIBRARY_DATABASE")
    tables: Shelf, Book

table Shelf:
    key shelf: Int
    owner: String
    title: String where len_between(1, 80)
    note: Option<String>

agent rename(shelf_id: Int, title: String, seen: Int) -> Result<Shelf, String>:
    return match Shelf.get(shelf_id):                  # Option<Shelf>
        None -> Err("there is no shelf " + shelf_id.to_string())
        Some(shelf) -> store_title(shelf, title, seen)

agent store_title(shelf: Shelf, title: String, seen: Int) -> Result<Shelf, String>:
    # `seen` is the revision the person was looking at.
    renamed = Shelf { title: title, revision: seen, ..shelf }
    return match Shelf.save(renamed):                  # Result<Shelf, Option<Shelf>>
        Ok(saved) -> Ok(saved)
        Err(current) -> Err("someone else changed this shelf")

agent shelves_of(user: String) -> List<Shelf>:
    return Shelf.where(fn (s) -> s.owner == user).order_by(fn (s) -> s.title).take(20)
```

## Decisions

1. **A table is a record type.** `table Shelf:` declares the record
   `Shelf` and its storage. Every row also has a `revision: Int`, its
   last field, which the database advances on each write. A row built
   in code may leave `revision` out, and then it is `0`, a row never
   stored. Queries return
   `Shelf` values, so the fields a program reads are checked like any
   record's, and renaming a column reports every query and field read
   that names it.
2. **One key per table, marked in the declaration.** `key shelf: Int`
   names the primary key. Omitting it is a compile error naming the
   decision, because a table without a key cannot be saved with a
   revision check. Composite keys come later.
3. **The database is declared, never defaulted.** A `database` block
   names its engine (`sqlite`, then `postgres` and `mysql`), where it
   is (`location:`, from configuration or the environment), and which
   tables it holds. A table outside every database, or a program with
   tables and no database, is a compile error. Which engine and where
   are consequential (data location, tenancy), so the source states
   them.
4. **Writes always carry the revision they expect: the row's own.**
   A row read from the database carries its revision, editing it with
   `{ title: t, ..shelf }` keeps it, and `Shelf.save(row)` succeeds
   only if the stored row still has that revision. A program that must
   check the revision a person saw writes it, `{ revision: seen, ..row
   }`, so the expectation is visible in the source. `Shelf.save(row)`
   returns `Result<Shelf, Option<Shelf>>`: `Ok` holds the stored row
   with its new revision, and `Err` holds the row stored now, or
   `None` if it was deleted. `Shelf.insert(row)` returns `Result<Shelf,
   Shelf>`, with the row already stored under that key on `Err`.
   `Shelf.delete(row)` follows `save`. There is no unchecked
   overwrite. The operations are methods on the table, like
   `Shelf.get`, so they add no global names.
5. **Queries are typed functions, not strings.** `Shelf.get(key)`,
   `T.where(fn (row) -> condition)`, `.order_by(fn (row) -> column)`,
   `.descending()`, `.take(n)` and `.all()`. A condition is a function
   over the row, so `s.owner` is an ordinary checked field read, and
   locals are parameters. The checker accepts only conditions it can
   translate to SQL: comparisons, `and`, `or`, `not`, `in` over a list,
   `contains` / `starts_with` on strings, and `== None`. Anything else
   is a compile error naming the part that cannot run in the
   database. SQL text never appears in source, so injection is not
   expressible.
6. **Effects and replay compose.** Reads use the built-in
   `db_egress_read` effect and writes `db_egress_write`, the effects
   `std.db` already declares, so effect rows and approvals see database
   work without a second set of database effects.
   Every query and write is recorded in the trace with its result, and
   replay returns the recorded rows without touching the database.
7. **Migrations come from declarations.** `corvid migrate plan` compares
   the declared tables with the schema the last applied migration
   left and writes a reviewed SQL migration into `migrations/`, which
   the existing `corvid migrate up` applies. A rename is declared,
   `title: String renamed from name`, because the planner cannot tell
   a rename from a drop and an add. Dropping a column loses data, so
   it is declared too: `retired legacy_note`. A plan that would drop
   an undeclared column is refused with the column's name.
8. **One query plan, a dialect per engine.** Queries lower to a small
   relational plan in the runtime; SQLite, Postgres and MySQL each
   render it. The same program runs on each engine named in its
   `database` block, and a test suite runs every query on all three.

## Non-scope

- Joins and aggregates in the first slices. A program reads two tables
  and combines them in Corvid; joins follow once the plan carries them.
- Raw SQL. `std.db` stays for programs that need it, and its results
  stay untyped.
- The native target until the interpreter slices land.

## Slices

- [x] P1. `table` and `database` declarations, the row record with
  `revision`, `Shelf.get`, `Shelf.insert`, `Shelf.save` and
  `Shelf.delete` with `Result`-typed conflicts, on SQLite through the
  interpreter, with effects and replay. `corvid migrate plan` writes
  the `CREATE TABLE` for new tables.
- [x] P2. Typed queries: `where`, `order_by`, `descending`, `take`,
  `all`, and the compile-time check that a condition can run in the
  database.
- [x] P3. Migrations from declaration changes: added and altered
  columns, `renamed from`, `retired`, and refusal of undeclared drops.
- [ ] P4. Postgres, with the cross-engine query suite. Split in two:
  - [x] P4a. Tables run on Postgres: every operation and query, recorded
    and replayed, with the same test suite run on both engines.
  - [x] P4b. `corvid migrate plan` for Postgres, planning against a
    scratch database the developer names, because Postgres migrations
    cannot be replayed into in-memory SQLite.
- [ ] P5. MySQL on the same suite.
- [ ] P6. Native target.
- [ ] P7. Port `examples/reading_list` from hand-written SQL to tables, in three parts: atomic writes (P7a, done), keys of two or more columns (P7b, done), and the port (P7c).

## Settled during implementation

- Corvid calls have no named arguments, and adding them would change
  grammar the frontend shares, so the expected revision travels in the
  row instead of an `expecting:` argument.
- Conflicts are ordinary `Result` values holding the stored row, so no
  new generic conflict type is needed.
- A query is a method chain read whole by the compiler, not a value
  built step by step, so it needs no new grammar and no query type: the
  chain has type `List<T>`. The checker and IR lowering read a `where`
  condition through one function, so what type-checks is exactly what
  runs. Results are ordered by key when nothing else decides, and `==`
  is SQL `IS`, so `== None` finds missing values and no condition is left
  undecided by `NULL`.
- `retired` is written without a colon, `retired legacy_note`, not
  `retired: legacy_note` as first proposed: with a colon the line reads as
  a field, and `retired: Bool` is too common a field to reserve.
- Simpler spellings considered for the history clauses: `@renamed(from: title)`
  adds an attribute's punctuation for no extra meaning; `name: String was title`
  is shorter but does not say a rename happened; `drop legacy_note` reads as
  an action that runs every time the program starts. `renamed from title`
  and `retired legacy_note` read as plain statements of the table's history.
- A new required field is refused rather than filled. Corvid fields have
  no defaults, and inventing a value for stored rows would be a hidden
  default; the refusal gives the path (add an `Option`, fill it, make it
  required). The only type changes planned are those that convert every
  value exactly; the rest are refused with the add-fill-retire path.
- `Bool` and JSON-stored fields get their own SQL types, `BOOLEAN` and
  `JSON TEXT`, instead of sharing `INTEGER` and `TEXT`, so the stored
  schema records how each column is stored and the planner can see every
  change. Tables created by P1's migrations with a `Bool` or JSON field
  plan as a type change and are refused; recreate them.

## Postgres (P4)

```corvid
database library:
    engine: postgres
    location: env("LIBRARY_DATABASE_URL")
    tls: required
    tables: Shelf
```

- **The location comes from the environment.** A Postgres connection
  string usually carries a password, so `location:` must be `env("NAME")`;
  a string written in source is a compile error.
- **TLS is declared.** Whether credentials and rows cross the network
  encrypted is a security decision with two reasonable answers (a
  hosted database needs TLS, a local one may not), so a Postgres block
  must say `tls: required` or `tls: disabled`, and leaving it out is a
  compile error. `required` verifies the server's certificate against
  the standard web roots; a private certificate authority is later work.
  A SQLite block refuses `tls:`, because a file has no connection.
- **Same meaning, each engine's SQL.** Every operation and query renders
  through a dialect: parameters are `$1` with an explicit type cast, `==`
  is `IS NOT DISTINCT FROM`, the string tests use `strpos`, and columns
  are `BIGINT`, `DOUBLE PRECISION`, `TEXT`, `BOOLEAN` and `JSONB`. A
  recorded query holds its SQL, so a replay of a Postgres run is checked
  against Postgres SQL.
- **One suite, both engines.** The table and query tests run on SQLite
  always and on Postgres when `CORVID_TEST_POSTGRES_URL` names a test
  database; without it they print `skipping:` for Postgres.
- **Across the tiers.** The engine is a deployment choice the backend
  owns, so nothing about it reaches the frontend or an assistant: a
  screen and an agent see the same typed rows, conflicts and query
  results on either engine, and moving from SQLite to Postgres changes
  no UI or agent code. That is the point of the slice, and the shared
  test suite proves it. What the backend learns here that the other
  tiers will use is the row type and its refinements, which B10's
  actions carry to forms and assistant tools from the one table
  declaration.
- **One connection per location in P4a.** Writes are compare-and-set, so
  concurrent connections stay correct; a connection pool is planned with
  B19's concurrency work, not designed out.

## Atomic writes (P7a)

- `atomic:` is a block, not a function taking a closure or a
  `transaction { }` keyword pair. Simpler spellings considered:
  `transaction:` names the mechanism rather than the guarantee the reader
  needs; `all or nothing:` reads well but is three words for a statement;
  a closure form (`Shelf.atomic(fn () -> ...)`) adds punctuation and a
  return value nobody needs. `atomic:` says what happens in one word and
  matches `parallel:`.
- Commit when the block ends, undo when it is left through an error. A
  conflict stays an ordinary value, so code chooses: `?` to undo the
  whole block, or record the outcome and keep the other writes, which the
  reading list's review flow needs.
- A plain `return` is refused inside the block rather than given a
  meaning, because "return early" could mean keep or discard and a reader
  should not have to know which.
- What the database cannot undo is refused at compile time, following
  agents the block calls. An approval inside a transaction would hold
  database locks while a person decides, and a model call would hold them
  for seconds and could not be taken back.
- Across the tiers: the guarantee is the backend's, and the frontend and
  assistants see its result through B10's actions. An action whose body
  is one `atomic:` block is exactly the "all or nothing" a form submit or
  an agent's tool call needs, with conflicts returned as typed values the
  form or the agent can show or retry.
- A key of several columns reuses `key`: each field that is part of it is
  marked, rather than a separate `key (proposal, shelf)` line. Simpler
  spellings considered: a key line restates names the fields already give,
  and a `primary key` block adds a second place to look. `get` takes the
  values in declaration order, `Edit.get(proposal, shelf)`, so the common
  one-field case reads exactly as before.

## Exact columns for money and time (P8)

A field typed `Decimal`, `Duration`, `Instant`, `ZonedTime`, `TimeZone`,
`Currency` or `Locale` is a column the database compares exactly as
Corvid does, so `where` and `order_by` work on it. Nothing is written in
source beyond the field's type: `price: Decimal`.

```corvid
table Invoice:
    key number: Int
    total: Decimal
    issued: Instant
    payment_terms: Duration
    currency: Currency

agent large_recent(since: Instant) -> List<Invoice>:
    return Invoice.where(fn (i) -> i.issued >= since and i.total > 1000.00).order_by(fn (i) -> i.total).descending()
```

| Type | Stored on SQLite as | Compared by |
| --- | --- | --- |
| `Decimal` | its exact text, such as `19.90` | value, through the `corvid_decimal` collation: `10 == 10.0`, and `9.50` sorts before `10.0` |
| `Duration` | its ISO 8601 text, such as `PT1H30M` | value, through the `corvid_duration` collation |
| `Instant` | UTC RFC 3339 text with all nine fractional digits, such as `2026-03-29T01:30:00.000000000Z` | text, which is time order: the year always has four digits in Corvid's range |
| `ZonedTime` | the instant's text and the zone, `2026-03-29T01:30:00.000000000Z[Europe/Berlin]` | equality only, as in Corvid: same instant and zone |
| `TimeZone`, `Currency`, `Locale` | their canonical names (`Europe/Berlin`, `EUR`, `en-GB`) | equality only |

Decisions, and the simpler alternatives considered:

- **Each type declares its own column type** (`DECIMAL TEXT`, `INSTANT
  TEXT` and so on), never a plain `TEXT` or `INTEGER`. The migration
  planner identifies a column by its declared type, so changing a field
  from `String` to `Decimal`, or from `Int` to `Duration`, is a type
  change it plans or refuses, never a silent reinterpretation of the
  stored values. Each name keeps SQLite's text affinity.
- **Decimals and durations compare through a collation.** Corvid's
  `Duration` holds 128-bit nanoseconds (about ±10,000 years), which no
  SQLite integer holds, and a decimal's equality is by value while its
  scale must round-trip (`19.90` stays `19.90`). An order-preserving
  encoding of a 38-digit decimal would need a second column for the
  scale and would be unreadable; a collation keeps one readable column
  and compares exactly. Corvid registers both collations on every SQLite
  connection it opens, including `corvid migrate`. A tool that opens the
  file without them still reads every value; comparing or sorting such a
  column there is refused with "no such collation sequence", never
  answered wrongly.
- **Instants need no collation.** Fixed-width UTC text sorts as time
  does, so the column also works in any SQLite tool. Instants carry no
  zone, so writing UTC loses nothing.
- **A zoned time stores its instant and zone, not its offset.** The
  offset follows from the two through the time-zone database, so a
  database update never makes two equal zoned times stop comparing equal.
  `ZonedTime`, `TimeZone`, `Currency` and `Locale` have no `<` in Corvid,
  so `order_by` on them is a compile error.
- **These types inside a JSON-stored column are allowed too.** A record
  or list field is stored as its public JSON text, which writes each of
  these types exactly; the database never compares inside it.
- **Postgres comes in a second slice.** Postgres identifies a column by
  its type as well, and `numeric` would hold decimals exactly, but
  telling an `Instant` text column from a `String` one needs a domain
  type per Corvid type, created by the migration that first needs it.
  Until that lands, these columns on a Postgres database are a compile
  error naming the slice, so a program never stores them a weaker way.
- **Across the tiers:** the backend knows the exact value, so a form
  field bound to one of these columns, an assistant's tool input and the
  stored row all carry the same exact text, which is already the JSON
  form B06 publishes. Nothing new reaches the frontend or assistants, and
  a query an assistant triggers through an action compares money and
  time as exactly as one a person triggers.

Slices: P8a, SQLite. P8b, Postgres through domains. P8c, `Bytes` as a
binary column.

**`Bytes` columns (P8c).** A `Bytes` field is a `BLOB` on SQLite and a
`bytea` on Postgres. Both store the bytes themselves and compare them
bytewise, so unlike P8a this works on Postgres at once, and the planner
tells the two apart by type. Decisions:

- **Raw queries keep redacting binary cells.** The runtime's database
  layer gained a binary value, but only table reads produce one:
  `query_table_rows` keeps a blob's bytes, while `std.db`'s `query`, whose
  rows go to agents as JSON, still reports `<blob:redacted>`. Splitting
  the reader rather than redacting at each JSON conversion keeps the one
  guarantee in one place.
- **Equality is allowed in `where`, ordering is not.** `Bytes` has no
  order in Corvid. Equality in the database is not constant-time as `==`
  in Corvid is. Finding a row by a hash is the common need (API keys
  stored as `sha256` of the token), and its timing reveals nothing about
  the token. The reference says to compare hashes, never secrets.
- **The trace holds bytes as `{"bytes_base64url": ...}`,** an object of
  its own so a replay never mistakes them for a text column's value.
- **Across the tiers:** a `Bytes` field already crosses JSON as base64url
  text for the frontend and assistants, so a form or tool sees the same
  value the table stores. Nothing new is exposed: the bytes leave the
  database only through a table's declared type.

## Open questions for review

- `renamed from` versus an attribute such as `@renamed(from: name)`.
  P3 ships the words, matching `where` refinements.
