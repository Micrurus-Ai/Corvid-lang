# Live queries at scale (limits plan row 13)

**Status:** proposal, 2026-11-26. No grammar change: the `live` declaration and its wire protocol are unchanged, so it is built without a review window.
**Owner:** backend (Claude Code).
**Roadmap:** [limits plan](removing-limits.md) row 13; [real-time](real-time.md) RT-3.

## The problem

A `live` declaration's handler returns one table query (`Card.where(fn (c) -> c.board == path.board).order_by(fn (c) -> c.card)`). For every open subscription, every commit that touches the table re-runs the whole query and diffs the result against the last one (`serve_cmd/live/feed.rs`). With N people watching the same board and W writes a second, the server runs N × W full queries a second, though one board's watchers all see the same rows. The cost grows with watchers times writes times table size, which caps a board at a few hundred watchers.

## The proposal, in two steps

### 1. One evaluation per distinct query

Subscriptions whose query would return the same rows share one evaluation. Two subscriptions share when they name the same `live` declaration with the same path arguments, and either the query does not read `actor` or both callers are the same actor with the same authority. Whether the query reads `actor` is decided from the IR: the handler's `actor` local appears in the query or not. Per commit, each shared query runs once and its changes go to every subscriber; a new subscriber receives the shared query's current snapshot. Authority is still checked per subscriber when it subscribes and on every authority change, exactly as now; sharing an evaluation never shares a permission.

Cost after step 1: distinct queries × writes, independent of watchers.

### 2. Re-checking only the written rows

A commit already names the keys it wrote (`TableCommit.rows`, RT-6). Instead of re-running the full query, the shared query asks the database for just those rows under the same `where` (the query's own SQL plus `AND key IN (...)`). A written row that comes back is added or changed, and one that does not but was in the previous result is removed. Its position follows the query's `order_by`, compared on the row's ordered fields as the database orders them. A commit that does not name its rows (another process, a resync) or that writes more rows than a bound falls back to the full query, as a lagged receiver does today.

Cost after step 2: distinct queries × rows written.

## Three tiers

- **Backend:** the shared evaluation registry and the written-row re-check, both behind the unchanged feed.
- **Frontend:** nothing changes on the wire; the same `snapshot` and `change` frames arrive sooner under load.
- **AI:** an assistant watching typed state (a future `live` for assistants) costs the same as a person watching.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Evaluate the `where` lambda in memory per written row | Duplicates the query's meaning in a second evaluator next to SQL; the two would drift on nulls, collation and decimals. Asking the database with the key filter keeps one meaning. |
| Database triggers that compute membership | Ties the feature to one engine's trigger language and splits logic between Corvid and SQL. |
| Share by path only, ignoring `actor` | Wrong for a query that filters by the caller; the IR says whether it does. |

## Slices

- [x] L1. Shared evaluation per (declaration, path, actor when read), with a test that 50 subscribers to one board run the query once per commit and each receives the same frames, and that two actors whose query reads `actor` do not share. Done 2026-11-26 (`serve_cmd/live/shared.rs`). The path and caller are keyed by each field's public JSON form. A subscriber that falls more than 64 frames behind starts again from the shared query's current snapshot, and the last subscriber to leave stops the evaluation.
- [x] L2. The written-row re-check with the full-query fallback, with a test that a commit writing one row of a 10,000-row table reads only that row, and a parity test that the frames equal those of the full query across inserts, moves out of the query, deletes and reorders. Done 2026-11-26. The query agent runs through `run_agent_reading_written_rows`, which adds `key = ... OR ...` to its own condition, and `corvid_runtime::tables::merge_rechecked` places the rows. A commit naming more than 64 rows, or none, re-runs the full query. Rows are placed by Rust comparison only where it equals the database's order exactly: `Int` and `Bool` columns, and text on SQLite. An `order_by` on any other kind (text on Postgres, which sorts by the database's locale, `Decimal`, times, optional columns) re-runs the full query for now; the roadmap holds widening it.

## Non-scope

- Joins and aggregates in `live` (limits plan row 9).
- Spilling large uploads to disk, the other row 13 item: it writes people's files to disk, a privacy choice the source must declare, so it needs its own proposal.
