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

`corvid serve` answers `POST /actions/<name>` for every `action`, and a program that declares actions and no `server` block is served too. The action's `requires` is checked first: no valid session is `401`, too little authority or a cookie session without its `X-CSRF-Token` is `403`, and the input is not read for either. The input comes next, as `application/json` or `application/x-www-form-urlencoded`. An action that takes one parameter of a declared type takes that value as its whole input, so a form posts the record's fields. Any other action takes an object with one field per parameter, and a form posts one field per parameter, read as a query field is: a text type once, an `Option` of one left out when absent, and a `List` of one by repeating the name. A parameter a form field cannot carry, such as a record beside other parameters, answers a form with `415` naming it; send that action JSON. A form cannot set `X-CSRF-Token`, so a form without that header may carry the token in its `corvid_csrf` field instead, checked by the same double-submit rule; `form_for(action, ...)` and `button_for` render the field, and it is never part of the input. A form that asks for `text/html` gets a page: `303 See Other` to the page in its `Referer` when that page has the server's own origin (otherwise `/`), or, for a refusal or failure, a page naming it with the same status and a link back. A queued approval answers a page with the approval id. An input its types refuse is `400` naming the field or parameter, and the body does not run. The result is the action's return value as public JSON, with the same `202` approval and failure answers as a route. When tracing is on, each call records an `action` event with the action, the surface (`http`) and the actor. `/openapi.json` lists each action with its input schema and session scopes, and `/.well-known/corvid` lists it under `actions`. An action reads its caller through a parameter of type `Caller`: `action claim(card: Int, by: Caller) -> Card requires authenticated:` gives `by.id`, `by.tenant`, `by.display_name`, `by.roles` and `by.permissions`. Serve fills it from the verified session, so it is never part of the input, and a sent `by` field is refused as undeclared. Corvid code passes one along, as in `claim(path.card, actor)` from a route that requires a sign-in, and the interpreter checks the action's `requires` against it: a caller without the authority is `403 insufficient_authority`, so a route that requires less cannot reach an action that requires more. An action with a `Caller` parameter must require a sign-in. A `Caller` cannot appear in an action's other inputs, a route's path, query or body, or a prompt's result, since the sender could name any caller. Native builds refuse a program with a `Caller` parameter for now. An action whose body can reach an `approve` declares `@approval(role: "...", risk: "...", data: "...", expires_ms: N, max_cost_usd: $N, irreversible: true|false)` above it, as a route does, and must require a sign-in; serve then answers `202` with an approval id, and a reviewer's `POST /__approvals/<id>/approve` runs the action and returns its result. The contract lists the policy under the action's `approval`. An action takes an agent's annotations too, such as `@budget($0.10)`, in any order with `@approval`. The TypeScript client from `corvid contract ts-client` has one method per action, `api.move_card(move)` or `api.rename(card, title)`, typed by the contract; an action with `@approval` returns `Promise<T | ApprovalPending>`, whose `approvalId` and `poll` name the pending approval, and an action returning `Stream<T>` returns a `RouteStream<T>`. Every state-changing call copies the `corvid_csrf` cookie into `X-CSRF-Token`. Public agents and prompts get no client method, since no server answers a call to one; declare an `action` for what a page or a client should call. The dev console (`corvid dev`) and the React scaffold list the same actions. [Shared actions](../proposals/shared-actions.md)

## Seeing what an action change breaks

A Corvid caller of a changed action fails to compile at the call. For the callers the compiler cannot see, save the contract with a release (`corvid contract app --out base.json`) and compare later with `corvid contract app --diff base.json`. Each action change is reported as `breaking`, `additive` or `note`, with every surface it reaches: `POST /actions/<name>`, the TypeScript client method, `corvid act`, and the assistant tool when the action is offered to assistants. Breaking: a removed action, a removed or renamed input field or parameter, a new required one, a changed input or result type, changed refinement bounds, a new signed-in requirement, role or permission, a new `@approval` (callers can get `202`), and a removed `assistants:` line. Additive: a new action, a new optional input, a dropped requirement. The command exits 1 when any change is breaking, so a CI step can stop the release. It also looks inside every record and choice the input and the result reach, and judges each change by direction: in the input, a new required field or a removed variant breaks a sender while a new variant does not; in the result, a new variant breaks a reader that handles every case while a new field does not.

## Calling an action from the command line

`corvid act move_card --card 3 --to Done --position 7` calls the action in `src/main.cor` (or `--file path`). Flags are read as form fields are: one per field of the record an action takes, otherwise one per parameter; a `List` repeats its flag (`--labels urgent --labels blocked`), an `Option` may be left out, and a flag the input does not have is refused. `--json '{"card": 3, ...}'` gives the whole input as JSON instead; put `--json`, `--file` and `--mode` before the action's name, since everything after it is read as input. The result prints as public JSON, the form an HTTP caller receives; a `Stream<T>` prints one JSON value per line. A refused input prints the field and the action does not run. The command line has no signed-in person yet, so an action that requires one is refused by name; call it through `corvid serve` with a session. Each call is traced as an `action` event with `surface: "cli"`.

`corvid run`'s positional arguments to the entry agent are read by the same rule, so any type a path segment or a query field takes works there too: `Decimal`, `Duration`, `Instant`, a plain choice, and the rest.

## Running an action as a background job

`corvid jobs enqueue --task <action> --payload '<json>' --source app.cor` queues a declared action, and the workers of `corvid jobs run --source app.cor` run it. The payload is the action's input as the HTTP endpoint takes it, public JSON (`{"card": 7, "label": "urgent"}` for an action taking a `Label` record). With `--source`, it is checked against the action's input before the job is stored, and a bad one is refused with the field it got wrong. A worker checks it again before running it, so a job queued without `--source` fails with `ActionInput` instead of running on a bad input. The call is traced as `surface: "worker"`, with the job's id.

A worker runs as no one, so an action that requires a signed-in caller is refused (`ActionNeedsSignedInCaller`) until B11 carries the enqueuing person's authority with the job. A job whose task names an action's body rather than the action is refused (`ActionNotRunAsAgent`), so a payload can never supply the `Caller` an action's `requires` is checked against.

## Assistants

An action whose first line is `assistants: confirm "Add a label to a card."` is offered to assistants at `POST /mcp`, the Model Context Protocol over HTTP (`initialize`, `ping`, `tools/list`, `tools/call`). The program needs an `identity` block, because an assistant acts only inside its person's signed-in session: `/mcp` authenticates as an action call does, with the session cookie and `X-CSRF-Token`, and answers `401` without one. `tools/list` lists, with each description and input schema, the offered actions whose `requires` the person meets. Any other name, including an action without the line, is answered as an unknown tool. `tools/call` checks the input through the action's types (a refused input is a tool error naming the field) and does not run the action: it answers `confirmation_required` with `confirm` and `decline` paths. The same person confirms with `POST /__confirmations/<id>/confirm` (anyone else gets `403`); the action then runs with that person as its `Caller` and answers as `POST /actions/<name>` does. `decline` drops it. A confirmation runs once and waits 15 minutes; pending confirmations are held in memory and do not survive a restart. Each call is traced as an `action` event with `surface: "assistant"`, and each confirmation as `assistant.confirmation` (`proposed`, `confirmed`, `declined`, `expired`). An action whose line says `assistants: allow "..."` runs at once instead: inside the person's own session the assistant's `tools/call` checks the action's `requires` fresh and runs it as that person, answering `structuredContent: {"status": "done", "result": ...}` (or `approval_required` when an `approve` inside it queued one, since allowing an assistant to start an action never skips its approval). An assistant outside the person's browser acts through a grant. The identity block declares `delegation: up_to 8 hours` (from `1 minute` to `720 hours`; without the line no grant can be issued and the grant routes are not served). The person then makes one with `POST /__delegations` (session and CSRF) and `{"label": "Desk assistant", "actions": [{"action": "pin_card", "mode": "allow"}, {"action": "tag_card", "mode": "confirm"}], "expires_in_secs": 3600}`, which answers `201` with the grant's `id` and a bearer `token` shown once and stored only as a hash. A lifetime beyond the cap, an action not offered to assistants, `allow` for an action the source offers with `confirm` only, and an action the person may not call themselves are each refused by name. `GET /__delegations` lists the person's grants (`live`, `expired` or `revoked`, never the token), and `DELETE /__delegations/<id>` revokes one, after which its token answers `401` everywhere. With `Authorization: Bearer <token>`, `/mcp` and `POST /actions/<name>` act as the person, without a cookie or CSRF. Each call is allowed only where the grant names the action, the source offers it, and the person may call it at that moment (their roles are read fresh, so a removed role removes it from every grant). Without confirmation it runs only when both the grant and the `assistants:` line say `allow`; otherwise `/mcp` asks the person to confirm and `/actions/<name>` answers `403 confirmation_required`. Each such call is traced as an `action` event with `surface: "assistant"` and the grant's `delegation` id and `label`, and each grant as a `delegation` event (`created`, `revoked`). [Assistant authority](../proposals/assistant-authority.md) [Assistant endpoint design](../proposals/shared-actions.md#b10-3b-design-the-assistant-endpoint)

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
