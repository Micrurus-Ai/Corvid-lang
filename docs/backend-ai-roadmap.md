# Backend and AI roadmap

**Design target, November 2026.** Anything a developer builds on the backend or AI side with Python or TypeScript and their libraries and frameworks can be built in Corvid. Corvid reaches that through two paths that ship together:

1. **Built into Corvid.** The language and standard library cover what nearly every service and agent needs: typed data, HTTP, persistence, text, cryptography, money, real-time connections, model calls, tools and durable work. Built-in capabilities compose with effects, approval, provenance, replay and budgets.
2. **Typed interoperability.** Any Python package or npm package can be called from Corvid with checked types, on every production target, so the long tail of the Python and TypeScript ecosystems (machine learning, data science, vendor SDKs) is available on day one.

The defining capability stays at the center: a person and an AI agent use the same application through the same typed actions. Backend and AI grammar is owned here. Browser views and client behavior follow the [browser frontend queue](frontend-roadmap.md#ordered-tasks).

## Backend and AI implementation queue

### Audit of the current implementation

First-pass audit on November 23, 2026, from the standard library, the references and the type checker. Each task re-verifies its row with a running program before implementation starts.

| Area | Python / TypeScript reference | Working baseline | Remaining constraint |
| --- | --- | --- | --- |
| HTTP services | FastAPI, Django, Express, NestJS | Typed routes with path, query, form and JSON input, OpenAPI, uploads, server-sent events, sessions and permissions. | No WebSockets, middleware contract, GraphQL or gRPC. |
| Identity and access | Django auth, Auth.js, Passport | OAuth providers, sessions, roles, tenants, JWT contracts and API keys. | No password hashing or local credential accounts. |
| Data interchange | pydantic, zod, json, csv, PyYAML | `std:json` parses and reads fields one at a time. `T.from_json(text)` decodes into a declared type with errors that name the field, and `value.to_json()` encodes, through the same mapping as HTTP routes and OpenAPI. | Field renaming, decoding imported types and the native target are not done yet. No YAML, TOML, XML or CSV. [JSON module](../std/json.cor) |
| Outbound HTTP | requests, httpx, fetch, axios | `http_get` and `http_post_json` with retry and timeout envelopes. | No PUT, PATCH or DELETE, custom headers, auth schemes, streaming bodies or multipart. [HTTP module](../std/http.cor) |
| Persistence | SQLAlchemy, Django ORM, Prisma, Drizzle | SQLite and Postgres through parameterized SQL, migrations, transactions and audit records. | Queries are strings and rows are read with `json_get_*`, so schema changes are not compile errors. No MySQL, Redis or typed query results. [Database module](../std/db.cor) |
| Text and encoding | re, base64, uuid, urllib | String methods with Unicode semantics. | No regular expressions, base64, hex, URL encoding or UUID generation. |
| Cryptography | hashlib, hmac, secrets, crypto, bcrypt | Ed25519 signing in release tooling. | No public hashing, HMAC (required to verify payment and webhook signatures), secure tokens or password hashing. |
| Money and time | decimal, Decimal.js, datetime, Temporal | ISO instants, UTC and monotonic clocks. | No decimal type, time zone arithmetic or durations. |
| Background work | Celery, RQ, BullMQ, Temporal | Durable jobs, schedules, retries, leases, idempotency keys and worker restart recovery. | Applications hand-write leases, pause flags and operation budgets in SQL. No Redis, SQS or Kafka queues. [Extraction worker](../examples/reading_list/src/extract.cor) |
| Model calls | OpenAI SDK, Anthropic SDK, Vercel AI SDK | Multi-provider dispatch, role prompts, structured output, streaming, routing, budgets and replay. | No audio, speech or image generation. |
| Agents and tools | LangChain, LangGraph, OpenAI Agents SDK | Typed tools, bounded `ask ... allow ... within`, approval, provenance, step limits, evals and replay. | Model and tool failure recovery is written as `match` code after the call rather than declared with it. |
| Retrieval | LlamaIndex, LangChain retrievers | Ingestion, chunking, OpenAI and Ollama embeddings, SQLite search with source references. | No vector databases, hybrid search or rerankers. [Retrieval module](../std/rag.cor) |
| Shared actions | Hand-built REST plus a separate agent API | Routes, handlers and `ui.assistant_action` link a control to a handler for both people and assistants. | The handler is named by a string, and permission, validation, conflict rules and assistant exposure are spread across the route, the form type and SQL. [Progress example](../examples/reading_list/src/progress.cor) |
| Python ecosystem | Every Python package | Typed synchronous Python function imports in the interpreter, with effects, approval and replay. | Native, Wasm and Python builds reject Python imports. No async calls, objects, classes or managed environments. |
| npm ecosystem | Every npm package | Browser-side custom elements consumed through forms (frontend F09 covers browser modules). | No server-side Node.js interoperability. |
| Services | smtplib, boto3, Stripe SDK | Gmail, Slack, calendar, Microsoft 365, tasks and file connectors. | No SMTP, S3-compatible object storage or webhook verification. |

### Ordered tasks

Task branches use descriptive names such as `backend/typed-json` and `ai/model-recovery`. The queue uses stable IDs to track dependencies. Each row may take several tested increments. A task is complete only when its entire acceptance column works through the public toolchain on the interpreter and native targets, with learning-guide, grammar and function-reference updates in the same increment.

| ID | Deliverable | Depends on | Acceptance |
| --- | --- | --- | --- |
| B01 | Typed JSON encode and decode | none | Decode JSON text into any record, sum, list, map or option type and encode it back, through the same decoder routes and prompts use. Unknown fields, missing fields, wrong kinds and numeric overflow produce typed errors naming the field path. Round-trip nested records on both targets. |
| B02 | Python interoperability on every target | B01 | A native build calls a typed Python import from a project-managed, locked virtual environment. Cover async functions, Python objects and classes held as typed handles, iterators, exceptions mapped to typed errors, and lifetime across calls. Effects, approval and replay apply exactly as in the interpreter. A packaged app runs on a clean machine. |
| B03 | Text and encoding | none | Regular expressions with compile-time pattern checking, captures and replacement; base64, hex and URL encoding; UUID generation and parsing. Unicode behavior matches string methods on both targets. |
| B04 | Cryptography | B03 | SHA-2 hashing, HMAC with constant-time comparison, secure random tokens, password hashing with Argon2id, and Ed25519 verification. Verify a recorded payment-provider webhook signature and reject a tampered body. Secrets never appear in traces. |
| B05 | Complete HTTP client | B01 | Every method, headers, bearer and basic auth, query building, typed JSON bodies through B01, streamed request and response bodies, multipart upload, redirects policy, timeouts and cancellation. Caller disconnect closes the upstream connection. Effects and replay record each request. |
| B06 | Decimal money, durations and time zones | none | A `Decimal` type with exact arithmetic, declared rounding and currency formatting; durations and time zone conversion across a daylight-saving change. Mixing Decimal and Float without a conversion is a compile error. |
| B07 | Python ecosystem acceptance | B02, B05 | From a fresh project, run pandas aggregation, scikit-learn inference, a PyTorch or Transformers model and a vendor SDK through Corvid on the native target. Each call is typed, traced and replayable with the package removed. |
| B08 | Server-side npm interoperability | B01 | Import a pinned npm package in a backend module with its TypeScript declarations, call async functions, handle exceptions and callbacks, and package it into a native build. Share package resolution and lock format with frontend F09. |
| B09 | Typed persistence | B01 | Declare tables in Corvid. Queries return typed records, and renaming a column reports every affected query at compile time. Every row carries a revision, and `save(record, expecting: revision)` reports a typed conflict. Generate migrations from declaration changes. Run the same program on SQLite, Postgres and MySQL. |
| B10 | Shared action declarations | B09 | Declare an action once with typed input and result, permission, validation, conflict outcome and assistant discoverability. The compiler derives its HTTP endpoint, form binding, assistant tool, CLI command and worker call. Changing its input reports affected UI, backend and agent callers. Build the project-board move-task example. Agree the shared grammar with frontend F15 before implementation. |
| B11 | Assistant authority | B10 | An assistant invoking an action on a person's behalf never exceeds that person's permissions. Each discoverable action declares `allow`, `confirm` or `never` for assistant calls, and omitting it is a compile error. Confirmation uses the existing approval path. Revocation during a pending call denies it with no writes. |
| B12 | Model and tool failure recovery | none | `ask` and prompt calls declare recovery next to the call: malformed output, tool failure, timeout, retry limit and fallback model. A reader can predict each failure path from the source. Replay reproduces the chosen path. |
| B13 | Durable work declarations | B09, B10 | Declare long-running agent work with checkpoints, pause and resume, revisable instructions, an operation budget and leases. The reading-list extraction worker loses its hand-written lease and budget SQL and keeps its restart, pause and correction behavior. |
| B14 | Real-time connections | B01 | WebSocket routes and clients with typed messages, backpressure, authentication, heartbeat and reconnect. Typed change streams publish committed B09 records to subscribers. Coordinate the client side with frontend F08. |
| B15 | Data formats | B01 | Typed YAML, TOML, XML and CSV reading and writing through the B01 type mapping, with streaming CSV for large files. |
| B16 | Queues, caches and service integrations | B04, B05 | Redis caching and queues, SQS or Kafka-backed durable jobs, SMTP email, S3-compatible object storage, and verified webhooks for a payment provider. Each integration records effects and redacts secrets. |
| B17 | AI breadth | B05, B09 | Vector databases (pgvector and one hosted store), hybrid search and reranking, speech-to-text, text-to-speech, image generation, batch requests and provider prompt caching. Each composes with budgets, provenance and replay. |
| B18 | GraphQL and gRPC | B01, B05 | Serve and call a GraphQL schema and a gRPC service from typed Corvid declarations. Schema changes report affected callers. |
| B19 | Performance under concurrent work | B09 | Measure action-to-visible-result latency and sustained writes with concurrent agents. Clear the recorded backlog at 160 saves/s without losing a confirmed edit, and compare equivalent FastAPI and Express services on the same workloads. [Sustained editing baseline](guides/performance.md#sustained-editing-and-backlog) |
| B20 | Parity acceptance | B07, B08, B11, B13, B16, B17 | Port a representative FastAPI service, a LangGraph agent and an Express with Vercel AI SDK service to Corvid. Each port passes the original's tests, and each remaining Python or npm dependency runs through typed interoperability. Record lines of code, change time and failure diagnostics against the originals. |

### Task progress

**B01 typed JSON encode and decode** (in progress)

- [x] One public JSON mapping for HTTP. Route bodies, JSON replies, stream chunks and approval results follow the published schema, and invalid bodies return a 400 naming `path`, `expected` and `found`. Single-file services publish schemas for their private route types, nullable fields use the OpenAPI 3.1 form, and the TypeScript client matches the wire shape. The live-contract tests send every accepted shape and validate each reply against the served schema. [JSON bodies and replies](guides/backend.md#json-bodies-and-replies)
- [x] `T.from_json(text)`, `T.from_json_strict(text)` and `value.to_json()` through the same mapping, returning a `JsonError` record with `path`, `expected`, `found` and `message`. Aliases name list and map targets. The checker refuses types without a JSON form, `from_json` refuses `Grounded` data, and `to_json` on `Tainted` data stays tainted. [Typed decoding and encoding](reference/stdlib/json.md#typed-decoding-and-encoding)
- [ ] Decode types imported from another module with `from_json`.
- [ ] Field renaming for external key styles such as `userId`. Agree the field syntax with frontend development first. [Proposal](proposals/b01-json-field-keys.md)
- [x] Retire the `decode_<Type>_from_json` tool convention in favour of `T.from_json`. Declaring such a tool is a compile error naming the replacement, and the book, tour and reference now use `from_json`. [Retired convention](reference/stdlib/json.md#retired-the-decode_type_from_json-tool-convention)
- [ ] Native target support.
- [ ] Swift, Kotlin and Python SDK sum types use the wire shape, and the SDKs describe `JsonError`.

B01 to B06 are the foundation. Interoperability (B02, B07, B08) and built-in capability (B03 to B06, B09 onward) advance together, so a developer always has a typed path to a capability before Corvid gains a built-in version of it.

### Coordination with frontend development

Backend and AI semantics are owned here. Changes to shared types, actions or parser rules are agreed with frontend development before either side implements them. The shared points in this queue are:

- **B10 and B11 with F15.** One meaning for an action across the UI, the backend and assistants.
- **B08 with F09.** One package resolution, lock format and type import path for npm packages on the server and in the browser.
- **B14 with F08.** One typed message and subscription contract for real-time updates.
- **B01 with all client data.** Browser code decodes the same records with the same errors.
