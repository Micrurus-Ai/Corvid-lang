# Tables

A `table` is a record type whose rows are stored in a database the
program declares. Rows come back as typed records, so the fields a
program reads are checked like any record's: renaming a column reports
every read of it at compile time. SQL text never appears in source.

```corvid
database library:
    engine: sqlite
    location: env("LIBRARY_DATABASE")
    tables: Shelf

table Shelf:
    key shelf: Int
    owner: String
    title: String where len_between(1, 80)
    note: Option<String>
```

## Declaring a table

A table declares fields with the same grammar as a `type`. The field
marked `key` identifies a row and must be `Int` or `String`. A table
without a key is a compile error, because rows could not be saved with a
revision check.

When no single field identifies a row, mark several; together they are
the key, in the order declared:

```corvid
table Edit:
    key proposal: String
    key shelf: Int
    title: String
```

`Edit.get("p1", 2)` takes one value per key field, in that order. Rows
may share a `proposal` or a `shelf` but not both, inserting a repeated
pair is a conflict, and queries come back in key order: by `proposal`,
then `shelf`. The table is created with `PRIMARY KEY (proposal, shelf)`.

Every row also carries `revision: Int`, its last field, which the
database advances on each write. A table cannot declare a field named
`revision`. A row built in code may leave `revision` out, in the
positional form `Shelf(1, "ana", "Reading", None)` or the named form
`Shelf { shelf: 1, ... }`; it is then `0`, a row never stored.

Tables are private to the module that declares them. Another module
reaches the rows through an agent this module exports.

## Declaring the database

Where data lives is a decision the source states, so a `database` block
has no defaults:

| Setting | Meaning |
| --- | --- |
| `engine:` | `sqlite` or `postgres`. `mysql` is refused with a pointer to its roadmap slice until it runs. |
| `location:` | For SQLite, a path, `"library.sqlite"`, or `env("NAME")`, the environment variable that holds the path; a path is opened relative to the directory the program runs in. For Postgres, `env("NAME")` holding the connection string. |
| `tls:` | Postgres only: `required` or `disabled`. |
| `tables:` | The tables it holds, separated by commas. |

Leaving out a setting is a compile error that names it and its allowed
values. Every table must be listed in exactly one `database` block.

### Postgres

```corvid
database library:
    engine: postgres
    location: env("LIBRARY_DATABASE_URL")
    tls: required
    tables: Shelf
```

A Postgres `location` is the environment variable that holds the
connection string, such as `postgres://app@db.example.com/library`; a
string written in source is a compile error, because a connection
string usually holds a password. `tls:` is required:

| `tls:` | Connection |
| --- | --- |
| `required` | Encrypted, and the server's certificate must verify against the standard web roots; a server whose certificate does not verify is refused. |
| `disabled` | Not encrypted, for a database on the same machine or a private network. |

A SQLite database is a file, so a SQLite block refuses `tls:`.

Every operation, query, conflict and replay behaves the same on both
engines; only the SQL differs. A program moves from SQLite to Postgres by
changing its `database` block, with no change to the code that reads and
writes rows. Migrations are planned and applied as on SQLite; see
[Migrations](#migrations).

## Operations

Each table offers four operations on single rows as methods; [queries](#queries)
read many rows.

| Call | Result |
| --- | --- |
| `Shelf.get(key)` | `Option<Shelf>` |
| `Shelf.insert(row)` | `Result<Shelf, Shelf>`. `Ok` holds the stored row, with revision 1; `Err` holds the row already stored under that key. |
| `Shelf.save(row)` | `Result<Shelf, Option<Shelf>>`. It writes only if the stored row still has `row.revision`. `Ok` holds the stored row with its new revision; `Err` holds the row stored now, or `None` if it was deleted. |
| `Shelf.delete(row)` | `Result<Shelf, Option<Shelf>>`, checked like `save`. `Ok` holds the deleted row. |

There is no unchecked overwrite. A row read from the database carries
its revision, and editing it with `Shelf { title: t, ..shelf }` keeps
that revision. To check the revision a person saw instead, write it:
`Shelf { title: t, revision: seen, ..shelf }`.

```corvid
agent rename(shelf_id: Int, title: String) -> String:
    return match Shelf.get(shelf_id):
        None -> "there is no such shelf"
        Some(shelf) -> store(Shelf { title: title, ..shelf })

agent store(renamed: Shelf) -> String:
    return match Shelf.save(renamed):
        Ok(saved) -> "saved revision " + saved.revision.to_string()
        Err(current) -> "someone else changed the shelf first"
```

## Atomic writes

An `atomic:` block's table writes commit together when the block reaches
its end. If the block is left through an error, every write in it is
undone: a `?` that passes an error out of it, or a runtime failure such
as a division by zero.

```corvid
agent move_title(from: Shelf, to: Shelf) -> Result<String, Option<Shelf>>:
    atomic:
        cleared = Shelf.save(Shelf { title: "", ..from })?
        filled = Shelf.save(Shelf { title: from.title, ..to })?
    return Ok(filled.title)
```

If `to` changed since it was read, its save is a conflict, the `?` passes
the conflict out, and `from` keeps its title. A conflict is still an
ordinary value, so a block can also record it and carry on, keeping the
other writes:

```corvid
atomic:
    outcome = Shelf.save(edited)
    Review.save(Review { outcome: describe(outcome), ..review })
```

The checker keeps a block to what the database can undo. Inside one:

| Refused | Why |
| --- | --- |
| A tool, prompt, Python or imported call, `ask` or `choose` | The database cannot undo it. Do it before or after the block. |
| `approve`, `yield`, `parallel:` | The same. |
| `return`, or a `break` or `continue` that leaves the block | It would leave without saying whether to keep the writes. Finish the block, then leave. A `break` inside a loop of its own within the block is fine. |
| Another `atomic:` block | Its writes already commit together. |
| Tables in more than one database | A transaction cannot span databases. |

An agent called inside a block runs inside its transaction, so the same
rules apply to that agent's body, and its table operations join the
block's transaction.

Each block runs on its own connection, so no other agent's writes land
inside it. On SQLite a block also holds the file's writer turn from start
to finish: other writers wait their turn instead of deadlocking against
the open transaction. On Postgres the block runs at Postgres's default
isolation, read committed; each write is still compare-and-set on its
row's revision, so no write is lost. The trace records each block's
start and how it ended, as `table:<database>.begin`, `.commit` or
`.rollback`, and a replay serves them from the trace.

## Queries

A query starts with `Shelf.where(fn (s) -> condition)` or `Shelf.all()`,
then may add `.order_by(fn (s) -> s.column)`, `.descending()` and
`.take(n)`, in that order. The whole chain is one expression of type
`List<Shelf>`, and it runs as one database query: the rows are filtered,
sorted and limited in the database, not in the program.

```corvid
agent recent(owner: String, count: Int) -> List<Shelf>:
    return Shelf.where(fn (s) -> s.owner == owner and s.note != None).order_by(fn (s) -> s.title).descending().take(count)
```

A `where` condition may use:

| Form | Example |
| --- | --- |
| A comparison of a column with a value | `s.owner == owner`, `s.shelf >= 10`, `3 < s.shelf` |
| A missing value | `s.note == None`, `s.note != None` |
| A string test on a column | `s.title.contains(word)`, `s.title.starts_with("A")`, `s.title.ends_with(suffix)` |
| A `Bool` column | `s.archived` |
| `and`, `or`, `not`, `true`, `false` | `not s.archived or s.shelf == 1` |

The value on the other side of a comparison is computed by the agent
before the query runs, so it may use the agent's variables and calls but
not the row. Anything else is a compile error that says which part the
database cannot run: comparing two columns, calling a function on a
column, or comparing a column stored as JSON. `order_by` reads exactly
one column whose type has an order and that is not stored as JSON, and `take` takes an `Int`; a negative count stops the run
with its reason.

Results always come back in a defined order. With `order_by`, rows with
equal values keep key order; without it, rows come back in key order.
`None` sorts before any value, so it comes last under `descending`. Every value is bound as a parameter, and
`==` matches missing values, so `s.note == None` finds the rows with no
note.

A query uses `db_egress_read`, like `Shelf.get`, so pure `fn`s and
`@deterministic` agents cannot run one.
## Effects and determinism

`Shelf.get` uses the built-in `db_egress_read` effect, and `insert`,
`save` and `delete` use `db_egress_write`, the effects `std.db` uses.
A pure `fn` cannot call a table operation, and neither can a
`@deterministic` agent. Table operations run through the same database
handles as `std.db`, so they share its one-writer-per-file lock and its
parameter-only binding.

## Replay

Each table operation is recorded in the trace as a call named
`table:<Table>.<op>`, with its argument and its outcome; a query is
recorded as `table:<Table>.query` with its SQL, its parameters and the
rows it returned. A replay serves
every operation from the trace: reads return the recorded rows and
writes return the recorded outcomes, and the database is never opened.
A replayed run whose table operations differ from the recording stops
with a divergence naming the operation.

## Storage

| Field type | SQLite column | Postgres column |
| --- | --- | --- |
| `Int` | `INTEGER` | `bigint` |
| `Float` | `REAL` | `double precision` |
| `String` | `TEXT` | `text` |
| `Bool` | `BOOLEAN`, 0 or 1 | `boolean` |
| `Option<T>` | `T`'s column, nullable | `T`'s column, nullable |
| `Decimal` | `DECIMAL TEXT COLLATE corvid_decimal`: its exact text, compared by value | not yet |
| `Duration` | `DURATION TEXT COLLATE corvid_duration`: its ISO 8601 text, compared by value | not yet |
| `Instant` | `INSTANT TEXT`: UTC with all nine fractional digits, which sorts in time order | not yet |
| `ZonedTime` | `ZONED TIME TEXT`: the instant and the zone, `2026-03-29T01:30:00.000000000Z[Europe/Berlin]` | not yet |
| `TimeZone`, `Currency`, `Locale` | `TIME ZONE TEXT`, `CURRENCY TEXT`, `LOCALE TEXT`: the canonical name | not yet |
| `Option<Option<T>>`, records, lists, maps, sums | `JSON TEXT` holding the value's public JSON | `jsonb` holding the value's public JSON |

The table is named as declared, and columns as the fields are named.
Each kind of field has its own column type, so the stored schema records
how every column is stored and the migration planner can see a change
from `Int` to `Bool`, from `String` to a list or from `String` to
`Decimal`. Every text column keeps SQLite's text handling, so text such
as `5` is never read back as a number.

### Money and time

A `Decimal`, `Duration`, `Instant`, `ZonedTime`, `TimeZone`, `Currency`
or `Locale` field is a column the database compares exactly as Corvid
does, so a query on it means what the same comparison means in the
agent:

```corvid
agent large_recent(since: Instant) -> List<Invoice>:
    return Invoice.where(fn (i) -> i.issued >= since and i.total > 1000.00).order_by(fn (i) -> i.total).descending()
```

- `i.total == 10` matches a stored `10.0`, and `9.50` sorts before
  `10.0`. The stored text keeps its scale, so `19.90` reads back as
  `19.90`.
- Instants compare on the timeline to the nanosecond, whatever offset
  they were written with, and read back in UTC.
- `ZonedTime`, `TimeZone`, `Currency` and `Locale` compare only by
  equality, as in Corvid, so `order_by` on one is a compile error. Order
  zoned times by an `Instant` column instead.
- Inside a record, list or other JSON-stored column these types are
  their exact JSON text; the database does not compare inside it.

Decimal and duration columns compare through two SQLite collations,
`corvid_decimal` and `corvid_duration`, which Corvid registers on every
connection it opens, `corvid migrate` included. Another tool that opens
the file still reads every value, but SQLite refuses to compare, sort or
index those two columns there ("no such collation sequence") rather than
compare their text. Instant columns need no collation and sort correctly
in any SQLite tool.

On Postgres these columns are a compile error for now: Postgres needs a
column type for each that compares it exactly, which is
[planned](../proposals/typed-persistence.md#exact-columns-for-money-and-time-p8).
`Bytes` columns are planned too and are refused on both engines.

## Migrations

```sh
corvid migrate plan src/main.cor     # writes migrations/NNN_create_<tables>.sql or NNN_change_<tables>.sql
corvid migrate up --database library.sqlite
```

`corvid migrate plan` applies the existing migrations in `migrations/`
to an empty database, compares what that leaves with the
declared tables, and writes one new migration that brings the stored
tables to the declarations. It runs the new migration on that database
before writing it, so a plan that would fail is never written. Review
it, then apply it with `corvid migrate up`, passing the database the
`location` names. `migrate up` runs each migration in one transaction,
so a migration that fails changes nothing. A program that uses a table
no migration created fails with a message naming these commands.

On SQLite the planner's database is in memory. On Postgres it is a scratch
database you name, and `migrate up` reads the database from the program:

```sh
corvid migrate plan src/main.cor --scratch SCRATCH_DATABASE_URL
corvid migrate up --program src/main.cor
```

`--scratch` names the environment variable holding an empty database's
connection string. The planner refuses a database that holds any table,
so it can never be pointed at real data, and it applies the migrations
inside a transaction it rolls back, so the scratch database ends empty.
`--program` connects to the database the program's `database` block
declares, with its `tls:` policy, so the connection string never appears
on the command line; `migrate status` and `migrate down` take it too.
A migration may not begin or end a transaction itself: one containing
`BEGIN`, `COMMIT` or `ROLLBACK` is refused before it runs, because each
migration runs in the runner's own transaction. Planning and migrating a
program whose tables live in more than one database is planned work.

The planner never guesses about data. A rename and a drop are declared
in the table:

```corvid
table Shelf:
    key shelf: Int
    name: String renamed from title
    note: Option<String>
    retired legacy_note, old_owner
```

| Declaration change | Migration |
| --- | --- |
| A new table | `CREATE TABLE` |
| `name: String renamed from title` | renames the stored column `title` to `name`, keeping its values |
| `retired legacy_note` | drops the stored column and its data |
| A new `Option<T>` field | adds the column; stored rows have `None` |
| `T` to `Option<T>`, `Option<T>` to `T`, `Int` to `Float` | rebuilds the table, copying every row; making a column required fails, changing nothing, while any row has no value |

Everything else is refused, naming the column and what to do instead:

- A stored column the table no longer declares, unless it is `retired`
  or named in a `renamed from`, because dropping it would lose data.
- A new field that is not an `Option`, because stored rows have no value
  for it. Add it as an `Option`, fill it, then make it required.
- A type change that does not convert every value exactly, such as
  `String` to `Int`. Add a new column, fill it from the old one, then
  retire the old one.
- A changed key. Create a new table.

`renamed from` and `retired` may stay in the source after their
migration has run; the planner does nothing for a name no longer stored.
A name in either clause cannot also be a declared field, and each stored
name appears in one clause. `retired` written without a colon is the
clause, so a table may still have a field named `retired: Bool`.

## Not yet

These are later slices of [B09](../proposals/typed-persistence.md):
MySQL and the native target. Programs that use tables run
on the interpreter.
