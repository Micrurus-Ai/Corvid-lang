# Proposal: the complete HTTP client (B05)

Status: **accepted 2026-11-25, in progress.** Decided by the macOS session under the design laws in `AGENTS.md`, as the user directed; the reasons are under [Decisions](#decisions). Slices are tracked under "Task progress" in [the backend and AI roadmap](../backend-ai-roadmap.md).
Owner: backend and AI development, macOS session ([parallel backend sessions](../internals/parallel-backend-sessions.md)). It adds connector grammar, which only backend code writes; whether that needs frontend agreement is open question 1.

## Problem

Every backend calls other services: a payment provider, a model API, a carrier, an internal service. Python reaches for `httpx` or `requests`, TypeScript for `fetch` or `axios`. Each leaves the consequential choices to defaults the author never sees: `requests` waits forever unless given a timeout, `fetch` follows up to 20 redirects to any host, both are easy to log with the `Authorization` header in place, and neither checks the response against the program's types.

B05's acceptance: every method, headers, bearer and basic auth, query building, typed JSON bodies through B01, streamed request and response bodies, multipart upload, redirect policy, timeouts and cancellation. Caller disconnect closes the upstream connection. Effects and replay record each request.

## What exists today

A survey before designing:

- **`connector` declarations** ([grammar](../reference/grammar.md#connector-declarations)) are already Corvid's declared HTTP client. A connector fixes its `base_url` in source, takes credentials by secret *name* (`bearer(secret("KEY"))`, `basic(...)`, `header("X-Api-Key", ...)`), declares `retry`, `rate_limit` and `circuit_breaker`, maps statuses to typed error variants (`on status 404 -> NotFound`), runs in `mock`, `replay` or `real` mode, and carries verified async protocols (52h). The credential is resolved at the last moment and placed only in a header, never in the recorded call (`corvid-runtime/src/runtime/connector_dispatch.rs`). Connectors appear in the Application Contract (`ContractConnector`).
- **`std/http.cor`** has two dynamic-URL tools, `http_get(url)` and `http_post_json(url, body)`, returning `Result<HttpResponseEnvelope, String>`. They are a documented invention with a tour demo and a book chapter (see [the dynamic-URL tools](#the-dynamic-url-tools)).
- **Egress policy.** `HttpEgressPolicy` (`corvid-runtime/src/http.rs`) applies an always-on SSRF block and requires the host in `[http] allow` (or `CORVID_HTTP_ALLOW`), with redirect destinations checked too. Replay quarantines the client.
- **Hidden defaults.** `HttpClient` silently uses a 30-second timeout and follows up to 10 redirects, for connectors and both tools. The parser accepts `GET`, `POST`, `PUT`, `PATCH` and `DELETE` only.
- **Bodies use the wrong JSON.** A connector encodes its body with the internal trace envelope (`corvid-vm/src/conv.rs`), not B01's public mapping (`corvid-public-json`), so an `Option` field is sent in Corvid's internal shape rather than as the plain value or `null` a real API expects. Response decoding goes through the same envelope.
- **Recording.** A connector call records one `ToolCall`/`ToolResult` pair with its arguments and decoded result. The request actually sent (method, URL, status, attempts, elapsed time) is not recorded; `RecordedHttpExchange` exists in `http.rs` but nothing emits it.
- **Native code.** Neither connectors nor standard library tools run in native code. A program that calls one runs on the interpreter (`corvid-driver/src/native_ability.rs`).
- **`Secret`** (B04) is the language type for sensitive text: a tool taking one records `<redacted>`.

## Design

B05 completes `connector` instead of adding a second client. There is one obvious way to call a service: declare it once, then call its operations like functions.

### A realistic program

```corvid
type Charge:
    id: String
    amount: Int
    currency: String
    receipt_url: Option<String>

type NewCharge:
    amount: Int
    currency: String
    customer: String

type ChargeQuery:
    customer: String
    limit: Int

type ChargeError:
    NotFound
    Declined
    RateLimited

type Receipt:
    title: String
    file: Upload

type CompletionDelta:
    text: String

connector payments:
    base_url: "https://api.payments.example"
    auth: bearer(secret("PAYMENTS_KEY"))
    timeout: 10s
    redirects: never
    retry: 2
    modes: [mock, replay, real]

    operation charge(new: NewCharge, idempotency_key: String) -> Result<Charge, ChargeError> dangerous:
        POST "/v1/charges" body new
        header "Idempotency-Key": idempotency_key
        on status 402 -> Declined
        on status 429 -> RateLimited

    operation charges(filter: ChargeQuery) -> Result<List<Charge>, ChargeError>:
        GET "/v1/charges" query filter

    operation attach_receipt(id: String, receipt: Receipt) -> Charge:
        POST "/v1/charges/{id}/receipts" multipart receipt
        timeout: 60s

connector model:
    base_url: "https://api.model.example"
    auth: header("X-Api-Key", secret("MODEL_KEY"))
    timeout: 120s
    redirects: never
    modes: [replay, real]

    operation complete(prompt: String) -> Stream<CompletionDelta>:
        POST "/v1/complete" body prompt
        streams: server_sent_events
```

A reader sees, without the docs, where each call goes, how long it may take, whether it follows a redirect, what it sends, and what comes back. Nothing consequential is left to a default.

### Declared policy, never a default

| Item | Values | Omitted |
| --- | --- | --- |
| `timeout:` | `<n>s` (a whole number of seconds, 1 to 3600), on the connector; an operation may override it | compile error naming `timeout:` and an example |
| `redirects:` | `never`, `same_host`, `allowed_hosts` | compile error naming the three values |

- `timeout` bounds the whole exchange: connecting, sending, the status line and the full body. For a streamed response it bounds the wait for each event instead, because a model stream legitimately runs longer than any one gap. Exceeding it cancels the request and is a failure that names the operation and the declared timeout.
- `redirects: never` treats a 3xx as the provider's answer. `same_host` follows only redirects to the base URL's scheme, host and port. `allowed_hosts` follows redirects to any host in `[http] allow`, as today. All three keep the SSRF block. The hop limit stays fixed at 10, as a structural bound like the SSRF block, not a policy.
- A redirect never forwards the credential to a different host, under any policy, and a `POST` answered with 301, 302 or 303 is not silently re-sent as a `GET`; it is refused, naming the status.
- Hosts. The source declares the host in `base_url`, and the deployment permits it in `[http] allow`. Today a missing host fails at the first call. Instead, `corvid run` and `corvid serve` refuse to start when a declared connector's host (in `real` mode) is not allowed, naming the connector, the host and the setting. That turns a 3am failure into a startup error.

Existing connectors (the `orders_api` example and about 100 test sources) gain `timeout:` and `redirects:` in the slice that makes them required.

### Methods, headers, query and auth

- **Every method.** `HEAD` and `OPTIONS` join `GET`, `POST`, `PUT`, `PATCH` and `DELETE`, in the parser rule routes and operations share. A `HEAD` response has no body, so a `HEAD` operation must return `Response<Nothing>` (see below); any other return type is a compile error that says so.
- **Headers.** `header "Name": param` sends a `String` parameter, and `header "Name": "literal"` a fixed value. `Authorization`, `Proxy-Authorization`, `Cookie`, `Host`, `Content-Type`, `Content-Length` and `Transfer-Encoding` cannot be set this way, nor the header the connector's `auth:` uses or its protocol's idempotency header; the checker names which item owns it. Header names are checked as tokens at compile time, each is set once, and a value with a line break is refused before sending. As built, a header does not take a `Secret` (see decision 4).
- **Query building.** `query filter` names a record parameter. Each field becomes a query parameter through B01's public mapping: an absent `Option` field is omitted, a `List` repeats the key, and nested records are a compile error (no standard encoding exists). Path and query values are percent-encoded; a path parameter can never insert `/`, `?` or `#`. This is the client half of the server's `route GET "/orders" query OrderFilter`, so one record type can describe both sides.
- **Auth.** `bearer` and `basic` already exist. Basic credentials are encoded once, at send time, and the encoded header is never recorded.

### Typed JSON through B01

Request bodies are encoded, and response bodies decoded, through `corvid-public-json`, the mapping `T.from_json`, routes and prompts use. A response that does not match the declared type fails with B01's error naming the field path, the expected kind and what was found (``the response does not match `Charge` at `amount`: expected Int, found a string``). Unknown fields in a response are ignored, as `T.from_json` ignores them, because providers add fields without notice; route bodies reject them instead, because the contract publishes `additionalProperties: false`. A 2xx body that is not JSON is a failure naming the status, never a silent `null`, and an empty body is `null`. The recorded result is unchanged in shape (the provider's body, inside Corvid's own `Result` envelope when the operation returns one), so a trace replays through the same decoder. This is a behavior change for any connector that sends an `Option` field, and an `Option` field in a response now decodes where it failed before, so the slice records it in `learnings.md`.

**One description of the wire format.** As built in 3d, the compiler describes what each connector operation sends and reads: its `body_type` (with `body_encoding`, `json` or `form`), `query_type`, `response_type` and header names, written by the same `type_ref_name` routes use, and every declared type those reach, described as the contract describes its public types. That description lives in the contract's `connector_wire` field, which is never serialized. `corvid serve` publishes the contract at `/.well-known/corvid` and OpenAPI at `/openapi.json` for clients, generated SDKs and assistants, and a connector's request and response records are backend detail the source never chose to publish, so under "nothing is exposed by default" they stay out of every published copy. (A first version put them in the public `types`; the main-line session caught that the contract is served, and it was corrected before any release.) `openapi::component_schemas` gives the wire types the same JSON Schema published types get, and `connector_contract_sync.rs` runs a real call and validates the body the provider received against it, covering `Option` both ways, `List`, `Decimal`, a nested record and a sum type, checks each query parameter is a field of the query record, and asserts that no connector-only type appears in the published contract or OpenAPI document. Generated clients never call connectors, so they gain nothing to generate. Slice 1 added `timeout` and `redirects` to the published connector entries, beside the base URL and paths they already carried.

### Responses with status and headers

Most operations want the body. When a program needs the status or a header (an `ETag`, `Location`, a rate-limit header), it declares `-> Response<T>`. A program that does not ask for headers does not receive them.

As built (slice 3c): `Response<T>` is read with methods, `r.status()`, `r.body()` and `r.header("ETag")` (an `Option<String>`, the first match, compared without case), the way `Upload` is read with `filename()` and `size()`; one accessor style for compiler-known values beats a second, field style. `Set-Cookie` is not recorded as `<redacted>` but never passed on at all: a connector is not a browser session, and a value the program can read is a value the trace must record for replay. An operation returns a `Response` whole or as its `Result`'s success type; it cannot be a record field, a route, tool or prompt type, JSON, or in `main`'s signature, because it describes one call, not data. In mock mode the `mock:` is the body, answered with status 200 and no headers, so mock mode needs no constructor. A route never declares `HEAD` or `OPTIONS`, which the server answers itself.

### Streaming

- **Streamed responses.** An operation returning `Stream<T>` declares `streams: server_sent_events` or `streams: json_lines`. Each event decodes through B01 into `T` as it arrives; a malformed event ends the stream with the field-path error. The format is required because the compiler cannot know it.
- **Streamed request bodies.** A `body` parameter of type `Upload` or `Bytes` is sent from its storage without the program holding a second copy; `Upload` streams from disk. A `Stream<T>` body is sent as `json_lines`.
- **Multipart.** `multipart receipt` names a record whose fields become parts: `String` and numbers as text parts, `Upload` and `Bytes` as file parts with the upload's filename and media type. Nested records are a compile error.

### Cancellation and caller disconnect

An HTTP call is a future the runtime owns. When it is dropped, the connection is closed, not left to finish. That happens when the call's timeout expires, when a `parallel` arm is cancelled, and when the caller of a `corvid serve` route disconnects. The acceptance test holds an upstream response open, disconnects the route's caller, and checks the upstream sees its connection close within a second, and that the trace records the call as cancelled.

### Recording and replay of each request

Every hop of every attempt records an `HttpExchange` trace event: connector, operation, method, the URL with its query, attempt and hop numbers, request header names, the request body's SHA-256 and size, status, response header names, response size, elapsed time, and the outcome (`response`, `redirected`, `timeout`, `refused` or `transport_error`, with a detail for the last three; a streamed call's cancellation is recorded on its stream record, slice 5). As built, header *values* are never recorded, not even literal ones: names are enough to audit what was sent, and a value-free record cannot leak a credential, a `Secret` header or a cookie. The tracer's redaction replaces any known secret inside the URL or detail. The existing `ToolCall`/`ToolResult` pair stays the unit replay substitutes, so retries and redirects replay exactly as recorded, and replay never reaches the network. The effect row is unchanged: operations still declare `uses`, and `dangerous` still routes a mutating call through approval.

### The dynamic-URL tools

`http_get(url)` and `http_post_json(url, body)` carry the hidden 30-second timeout and 10 redirects, and send bodies outside B01. Under the no-hidden-defaults rule they cannot stay as they are.

They are a public, documented surface. A search of code and docs finds them in:

- `std/http.cor`;
- the invention catalog's executing-HTTP entry;
- the `corvid tour` catalog;
- chapter 18 of the book;
- the stdlib reference (`docs/reference/stdlib/http.md`, `README.md`, `json.md`), `functions.md` and `core-semantics.md`;
- three guarantee IDs in `corvid-guarantees` (SSRF block, allowlist, replay quarantine) and their tests;
- the skill audit in `corvid-driver/src/skills`;
- the Python codegen's stdlib scan;
- serve, build and driver integration tests, and one CLI fixture.

A program that fetches a URL it learns at run time needs this surface; the book's example does exactly that. So the choice is between two ways to give it a declared policy:

- **A. Replace them with a dynamic-host connector (recommended).** A connector declares `hosts: allowed_hosts` in place of `base_url`, and an operation takes the URL as a parameter: `operation fetch(url: Url) -> Response<String>` with `GET url`. It declares `timeout:` and `redirects:` like every connector and gets every other connector feature. The tools are removed, and every surface listed above moves to the connector form in the same slice: the catalog entry, the tour, the book chapter, the reference pages, the guarantees re-anchored to the connector dispatch (which already runs the same SSRF and allowlist check), and the tests. The removal is recorded in `learnings.md`, with the old and new form side by side. This leaves one way to make a request.
- **B. Keep them with required policy arguments**: `http_get(url, timeout: 10s, redirects: never)`. A smaller change, but it keeps two ways to make a request, and the dynamic tools would lack headers, auth, typed bodies and streaming unless each feature is built twice.

See open question 2.

### Across the tiers

- **Backend.** The checker knows every outbound call's host, timeout, redirect policy, effects and cost. The startup host check turns a late failure into an early refusal.
- **Frontend.** A route that returns a connector's `Stream<T>` forwards it to the browser as the Server-Sent Events routes already send, with the same `T`. The generated form for a route taking an `Upload` already exists; a multipart operation takes that same `Upload` and passes it through without buffering. Nothing new in frontend grammar.
- **AI.** An assistant never calls a connector directly. It reaches one only through a declared action (B10), with that action's permission, approval and budget. Each request's `HttpExchange` record is what B27's visible agent activity shows: which service was called, how long it took, and whether it changed anything. Nothing is exposed to an assistant by default.
- **Gain.** The `charge` operation above is about 6 lines. The `httpx` equivalent needs a client with a timeout and redirect policy, a header builder that keeps the key out of logs, a body encoder, a status-to-error mapping, a response model and a replay fixture, typically 40 to 60 lines, and its mistakes (no timeout, a logged key) ship silently.

### Native code

Connectors run only on the interpreter today, for the same reason native `Secret`, native UUID and the B06 types at native tool boundaries do: native code has no dispatch of runtime-owned tools through the runtime's recording. The roadmap counts a task complete only when it works on both targets. B05's native slice therefore depends on that shared decision, which belongs to the user (open question 3). Every interpreter slice is designed so the native one reuses the same runtime code.

## Syntax alternatives considered

- **A request-builder library** (`http.post(url).header(...).timeout(...).send()`), as in `httpx`. Rejected: it duplicates `connector`, puts the host in a run-time string the compiler cannot check, and makes timeouts optional calls a reader can miss.
- **Policy in `corvid.toml`** (`[http] timeout = 30`). Rejected for the policy: the rule requires consequential choices in source, and one project-wide timeout is wrong for a model stream and a payment call alike. The host allowlist stays in configuration because it is deployment policy, and the startup check ties it to the source.
- **`timeout: 10 seconds`** with B06's duration literal. Better reading, but that literal is waiting on Codex. `10s` matches the existing `deadline: 30s` and `rate_limit: 10 per 60s`, and moves to the duration literal with them if it lands.
- **`redirects: 0` / `redirects: 5`.** A count says how many, not where to, and where to is the security question. The three named policies say where; the hop limit is a fixed structural bound.
- **`headers: { "A": a, "B": b }`.** A map literal for one or two headers is noisier than one `header` line each, and per-line items match `on status`.
- **Inferring the stream format from `Content-Type`.** The response is not available at compile time, and a provider that mislabels its stream would change how the program parses it.

## Slices

Each slice ships with interpreter tests, its reference page and roadmap checklist, and `dev-log.md` and `learnings.md` entries.

1. **Declared timeouts and redirects.** `timeout:` and `redirects:` are required on every connector, with per-operation `timeout:`. Corvid follows redirects itself, so the declared policy, the egress check and the credential rule apply to every hop: a cross-origin redirect never carries the credential (including a custom auth header), a `POST` turned into a `GET` is refused, and every policy stops after 10 hops. Startup refuses a connector whose host is not allowed. `timeout` and `redirects` are in the contract. Existing connectors migrate.
2. **Each request recorded.** `HttpExchange` events for every attempt, with credentials, `Secret` headers and `Set-Cookie` redacted; replay tests over retries and redirects; a test that a key never reaches the trace file.
3. **Typed requests and responses**, as four commits:
   - 3a. Bodies and responses through B01's public mapping, with field-path errors and a refused non-JSON body.
   - 3b. `query` records and `header` items, including from a `Secret`.
   - 3c. `Response<T>`, `HEAD` (returning `Response<Nothing>`) and `OPTIONS`.
   - 3d. Request, query and response types in the Application Contract, with the test that the runtime's encoding matches the contract's schema.
4. **One way to make a request.** The dynamic-host connector replaces `http_get` and `http_post_json`, with every doc, tour, guarantee and test that names them, and a `learnings.md` entry. It needs `Response<T>` from slice 3, because the tools return a status.
5. **Streamed responses and cancellation.** `Stream<T>` with `server_sent_events` and `json_lines`, per-event timeouts, and the caller-disconnect acceptance test through `corvid serve`.
6. **Streamed request bodies and multipart.** `Upload` and `Bytes` bodies sent without buffering, `Stream<T>` bodies, and `multipart` records.
7. **Native code.** After the shared native-dispatch decision.

Order note: `HEAD` and the tool replacement were first planned for slice 1. Both depend on `Response<T>` (a `HEAD` response has no body, and the tools return a status), so they moved after it. Until slice 4 lands, `http_get` and `http_post_json` keep their 30-second timeout and redirects to allowed hosts, now written at their one call site in `corvid-runtime` rather than as a client-wide default.

## Not in scope

- Per-environment base URLs (staging and production). A deployment concern, filed when an example needs it.
- DNS-time SSRF checks (resolving a hostname and re-checking its addresses). Already named as follow-up work in `http.rs`.
- Cookie jars, proxies, mutual TLS, HTTP/3 and custom certificate roots. Each is named in the roadmap as planned work when an example needs it.
- Response caching (B44), verified incoming webhooks (B16), WebSocket clients (B14).
- Recovery syntax at the call site (`try ... else`, B12). Failures here are the existing connector failures and typed `Result` errors.

## Decisions

Recorded 2026-11-25. The user directed the sessions to decide by the laws in `AGENTS.md` rather than ask for approval.

1. **No frontend agreement is needed.** Only backend code writes a `connector`, so its items are not grammar shared with the frontend. `HEAD` and `OPTIONS` extend the method rule routes also use; routes are backend declarations too, and the main-line session confirmed the change is safe for B10, whose action endpoints use `POST`.
2. **`http_get` and `http_post_json` are replaced by a dynamic-host connector (option A).** "One obvious way" rules out keeping a second request path, and "no hidden defaults" rules out keeping it as it is. Option B would also mean building headers, auth, typed bodies and streaming twice. The replacement and the migration of every surface that names the tools (invention entry, tour, book chapter 18, reference pages, guarantees, skills audit, Python codegen scan, tests) land together in slice 4, with a `learnings.md` entry.
3. **B05 ships on the interpreter first; slice 6 (native code) stays open.** Native dispatch of runtime-owned tools through the runtime's recording is shared architecture that native `Secret`, native UUID and the B06 boundary types also wait on. It is decided as its own task, not inside B05, so the five interpreter slices are designed to reuse the same runtime code natively.

4. **A per-call credential is its own design, not a `Secret` header (decided in slice 3b).** The proposal let a `header` line take a `Secret` parameter, for calls made on a user's behalf with that user's token. Building it showed the form does not carry its own weight: such a token almost always travels as `Authorization: Bearer <token>`, a header line cannot build that value from a `Secret` (a `Secret` cannot be joined with text, by design), and `Authorization` belongs to `auth:`. Shipping `Secret` headers would have given a feature that fits almost no real call. B04's rule stays (a connector operation's signature holds no `Secret`), and the roadmap names the follow-up: an operation-level credential such as `auth: bearer(token)` taking a `Secret` parameter, recorded as `<redacted>`, kept out of a protocol's persisted intent.

5. **A dynamic-host connector, `response: text`, and `on failure` (decided in slice 4).** A connector declares `hosts: allowed_hosts` in place of `base_url`, and its operations name a `String` URL parameter on the request line (`GET url`). The checker refuses a path there, a URL parameter on a `base_url` connector, a URL that is not a `String`, and an `async:` protocol, whose poll path is relative to one base URL. `response: text` reads a page as a `String`, because a page is not JSON and guessing the format from the type would make `String` mean two things. The migration showed one more need: `http_get` returned `Result`, so a refused URL or a timeout was a value the program handled, and the book's pipeline relied on it. A connector call that failed stopped the program, so replacing the tools as they were would have taken that away. `on failure -> Variant`, the sibling of `on status 404 -> NotFound`, turns every failure without a mapped answer into that variant of the operation's `Result` error type, with the reason when the variant has one `String` field: a refused URL, a timeout, a network error, an unmapped status, and a body that does not match the declared type. The runtime records it as the operation's answer, so replay reproduces it. It is declaration-level and backend-only; call-site fallbacks (`try ... else`) remain the main line's proposal. Its scope is exactly the request's own failures (building it, the client rate limit, the SSRF block and allowlist, the timeout, transport, redirects, an unmapped status, decoding) and an open circuit breaker, which is an outage, not a decision; governance outcomes around the call (a denied approval, an exceeded budget, a replay divergence) are decisions and still stop the program, and an `async:` operation cannot declare it, because a protocol fails through its own lifecycle. The main line's recoverable-failures proposal uses that boundary to keep one way per failure.

6. **Streams as built (slice 5).** Recording follows streamed prompts: the `ToolCall` when the stream opens, and one `ToolResult` holding every event delivered when it ends, with `end` set to `complete`, `error` or `cancelled`. A guard records a stream the program drops, so cancellation is in the trace too. Replay yields the recorded events and opens no connection. Retries happen only before the first byte, so no event is delivered twice. The declared `timeout` bounds opening and then each wait between events. Some providers close a server-sent stream with a sentinel that is not JSON, OpenAI's `data: [DONE]` among them. A silent vendor special case would be a hidden default, so `until "[DONE]"` declares it. `on failure` does not combine with a stream yet, because a failure mid-stream ends the stream with an error; typed per-event failures (`Stream<Result<T, E>>`) are on the roadmap. A streamed operation's mock is its `List<T>` of events. The `HttpExchange` record of a streamed request shows the response opening, with size 0; the stream's own record holds what was read.

7. **Typed per-event failures, planned after B05.** A streamed operation may declare `on failure -> Variant` when it returns `Stream<Result<T, E>>`. Each event then arrives as `Ok(T)`. An outage arrives as one final `Err(Variant(reason))`, after which the stream ends: a failure to open, the per-event timeout, a broken connection, an event that is not JSON or does not decode, or an open circuit breaker. A `for` loop over the stream therefore never raises for an outage, and `try ... else` has nothing to catch around it, so each failure keeps one way to be handled. Governance stays loud mid-stream exactly as it does for a single request: a budget exceeded while streaming and a replay that diverges from its trace are raised, never folded into the `Err` item. The recording keeps its shape, with the final `Err` item among the delivered events and `end: "complete"`. The main-line session's recoverable-failures proposal points here for the loop case.

8. **File bodies as built (slice 6), and streamed request bodies deferred.** `body file` sends a `Bytes` or `Upload` parameter as the body itself, and `multipart receipt` sends a record whose `Upload` and `Bytes` fields become file parts. A file is recorded by its name, type, size and SHA-256, never its content. The interpreter hands the runtime two forms of the arguments: the recorded one, which carries the digest, and the one the request is built from, which carries the bytes. It uses the recorded-versus-revealed mechanism B04 built for `Secret` arguments, so a person's uploaded document never lands in a trace, and replay matches the same file by its digest. The multipart boundary is derived from the content, so the same request has the same bytes and the same recorded digest. The checker now also requires a JSON body and a decoded response to have a JSON form; an `Upload` inside a JSON body record used to fail only at run time. Two things were planned for this slice and are deferred to the roadmap. One is a `Stream<T>` request body, which needs a channel from the interpreter's stream into the runtime's request and a way to record a stream the request consumed. The other is reading an upload from disk instead of memory, which needs `corvid serve` to keep large uploads out of memory first; today it reads an upload into memory before the program sees it.

## Questions that were open

All three are settled under [Decisions](#decisions).

1. **Frontend agreement.** The new connector items (`timeout:`, `redirects:`, `header`, `query`, `multipart`, `streams:`) are grammar, but only backend code writes connectors. Does Codex need to agree them, or only the `HEAD` and `OPTIONS` change to the method rule routes share?
2. **The dynamic-URL tools.** Replace `http_get` and `http_post_json` with a dynamic-host connector, migrating the invention entry, tour, book chapter, guarantees and tests (A, recommended)? Or keep them with required `timeout` and `redirects` arguments (B)?
3. **Native code.** B05's native slice needs native dispatch of runtime-owned tools and connectors through the runtime's recording, the same architecture native `Secret` waits on. Decide it now, or ship B05 on the interpreter and keep its native slice open behind that decision?
