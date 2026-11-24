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

A table declares fields with the same grammar as a `type`. One field is
marked `key`; it identifies a row and must be `Int` or `String`. A table
without a key is a compile error, because rows could not be saved with a
revision check.

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
| `engine:` | `sqlite`. `postgres` and `mysql` are refused with a pointer to their roadmap slices until they run. |
| `location:` | A path, `"library.sqlite"`, or `env("NAME")`, the environment variable that holds the path. A path is opened relative to the directory the program runs in. |
| `tables:` | The tables it holds, separated by commas. |

Leaving out a setting is a compile error that names it and its allowed
values. Every table must be listed in exactly one `database` block.

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
one column, and `take` takes an `Int`; a negative count stops the run
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

| Field type | Column |
| --- | --- |
| `Int` | `INTEGER` |
| `Float` | `REAL` |
| `String` | `TEXT` |
| `Bool` | `BOOLEAN`, 0 or 1 |
| `Option<T>` | `T`'s column, nullable |
| `Option<Option<T>>`, records, lists, maps, sums | `JSON TEXT` holding the value's public JSON |

The table is named as declared, and columns as the fields are named.
Each kind of field has its own column type, so the stored schema records
how every column is stored and the migration planner can see a change
from `Int` to `Bool` or from `String` to a list. `JSON TEXT` columns keep
SQLite's text handling, so JSON such as `5` is never read back as a
number.

## Migrations

```sh
corvid migrate plan src/main.cor     # writes migrations/NNN_create_<tables>.sql or NNN_change_<tables>.sql
corvid migrate up --database library.sqlite
```

`corvid migrate plan` applies the existing migrations in `migrations/`
to an empty in-memory database, compares what that leaves with the
declared tables, and writes one new migration that brings the stored
tables to the declarations. It runs the new migration on that database
before writing it, so a plan that would fail is never written. Review
it, then apply it with `corvid migrate up`, passing the database the
`location` names. `migrate up` runs each migration in one transaction,
so a migration that fails changes nothing. A program that uses a table
no migration created fails with a message naming these commands.

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

These are later slices of [B09](../proposals/b09-typed-persistence.md):
Postgres and MySQL, and the native target. Programs that use tables run
on the interpreter.
