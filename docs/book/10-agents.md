# Agents

## What an agent is

An agent is a Corvid function that composes prompts and tools, has an
inferred or declared effect row, and is the unit of execution that
`corvid run` and `corvid build` operate on.

```corvid
agent main(ticket: String) -> String:
    summary = summarize(ticket)
    return summary
```

## Annotations

```corvid
@budget($0.50)
@max_steps(10)
@max_wall_time(30s)
@retry(max_attempts: 3, backoff: exponential(base: 2s))
@idempotency(key: ticket.id)
@replayable
agent handle_ticket(ticket: Ticket) -> Decision:
    ...
```

Each annotation produces a compile-time guarantee:

- `@budget($X)` — composed cost across the agent's call graph cannot
  exceed `$X`. The compiler refuses if the worst case is over.
- `@max_steps` / `@max_wall_time` — runtime limits enforced by the
  agent runner.
- `@retry` — the durable job runner (Phase 38) honors these on
  scheduled or queued runs.
- `@idempotency(key: ...)` — duplicate runs collapse to one
  side-effect; enforced at the SQL layer for durable jobs.
- `@replayable` — the agent's run is recorded as a deterministic
  trace; replay reproduces it byte-for-byte.

## Calling other agents

```corvid
agent outer(input: String) -> String:
    inner_result = inner(input)
    return process(inner_result)

agent inner(input: String) -> String uses llm_effect:
    ...
```

Calling an agent inside another agent composes effect rows and budgets.
The outer agent's `@budget` must cover the inner agent's worst case.

## Loop bounds

Agent loops have hard caps:

```corvid
agent loop_until_done(state: State) -> State:
    while not state.done:
        state = step(state)
    return state
```

Without `@max_steps` or `@max_wall_time`, the compiler emits a warning
that the loop is unbounded. Production agents should always declare
bounds.

## Durability

Agents tagged `@replayable` and run via the durable job runner survive
process restart. Their step checkpoints, tool-call results, and
approval-wait state persist. See **[Jobs](/docs/jobs)** for the
runner story.

## Examples in the wild

The reference apps gallery (`examples/refund_bot.cor`,
`examples/rag_qa_bot.cor`, `examples/support_escalation_bot.cor`)
ships canonical agent shapes. Read them.
