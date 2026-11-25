# Proposal: the complete HTTP client (B05)

Status: **proposed, awaiting agreement.** No code before agreement ([pre-phase chat](../../AGENTS.md#pre-phase-chat-mandatory)).
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
- **Headers.** `header "Name": param` sends a `String` parameter, and `header "Name": "literal"` a fixed value. A `Secret` parameter may be sent as a header, and every record of the call holds `<redacted>`, through the same path B04 uses for a tool taking a `Secret`. `Authorization`, `Host`, `Content-Length` and `Content-Type` cannot be set this way, because `auth:` and the body own them; the checker says which item does. Header names are checked as tokens at compile time.
- **Query building.** `query filter` names a record parameter. Each field becomes a query parameter through B01's public mapping: an absent `Option` field is omitted, a `List` repeats the key, and nested records are a compile error (no standard encoding exists). Path and query values are percent-encoded; a path parameter can never insert `/`, `?` or `#`. This is the client half of the server's `route GET "/orders" query OrderFilter`, so one record type can describe both sides.
- **Auth.** `bearer` and `basic` already exist. Basic credentials are encoded once, at send time, and the encoded header is never recorded.

### Typed JSON through B01

Request bodies are encoded, and response bodies decoded, through `corvid-public-json`, the mapping `T.from_json`, routes and prompts use. A response that does not match the declared type fails with B01's error naming the field path, the expected kind and what was found (`$.receipt_url: expected String or null, found 42`). Unknown fields are ignored, as in route bodies. The recorded result is unchanged in shape, so existing recordings still replay. This is a behavior change for any connector that sends an `Option` field, so the slice records it in `learnings.md`.

**One description of the wire format.** The Application Contract's `ContractOperation` lists a connector operation's method, path, effects, status mapping and protocol, but not its body types, so nothing describes connector bodies today that could drift. Slice 3 adds the request body, query and response types to `ContractOperation`, described by the same schema code the contract already uses for route bodies, and the generated TypeScript and Swift clients read them from there. A test encodes a value with the runtime's connector encoder and checks it against the contract's schema for that operation, including an absent `Option` field and a `List`. The runtime, contract and SDK then share one mapping, and the test fails if they diverge. Slice 1 likewise adds `timeout` and `redirects` to the contract, so a client or an assistant can see an operation's declared bounds.

### Responses with status and headers

Most operations want the body. When a program needs the status or a header (an `ETag`, `Location`, a rate-limit header), it declares `-> Response<T>`, a standard type with `status: Int`, `headers: List<HttpHeader>`, `header(name) -> Option<String>` and `body: T`. A program that does not ask for headers does not receive them. Response headers named `Set-Cookie`, or any header a connector's `auth:` echoes, are recorded as `<redacted>`.

### Streaming

- **Streamed responses.** An operation returning `Stream<T>` declares `streams: server_sent_events` or `streams: json_lines`. Each event decodes through B01 into `T` as it arrives; a malformed event ends the stream with the field-path error. The format is required because the compiler cannot know it.
- **Streamed request bodies.** A `body` parameter of type `Upload` or `Bytes` is sent from its storage without the program holding a second copy; `Upload` streams from disk. A `Stream<T>` body is sent as `json_lines`.
- **Multipart.** `multipart receipt` names a record whose fields become parts: `String` and numbers as text parts, `Upload` and `Bytes` as file parts with the upload's filename and media type. Nested records are a compile error.

### Cancellation and caller disconnect

An HTTP call is a future the runtime owns. When it is dropped, the connection is closed, not left to finish. That happens when the call's timeout expires, when a `parallel` arm is cancelled, and when the caller of a `corvid serve` route disconnects. The acceptance test holds an upstream response open, disconnects the route's caller, and checks the upstream sees its connection close within a second, and that the trace records the call as cancelled.

### Recording and replay of each request

Every attempt records an `HttpExchange` trace event: connector, operation, method, the URL with its query, request header *names* (values only for headers the source wrote as literals), a SHA-256 of the request body, status, response header names, elapsed time, attempt number, and the outcome (`ok`, `status`, `timeout`, `cancelled`, `refused`). Credentials, `Secret` headers and `Set-Cookie` values never appear. The existing `ToolCall`/`ToolResult` pair stays the unit replay substitutes, so retries and redirects replay exactly as recorded, and replay never reaches the network. The effect row is unchanged: operations still declare `uses`, and `dangerous` still routes a mutating call through approval.

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

1. **Declared timeouts and redirects.** `timeout:` and `redirects:` are required on every connector, with per-operation `timeout:`; the runtime builds one client per policy; a cross-host redirect never carries the credential, and a `POST` turned into a `GET` is refused; startup refuses a connector whose host is not allowed; `HEAD` and `OPTIONS`; `timeout` and `redirects` in the contract. Existing connectors migrate. The dynamic-URL tools are replaced or reworked (question 2), with every doc, tour, guarantee and test that names them, and a `learnings.md` entry.
2. **Each request recorded.** `HttpExchange` events for every attempt, with credentials, `Secret` headers and `Set-Cookie` redacted; replay tests over retries and redirects; a test that a key never reaches the trace file.
3. **Typed requests and responses.** Bodies and responses through B01's public mapping with field-path errors; `query` records; `header` items, including from a `Secret`; `Response<T>`.
4. **Streamed responses and cancellation.** `Stream<T>` with `server_sent_events` and `json_lines`, per-event timeouts, and the caller-disconnect acceptance test through `corvid serve`.
5. **Streamed request bodies and multipart.** `Upload` and `Bytes` bodies sent without buffering, `Stream<T>` bodies, and `multipart` records.
6. **Native code.** After open question 3 is decided.

## Not in scope

- Per-environment base URLs (staging and production). A deployment concern, filed when an example needs it.
- DNS-time SSRF checks (resolving a hostname and re-checking its addresses). Already named as follow-up work in `http.rs`.
- Cookie jars, proxies, mutual TLS, HTTP/3 and custom certificate roots. Each is named in the roadmap as planned work when an example needs it.
- Response caching (B44), verified incoming webhooks (B16), WebSocket clients (B14).
- Recovery syntax at the call site (`try ... else`, B12). Failures here are the existing connector failures and typed `Result` errors.

## Open questions

1. **Frontend agreement.** The new connector items (`timeout:`, `redirects:`, `header`, `query`, `multipart`, `streams:`) are grammar, but only backend code writes connectors. Does Codex need to agree them, or only the `HEAD` and `OPTIONS` change to the method rule routes share?
2. **The dynamic-URL tools.** Replace `http_get` and `http_post_json` with a dynamic-host connector, migrating the invention entry, tour, book chapter, guarantees and tests (A, recommended)? Or keep them with required `timeout` and `redirects` arguments (B)?
3. **Native code.** B05's native slice needs native dispatch of runtime-owned tools and connectors through the runtime's recording, the same architecture native `Secret` waits on. Decide it now, or ship B05 on the interpreter and keep its native slice open behind that decision?
