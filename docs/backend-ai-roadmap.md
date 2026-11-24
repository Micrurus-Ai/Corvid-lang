# Backend and AI roadmap

**Design target, November 2026.** Anything a developer builds on the backend or AI side with Python or TypeScript and their libraries and frameworks can be built in Corvid. Corvid reaches that through two paths that ship together:

1. **Built into Corvid.** The language and standard library cover what nearly every service and agent needs: typed data, HTTP, persistence, text, cryptography, money, real-time connections, model calls, tools and durable work. Built-in capabilities compose with effects, approval, provenance, replay and budgets.
2. **Typed interoperability.** Any Python package or npm package can be called from Corvid with checked types, on every production target, so the long tail of the Python and TypeScript ecosystems (machine learning, data science, vendor SDKs) is available on day one.

The defining capability stays at the center: a person and an AI agent use the same application through the same typed actions. The organizing principle, from the [platform plan](proposals/corvid-platform-plan.md), is that the application and the AI can always see each other, within what the author declares. Backend and AI grammar is owned here. Browser views and client behavior follow the [browser frontend queue](frontend-roadmap.md#ordered-tasks).

## Backend and AI implementation queue

### Audit of the current implementation

First-pass audit on November 23, 2026, from the standard library, the references and the type checker. Each task re-verifies its row with a running program before implementation starts.

| Area | Python / TypeScript reference | Working baseline | Remaining constraint |
| --- | --- | --- | --- |
| HTTP services | FastAPI, Django, Express, NestJS | Typed routes with path, query, form and JSON input, OpenAPI, uploads, server-sent events, sessions and permissions. | No WebSockets, middleware contract, GraphQL or gRPC. |
| Identity and access | Django auth, Auth.js, Passport | OAuth providers, sessions, roles, tenants, JWT contracts and API keys. | No password hashing or local credential accounts. |
| Data interchange | pydantic, zod, json, csv, PyYAML | `std:json` parses and reads fields one at a time. `T.from_json(text)` decodes into a declared type with errors that name the field, and `value.to_json()` encodes, through the same mapping as HTTP routes and OpenAPI. | Field renaming and the native target are not done yet. No YAML, TOML, XML or CSV. [JSON module](../std/json.cor) |
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
| B21 | First run without an API key | none | `corvid new` model calls run from recorded responses offline, with a one-line switch to a live model; `corvid model pull` adds an optional local model. A clean machine reaches a running AI step with no account. |
| B22 | Compiler questions for coding AIs | none | Over MCP, a coding assistant asks who calls an action, what an input change affects across UI, backend and agents, a route's effects and costs, and what a recorded run did, answered by the same checker and contract the build uses. |
| B23 | First-hour friction audit | none | Install, create, run and reach the AI step on clean Windows, macOS and Linux machines; file each stumble as a task. Repeat after B21 and B24. |
| B24 | Flagship task-board demo | B10, B11, frontend F-P1 | A person drags a card, an assistant moves another through the same typed action and permission, and changing the action's input lists affected UI, backend and agent code. Ships as `corvid new board` and runs with B21. |
| B25 | Screen state for assistants | B10, frontend F-P2 | An assistant receives the typed, declared state of the person's current screen and acts on it. |
| B26 | Outside agent access | B11 | A deployed app offers declared actions to outside assistants over MCP; omitting the exposure declaration is a compile error. |
| B27 | Visible agent activity | B11, frontend F-P3 | Each action records person or agent, reason, provenance, cost and reversibility; reversible actions offer undo without custom code. |
| B28 | One timeline with rewind | main-queue A10, frontend F-P4 | One trace across UI, backend, agent and model steps; rewind to a step, change a prompt or code, continue from recorded earlier steps. |
| B29 | Live shared state with declared scope | B14, frontend F08 | Shared state declares scope and read/write permission, changes go through typed actions, and omitting the scope is a compile error. |
| B30 | Only the fields that are used | B09, B10 | The compiler narrows payloads, and queries Corvid issues, to the fields a page or agent uses; reductions are measured and published. |
| B31 | Checked generation | B12 | `with repair` covers model-generated SQL and code, run in a sandbox and checked against the app's types and rules. |
| B32 | Production traces on a laptop | main-queue A10 | A production failure exports as a redacted trace that replays locally with external writes disabled. |
| B33 | Personal data as a checked type | none | Personal data cannot reach logs, traces or a model except through a declared redaction step, checked at compile time. |
| B34 | Tensors with checked shapes | B02, B07 | Shapes, including symbolic dimensions, are checked at compile time; tensors run through typed PyTorch interoperability. |
| B35 | Automatic differentiation | B34 | `grad` over tensor functions, verified against numerical gradients. |
| B36 | MLIR code generation | B34 | Tensor code compiles through MLIR for CPU, then CUDA, Metal and ROCm, matching the interoperability results. |
| B37 | Declared placement of model work | B36, B11 | The program declares where inference may run and the fallback order; the runtime never picks an undeclared placement. |
| B38 | Structured concurrency at scale | none | Lightweight tasks with structured cancellation serve 100,000 concurrent connections in a published benchmark against equivalent Go and Node.js services. |
| B39 | Ownership-based memory on every target | none | The native backend's ownership analysis applies on every target, with published reference-count and pause measurements. |
| B40 | Type inference design | none | A design discussion settles the inference scheme by what it infers and the clarity of its errors. |
| B41 | C and C++ headers | B02 | Import a C header with checked types into native builds; C++ follows. |
| B42 | Converting an existing application | B20 | `corvid absorb` converts a FastAPI or Next.js app with assistant help, reported complete only when the original's tests pass. |
| B43 | One-command deployment | none | `corvid ship` deploys to an existing provider, asks who may reach the app, and never publishes without confirmation. |
| B44 | Model caching, rate limits and long-term memory | B09 | Declared response caching and provider rate limits that replay reproduces, and a long-term memory store shared across sessions with namespaces and semantic search. [LangChain parity](langchain-parity.md#memory-and-persistence) |
| B45 | Documents for retrieval | B15 | Loaders for PDF, HTML, Markdown and web pages, structure-aware splitting, query-rewriting retrievers, and re-indexing that skips unchanged documents. Retrieved text keeps its provenance. [LangChain parity](langchain-parity.md#retrieval) |
| B46 | Dynamic fan-out | none | Run an agent or prompt over a list of any length in parallel with a declared concurrency limit, and combine the results. Budgets account for the declared limit, and replay reproduces the order of results. [LangChain parity](langchain-parity.md#tools-and-agents) |
| B47 | The agent graph drawn from the code | none | `corvid graph` renders an agent's steps, branches, tools, approvals and worst-case cost from the compiled program and flags tools no path reaches. The rendering is generated, so it cannot drift from the code. [Beyond LangChain](langchain-parity.md#tools-and-agents) |
| B48 | Human-in-the-loop interface without interface code | B11, B13, frontend F-P7 | An approval request or an agent's proposal appears in the application; a person approves, rejects, or edits the typed proposal in a generated form, and the run continues with the edit. |
| B49 | Evaluations in the build | B21 | Evaluations run against recorded responses in `corvid test` with no API key, and a score below the recorded baseline fails the run with the cases that regressed. |
| B50 | Typed multi-agent handoffs | B11, B13 | One agent hands work to another with a typed request and result; each agent's authority and share of the operation budget are declared and checked, and a handoff between a person and an agent uses the same mechanism. [Beyond LangChain](langchain-parity.md#tools-and-agents) |
| B51 | Native model inference | B01 | ONNX Runtime, GGUF through llama.cpp, whisper.cpp, Hugging Face tokenizers and safetensors run inside a Corvid binary with no Python. A model file imports as a typed function whose tensor signature comes from the file, so a wrong shape is a compile error; every prediction carries the model's identity; latency and GPU budgets are declared. [Machine learning in Corvid](ml-ecosystem.md#running-models) |
| B52 | Typed dataframes | B09, B15 | Polars, Arrow and DuckDB built in; column names and types are known at compile time from Parquet and CSV schemas, renaming a column lists every use, and a trained model's expected feature schema is checked at the call. [Data](ml-ecosystem.md#data) |
| B53 | Computer vision | B51 | Images typed by format, channels and size; OpenCV operations built in; detection and OCR through B51 with typed results and provenance; recognized text untrusted by default; images of people follow B33. [Computer vision](ml-ecosystem.md#computer-vision) |
| B54 | Generation constrained to declared types | B51 | A local model generates only values of the declared type, so malformed structured output cannot occur. [Generation](ml-ecosystem.md#generation-and-prompt-programs) |
| B55 | Prompt optimization from evaluations | B49 | Prompts are optimized against the program's `eval` declarations within a declared budget, with every candidate run recorded and replayable. |
| B56 | Model registry and experiment tracking | B32, B49 | Experiments compared from traces; typed model artifacts with weights hash, license, data lineage and evaluation scores; datasets versioned by content hash; promotion requires passing evaluations and approval. [Experiments](ml-ecosystem.md#experiments-registries-and-feedback) |
| B57 | Embedding types with model identity | B17 | An embedding's type records its model and dimension, so searching an index with embeddings from a different model is a compile error. |
| B58 | Feedback that becomes data | B10, B56, frontend F-P8 | People correct predictions in the application, and the corrections become typed training and evaluation data with provenance and consent, without interface code. |
| B59 | Training as durable work | B13, B35 | Training and fine-tuning runs declare a GPU-time budget, checkpoint and resume across restarts, and record seeds and data snapshots so any run reproduces exactly. |
| B60 | Knowledge and property graph indexes | B45, B52 | Graph indexes whose node and relation types are declared, extracted with typed prompts, queried with typed results, and every fact carries its source. [LlamaIndex](langchain-parity.md#llamaindex) |
| B61 | Sources kept through synthesis and extraction | B45 | Refine, tree-summarize and compact synthesis, and document-scale extraction, keep the sources of each part of the answer or each extracted field as `Grounded` provenance. |
| B62 | Retrieval evaluation | B49 | Hit rate and mean reciprocal rank computed from recorded queries and the documents that answered them, with generated evaluation sets, failing `corvid test` when they drop below the recorded baseline. |

**Order of work, amended 2026-11-23.** B44 to B46 and B47 to B50 come from [Beyond LangChain, LangGraph, LangSmith and LlamaIndex](langchain-parity.md), which gives every feature of the three a stronger Corvid version; B20 verifies it. B51 to B59 come from [machine learning, deep learning and vision in Corvid](ml-ecosystem.md), and B60 to B62 from the document's LlamaIndex section. Amended 2026-11-24: B03 (text and encoding) runs in parallel with B09 on a second worker, because it depends on nothing and touches different code; the main line continues B09, B10 and B11 in order. Amended again 2026-11-24: a second backend session on macOS takes B06 (decimal money, durations and time zones), which depends on nothing, in parallel with the main line, then B04 once B03 is on `main`; [parallel backend sessions](internals/parallel-backend-sessions.md) describes how the sessions split the work. B21 to B43 come from the [platform plan](proposals/corvid-platform-plan.md), which gives the reasons and the frontend proposals they pair with. Language work comes first (amended the same day): the rest of B01 that does not wait on frontend agreement; B09, B10 and B11, moved ahead of B02 to B08 because the flagship demo and the plan's second track depend on them; then B21, B22 and the B23 audit; B24; then the remaining tasks by dependency.

### Task progress

**B01 typed JSON encode and decode** (in progress)

- [x] One public JSON mapping for HTTP. Route bodies, JSON replies, stream chunks and approval results follow the published schema, and invalid bodies return a 400 naming `path`, `expected` and `found`. Single-file services publish schemas for their private route types, nullable fields use the OpenAPI 3.1 form, and the TypeScript client matches the wire shape. The live-contract tests send every accepted shape and validate each reply against the served schema. [JSON bodies and replies](guides/backend.md#json-bodies-and-replies)
- [x] `T.from_json(text)`, `T.from_json_strict(text)` and `value.to_json()` through the same mapping, returning a `JsonError` record with `path`, `expected`, `found` and `message`. Aliases name list and map targets. The checker refuses types without a JSON form, `from_json` refuses `Grounded` data, and `to_json` on `Tainted` data stays tainted. [Typed decoding and encoding](reference/stdlib/json.md#typed-decoding-and-encoding)
- [x] Decode public types from another module, written `Customer.from_json` after `use Customer` or `m.Customer.from_json` after `as m`. The checker and the IR lowerer share one function that decides which type a receiver names.
- [x] Expand type aliases declared in another module in every type position. The checker records the type of every type reference, and IR lowering reads that table instead of resolving types a second time, which also fixed record fields typed with any alias lowering to an empty record.
- [x] The ABI emitter reads the checker's table too, so the descriptor describes aliases and imported types exactly as the compiled program uses them. Its own resolver, which returned `Unknown` for every imported type, is deleted.
- [ ] Field renaming for external key styles such as `userId`. Agree the field syntax with frontend development first. [Proposal](proposals/b01-json-field-keys.md)
- [x] Retire the `decode_<Type>_from_json` tool convention in favour of `T.from_json`. Declaring such a tool is a compile error naming the replacement, and the book, tour and reference now use `from_json`. [Retired convention](reference/stdlib/json.md#retired-the-decode_type_from_json-tool-convention)
- [x] Native decoding. `T.from_json` and `T.from_json_strict` compile to native code through the decoder the interpreter uses, now in `corvid-public-json`. The compiler embeds one decode plan per target type, naming the typeinfo of every shape the runtime allocates, so the runtime never decides a native layout. `JsonError` is an ordinary native record, so `problem.message` works in match arms. Parity tests compare both tiers and check that a failed decode releases what it built.
- [x] Native encoding. `value.to_json()` walks the type instead of the value, through one encoder both tiers share, and compiles to native code using the plan decoding embeds. An empty map with non-String keys now encodes as `[]` and decodes back. `Grounded` values stay on the interpreter, because native code keeps no provenance to encode.
- [x] An absent field typed `Tainted<Option<T>>` decodes as `None`, as an absent `Option<T>` field does, on both tiers.
- [x] Swift, Kotlin and Python SDK models read and write the public JSON mapping: sum types in the wire shape, `Result`, maps with non-String keys as pairs, `null` for absent values, `JsonError` and the `Page` envelope. Kotlin `Int` fields are `Long`, as Corvid's `Int` is 64-bit. Tests run the generated Python and compile and run the generated Kotlin and Swift.
- [ ] The Application Contract names field types as written in source, so a field typed with an alias (`orders: Orders`) names `Orders`, which no SDK declares. Name types from the checker's resolved types instead, as IR lowering does. The TypeScript client and the frontend read the contract, so agree the change with frontend development first.

**B03 text and encoding** (in progress). [Proposal](proposals/b03-text-and-encoding.md)

- [x] Encoding and UUID parsing on the interpreter: base64 (standard and URL-safe), hex and URL-component encoders and decoders, and `parse_uuid`, all pure String methods backed by one `corvid-text` crate. Decoders return `Result` values and never trap. [Text and encoding](reference/text-and-encoding.md)
- [x] Regular expressions on the interpreter: `contains_regex`, `find_regex`, `find_all_regex`, `replace_regex` and `split_regex`, with the pattern and a literal replacement checked at compile time, and the `RegexMatch` record with character positions and capture groups. [Regular expressions](reference/text-and-encoding.md#regular-expressions)
- [ ] `JsonError` cannot be written in a signature: the resolver does not register it, though the checker knows the name. Register it as `RegexMatch` is. Found during B03; belongs to B01.
- [x] UUID generation: `uuid_v4` and `uuid_v7` tools in `std/uuid`, traced and replay-substituted. [Generating UUIDs](reference/text-and-encoding.md#generating-uuids)
- [x] Native encoding and UUID parsing, with parity tests. Native code calls the `corvid-text` functions the interpreter calls, and a decoder fills a native `Result<String, String>`.
- [x] Native `contains_regex`, `replace_regex` and `split_regex`, with parity tests, through the `corvid-text` functions the interpreter calls.
- [ ] Native `find_regex` and `find_all_regex`. Native code must allocate `RegexMatch` records with a `List<Option<String>>` field; represent `RegexMatch` as an ordinary native record before ownership analysis, as `JsonError` is, and let the compiler pass the layout to the runtime, as the JSON decode plan does. Until then these programs run on the interpreter.
- [ ] Patterns built at run time, through a validated pattern value. Deferred until a backend example needs one.
- [ ] Named-group lookup on `RegexMatch` checked against the pattern's group names.
- [ ] Raw string literals for patterns. Agree the lexical rule with frontend development first.
- [ ] Byte-list encoders and decoders, with B04's byte representation.
- [ ] Native UUID generation, with native dispatch of the other stdlib tools.

**B09 typed persistence** (in progress). [Proposal](proposals/b09-typed-persistence.md) · [Reference](reference/tables.md)

- [x] P1. `table` and `database` declarations, the row record with `revision`, `Shelf.get`, `insert`, `save` and `delete` with `Result`-typed conflicts, on SQLite through the interpreter. Operations use the `db_egress_read` and `db_egress_write` effects, are refused in pure `fn`s and `@deterministic` agents, and share the `std.db` handles, writer lock and replay write quarantine. `corvid migrate plan` writes `CREATE TABLE` for declared tables.
- [x] P1b. Table operations are recorded in the trace as `table:<Table>.<op>` calls, and a replay serves reads and write outcomes from it without opening the database.
- [x] P2. Typed queries (`where`, `order_by`, `descending`, `take`, `all`), refused at compile time when a condition cannot run in the database. A chain runs as one parameterized `SELECT`, ordered by key when nothing else decides, and is recorded and replayed as `table:<Table>.query`.
- [ ] P3. Migrations from declaration changes, with declared renames (`renamed from`) and drops (`retired`).
- [ ] P4. Postgres, with a query suite every engine runs.
- [ ] P5. MySQL.
- [ ] P6. Native target.
- [ ] P7. Port `examples/reading_list` from hand-written SQL to tables.

**Test infrastructure**

- [x] Integration tests that link Corvid programs find Cargo's build output wherever it is. The `corvid-test-paths` crate resolves the target directory from `CARGO_TARGET_DIR`, then from the `CACHEDIR.TAG` marker above the running test binary (which covers `build.target-dir`), then `<workspace>/target`. It replaces 15 copies of a `<workspace>/target/release` path, and `build_integration` no longer builds the whole workspace a second time into `<workspace>/target`.

B01 to B06 are the foundation. Interoperability (B02, B07, B08) and built-in capability (B03 to B06, B09 onward) advance together, so a developer always has a typed path to a capability before Corvid gains a built-in version of it.

### Coordination with frontend development

Backend and AI semantics are owned here. Changes to shared types, actions or parser rules are agreed with frontend development before either side implements them. The shared points in this queue are:

- **B10 and B11 with F15.** One meaning for an action across the UI, the backend and assistants.
- **B08 with F09.** One package resolution, lock format and type import path for npm packages on the server and in the browser.
- **B14 with F08.** One typed message and subscription contract for real-time updates.
- **B01 with all client data.** Browser code decodes the same records with the same errors.
- **B24, B25, B27 and B28 with the platform plan's proposals F-P1 to F-P4.** The flagship demo, screen state for assistants, agent activity and the timeline inspector each need both halves. [Platform plan](proposals/corvid-platform-plan.md#proposals-for-frontend-development)
