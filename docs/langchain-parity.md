# Beyond LangChain, LangGraph and LangSmith

Readers: backend and AI development, and anyone judging whether an application built with LangChain or LangGraph is better built in Corvid.

LangChain is the most widely used framework for LLM applications in Python and TypeScript. LangGraph builds stateful agents on it, and LangSmith traces and evaluates them. **The target is not parity. Every feature developers use from the three has a stronger Corvid version**, listed below with the task that delivers it. A row is done only when its stronger version works through the public toolchain with tests. Task B20 is the final check: a real LangGraph agent ported to Corvid passes its original tests with less code and fewer ways to fail.

## Why Corvid can do each of these better

LangChain, LangGraph and LangSmith are libraries and a hosted service wrapped around a program they cannot see into, so their checks happen while the program runs, or afterwards. Corvid is one language for the interface, the backend and the AI, and its compiler sees all three. That gives four advantages no library can copy:

1. **The compiler checks AI behavior before the program runs:** output types, costs, approvals, provenance and untrusted data.
2. **Every run is recorded and replays exactly,** offline and with side effects blocked, so debugging, testing and evaluation work from real runs instead of guesses.
3. **Application actions are the agent's tools.** An action declared once is the button, the endpoint and the assistant tool, with one set of permissions.
4. **The interface is part of the same program,** so approvals, agent progress and agent proposals can appear in the application without interface code.

In the tables, **Shipped** means the stronger version works today. A task ID means the stronger version is that task's acceptance.

## Models and prompts

| Feature | LangChain | Corvid's stronger version | Status |
| --- | --- | --- | --- |
| Many providers | Chat model integrations, chosen in code | The provider is chosen by typed model routing at deploy time, not hardcoded, and the same program replays identically whichever provider served it. | Shipped |
| Prompt templates with roles | `ChatPromptTemplate` | `prompt` declarations with `system:`, `user:` and `assistant:` blocks, sent in each provider's native shape. B12 makes an unknown placeholder a compile error if it is not one already. | Shipped; placeholder check in B12 |
| Conversation history | Message placeholders | A `List<AiMessage>` parameter is the history, with no new syntax, and every call that uses it is traced and replayed. | Shipped |
| Structured output | `with_structured_output`, output parsers | The prompt's return type is declared and checked, including field refinements such as `between(1, 99)`. | Shipped |
| Repairing malformed output | Output-fixing and retry parsers | `with repair N` re-asks with the exact validation error, every attempt is traced, and `@budget` counts the worst case before the program runs. | Shipped |
| Fallback models | `with_fallbacks` | Recovery is declared on the call: malformed output, tool failure, timeout and fallback model, each predictable from the source and reproduced by replay. | B12 |
| Retries | `with_retry` | Retries are declared with the call and counted in the budget, so a retry policy can never silently exceed a cost limit. | B12 |
| Streaming | `.stream()` | `Stream<T>` returns typed partial values, and an agent's stream reaches an interface component with no WebSocket or serialization code. | Shipped for agents; interface delivery B14, B29 |
| Image input | Multimodal messages | Images passed to prompts are typed parameters, traced and replayed. | Shipped |
| Model routing | Custom routing chains | Typed routing with routing reports from real traces and a cost-quality frontier (`corvid cost-frontier`, `corvid routing-report`). | Shipped |
| Batch requests | `.batch()` | Batch calls with declared concurrency, accounted in the budget at the declared limit, replaying in recorded order. | B17, B46 |
| Provider prompt caching | Provider options | Caching declared on the prompt, with the saving shown in the cost estimate. | B17 |
| Response caching | LLM caches | Declared caching whose hits are recorded, so replay and evaluation know which answers were cached. | B44 |
| Rate limiting | Rate limiters | Provider rate limits declared in configuration and enforced by the runtime, with waiting time visible in traces. | B44 |
| Cost accounting | Callbacks, usage metadata | Worst-case cost proven before running, actual cost recorded per call, and `corvid observe cost-optimise` proposes typed changes from real traces. | Shipped |
| Audio, speech and image generation | Provider integrations | Typed audio and image values with the same budget, trace and replay guarantees as text. | B17 |

## Tools and agents

| Feature | LangChain / LangGraph | Corvid's stronger version | Status |
| --- | --- | --- | --- |
| Declaring tools | `@tool`, toolkits | `tool` declarations with typed parameters, returns and declared effects (cost, trust, reversibility, data). | Shipped |
| Tools from the application | Hand-written wrappers around an API | Every action declared in the application is an assistant tool, with the person's permissions and no wrapper code. | B10, B11 |
| Agent loop | `create_react_agent`, `AgentExecutor` | `ask ... allow ... within` bounds an agent by tool calls and time, and the bound is part of the cost proof. | Shipped |
| Tool errors | Error handling callbacks | Tool failure handling is declared with the call and reproduced by replay. | B12 |
| Human approval | `interrupt()` and resume | `approve` is required by the compiler before any dangerous tool; forgetting it is a compile error. | Shipped |
| Approval interface | Built by the developer | Approval requests and agent proposals appear in the application, and a person can edit a typed proposal before the run continues, without interface code. | B48 |
| Pause, correct and resume | Interrupts with checkpoints | Declared durable agent work with checkpoints, pause, revisable instructions, an operation budget and leases, surviving worker restarts. | B13 |
| Prompt injection | Careful prompting | Untrusted content is typed `Tainted<T>` and cannot reach a dangerous call without an explicit `trusted(...)` boundary, so injection into a consequential action is a compile error. [Injection defense](book/08-approve.md#injection-defense-untrusted-content-cant-reach-dangerous-calls) | Shipped |
| MCP tools | MCP adapters | `std.mcp` with governance: MCP output declared untrusted goes through the same approval and taint checks as any untrusted source. | Shipped |
| Multi-agent supervisor and handoffs | Supervisor, swarm, handoffs | Typed handoffs between agents in which each agent's authority and share of the budget are declared and checked, and handoff between people and agents uses the same mechanism. | B50, main-queue A09 |
| Parallel branches | `RunnableParallel` | `parallel:` named arms with typed joined results, and cost composed across arms. | Shipped |
| Fan-out over a list | `Send` map-reduce | Parallel calls over a list of any length with a declared concurrency limit, budgeted at that limit, replayed in recorded order. | B46 |
| Conditional routing | Conditional edges | `match` and `if` on typed values: the program's control flow is the graph, so there is no separate graph definition to keep in sync. | Shipped |
| Typed state | `StateGraph` state with reducers | State is typed at compile time across the interface, backend and agent, and changing a field lists every use that must change. | B10, B13, B29 |
| Graph visualization | LangGraph Studio | `corvid graph` draws an agent's steps, tools, approvals and worst-case cost from the compiled program, so the picture cannot drift from the code, and flags tools no path reaches. | B47 |

## Memory and persistence

| Feature | LangGraph | Corvid's stronger version | Status |
| --- | --- | --- | --- |
| Checkpoints and threads | Checkpointers on memory, SQLite, Postgres | Checkpoints are part of declared durable work, stored with typed records, and every checkpoint is also a replay point. | B13 |
| Time travel and forking | Replay and fork from a checkpoint | Rewind to any step across the interface, backend and agent, change a prompt or code, and continue from the recorded earlier steps. | B28 |
| Long-term memory | `Store` with namespaces and semantic search | A typed memory store whose entries carry provenance, so remembered facts keep their sources and personal data stays under the declared redaction rules. | B44, B33 |

## Retrieval

| Feature | LangChain | Corvid's stronger version | Status |
| --- | --- | --- | --- |
| Embeddings | Embedding integrations | Embedders are typed values, traced and replayable. | Shipped (OpenAI, Ollama) |
| Search with sources | Vector store retrievers | Results carry their sources as `Grounded<T>`, and the compiler refuses to present an unsourced value as grounded. | Shipped (SQLite) |
| Vector databases | Many integrations | pgvector and a hosted store behind the same typed retrieval interface, with provenance kept. | B17 |
| Hybrid search and reranking | Ensemble retrievers, rerankers | Hybrid search and reranking as declared retrieval steps, costed and traced. | B17 |
| Chunking | Text splitters | Structure-aware splitting for documents, with each chunk's position kept for citations. | B45 |
| Query-rewriting retrievers | Multi-query, compression, parent document | Retrieval strategies as typed, composable steps whose cost and provenance are checked like any other call. | B45 |
| Document loaders | PDF, HTML, Markdown, web and more | Loaders whose output is typed and marked untrusted by default, so injected instructions in a document are contained. | B45 |
| Re-indexing | Indexing API with a record manager | Re-indexing that skips unchanged documents and keeps citations stable. | B45 |

## Tracing and evaluation

| Feature | LangSmith | Corvid's stronger version | Status |
| --- | --- | --- | --- |
| Tracing | Hosted traces | Every model call, tool call and approval is traced locally, with no hosted service required. | Shipped |
| Replay | Viewing past runs | `corvid replay` re-runs a trace exactly, offline, with side effects blocked. | Shipped |
| One timeline across the application | — | One trace across interface events, backend calls, agent steps and model calls. | B28 |
| Evaluations and datasets | Datasets and evaluators | `eval` declarations with model-judged assertions, written in Corvid and checked by the compiler. | Shipped |
| Evaluations in the build | Run separately | Evaluations run in `corvid test` against recorded responses with no API key, and a score below the recorded baseline fails the run. | B49, B21 |
| Explaining a quality change | Compare experiments | `corvid eval-drift` attributes a change to the model, the prompt, the retrieval index or the input. | Shipped |
| Datasets from feedback | Annotation queues | An evaluation case is generated from a "wrong answer" feedback record. | Shipped |
| Checking a change before merging | — | `corvid trace-diff` replays recorded traces against the base and head of a change and reports the counterfactual impact. | Shipped |
| Production failures | Monitoring dashboards | A production failure exports as a redacted trace that replays locally and becomes a regression test. | B32, main-queue A10 |
| Prompt management | Prompt hub | Prompts are code: versioned with the program, typed, and compared across recorded runs with `corvid trace-diff`. | Shipped |
| OpenTelemetry | OpenTelemetry export | OpenTelemetry export from the runtime, with conformance verified. | Open audit item in the main roadmap |

## The ecosystem

LangChain's largest advantage is its integrations: hundreds of model providers, vector stores, loaders and tools. Corvid does not wait to rebuild them. Typed Python interoperability on every target (B02, accepted by B07) and server-side npm interoperability (B08) let a Corvid program call any LangChain integration with checked types, and with effects, approval, taint and replay applied to each call. That already makes each integration safer to use from Corvid than from Python. Built-in versions follow for the integrations nearly every application needs.

## Checklist

- [ ] B10, B11: application actions are assistant tools with the person's permissions.
- [ ] B12: recovery, fallback models, retries and tool errors declared on the call; confirm unknown template placeholders are compile errors.
- [ ] B13: durable agent work with checkpoints, pause, revision and resume.
- [ ] B14, B29: typed streams and shared state reach the interface without glue code.
- [ ] B17: batch requests, provider prompt caching, vector databases, hybrid search, reranking, and audio, speech and image generation.
- [ ] B28: one timeline and rewind across the interface, backend and agent.
- [ ] B32: production traces replay locally and become regression tests.
- [ ] B44: response caching, rate limiting and a typed long-term memory store with provenance.
- [ ] B45: typed, untrusted-by-default document loaders, structure-aware splitting, retrieval strategies and stable re-indexing.
- [ ] B46: fan-out over lists with a declared concurrency limit.
- [ ] B47: `corvid graph` drawn from the compiled program.
- [ ] B48: approvals and editable proposals in the application without interface code (frontend proposal F-P7).
- [ ] B49: evaluations in `corvid test` with recorded baselines.
- [ ] B50: typed multi-agent handoffs with declared authority and budget shares.
- [ ] B02, B07, B08: any LangChain integration callable from Corvid with checked types.
- [ ] B20: port a LangGraph agent; it passes the original's tests, and lines of code, change time and failure diagnostics are recorded against the original.
- [ ] Publish a side-by-side example of the same agent in LangGraph and in Corvid once B13 lands.
