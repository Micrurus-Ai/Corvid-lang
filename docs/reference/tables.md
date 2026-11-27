# Tables

A `table` is a record type whose rows are stored in a database the
program declares. Rows come back as typed records, so the fields a
program reads are checked like any record's: renaming a column reports
every read of it at compile time. SQL text never appears in source.

```corvid
database library:
    engine: sqlite
    location: env("LIBRARY_DATABASE")

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
| `connections:` | Postgres only: how many connections the program may open, from 1 to 1000. |

Leaving out a setting is a compile error that names it and its allowed
values.

A table lives in its module's `database` block, so most tables say
nothing about where they live. When a module declares more than one
database, each table names its own with `in`:

```corvid
database orders:
    engine: postgres
    location: env("ORDERS_DATABASE_URL")
    tls: required
    connections: 10

database analytics:
    engine: sqlite
    location: "analytics.sqlite"

table Order in orders:
    key id: Int
    total: Decimal

table PageView in analytics:
    key id: Int
    path: String
```

A table without `in` in a module with several databases is a compile
error that names them, and so is a table in a module with none.

### One database, many modules

An application has one database and many modules. Declare the database
once as `public database` and import it where tables live:

```corvid
# storage.cor
public database library:
    engine: postgres
    location: env("LIBRARY_DATABASE_URL")
    tls: required
    connections: 10

# shelves.cor
import "./storage" use library

table Shelf in library:
    key shelf: Int
    title: String

public agent rename(shelf: Int, title: String) -> Result<Shelf, String>:
    ...
```

Every module's tables in `library` are one database:

- An `atomic:` block may call agents in other modules, and their writes
  join the block's transaction. Each one is checked in its own module, so
  an imported agent that calls a tool is refused inside the block, by
  name.
- `corvid migrate plan src/main.cor` plans every table of the program,
  from every module, in one migration history, with the indexes their
  queries need. Two modules declaring a table of the same name is an
  error, because a database stores one table under each name.
- Tables stay private: other modules reach the rows only through the
  agents a table's module exports. Sharing the database shares where rows
  live, not access to them.

### Postgres

```corvid
database library:
    engine: postgres
    location: env("LIBRARY_DATABASE_URL")
    tls: required
    connections: 10
```

A Postgres `location` is the environment variable that holds the
connection string, such as `postgres://app@db.example.com/library`; a
string written in source is a compile error, because a connection
string usually holds a password. `tls:` is required:

| `tls:` | Connection |
| --- | --- |
| `required` | Encrypted, and the server's certificate must verify against the standard web roots; a server whose certificate does not verify is refused. |
| `disabled` | Not encrypted, for a database on the same machine or a private network. |

`connections:` is required too: how many connections the program may
open to the database, from 1 to 1000. A managed Postgres caps its
connections, so going over is an outage, and how many to use is a
decision about cost and capacity, not a default. Every table operation
borrows a connection for its one statement and returns it, and an
`atomic:` block holds one until it ends. When all are in use, the next
operation or block waits its turn rather than opening another, so a
burst of agents queues in the program instead of failing at the
database. A connection the server closed is replaced on the next borrow.

A SQLite database is a file, so a SQLite block refuses `tls:` and
`connections:`; Corvid opens the file as needed and queues its writers.

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
| `Shelf.insert(row)` | `Result<Shelf, Conflict<Shelf>>`. `Ok` holds the stored row, with revision 1. Its conflict is only `Taken(holder)`: the row with that key or one of its [unique values](#unique-values). |
| `Shelf.save(row)` | `Result<Shelf, Conflict<Shelf>>`. It writes only if the stored row still has `row.revision`. `Ok` holds the stored row with its new revision. Its conflict is `Changed(current)` or `Deleted`, and also `Taken(holder)` when the table declares [unique values](#unique-values). |
| `Shelf.delete(row)` | `Result<Shelf, Conflict<Shelf>>`, checked like `save`. `Ok` holds the deleted row. Its conflict is `Changed(current)` or `Deleted`. |

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
        Err(Changed(current)) -> "someone else changed the shelf first"
        Err(Deleted) -> "the shelf was removed"
```

### Conflicts

`Conflict<Row>` names why a write did not happen, one case per reason:

| Case | Meaning |
| --- | --- |
| `Changed(current)` | The row changed since it was read; `current` is the row now. |
| `Deleted` | The row is gone. |
| `Taken(holder)` | Another row holds its key or one of its unique values. |

```corvid
agent explain(conflict: Conflict<Member>) -> String:
    return match conflict:
        Changed(current) -> "someone else edited this member first"
        Deleted -> "this member was removed"
        Taken(holder) -> holder.email + " already has that value"
```

A `match` covers the cases the value can be. Each operation returns
only the conflicts it can produce, so a match on an `insert` is complete
with `Taken` alone, and one on a `save` needs `Taken` only when the table
has `unique` values:

```corvid
agent join(member: Member) -> String:
    return match Member.insert(member):
        Ok(stored) -> "welcome"
        Err(Taken(holder)) -> "that email is already a member"
```

A written `Conflict<Row>` can be any of the three, so a function taking
one needs all three arms, or `_`, and the conflict of any operation can
be passed to it. An arm naming a case the value can never be, such as
`Err(Changed(_))` on an `insert`, is a compile error, because it could
never run. Code can
build each case, `Changed(row)`, `Deleted` and `Taken(row)`, so a test
or a fake store returns the same value a table does. A conflict crosses
JSON as a sum type: `{"tag": "variant", "type": "Conflict", "variant":
"Taken", "fields": [holder]}`. The generated clients name it too: a
union of the three cases in TypeScript, `Conflict[Row]` with
`Conflict.Taken(holder)` in Python, and `CorvidConflict<Row>` in Swift
and Kotlin.

The trace records which case each write returned, so a replay returns
the same case. [Why one conflict type](../proposals/write-conflicts.md).

## Atomic writes

An `atomic:` block's table writes commit together when the block reaches
its end. If the block is left through an error, every write in it is
undone: a `?` that passes an error out of it, or a runtime failure such
as a division by zero.

```corvid
agent move_title(from: Shelf, to: Shelf) -> Result<String, Conflict<Shelf>>:
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
| A column holding one of a list | `owners.contains(s.owner)`, `[1, 3].contains(s.shelf)` |

`values.contains(s.column)` runs as `column IN (...)` with each value
bound, and keeps exactly the rows the same test keeps in Corvid: a `None`
in the list matches a missing value, an empty list keeps nothing, and a
`Decimal` matches by value. The list is computed by the agent before the
query runs. A list holds at most 10,000 values; a larger one stops the run
with its reason.

To filter by another table, use its query's keys directly:

```corvid
agent my_cards(owner: String) -> List<Card>:
    return Card.where(fn (c) -> Board.where(fn (b) -> b.owner == owner).keys().contains(c.board))
```

`keys()` ends a query with its rows' keys, in key order, for a table keyed
by one column. Inside a `where` on a table in the same database, it runs
as an inner query in the same statement, `board IN (SELECT board FROM
Board WHERE ...)`, so both tables are read at one moment, the keys never
leave the database, and there is no limit on how many there are. A table
in another database is read first, as a list. The inner query cannot read
the outer row.

`by_key()` ends a query with a `Map` from each matching row's key to the
row, in key order, for a table keyed by one column. A program that shows
each card with its board reads two queries instead of one per card:

```corvid
agent card_titles() -> String:
    boards = Board.all().by_key()
    out = ""
    for card in Card.where(fn (c) -> boards.keys().contains(c.board)):
        out = out + card.title + " on " + match boards[card.board]:
            Some(board) -> board.title
            None -> "no board"
    return out
```

A `live` query may filter by another table's keys, or by a list built from
literals, `path`, `actor` and table queries. It then follows every table
its query reads: giving a board to a person brings that board's cards into
their live list, though no card was written. A write to the delivered
table is re-checked row by row, as always. A write to another table it
reads re-runs the query and sends a snapshot when rows leave, because those
rows were not written and have no removal revision of their own. A list
that calls an agent, tool, prompt or function by name is refused in `live`,
because the server cannot see what the call reads.

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

### Counts, totals and extremes

A query may end in one stage that reduces its rows to one value, run by
the database instead of reading every row:

| Stage | Type | For no rows |
| --- | --- | --- |
| `.count()` | `Int` | `0` |
| `.sum(fn (s) -> s.column)` | the column's type: `Int`, `Decimal` or `Duration` | `0` |
| `.min(fn (s) -> s.column)`, `.max(...)` | `Option` of the column's type | `None` |
| `.count_by(fn (s) -> s.key)` | `Map<K, Int>` | an empty map |
| `.sum_by(fn (s) -> s.key, fn (s) -> s.column)` | `Map<K, T>`, `T` as for `sum` | an empty map |

`count_by` and `sum_by` group the rows by a column and give one entry per
value that has rows, in the column's order, so `m[k]` is `None` for a key
with none. A group's total follows the rules of `sum`. The key column
must be one the database compares as Corvid does: not one stored as JSON,
and not a `Float`, whose values do not compare exactly. `Decimal` keys
group by value, so `1.0` and `1.00` are one key.

```corvid
agent shelf_summary(owner: String) -> String:
    count = Shelf.where(fn (s) -> s.owner == owner).count()
    return match Shelf.where(fn (s) -> s.owner == owner).max(fn (s) -> s.shelf):
        Some(top) -> count.to_string() + " shelves, up to " + top.to_string()
        None -> "no shelves"
```

- An `Int` total that does not fit in 64 bits stops the run with its
  reason, as `+` does, never a wrapped or rounded number.
- A `Decimal` or `Duration` total is exact: the column's values are read
  and added with Corvid's own `+`, because SQLite stores both as text.
- `sum` of a `Float` column is a compile error, because a float total
  depends on the order the rows are added. Store an exact total as a
  `Decimal`.
- `min` and `max` compare values as `order_by` does, so `9.50` is less
  than `10.0`, and they skip missing values. A column `order_by` refuses
  is refused here too.
- `order_by`, `descending` and `take` cannot change an aggregate, so a
  chain with one is a compile error that says to remove them.
- There is no `average`, because dividing needs a rounding decision.
  Divide `sum` by `count` with the rounding you choose.
- A `live` declaration delivers rows, so its query cannot end in one. A
  live count is planned in [aggregates and joins](../proposals/query-aggregates-and-joins.md).

A recorded run replays each aggregate from the trace, as it replays a
query.

## Unique values

A `unique` line names columns no two rows may share: `unique email` for
one column, `unique tenant, slug` for columns that must not repeat
together, so two tenants can each have a page called `home`. Each line
is its own rule.

```corvid
table Member:
    key id: Int
    email: String
    nickname: Option<String>
    unique email
    unique nickname
```

A write that would repeat a value is a conflict, returned as a value
like any other:

| Call | When another row holds the value |
| --- | --- |
| `Member.insert(row)` | `Err(Taken(holder))`: the row that has it, as for a repeated key. |
| `Member.save(row)` | `Err(Taken(holder))`, and nothing is written. A row that changed since it was read is `Err(Changed(current))` instead. |

A missing value never conflicts, so many members may have no nickname.
Each rule is a unique index that `corvid migrate plan` creates and drops
with the line (`Member_unique_email`), and it also serves lookups by those
columns. Adding a `unique` line to a table whose rows already repeat the
value makes `corvid migrate up` fail and name the index, rather than
choose which row to keep: change or remove the repeated rows first. The
key's columns are unique already, so a line that restates them is a
compile error, as is one naming a JSON-stored column.

## Indexes

You never write an index. A table is private to its module, so the
compiler sees every query the table gets, and `corvid migrate plan`
creates the index each query needs: the columns the condition requires
to be equal (joined by `and` at the top of the condition), in
declaration order, then the column it orders by, or else the column it
compares with `<`, `<=`, `>` or `>=`.

```corvid
agent latest(owner: String) -> List<Job>:
    return Job.where(fn (j) -> j.owner == owner and not j.paused).order_by(fn (j) -> j.job).descending().take(1)
```

plans

```sql
-- `Job`: index (owner, job) for the queries in `latest`.
CREATE INDEX IF NOT EXISTS "Job_by_owner_job" ON "Job" ("owner", "job");
```

A query the key serves needs no index, and an index whose columns lead a
longer one's is served by the longer one. When a query changes, the next
plan replaces its index; when no query needs an index Corvid made, the
plan drops it. Corvid names its indexes `Table_by_columns` and touches
no other index, so an index a database administrator adds by hand
stays. Conditions under `or` or `not` are filtered by the database
without an index of their own.

## Effects and determinism

`Shelf.get` uses the built-in `db_egress_read` effect, and `insert`,
`save` and `delete` use `db_egress_write`, the effects `std.db` uses.
A pure `fn` cannot call a table operation, and neither can a
`@deterministic` agent. Table operations run through the same database
handles as `std.db`, so they share its one-writer-per-file lock and its
parameter-only binding.

A query returns at most its `take` count of rows, so a `@budget` agent
that loops over `Member.all().take(3)` is charged three times the loop's
body. The count may be a refined field, `take(search.limit)` with `limit:
Int where between(1, 50)`, which charges fifty times: every value of a
refined field holds its refinement, so the maximum is a real bound. A
loop over a query whose count has no such bound has none the compiler
can prove, and the budget check warns rather than guessing.

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
| `Decimal` | `DECIMAL TEXT COLLATE corvid_decimal`: its exact text, compared by value | `corvid_decimal`, a `numeric` domain |
| `Duration` | `DURATION TEXT COLLATE corvid_duration`: its count of nanoseconds, compared by value | `corvid_duration`, a `numeric(39, 0)` domain |
| `Instant` | `INSTANT TEXT`: UTC with all nine fractional digits, which sorts in time order | `corvid_instant`, a byte-ordered `text` domain that checks the form |
| `ZonedTime` | `ZONED TIME TEXT`: the instant and the zone, `2026-03-29T01:30:00.000000000Z[Europe/Berlin]` | `corvid_zoned_time`, a byte-ordered `text` domain |
| `TimeZone`, `Currency`, `Locale` | `TIME ZONE TEXT`, `CURRENCY TEXT`, `LOCALE TEXT`: the canonical name | `corvid_time_zone`, `corvid_currency`, `corvid_locale`, byte-ordered `text` domains |
| `Bytes` | `BLOB`: the bytes themselves | `bytea` |
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

On Postgres each of these types is declared with a domain of its own,
so the stored schema says which Corvid type a column holds. A decimal is
`numeric`, which is exact at any scale, and a duration is its nanosecond
count as `numeric`; the text forms compare in byte order, so a locale's
sorting rules never reorder instants. The migration that first needs a
domain creates it, and running that statement again does nothing. Other
tools can read these columns as ordinary numbers and text.
### Binary values

A `Bytes` field is stored as the bytes themselves, on both engines, and
reads back exactly. A query can find a row by one, which is how a
program looks up an API key by its token's hash:

```corvid
agent owner_of(token: String) -> List<ApiKey>:
    digest = token.utf8().sha256()
    return ApiKey.where(fn (k) -> k.token_hash == digest)
```

`Bytes` has no order, so `order_by` on one is a compile error. In Corvid
code, `==` on two `Bytes` takes the same time whatever they hold; a
database lookup does not, which is harmless for a hash of a random token
and is why a lookup compares a hash, never the secret itself. A raw
`std.db` query still reports a binary cell as `<blob:redacted>`, so the
bytes reach an agent only through a table's declared type. A recorded
run keeps them in the trace as base64url text.

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

### Several databases

Each database keeps its own migration history. A program with one
database keeps it in `migrations/`. A program with several keeps each in
`migrations/<database>/`, and `corvid migrate plan src/main.cor` plans
every database's tables into its own folder:

```text
migrations/
  orders/001_create_order.sql
  analytics/001_create_pageview.sql
```

`corvid migrate up --program src/main.cor` applies each database's
history to that database, and `status` and `down` work the same way; a
`--state target/state.json` becomes `target/state.orders.json` and
`target/state.analytics.json`. When a program gains a second database,
the files already in `migrations/` belong to the first one. The planner
refuses to guess: it stops and asks you to move them into that
database's folder.

## Not yet

These are later slices of [B09](../proposals/typed-persistence.md):
MySQL and the native target. Programs that use tables run
on the interpreter.
