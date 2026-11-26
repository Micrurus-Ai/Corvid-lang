# CLI reference

## Overview

`corvid <command> [args]`. Every command supports `--help`. The
canonical help output lives in `corvid --help`; this page is the
overview.

## Live client transport

`corvid contract app` publishes each live declaration's `server`, `name`,
`path`, ordered `path_params`, `row_type`, ordered `key_fields` and policy.
Look up `row_type` in `types` for its public JSON fields. Both `route` and
`live` accept `{board: Int}` in the source path. The contract path uses
`{board}` and records its type separately. An unannotated segment is a String.

`corvid contract ts-client` generates a method per live declaration. For
`live "/boards/{board: Int}/cards" -> Card` on `board_app`, subscribe with
`const stop = api.live.board_app.cards(7, observer)`. Path arguments appear
in declaration order, followed by the observer. Its `rows` callback receives
`readonly Card[]`. Call `stop()` when the component unmounts. If a public
callable uses the name `live`, the generated group appends underscores until
its name is available.

The SDK exports `livePath(template, values)`. Pass a normalized contract path
and a record of named string or safe integer arguments. The result contains
one percent-encoded segment per placeholder. Both browser clients use this
function. The compiler's template helper
`ui.live_rows(server_name: String, subscription: String, arguments: List<String>, content: List<ui.View>) -> ui.View`
takes the positional arguments as text and wraps the child views in a live row
template. Application code uses the checked child loop.
[Live rows](grammar.md#live-rows-in-view-layouts)

The TypeScript SDK provides
`CorvidClient.subscribe<Row>(path, keyFields, observer): () => void` for client
bindings. The binding supplies the concrete encoded path and the ordered names
of the table's key fields. String keys and exactly representable safe integer
keys are supported. An inexact numeric key terminates delivery. The browser
uses its session cookie on the WebSocket connection. HTTP request headers in
the client options apply only to fetch requests.

`LiveObserver<Row>.rows(rows)` receives a complete ordered list after a
snapshot or change. Rows with the same key and revision keep their identity.
`LiveObserver<Row>.error(error)` signals terminal failure. Clear the displayed
result on error. A scoped denial yields a `CorvidError` with status 403 and
body code `forbidden`. Session invalidation ends every subscription on the
connection. Call the returned function when the owner unmounts. Multiple
calls are safe. The Corvid component binding remains pending.

## Project commands

```sh
corvid new <name>                       # scaffold a new project
corvid new <name> --web                 # scaffold a page, GET / route and rendering test
corvid init                             # add corvid.toml to existing dir
corvid build <file>                     # compile to target (default native)
corvid build <file> --target=wasm       # WASM target
corvid build <file> --target=server     # rendered axum binary
corvid build src/main.cor --target web  # package routes and browser assets in target/web.tar
corvid build <file> --sign              # sign the cdylib + receipts
corvid run <file>                       # build and run an agent
corvid check [file]                     # typecheck file or project src/main.cor
corvid test                             # run tests in tests/
corvid bench <file>                     # run a benchmark
```

### Web packages

Extract `target/web.tar` and run `corvid serve src/main.cor` in the extracted directory. The archive contains server sources for the Corvid process. The browser receives the rendered page and its public assets. The build orders component source paths before compiling their handlers and sorts the archive entries.

The server adds subresource integrity to the standard interactive page footer. Unversioned compiled assets use ETags and require revalidation. A matching SHA-256 version query permits immutable caching. A stale version receives HTTP 404 with `Cache-Control: no-store`. The split quantity module pins its lazy runtime import to that runtime's digest.

The event JavaScript includes inline version 3 source maps for generated export wrappers and for direct calculations in the quantity module. Generated component handlers map to the component's source file. Mappings refer to project-relative source files and omit source contents. Browser compilation of `src/events.cor` checks its resolved import graph for Python and the server modules `std:db`, `std:env`, `std:io` and `std:secrets`. A rejected web build preserves the previous archive.

## Diagnostics & audit

```sh
corvid doctor                           # environment health check
corvid audit <file>                     # static report: approvals, replay, budgets, secrets
corvid contract list                    # registry of compile-time guarantees
corvid contract list --format=json
corvid claim --explain <id>             # human-readable rationale for a guarantee
```

## Tour & examples

```sh
corvid tour --list                      # every shipped invention demo
corvid tour --topic <name>              # run a specific demo
corvid examples list                    # canonical reference apps
```

## Replay & traces

```sh
corvid trace list                       # all recorded traces
corvid trace dag <trace-id>             # provenance DAG for one trace
corvid replay <trace-id>                # deterministic re-execution
corvid receipt verify <path>            # verify a signed run receipt
```

## Eval & model upgrade

```sh
corvid eval <file>                      # rerun against a saved trace set
corvid eval --swap-model <m> --source <file> <trace-dir>
corvid bench compare python|js         # published-archive comparison
```

## Jobs (Phase 38)

```sh
corvid jobs run --source app.cor --queue=default --workers=4    # --source is required
corvid jobs schedule list
corvid jobs inspect <id>
corvid jobs explain <id>                # AI-assisted root-cause from typed trace
corvid jobs dlq triage                  # AI-assisted DLQ pattern clustering
corvid jobs retry <id>
corvid jobs cancel <id>
corvid jobs export-trace <id>
corvid jobs pause --queue=default
corvid jobs drain --workers=all
```

## Auth (Phase 39)

```sh
corvid auth keys generate
corvid auth keys rotate
corvid auth jwt verify --jwks <url>
corvid approvals pending
corvid approvals approve <id>
corvid approvals deny <id>
```

## Connectors (Phase 41)

```sh
corvid connectors list
corvid connectors test <connector>      # mock + replay + real-mode self-test
corvid connectors auth <connector>      # OAuth or API-key setup
```

## Migrations (Phase 37)

```sh
corvid migrate status
corvid migrate up [--dry-run]
corvid migrate down [--dry-run]
```

## Actions

`corvid serve` answers `POST /actions/<name>` for every `action`, and a program that declares actions and no `server` block is served too. The action's `requires` is checked first: no valid session is `401`, too little authority or a cookie session without its `X-CSRF-Token` is `403`, and the input is not read for either. The input comes next, as `application/json` or `application/x-www-form-urlencoded`. An action that takes one parameter of a declared type takes that value as its whole input, so a form posts the record's fields. Any other action takes an object with one field per parameter, and a form posts one field per parameter, read as a query field is: a text type once, an `Option` of one left out when absent, and a `List` of one by repeating the name. A parameter a form field cannot carry, such as a record beside other parameters, answers a form with `415` naming it; send that action JSON. An input its types refuse is `400` naming the field or parameter, and the body does not run. The result is the action's return value as public JSON, with the same `202` approval and failure answers as a route. When tracing is on, each call records an `action` event with the action, the surface (`http`) and the actor. `/openapi.json` lists each action with its input schema and session scopes, and `/.well-known/corvid` lists it under `actions`. An action reads its caller through a parameter of type `Caller`: `action claim(card: Int, by: Caller) -> Card requires authenticated:` gives `by.id`, `by.tenant`, `by.display_name`, `by.roles` and `by.permissions`. Serve fills it from the verified session, so it is never part of the input, and a sent `by` field is refused as undeclared. Corvid code passes one along, as in `claim(path.card, actor)` from a route that requires a sign-in, and the interpreter checks the action's `requires` against it: a caller without the authority is `403 insufficient_authority`, so a route that requires less cannot reach an action that requires more. An action with a `Caller` parameter must require a sign-in. A `Caller` cannot appear in an action's other inputs, a route's path, query or body, or a prompt's result, since the sender could name any caller. Native builds refuse a program with a `Caller` parameter for now. An action whose body can reach an `approve` declares `@approval(role: "...", risk: "...", data: "...", expires_ms: N, max_cost_usd: $N, irreversible: true|false)` above it, as a route does, and must require a sign-in; serve then answers `202` with an approval id, and a reviewer's `POST /__approvals/<id>/approve` runs the action and returns its result. The contract lists the policy under the action's `approval`. An action takes an agent's annotations too, such as `@budget($0.10)`, in any order with `@approval`. [Shared actions](../proposals/shared-actions.md)

## Live subscriptions

`corvid serve` reserves GET `/.well-known/corvid/live` for the WebSocket subprotocol `corvid.live.v1`. The Origin must match the configured public origin, or the listener's HTTP origin when serving directly. A cross-origin upgrade gets HTTP 403. A missing or unsupported subprotocol gets HTTP 400. A valid handshake to a running app without a live provider gets HTTP 503. The upgrade needs a valid session cookie from the declared identity. Serve checks the session store before it queues a delivery and checks idle sockets at expiry or within one second of an authority change. A surviving session reopens its subscriptions with fresh GET authorization. The subscription path selects a compiled live declaration, including its own permission and path argument types, even when a GET route uses the same path. When a row leaves a query, a removal frame carries its last visible contents and the committed revision of the write. Serve sends a replacement snapshot when that revision is unavailable or the snapshot takes fewer changes.

Once connected, a page subscribes with `{"type":"subscribe","id":"cards","path":"/boards/7/cards"}` and unsubscribes with `{"type":"unsubscribe","id":"cards"}`. Each open starts with an ordered snapshot. A denied id gets a `forbidden` error. Reusing an open id or sending a malformed message closes the socket with 1002. An invalidated session closes it with 1008 and server shutdown uses 1000. After 30 idle seconds the transport sends Ping and allows 30 seconds for its matching Pong. [Frame formats and reconnect semantics](../proposals/real-time.md#frontend-agreement-and-wire-protocol)

SQLite delivers commits within one serve process. For Postgres, each serve process listens for commits from its database and schema. Processes sharing login state must use the same durable auth store. Run the transport check with `python scripts/verify_live_subscriptions.py --cli <corvid>`. Add `--channel msedge` to exercise both SQLite and two Postgres processes in Edge. Use `--postgres` for the two-process wire check alone. The check starts a disposable loopback cluster with the local PostgreSQL tools or uses empty disposable databases supplied in `CORVID_TEST_POSTGRES_URL` and `CORVID_TEST_POSTGRES_SCRATCH_URL`.

The Rust host interface starts with `SessionAuthRuntime::observe_session_cookie`, which verifies the cookie and captures its authority revision using the supplied epoch-millisecond clock. Inside Tokio, `watch_session_cookie` performs the same validation and returns both the observation and a receiver for local session changes. Serve waits on that receiver while idle and checks the durable revision for writes from other processes. On the returned `SessionObservation`, `remaining_ms` gives the time until expiry and `is_current` checks expiry and committed authority changes. `while_current` runs a synchronous enqueue while it holds a SQLite writer reservation. A stale observation returns `None`. The callback must finish before awaiting I/O or accessing the auth store again.

## Web package assets

For a web project with entry point `src/main.cor`, `corvid build src/main.cor --target web` includes the files from the project root's optional `assets/` directory in `target/web.tar`. Running `corvid serve src/main.cor` in the project or extracted archive serves each at `/assets/<filename>` with its media type. These files are public. A declared GET route at the same URL causes a startup error.

Use regular files directly inside `assets/` with lowercase extensions `.png`, `.jpg`, `.jpeg`, `.webp`, `.ttf`, `.woff`, `.woff2`, or `.txt` (for accompanying licenses). Names may contain ASCII letters, digits, hyphens, underscores and dots and must start with a character other than a dot. Linked directories, linked files and subdirectories are rejected.

## Package management (Phase 25)

```sh
corvid package metadata <file> --name <n> --version <v>
corvid package publish
corvid import-summary <file>
```

## Self-management

```sh
corvid --version
corvid self update
corvid self uninstall
```
