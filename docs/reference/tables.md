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

Each table offers four operations as methods:

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

## Effects and determinism

`Shelf.get` uses the built-in `db_egress_read` effect, and `insert`,
`save` and `delete` use `db_egress_write`, the effects `std.db` uses.
A pure `fn` cannot call a table operation, and neither can a
`@deterministic` agent. Table operations run through the same database
handles as `std.db`, so they share its one-writer-per-file lock and its
parameter-only binding.

## Replay

Each table operation is recorded in the trace as a call named
`table:<Table>.<op>`, with its argument and its outcome. A replay serves
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
| `Bool` | `INTEGER`, 0 or 1 |
| `Option<T>` | `T`'s column, nullable |
| `Option<Option<T>>`, records, lists, maps, sums | `TEXT` holding the value's public JSON |

The table is named as declared, and columns as the fields are named.

## Migrations

```sh
corvid migrate plan src/main.cor     # writes migrations/NNN_create_<tables>.sql
corvid migrate up --database library.sqlite
```

`corvid migrate plan` applies the existing migrations in `migrations/`
to an empty in-memory database and writes a new migration creating each
declared table they do not create. Review it, then apply it with
`corvid migrate up`, passing the database the `location` names. A table
whose columns differ from what the migrations create is reported and
refused; planning changes to existing tables is slice P3. A program that
uses a table no migration created fails with a message naming these
commands.

## Not yet

These are later slices of [B09](../proposals/b09-typed-persistence.md):
queries beyond get-by-key (`where`, `order_by`, `take`), migrations of
changed tables with declared renames and drops, Postgres and MySQL,
and the native target. Programs that
use tables run on the interpreter.
