# B09: Typed persistence

Status: proposed, 2026-11-24. Backend and AI syntax; no frontend grammar
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
    renamed = Shelf { title: title, ..shelf }
    return match save(renamed, expecting: seen):       # Result<Shelf, SaveConflict<Shelf>>
        Ok(saved) -> Ok(saved)
        Err(conflict) -> Err("someone else changed shelf " + conflict.key.to_string())

agent shelves_of(user: String) -> List<Shelf>:
    return Shelf.where(fn (s) -> s.owner == user).order_by(fn (s) -> s.title).take(20)
```

## Decisions

1. **A table is a record type.** `table Shelf:` declares the record
   `Shelf` and its storage. Every row also has a read-only `revision:
   Int` that the database advances on each write. Queries return
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
4. **Writes always carry the revision they expect.** `save(row,
   expecting: revision)` returns `Result<T, SaveConflict<T>>`, where
   `SaveConflict<T>` has `key`, `expected`, and `current:
   Option<T>` (`None` when the row was deleted). `insert(row)` returns
   `Result<T, InsertConflict<T>>` when the key exists. `delete(row,
   expecting: revision)` follows `save`. There is no unchecked
   overwrite; a program that wants last-writer-wins reads the current
   revision and passes it, which is visible in the source.
5. **Queries are typed functions, not strings.** `T.get(key)`,
   `T.where(fn (row) -> condition)`, `.order_by(fn (row) -> column)`,
   `.descending()`, `.take(n)` and `.all()`. A condition is a function
   over the row, so `s.owner` is an ordinary checked field read, and
   locals are parameters. The checker accepts only conditions it can
   translate to SQL: comparisons, `and`, `or`, `not`, `in` over a list,
   `contains` / `starts_with` on strings, and `== None`. Anything else
   is a compile error naming the part that cannot run in the
   database. SQL text never appears in source, so injection is not
   expressible.
6. **Effects and replay compose.** Each table gets `reads_<table>` /
   `writes_<table>` effects (reads and writes of `Shelf` are
   `reads_shelf` / `writes_shelf`), so `@budget`, approvals and effect
   rows see database work, and an agent that only reads cannot write.
   Every query and write is recorded in the trace with its result, and
   replay returns the recorded rows without touching the database.
7. **Migrations come from declarations.** `corvid migrate plan` compares
   the declared tables with the schema the last applied migration
   left and writes a reviewed SQL migration into `migrations/`, which
   the existing `corvid migrate up` applies. A rename is declared,
   `title: String renamed from name`, because the planner cannot tell
   a rename from a drop and an add. Dropping a column loses data, so
   it is declared too: `retired: legacy_note`. A plan that would drop
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

- [ ] P1. `table` and `database` declarations, the row record with
  `revision`, `T.get`, `insert`, `save(expecting:)`, `delete`,
  `SaveConflict<T>` and `InsertConflict<T>`, on SQLite through the
  interpreter, with effects and replay. `corvid migrate plan` writes
  the `CREATE TABLE` for new tables.
- [ ] P2. Typed queries: `where`, `order_by`, `descending`, `take`,
  `all`, and the compile-time check that a condition can run in the
  database.
- [ ] P3. Migrations from declaration changes: added and altered
  columns, `renamed from`, `retired`, and refusal of undeclared drops.
- [ ] P4. Postgres, with the cross-engine query suite.
- [ ] P5. MySQL on the same suite.
- [ ] P6. Native target.
- [ ] P7. Port `examples/reading_list` from hand-written SQL to tables.

## Open questions for review

- `renamed from` versus an attribute such as `@renamed(from: name)`.
  The proposal uses words, matching `where` refinements.
- Whether `Shelf { ..shelf }` should be required for updates, or a
  dedicated `shelf.with(title: ...)` form is clearer.
