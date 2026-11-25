# Recoverable failures: `try ... else`

Status: proposed, awaiting frontend agreement (shared grammar).

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

## Not in this proposal

- Seeing which failure happened in the fallback (`else (error) -> ...`).
  It can follow once a failure type is agreed.
- A failure in the middle of a stream. A streamed connector operation
  (`Stream<T>`, B05) cannot declare `on failure`, so an error after the
  stream has opened is raised by the `for x in stream` loop reading it.
  That is an outage, so it should be catchable, but a loop is a
  statement and `try ... else` wraps an expression. Two directions to
  decide with the frontend: a `try` form for a loop
  (`try for event in stream: ... else: ...`), or typed per-event
  failures (`Stream<Result<T, E>>`, on the B05 roadmap), which would
  make the failure a value and leave nothing for `try` to catch, as for
  `on failure`. The second keeps one way per failure and is preferred.
