# Recoverable failures: `try ... else`

Status: TE-1 and TE-2 built (interpreter); TE-3 (native) next. Proposed to frontend development before 2026-11-25 with no objection, so it is built under the amended grammar law (AGENTS.md, 2026-11-26) and revised on both sides if an objection comes later.

## The problem

A database that is down, a network call that fails and a model call that
times out are expected events in a real application. Today a table
operation, a tool call or a prompt that fails stops the request with a
runtime error, so a page cannot say "Search unavailable, try again" and
an agent cannot fall back to a cached answer. `try ... retry` and
`try ... timeout` retry a failed attempt, then re-raise the failure.

## The form

```corvid
results = try find_titles(term) else Unavailable
summary = try summarize(text) timeout 5000 else "Summary unavailable."
```

`try <expression> else <fallback>` evaluates the expression. If it fails
because of the outside world, the whole form evaluates to the fallback
instead. It combines with the existing modifiers:
`try fetch(url) retry 3 timeout 2000 else cached`.

## What it catches, and what it never catches

| Caught (an outage) | Never caught (a bug, or a decision) |
| --- | --- |
| A table or database operation that fails: no connection, a missing table, a lost transaction | Arithmetic overflow, division by zero, an index out of range, a failed assertion |
| A tool call that fails | A denied approval: a person said no |
| A prompt or model call that fails or returns no usable reply | A budget the run would exceed |
| A `timeout` that expires | A replay divergence |

Programming errors stay loud, so `try ... else` cannot hide a bug; a
person's or a policy's decision is not an outage to paper over.

### One way per failure: connectors declare theirs

A connector operation can declare `on failure -> Variant` (B05). That
maps every failure of the request itself to a typed error value at the
declaration: a missing credential or argument, the client rate limit, a
blocked or unlisted host, the timeout, a transport error or refused
redirect, an unmapped status, a body that does not decode, and an open
circuit breaker (the dependency is known to be down, so nothing is sent).
Governance is not mapped and stays loud, exactly as here: a denied
approval, an exceeded budget, a replay divergence.

So the two never overlap. A call to an operation that declares `on
failure` already returns every outage as a value, and wrapping it in
`try ... else` is a compile error that says there is nothing to catch and
to match on the declared variant instead. `try ... else` is for the
operations that have no typed failure of their own: tables, tools,
prompts and model calls, and `timeout`. A connector operation without
`on failure` is such an operation, and `try` catches its request
failures. Because outages are catchable only through one of the two, a
connector's `circuit_breaker:` matters only once its failures are caught
one way or the other; breaker state is per interpreter run.

## Rules

- The fallback's type must be assignable to the expression's type; the
  form has the expression's type.
- The fallback is evaluated only on a caught failure, so a fallback that
  calls a tool costs only on that path; cost analysis counts the worse of
  the two paths against `@budget`.
- `try ... else` is refused inside an `atomic:` block. A failure there
  leaves the transaction unusable, so the block must roll back; wrap the
  call to the agent that holds the block instead.
- A caught failure is recorded in the trace like the failure it replaces,
  so a replay takes the same fallback.

## Simpler spellings considered

- `try X or Y` reads as boolean `or`.
- `X ?? Y` is a symbol a first-time reader must look up, and in other
  languages it means "if missing", not "if it failed".
- `catch` blocks add a second block structure for one value.
- Changing `try X retry N` to return `Err` on a final failure needs no
  syntax, but changes what existing programs do and works only where the
  expression is a `Result`.

`try X else Y` reuses the existing `try` and `else`, says "attempt this,
otherwise that" in plain words, and covers any type.

## Across the tiers

The backend uses it for pages that must degrade gracefully, such as the
reading list's search during a storage outage. The frontend can use the
same form when loading data for a screen. An agent's tool call gets a
typed fallback instead of an aborted run. One rule, the same in all
three, decides what counts as an outage.

## Building it

### One rule decides what an outage is

The table above becomes one function that `try ... retry` and `try ... else` both use: `RuntimeError::is_outage` in `corvid-runtime-core`, with `InterpError::is_outage` adding the interpreter's own kinds. Building it found three weaknesses, each fixed in the first slice:

1. **`try ... retry` retried every error.** That included a division by zero, which fails the same way every time, and a denied approval, which re-asks a person who already said no. Retry now retries outages only and re-raises anything else at once. Programs that retried a bug or a decision now fail at the first attempt, which is the behavior the table above promises.
2. **A table failure was untyped.** It arrived as `RuntimeError::Other(text)` and could not be told apart from an internal error, and a connection failure skipped the path that names the database. Table and database failures become `RuntimeError::DatabaseFailed { database, message }`, built where the runtime already names them, connection errors included.
3. **A replayed failure lost its kind.** The trace kept a failed call as text, and replay rebuilt every one as `RuntimeError::Other`. So a failure that was an outage live would not be one in replay, and replay would take a different branch. Every recorded failure now also records whether it was an outage, through one helper that every recording site uses. Replay rebuilds it as `RuntimeError::Recorded { message, outage }`, so the same rule gives the same answer in both runs.

A reply that does not fit the declared type is separated from a value that cannot be converted on the way out. Today both are `Marshal`. A model's reply, a tool's result or a provider's body that does not decode becomes `UnusableReply`, which is an outage. Converting the program's own arguments stays `Marshal`, which is a bug and is never caught.

### Slices

- **TE-1.** The outage rule, `DatabaseFailed`, `UnusableReply`, failures recorded with their kind, and `retry` using the rule. Tests: retry stops at once on a division by zero and on a denied approval; a table outage, a failing tool and an unusable model reply are outages; a recorded outage is still one in replay.
- **TE-2.** `try <expr> [timeout] [on error retry ...] [else <fallback>]` through the parser, checker, IR, cost analysis and interpreter:
  - the fallback must be assignable to the expression's type;
  - `else` is refused inside `atomic:`;
  - `else` around an operation with `on failure` is refused;
  - `@budget` counts the worse of the two paths;
  - a replay of a caught failure takes the same fallback.
- **TE-3.** Native code. Until it lands, a native build refuses `try ... else` by name, under "Narrowing is temporary". This is scheduled on the roadmap, not left for later.

## Not in this proposal

- Seeing which failure happened in the fallback (`else (error) -> ...`).
  It follows TE-2 as its own slice, once the failure value's type is
  proposed. The outage rule TE-1 builds is what that type will carry.
  Scheduled on the roadmap under "Narrowing is temporary".
- A failure in the middle of a stream is settled by B05, not here: a
  streamed operation that declares `on failure` returns
  `Stream<Result<T, E>>`, its events arrive as `Ok(T)`, and an outage
  becomes one final `Err(Variant(reason))` after which the stream ends
  ([HTTP client decision 7](http-client.md#decisions)). The loop reading
  it never raises for an outage, so there is nothing for `try ... else`
  to catch and no loop form is needed. A budget exceeded mid-stream and
  a replay divergence are still raised, never folded into the `Err`.
