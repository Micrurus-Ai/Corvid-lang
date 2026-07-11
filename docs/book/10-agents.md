# Agents

Every `corvid`-tagged block compiles through the real driver in CI.

## What an agent is

An agent is a Corvid function that composes prompts and tools, has an
inferred or declared effect row, and is the unit of execution that
`corvid run` and `corvid build` operate on.

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

prompt summarize(text: String) -> String uses llm_call:
    "Summarize: {text}"

agent main(ticket: String) -> String:
    summary = summarize(ticket)
    return summary
```

## Annotations

Annotation arguments are dimensional constraint values —
`@budget($0.50)`, `@max_steps(10)`, `@max_wall_time(30)`:

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

prompt classify(ticket: String) -> String uses llm_call:
    "Classify this ticket: {ticket}"

@budget($0.50)
@max_steps(10)
@max_wall_time(30)
@replayable
agent handle_ticket(ticket: String) -> String:
    return classify(ticket)
```

Each annotation produces a compile-time or runtime guarantee:

- `@budget($X)` — composed cost across the agent's call graph cannot
  exceed `$X`. The compiler refuses (error E0250) if the static
  worst case is over.
- `@max_steps` / `@max_wall_time` — runtime limits enforced by the
  agent runner (wall time in seconds).
- `@replayable` — the agent's run is recorded as a deterministic
  trace; replay reproduces it byte-for-byte.

- `@retry(max_attempts: 3, backoff: exponential 250)` — the
  agent-side default retry policy for durable jobs executing this
  agent (`backoff` takes `linear` or `exponential` with a
  milliseconds base; enqueue-time flags take precedence).
- `@idempotency(key: order_id)` — names the agent PARAMETER whose
  value derives the durable job's idempotency key. The checker
  verifies the name matches a declared String or Int parameter.

Both compile into the agent's IR metadata; the durable job queue's
enqueue surface (`corvid jobs enqueue`) reads its values from flags
today — annotation-derived defaults at enqueue time are the queue's
follow-up (tracked in ROADMAP).

## Calling other agents

```corvid
effect llm_effect:
    cost: $0.01
    latency: medium
    confidence: 0.9

prompt rewrite(input: String) -> String uses llm_effect:
    "Rewrite cleanly: {input}"

agent inner(input: String) -> String:
    return rewrite(input)

agent outer(input: String) -> String:
    inner_result = inner(input)
    return inner_result
```

Calling an agent inside another agent composes effect rows and budgets.
The outer agent's `@budget` must cover the inner agent's worst case.

## Loop bounds

Agent loops iterate with `for … in` today (`while` lands with slice
45k):

```corvid
agent drain(work_items: List<Int>) -> Int:
    completed = 0
    for item in work_items:
        completed = completed + 1
    return completed
```

Production agents that loop over model calls should declare
`@max_steps` / `@max_wall_time` bounds.

## Durability

Agents tagged `@replayable` and run via the durable job runner survive
process restart. Their step checkpoints, tool-call results, and
approval-wait state persist. See **[Jobs](/docs/jobs)** for the
runner story.

## Examples in the wild

The reference apps under `examples/backend/` ship canonical agent
shapes. Read them.
