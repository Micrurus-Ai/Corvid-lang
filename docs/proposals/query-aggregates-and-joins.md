# Aggregates and joins in table queries (limits plan row 9)

**Status:** proposal, 2026-11-26. No grammar change: every form below is a method call the parser already reads, so it is built without a review window. A `live` declaration that returns a single value instead of rows would change the wire protocol and is left to its own proposal.
**Owner:** backend (Claude Code).
**Roadmap:** [limits plan](removing-limits.md) row 9; [typed persistence](typed-persistence.md) non-scope.

## The problem

A table query returns rows: `Card.where(fn (c) -> c.board == board).order_by(fn (c) -> c.rank)`. Anything else is done in Corvid after reading every row:
- "how many open cards", "the total of this customer's orders" and "the newest due date" read the whole result to produce one number;
- "cards on boards I own" reads every board, then every card, and filters in the agent;
- showing each card with its board's title takes one query per card, or a hand-built map.

Python and TypeScript developers get these from SQL or an ORM. Corvid should give them as typed queries the compiler checks, that run in the database, and that mean exactly what the same code would mean in memory.

## The design, in a realistic program

```corvid
table Board:
    key board: Int
    owner: String
    title: String

table Card:
    key card: Int
    board: Int
    status: Status
    points: Int
    due: Option<Instant>

agent my_cards(by: Caller) -> List<Card>:
    return Card.where(fn (c) -> Board.where(fn (b) -> b.owner == by.id).keys().contains(c.board) and c.status != Done)

agent board_summary(board: Int) -> Summary:
    open = Card.where(fn (c) -> c.board == board and c.status != Done).count()
    points = Card.where(fn (c) -> c.board == board).sum(fn (c) -> c.points)
    next_due = Card.where(fn (c) -> c.board == board).max(fn (c) -> c.due)
    by_status = Card.where(fn (c) -> c.board == board).count_by(fn (c) -> c.status)
    return Summary(open, points, next_due, by_status)

agent cards_with_titles(by: Caller) -> List<CardView>:
    boards = Board.where(fn (b) -> b.owner == by.id).by_key()
    out = []
    for card in Card.where(fn (c) -> boards.keys().contains(c.board)):
        out = out + [CardView(card, boards[card.board])]
    return out
```

Each stage ends one query, which runs once, in the database. As today, a query chain is one expression whose parts are not values, so `count()` is `SELECT count(*)`, never a read of every row.

### Aggregates

A query (`where` or `all`, with no `order_by` or `take`, since neither changes an aggregate) ends in at most one of:

| Stage | Type | SQL | Notes |
| --- | --- | --- | --- |
| `count()` | `Int` | `count(*)` | |
| `sum(fn (r) -> r.col)` | the column's type: `Int`, `Decimal` or `Duration` | `sum` | `0` for no rows. An `Int` sum that leaves 64 bits is a trap naming the query, as `+` is, never a wrapped or float result. A `Decimal` or `Duration` sum is added exactly by the runtime over the column's stored values, because SQLite has no exact decimal sum. `Float` is refused: its sum depends on the order rows are added. |
| `min(fn (r) -> r.col)`, `max(...)` | `Option<T>` | `min`, `max` | `None` for no rows or only missing values. Any column `order_by` takes, compared as `order_by` compares it. |
| `count_by(fn (r) -> r.col)` | `Map<K, Int>` | `GROUP BY` | Keys with no rows are absent: `m[k]` is `None`. |
| `sum_by(fn (r) -> r.key, fn (r) -> r.col)` | `Map<K, T>` | `GROUP BY ... sum` | As `sum`, per key. |

There is no `average`. It needs a rounding decision, and Corvid never rounds silently, so a program writes `sum` and `count` and divides with the rounding it chooses.

### Filtering by another table

Inside a `where`, `List.contains` on another query's `keys()` runs in the database as `col IN (SELECT key FROM ...)`:

```corvid
Card.where(fn (c) -> Board.where(fn (b) -> b.owner == by.id).keys().contains(c.board))
```

- `keys()` on a query is a `List` of its table's key, `List<Int>` here, so `contains(c.board)` type-checks as ordinary code, and means exactly what it would in memory.
- It combines with `and`, `or` and `not` like any condition, and the inner query takes the same conditions as any `where`.
- **Narrowed for now:** the inner table must have a key of one column. A key of several columns is a compile error naming the table, until rows can be compared whole.
- The inner query runs inside the outer one's statement, so it reads the same snapshot.
- `contains(row.col)` on a list the agent already holds, such as `boards.keys()` in `cards_with_titles`, runs as `col IN (?, ?, ...)` with the list's values bound, the way a compared value is today. An empty list matches nothing.

### Looking rows up by key

`by_key()` on a query is a `Map<K, Row>` from each row's key to the row: one query, so a program that shows each card with its board reads two queries, not one per card. A key of several columns is a compile error, for the same reason as above.

### `live`

A `live` query whose `where` filters by another table depends on both tables. A commit to either re-runs it in full: re-checking only the written rows is correct for the outer table (L2) but not when the inner table changes, because that can change which rows match without writing them. The L2 re-check still applies when only the outer table was written. A `live` that returns a count or a sum is a new kind of `live` with a different wire message, and waits on its own proposal with frontend development.

## Three tiers

- **Backend:** one statement per question, typed results, no N+1 reads, and the same compile-time check that a condition can run in the database.
- **Frontend:** a summary route returns typed counts and totals the UI shows directly. A future `live` count would update a badge without reading rows.
- **AI:** an assistant asking "how many open cards are on my boards" calls a declared action that runs one counted query, at a cost the budget can see, instead of reading every row into a prompt.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| A `join` stage returning pairs | Needs a tuple or generated pair type, and most joins are filters or lookups, which `contains` and `by_key` say directly. |
| Declared references (`board: Int references Board`) navigated as `c.board.owner` | `c.board` would be an `Int` outside a query and a row inside one. References belong with foreign keys and a declared delete policy, a later proposal. |
| `average()` | Hides a rounding decision. |
| SQLite's `sum` for decimals | Decimals are stored as text, so it would add floats. |
| Running aggregates in Corvid over the rows | Reads every row to produce one number, the cost this removes. |

## Slices

- [x] J1. `count`, `sum`, `min` and `max`: checker, lowering, SQL on both engines, the exact `Decimal` and `Duration` sums, a trap on `Int` overflow, and parity tests against the same computation in Corvid over the rows. Built 2026-11-26. `min` and `max` run as the query ordered by the column, without missing values, limited to one row, so the database's own collations decide and the row reads back through the existing decoder. `count` and an `Int` `sum` return one column through the same recorded select. The tests run on SQLite here, and on Postgres where `CORVID_TEST_POSTGRES_URL` is set.
- [x] J2. `count_by` and `sum_by` to a `Map`. Built 2026-11-26: `GROUP BY` the key, ordered by it, for counts and `Int` totals; an exact total reads each row's key and value and adds per key with Corvid's `+`. A test requires each group to equal the same aggregate over that key's rows alone.
- [x] J3a. `contains(row.col)` on a list the agent holds, as `col IN (values)`, with `None` matched as `==` matches it, parity tests against filtering in memory, a refusal of `text.contains(row.col)`, and a refusal in `live` until J5. Built 2026-11-26. Narrowed: at most 10,000 values per list, a parameter bound both engines can take.
- [ ] J3b. `Other.where(...).keys().contains(row.col)` as `IN (SELECT ...)` in one statement, with the composite-key refusal. This also widens J3a: a list of any size, and one snapshot for both tables.
- [ ] J4. `by_key()`.
- [ ] J5. `live` over a query that filters by another table: re-run on commits to either table, with a test that a write to the inner table that changes membership reaches the subscriber.
- [ ] J6. Native code, with native table queries.

## Non-scope

- Foreign keys and their delete policy.
- A `live` count or total (new wire message).
- Window functions, `HAVING`, and subqueries other than `keys()`.
