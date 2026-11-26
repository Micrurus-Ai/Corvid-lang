# Proposal: typed failures in streams, and one stream protocol for both tiers

Status: **part 1 (slice S1) built 2026-11-26; part 2 proposed, waiting for frontend agreement.** Owner: backend and AI development, macOS session. Part 2 changes the event protocol a served stream sends, which the TypeScript client parses, so the proposal goes to frontend development (Codex) through the owner before either side changes.

## Part 1: `Stream<Result<T, E>>` with `on failure`

This is decision 7 of the [HTTP client proposal](http-client.md#decisions), now built.

```corvid
type Delta:
    text: String

type ModelError:
    | Unreachable(reason: String)

connector model:
    base_url: "https://api.model.example"
    timeout: 30s
    redirects: never
    modes: [real, replay]
    operation complete(prompt: String) -> Stream<Result<Delta, ModelError>> uses model_read:
        POST "/v1/complete" body prompt
        streams: server_sent_events until "[DONE]"
        on failure -> Unreachable

agent answer(prompt: String) -> String uses model_read:
    text = ""
    for item in complete(prompt):
        text = text + match item:
            Ok(delta) -> delta.text
            Err(Unreachable(reason)) -> "\n[the model stopped: " + reason + "]"
    return text
```

- Each event arrives as `Ok(T)`.
- An outage arrives as one final `Err(Unreachable(reason))`, and then the stream ends. Outages are: a failure to open, the per-event timeout, a broken connection, an event that is not JSON or does not decode, and an open circuit breaker.
- A `for` loop over the stream therefore never raises for an outage, and each failure has one way to be handled. This settles the loop case in [recoverable failures](recoverable-failures.md): `try ... else` wraps an expression and cannot reach a failure a loop raises.
- Governance stays loud mid-stream, as for a single request. A budget exceeded while streaming and a replay that diverges from its trace are raised, never folded into the `Err` item.
- The recording keeps its shape. The final `Err` item is among the delivered events, with `end: "complete"`, so replay yields it as it yields the others.
- The checker rejects `on failure` on a stream whose item type is not `Result<T, E>` with that variant in `E`, as it rejects it on a request today. A stream whose item type is a plain `T` keeps today's behaviour: an outage ends it with an error.

## Part 2: the stream protocol (needs frontend agreement)

**What exists today, found while writing this.** The two tiers do not speak the same protocol.

- **`corvid serve`**: a route that returns a `Stream<T>` answers with Server-Sent Events. Each item is a bare `data: <public JSON>`, the end is `event: done` with empty data, and a failure is `event: error` whose data is the Rust error's debug text.
- **The generated TypeScript client**: `client.stream(name, input)` posts to `/agents/{name}/stream`, which `corvid serve` does not serve, and parses the typed events `started`, `chunk`, `tool_started`, `tool_completed`, `approval_required`, `completed` and `failed` (`sdk/typescript/client/src/transport.ts`). Its parser drops a bare `data:` event, `done` and `error`.

So a frontend that uses the generated client cannot read a served stream at all. The `error` event's debug text is untyped and may show internal detail, such as a URL or a file path, to a browser.

**Proposal: one protocol, the client's typed events, sent by every served stream.**

| Event | Data | When |
| --- | --- | --- |
| `chunk` | the item's public JSON, so a `Result` item is `{"tag":"ok","ok":...}` or `{"tag":"err","err":...}` | each item |
| `completed` | `null` | the stream ended |
| `failed` | `{"error": "<code>", "detail": "<message>"}` | a failure the program did not turn into a value: a governance stop (budget, replay divergence) or an unhandled runtime error |

- The `failed` data uses the same `{error, detail}` shape as every other error answer `corvid serve` gives, and its `detail` is the message the program's own error reports, never a debug dump.
- The client reads a route's stream by the route's own path and method, the way it calls a non-streaming route, rather than through `/agents/{name}/stream`. The generated method for a streaming route returns `AgentStream<Item>`. For a `Stream<Result<Delta, ModelError>>` route, `Item` is the existing `Result` union, so a frontend handles `Err` items as values with the types the contract already gives.
- `started`, `tool_started`, `tool_completed` and `approval_required` stay in the client's union for the agent-invocation protocol in `corvid-ai.json`; a route stream sends none of them.

**Questions for frontend development.**
1. Agree that the server sends `chunk`, `completed` and `failed` as above, or name the protocol the client should read instead.
2. Agree that the client calls a streaming route by its path, replacing the `/agents/{name}/stream` URL, which nothing serves.
3. Whether `completed` should carry a value. A route stream has none; the agent protocol's `completed` carries the final result.

## Alternatives considered

- **Keep the server's bare `data:` events and change only the client's parser.** That would be less change on the server, but the client's typed union is the richer protocol and is already documented in `corvid-ai.json`. Two protocols for streams would be two ways to say one thing.
- **Send an `Err` item as a `failed` event.** That would fold a value the program handles into the transport's failure. It goes against the loop rule above, and would make a typed `Err` look like an outage to the browser.
- **A separate `error` event type for `Err` items.** It is not needed: the `Result` union already says it, inside `chunk`.

## Across the tiers

- **Backend:** one way for a streamed provider call to fail, as a value, with governance still loud.
- **Frontend:** a generated client that can read a served stream, with typed `Err` items and a typed `failed` event.
- **AI:** an assistant consuming the stream through declared actions sees `Err` items as typed values and `failed` as a typed stop.

## Slices

1. **S1** (backend only, not waiting on agreement): `Stream<Result<T, E>>` with `on failure` in the checker and runtime, recording and replay, a parity test on both tiers where streams run (the interpreter; native code has no streams yet), and the governance cases staying loud.
2. **S2** (after agreement): the served stream protocol, the `failed` event's typed data, and the client's route streaming, with an end-to-end test of a generated client reading a served `Stream<Result<...>>` route.

## Not in scope

- Native streams, which are planned work of their own (`StreamLoweringNotImplemented`).
- A `Stream<T>` request body.
