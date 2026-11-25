# Real-time connections (B14)

**Status:** proposal, awaiting frontend agreement (F08). The grammar is shared, so neither side builds it until it is agreed.
**Owners:** split by the owner on 2026-11-25. Backend and AI grammar and the table change streams: Claude Code (Windows session). The WebSocket transport in `corvid serve`, its protocol and the client libraries: Codex, with frontend F08.
**Roadmap:** [B14](../backend-ai-roadmap.md), with frontend F08; B29 (live shared state) builds on it.

## The problem

A screen that shows shared data goes stale the moment someone else changes it. Today a Corvid app can stream one agent's output to one caller (`-> json Stream<T>`, served as server-sent events), but it cannot keep a list of cards current while three people and an assistant move them. In other stacks that takes a WebSocket server, a message format, a pub/sub layer, a cache invalidation scheme and client code to reconcile it, each written by hand and each able to drift from the database.

## The idea

A real-time connection carries two things Corvid already has, so it adds no new programming model:

- **From the client: actions.** A person or an assistant changes data through a typed action (B10), with its permission, validation and conflicts. Over a live connection the call travels on the socket instead of a separate request.
- **To the client: changes to what it is looking at.** A `live` declaration names a table query. A subscriber receives the rows that match it now, then every committed change to those rows.

There are no message handlers to write, and no second place where permission or validation could differ from the action's.

## The proposal

```corvid
table Card:
    key card: Int
    board: Int
    column: String
    title: String where len_between(1, 120)

server board_app:
    live "/boards/{board}/cards" -> Card requires permission("board.view"):
        return Card.where(fn (c) -> c.board == board).order_by(fn (c) -> c.card)
```

`live` is one new word, written beside `route` in a `server` block. It has a path with parameters, the row type it delivers, the same `requires` clause routes use, and a body that returns one table query. The query uses the same `where` and `order_by` forms the database already runs.

A subscriber receives:

```text
Snapshot(rows)     the rows that match now, in the query's order
Added(row)         a committed insert, or a save that moved a row into the query
Updated(row)       a committed save of a row that still matches
Removed(row)       a committed delete, or a save that moved a row out of the query
```

These are the cases of a built-in `Change<Row>`. Each row carries its revision, so a client always knows which version it holds. A change is sent only after its transaction commits, so a subscriber never sees a write that an `atomic:` block later undid.

### What each tier gets

| Tier | Derived from the `live` declaration |
| --- | --- |
| Backend | The server matches each committed write against every open subscription's filter, using the query it already compiles to SQL. No pub/sub code, no cache. |
| Interface | `ui.live(board_app.cards, board)` (name to be agreed in F08) gives a typed, always-current list. The client applies `Added`, `Updated` and `Removed` by key and revision, so unaffected rows keep their identity, focus and selection (F05). |
| Assistants | Not exposed by default. A later B11 slice may let an action-holding assistant subscribe, declared in source like any exposure. |
| Contract | `corvid contract app` lists each `live` with its path, row type and permission, so the generated TypeScript client gets a typed subscription and a renamed column is a compile error in every caller. |

### Permission and tenancy

- `requires` is checked when a client subscribes, and again when the session changes: a signed-out or downgraded session closes its subscriptions.
- Which rows a subscriber sees is the query, nothing else. A query that reads the caller (`c.owner == actor.id`) is evaluated per subscriber. There is no channel name a client can guess to receive another tenant's rows.
- A row is checked against a subscriber's filter on the server before it is sent; the client is never trusted to filter.

### Slow subscribers and reconnect

- **Backpressure.** Pending changes for one subscriber are kept as the latest version of each row, by key. A slow client receives fewer, newer updates and never loses the final state, and the server's memory per subscriber is bounded by the rows it can see. This is lossless for state, so there is no policy to choose.
- **Reconnect.** A client that reconnects subscribes again and receives a fresh `Snapshot`. It keeps every row whose revision has not changed, so the screen does not flicker or lose focus. Resuming from a position in a change log is a later optimization with the same meaning.
- **Heartbeat.** The transport pings an idle connection and closes one that does not answer. The interval is part of the protocol, documented with it.

### Composition with the guarantees

- **Effects.** A `live` body is a read (`db_egress_read`). Pure functions and `@deterministic` agents still cannot run one.
- **Replay.** The snapshot query is recorded like any query. Changes are produced by recorded writes, so a replay of the writes reproduces the changes.
- **Conflicts.** A client that writes through an action still gets `Changed(current)` when it lost a race. A live list and a conflict show the same truth.
- **Scale.** One server process matches writes in memory. Several processes on one Postgres database share commits through `LISTEN`/`NOTIFY`, a named slice below, so the meaning does not change with the deployment.

## Alternatives considered

| Form | Why not |
| --- | --- |
| A `socket` declaration with `on open`, `on message`, `on close` handlers | A second programming model beside actions, where permission and validation are written again by hand and can drift from the action's. |
| Publish and subscribe to named channels (`publish("board:7", event)`) | Channel names are strings a client can guess, events are not tied to committed data, and a forgotten `publish` leaves screens stale. |
| Push the whole query result on every change | Simple but costly for large lists, and it resets client state on every change. |
| Server-sent events only | Already shipped for one-shot streams. They cannot carry the client's action calls, so a live screen would need a second connection. |
| `subscription`, `watch` or `stream` as the word | `live` is the shortest and reads as what it is: `live "/boards/{board}/cards" -> Card`. `stream` already means `Stream<T>`. |

## Slices

| Slice | What | Who |
| --- | --- | --- |
| RT-1 | `live` declarations: parse, check (one table query, a `requires` clause, path parameters typed), the contract entry | Claude Code |
| RT-2 | Change matching: each committed write produces `Change<Row>` events for the open subscriptions whose filter it meets, with coalescing by key, on SQLite and Postgres | Claude Code |
| RT-3 | The transport: one WebSocket per page in `corvid serve`, subscribe and unsubscribe messages, `requires` enforced on subscribe and on session change, heartbeat, reconnect with a fresh snapshot | Codex |
| RT-4 | The clients: the generated TypeScript client's typed subscriptions, and the Corvid browser binding (F08) | Codex |
| RT-5 | Action calls over the live connection, once B10 lands | Codex, with Claude Code for B10 |
| RT-6 | Several server processes: Postgres `LISTEN`/`NOTIFY` carrying commits between them | Claude Code |

RT-1 and RT-3 can proceed in parallel once the grammar and the wire messages are agreed. RT-2 and RT-3 meet at one runtime interface, which RT-1 writes down: a subscription handle the transport opens and closes, and a stream of `Change<Row>` values it forwards.

## What frontend development needs to agree (F08)

1. The `live` declaration and the four `Change<Row>` cases as written above.
2. The wire messages: `subscribe` and `unsubscribe` from the client; `snapshot` and `change` from the server, each a public-JSON value of the declared row type; and the heartbeat and close codes.
3. The client binding's name and shape, and that reconnect means a fresh snapshot reconciled by key and revision.

## Non-scope

- `take(n)` in a `live` query. Keeping a moving top-N correct needs the rows just outside the window; a later slice.
- Live queries that join tables. Queries cannot join yet.
- Live values that are not table rows, such as an agent's progress. `-> json Stream<T>` covers one caller today; a shared version is B29.
- Assistant subscriptions (B11).
