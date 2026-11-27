# Lineage from real runs

**Status:** proposal, 2026-11-27. Slices L1 and L2 add no syntax and are built first. L3 and L4 add a top-level `traces:` block, which is grammar shared with the frontend, so they go to the frontend review and are built after one working day without objection.
**Owner:** backend (Claude Code).
**Found while:** measuring the Postgres pool. The roadmap item to show a full pool in production led here.

## The problem

Corvid has a full observability stack:
- a lineage model with stable span ids;
- `corvid observe list | show | drift | explain | cost-optimise`;
- `corvid trace lineage`;
- `corvid eval promote`;
- an OTLP export with redaction.

Every one of them reads `<run_id>.lineage.jsonl`, and **nothing writes that file**. A real run writes a different record, the runtime trace `target/trace/<run_id>.jsonl`. That trace is what replay reads. So `observe` and the export only ever see checked-in fixtures such as the reference apps' `traces/demo.lineage.jsonl`.

Three more gaps sit behind that:
- **`corvid serve` records no trace at all.** Its runtime is built with the null tracer.
- **The observability guide promised configuration that did not exist:** `CORVID_OTEL_ENDPOINT`, `CORVID_OTEL_SERVICE_NAME`, `CORVID_OTEL_SAMPLE_RATE` and `corvid run --trace-out`. It was corrected on 2026-11-27 to say what runs today.
- **A compiled binary writes traces into `./target/trace` of whatever directory it starts in,** unless `CORVID_TRACE_DISABLE=1` is set. A production binary records its users' arguments to disk without the source ever saying so, which is a hidden default for a privacy decision.

The organizing principle is that people and developers see one timeline across UI, backend, agent and model steps. Until a real run produces that timeline, the principle is not delivered.

## The design

### One record, one projection (L1)

The runtime trace stays the one record of a run. Lineage becomes a projection of it, computed by one function, `corvid_runtime::lineage_from_trace(&[TraceEvent]) -> Vec<LineageEvent>`. Every reader calls that function, never a copy. Readers that receive a run trace (`<run_id>.jsonl`) derive its lineage. A `.lineage.jsonl` file is still read as it is.

| Trace events | Lineage span |
| --- | --- |
| `RunStarted` … `RunCompleted` | the root: `Agent`, named after the agent; `Failed` when the run ended in an error, with an `Error` child holding the message |
| `ToolCall` … `ToolResult` named `table:<Table>.<op>` | `Db`, named `<Table>.<op>` |
| other `ToolCall` … `ToolResult` | `Tool` |
| `LlmCall` … `LlmResult` | `Prompt`, with the model, and tokens and cost when recorded |
| `ApprovalRequest` … `ApprovalDecision` | `Approval`: `Ok` if accepted, `Denied` if not |
| `HttpExchange` | a child of the tool span it belongs to; a retry (`attempt > 1`) is a `Retry` span |

- **Pairing.** A call is paired with the next result of the same name in the same run, in trace order.
- **Timing.** A span's start and end are the two events' timestamps.
- **Span ids.** They come from `lineage_span_id`, so deriving the same trace twice gives the same ids. Redaction, drift comparison and eval promotion depend on that.
- **Incomplete traces.** A trace cut off mid-call keeps the call as a span with status `Failed`, ending at the trace's last event, and the run's root fails too. It is never dropped silently.
- **Which files are traces.** A reader judges a file by its first line, not its name. A run trace's first line is an event with `kind` and `run_id`, and a lineage trace's carries the lineage `schema`. A directory scan skips any other JSON lines file, and naming one directly is refused with an error saying it is neither.

### Time spent in the database (L2)

Just before a table operation's `ToolResult`, the trace gets a `DbTiming` event. It records, in microseconds, how long the operation waited for a turn and how long its statements ran, and how many statements it ran. On Postgres the turn is a connection from the pool; on SQLite it is the writer queue. A read on SQLite takes no turn.

`DbTiming` is evidence, like `HttpExchange`. Replay skips it through the same list (`is_dispatch_metadata`), because timing is not part of a result, and older traces simply have none. The `Db` span carries the two times as `db_wait_us` and `db_statement_us`, which are left out of serialized lineage when zero. The tree prints them as `waited=0.031ms ran=0.241ms`. Both OTLP exporters send them as `corvid.db.wait_us` and `corvid.db.statement_us`, through one helper.

A saturated pool then shows in every trace: wait grows while statement time stays flat, which is the pattern the [pool measurement](../guides/performance.md#choosing-a-postgres-pool-size) shows.

A field on `ToolResult` was rejected because 24 places build one. An evidence event beside it, skipped by replay's existing rule, changes nothing those places rely on.

### A served program declares its traces (L3, shared grammar)

```corvid
traces:
    location: env("TRACE_DIR")
    keep: 30 days

server api:
    route post "/cards" -> json Card requires authenticated:
        ...
```

- **One block per program.** `location:` is a path or `env("NAME")`, as for a database. `keep:` is how long a trace file is kept before `corvid serve` deletes it.
- **Recording.** Each request becomes one run, with the route as the root span (`Route`, `POST /cards`). The run id is returned in a `Corvid-Run` response header. An action called by an assistant is a run whose root names the action and the assistant.
- **No default.** A program with a `server` block must declare `traces:`. Omitting it is a compile error that names the block. The choice has a privacy consequence (arguments are kept on disk) and a cost consequence (disk), so it is not a default. The error offers `traces: none` for a program that records nothing.
- **Existing programs.** The slice adds the block to every served example, after the frontend review agrees the form.

### Export, declared (L4, shared grammar)

```corvid
traces:
    location: env("TRACE_DIR")
    keep: 30 days
    export: otlp(env("OTEL_EXPORTER_OTLP_ENDPOINT"))
```

`export:` sends each finished run's lineage to an OTLP/HTTP collector, after the redaction policy. Leaving it out exports nothing. Sending what users did to another system is a decision the source makes, never an environment variable alone. The endpoint's URL comes from the environment, so a deployment can point it at its own collector.

### Every tier follows the declaration (L5)

`corvid run` keeps recording under `target/trace`, because it is a development command run by the developer on their own machine. A compiled binary follows its program's `traces:` block like `corvid serve`, instead of writing into its working directory. `CORVID_TRACE_DISABLE` stays as an operator's emergency off switch.

## Three tiers

- **Backend.** Every run yields one timeline: route, agent, prompt, tool, approval and database steps, with cost, tokens and wait times. Developers already have this record, because replay uses it.
- **Frontend.** With L3 each request's run has an id that `corvid serve` returns in a response header. A slow save in the UI can then be looked up as the run that served it. A later slice, agreed with frontend development, can show the browser's own steps in the same timeline.
- **AI.** An assistant's action call is a run whose root names the action and the assistant. People can then see what an agent did, which action, at what cost and with which approval, from the record the server keeps. A coding AI reads the same timelines as typed JSON through `corvid observe show --json`. Nothing new is exposed to an assistant: traces are files on the server, read by its operators.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Write a `.lineage.jsonl` file beside each trace as the run happens | Two records of one run that can disagree, and twice the writes. A projection cannot drift from what replay uses. |
| Switch serve tracing on with `CORVID_TRACE_DIR` | An environment variable alone would decide whether users' arguments are kept, which the no-hidden-defaults law forbids. The source declares the policy, and the environment only says where. |
| Record serve traces by default, like `corvid run` | The same hidden default. `corvid run` is a development command; a server keeps strangers' data. |
| A `traces:` item inside the `server` block | A program can have traces with no server (a compiled job runner), and the `server` block holds only routes and live queries. A top-level block parallels `database`. |
| Configure export in a separate YAML file | A second source of truth for a decision the compiler should see. |

## Slices

- [x] L1. `lineage_from_trace`, with a test per row of the table. `corvid observe list | show`, `corvid trace lineage` and `corvid eval promote` accept a run trace, and a trace from a real `corvid run` renders as a tree. The tests run a program, not a fixture. Built 2026-11-27. Four copies of the lineage reader across `observe`, `trace lineage`, `eval promote` and the observe helpers became one, `corvid-cli`'s `lineage_input`. One copy had already drifted: it read every `.jsonl` in a directory as lineage.
- [ ] L1b-native. Agent nesting in compiled native code. Native code keeps no agent frames yet, so its steps sit directly under the run. Stronger: native agent calls push and pop the same frames through the runtime, so both tiers record the same tree.
- [x] L1b. A call from one agent to another is recorded in the trace, so lineage shows the agent nesting. Built 2026-11-27 for the interpreter. An agent call is a frame that writes nothing on entry. The first traced event inside it writes `AgentStarted` for every pending frame, outermost first, and leaving writes `AgentFinished` only if the start was written. A helper that records nothing never appears, so recording every call, which would bury a run's steps under helpers, was rejected. A static "does this agent reach a traced step" analysis was rejected too, because it would be a second list of traced steps to keep in step with the runtime; the check sits in `Tracer::emit`. A `parallel:` block writes pending starts before its arms get buffered tracers, so no arm's buffer holds an ancestor's start.
- [x] L1c. Model calls carry their token counts and cost, so `Prompt` spans and `observe`'s cost totals are real. Built 2026-11-27. The trace already held each call's usage as the `llm.usage` host event the usage ledger also gets, so the projection reads that one record, and a test checks that the lineage cost equals the ledger's. The event's name is now one constant at the emitter and the reader.
- [x] L1d. A streamed model call recorded no usage: not in the usage ledger, not in `llm.usage`, not in the cost observations. The provider's final chunk carries it (`LlmStreamChunk.usage`), and the runtime dropped it. Built 2026-11-27: a stream records its usage when it ends, through `Runtime::record_model_usage`, the same method a whole call now uses. A stream that ends without a done marker records zero tokens, as a whole call with no reported usage does. A test streams a priced call and checks the ledger, the trace order and the derived lineage. `@budget` was not affected: the interpreter charges a streamed prompt its declared per-call cost on the final chunk, not the ledger's measured cost.
- [x] L2. The time each table operation waited for a turn and ran, on both engines: a `DbTiming` evidence event in the trace, `db_wait_us` and `db_statement_us` on `Db` spans, `waited=` and `ran=` in the tree, and `corvid.db.wait_us` and `corvid.db.statement_us` in both OTLP exporters. Replay skips it. Built 2026-11-27. A run's trace replays with the database deleted.
- [ ] L3. The `traces:` block (`location:`, `keep:`, `none`) parsed and checked, and required with a `server` block. `corvid serve` records one run per request, with the route as root, and deletes traces older than `keep:`. Served examples declare it. Needs the frontend review.
- [ ] L4. `export: otlp(env(...))`, exporting each finished run's redacted lineage. Needs the frontend review.
- [ ] L5. Compiled binaries follow the `traces:` block, not the working directory.

## Non-scope

- The source-level `obs.log` and `obs.span` API: a later slice, once runs produce lineage.
- Sampling. Every request is recorded once L3 lands. A `sample:` item can follow when a measurement shows the cost matters.
- Shipping traces anywhere but disk and an OTLP collector.
