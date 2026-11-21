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

## Parallel arms

`parallel:` runs two or more calls CONCURRENTLY and joins when all
complete — each arm's name binds after the block (compiled in CI):

```corvid
agent fetch_weather(city: String) -> String:
    return "sunny in " + city

agent fetch_news(city: String) -> String:
    return "quiet in " + city

agent brief(city: String) -> String:
    parallel:
        weather = fetch_weather(city)
        news = fetch_news(city)
    return weather + " / " + news
```

The governance is what makes this Corvid's: arm COSTS SUM into the
enclosing `@budget` (both are paid — parallelism hides latency,
not money); each arm's trace events buffer and flush IN ARM ORDER
at the join, so the recorded trace reads like sequential execution
and `corvid replay` reproduces the run deterministically; failures
are arm-ordered too (the first failed arm by position, not by
completion time). Each arm must be a single call — wrap richer
logic in an agent. Streams, racing, and cancellation are
post-v1.0.

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

## Let a model choose a local function

For a chat model with native function calling, pass the Corvid function
directly as a prompt argument. See [Pass a function to a prompt](11-prompts.md#pass-a-function-to-a-prompt).
For a model that selects actions through structured output, use the following pattern.

Use a sum type to list the actions your application accepts. Each variant
carries its own argument types. A prompt can return the action inside a
record, and an exhaustive match calls the corresponding Corvid function.

```corvid
type Action:
    | Add(amount: Int)
    | Finish

type Decision:
    action: Action

prompt choose(total: Int) -> Decision:
    "The current total is {total}. Choose Add or Finish."

fn add(total: Int, amount: Int) -> Result<Int, String>:
    if amount < 0:
        return Err("negative amount")
    return Ok(total + amount)

agent next_total(total: Int) -> Result<Int, String>:
    decision = choose(total)
    return match decision.action:
        Add(amount) -> add(total, amount)
        Finish -> Ok(total)
```

The prompt sends the available variants and their positional argument types
in its JSON schema. Corvid checks the returned action before entering the
match. A response with `"two"` in the integer position fails at the prompt.
A negative integer reaches `add`, which returns the application's own error.
For a multi-turn workflow, pass the updated total to the next prompt, return
on `Finish`, and bound the number of calls with a finite loop.

## Durability

Agents tagged `@replayable` and run via the durable job runner survive
process restart. Their step checkpoints, tool-call results, and
approval-wait state persist. See **[Jobs](/docs/jobs)** for the
runner story.

### Resume a worker that calls local functions

When a durable job calls a prompt with a function parameter, Corvid saves
each completed tool result with its call ID. A replacement worker opens the
same queue, rebuilds the conversation, and sends saved results back to the
model. Run the same Corvid source and model configuration on each attempt.
Use capture-free functions for durable prompts so all arguments and results
can travel with the conversation.

For the `main` agent in the [function-calling example](11-prompts.md#pass-a-function-to-a-prompt),
these commands allow one tool call and a final answer:

```sh
corvid jobs enqueue --task main --payload '[]'
corvid jobs loop limits --job job_1 --max-steps 2 --max-tool-calls 1
corvid jobs run --source main.cor --workers 1
```

Use the job ID printed by enqueue. Workers read `CORVID_MODEL` and provider
credentials just as `corvid run` does. If the worker exits, run the worker
command again. After the old lease expires, the next worker can recover the
job. A pending model request may be sent again. The recovered conversation
uses the committed tool results and retains the remaining job allowance.

A worker can store a function result and charge it to a durable job in
one SQLite transaction. For example, a job with two steps and one tool
call available can save the first result like this:

```sh
corvid jobs loop limits --job job_1 --max-steps 2 --max-tool-calls 1
corvid jobs checkpoint add --job job_1 --step-sequence 1 --kind tool-result --label add --payload '{"total":2}'
corvid jobs checkpoint resume --job job_1 --json
```

Run the commands against the same queue, using `--state PATH` when the job
lives outside the default queue. The JSON snapshot gives the new worker
the saved total, next sequence (2), remaining steps (1), and remaining tool
calls (0). An unset limit appears as `null`. Pass the next sequence when
saving the next result. A stale sequence or exhausted allowance fails the
write and leaves the saved state intact. Each sequenced checkpoint counts
as one step, and a tool result also counts as one tool call.

This transaction covers the checkpoint and its accounting. For functions
that change an external service, use that service's idempotency mechanism
to handle a crash between the external change and the local commit.

## Examples in the wild

The reference apps under `examples/backend/` ship canonical agent
shapes. Read them.
