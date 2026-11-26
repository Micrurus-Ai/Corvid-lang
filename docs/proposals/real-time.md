# Real-time connections (B14)

**Status:** shared design agreed with F08 on 2026-11-25. RT-1 and RT-3 can start in parallel. The examples below specify the interface being built.
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
    live "/boards/{board: Int}/cards" -> Card requires permission("board.view"):
        return Card.where(fn (c) -> c.board == path.board).order_by(fn (c) -> c.card)
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
| Interface | `ui.live(board_app.cards, board)` (the agreed F08 binding) gives a typed, always-current list. The client applies `Added`, `Updated` and `Removed` by key and revision, so unaffected rows keep their identity, focus and selection (F05). |
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

## Frontend agreement and wire protocol

The shared declaration is `live`, with the four `Change<Row>` cases above. The Corvid binding is `ui.live(board_app.cards, board)`. The generated client uses the same contract entry and typed path arguments. One page connection can carry multiple subscriptions. Each component owns its subscription and releases it on unmount.

RT-3 uses the WebSocket subprotocol `corvid.live.v1` at `/.well-known/corvid/live`. One text frame carries one JSON object. The client chooses an id unique among its open subscriptions. Here is a subscription to board 7 and the server's initial reply:

```json
{"type":"subscribe","id":"cards","path":"/boards/7/cards"}
{"type":"snapshot","id":"cards","rows":[{"revision":"1","row":{"card":4,"board":7,"column":"todo","title":"Read"}}]}
```

Each line above is a separate frame. The row uses the contract's public JSON mapping. Its declared table key identifies it; the revision is encoded as a decimal string to preserve integer precision in JavaScript. The server installs the subscription and takes its snapshot at one consistent commit boundary, then delivers changes in commit order. For example:

```json
{"type":"change","id":"cards","kind":"updated","revision":"2","index":0,"row":{"card":4,"board":7,"column":"done","title":"Read"}}
{"type":"unsubscribe","id":"cards"}
```

An added or updated row carries its zero-based destination index in the ordered result after the change. A removed row carries its last visible row and the change revision, without an index. If coalescing would make ordered deltas ambiguous, the server sends a replacement snapshot. Clients reconcile snapshots by key and revision, remove absent keys, and keep unchanged row identities. A new connection subscribes again. Its first snapshot includes commits made while offline, followed by newly committed changes.

Use the existing server session and route permission rules. Match the browser origin at upgrade and check permission and row visibility on the server. On session change, stop delivery before reevaluation; discard queued data that is no longer authorized. A denied subscription or revoked permission gets an id-scoped `error` frame with code `forbidden` and closes that subscription. Unrelated authorized subscriptions can continue. An invalid frame closes the connection with 1002, an invalidated session with 1008, and a normal shutdown with 1000. The server sends a WebSocket Ping after 30 idle seconds and closes an unresponsive connection after another 30 seconds without a Pong. The browser handles Pong at the transport layer.

RT-1 supplies the contract entry and the runtime open/close handle described above. RT-2 supplies committed, filtered changes. RT-3 and RT-4 use those interfaces. RT-5 reuses B10's action contract when available. F08/F18 acceptance exercises a missed edit in the new snapshot and a subsequent live edit over the same subscription.

### RT-3 integration boundary

The serve transport owns the page socket, ids, heartbeat and handle lifetimes.
Serve indexes the compiled RT-1 live paths separately from HTTP routes. RT-2 must
return the initial ordered rows and a filtered change stream at the same
commit boundary. Dropping the stream releases the subscription. Session
invalidation must wake an idle socket, and an authority change must invalidate
pending rows before delivery. Until serve has those interfaces it refuses the
upgrade. A synthetic table source would not exercise that boundary.

The serve adapter resolves the declared live path, binds its captures with the
GET handler's path conversion, and checks its declared route policy. It passes
the declaration, typed path and verified actor to `Provider::open`.
`Provider::observe_session` must validate the existing session and publish
role changes, expiry and logout even while the socket is idle. RT-2 owns
coalescing and replacement snapshots. The transport polls the handles
directly so a slow socket does not accumulate a second queue of changes.
[Adapter interface](../../crates/corvid-cli/src/serve_cmd/live/source.rs)

## Built in RT-1, and two points to agree

RT-1 shipped the declaration: it parses beside `route`, a body is checked to be one `return` of a query on the table it delivers (no `take`), a `live` is named by its path's last fixed segment (`cards`), unique in its server, and `corvid contract app` lists it under `live` with `name`, `path`, `path_params`, `row_type` and `policy`. `corvid serve` refuses to start a program that declares one (`E5204`, `live /boards/{board}/cards needs live subscriptions over the WebSocket transport`) until RT-3 sets `RuntimeCapabilities::live_subscriptions`.

**The snapshot, for RT-3.** Each `live` lowers to a handler agent named `__live__<mangled path>` (`IrRoute::handler_agent`, with `response_kind: Live`), whose result is the snapshot: the query's rows, in its order, as `List<Row>`. RT-3 runs it exactly as it runs a GET route's handler, with the same `path` and `actor` arguments, so permission and row visibility are the route machinery's. RT-2 adds the change feed for an open subscription.

Frontend agreement for RT-3 and RT-4 (2026-11-25). Both tiers use the route parameter record and the declared parameter type. The example above shows the agreed syntax; typed placeholders still require implementation.

1. **Path parameters are read as `path.board`**, as a route reads them. A subscription handler receives the same path record as a route handler.
2. **Typed path placeholders, agreed for `route` and `live` alike: `"/boards/{board: Int}/cards"`.** A path parameter is a `String` today, so `c.board == path.board` against an `Int` column does not check; with the placeholder, `path.board` is an `Int`, a request whose segment does not parse is refused before the handler runs, and the contract and the generated clients carry the type (the "typed path arguments" F08 asked for). An untyped `{board}` stays a `String`. The generated client takes its parameter types from the contract. A client subscribing to board 7 sends the concrete path `/boards/7/cards`.

## Non-scope

- `take(n)` in a `live` query. Keeping a moving top-N correct needs the rows just outside the window; a later slice.
- Live queries that join tables. Queries cannot join yet.
- Live values that are not table rows, such as an agent's progress. `-> json Stream<T>` covers one caller today; a shared version is B29.
- Assistant subscriptions (B11).
