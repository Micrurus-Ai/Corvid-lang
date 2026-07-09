# Prompts

## What a prompt is

A prompt is a function whose body is a single string template, whose
return type is a typed value (or an `Option<T>` / `Result<T,E>` if
the parse might fail), and whose effect row carries the LLM call's
cost, latency, and confidence.

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

prompt summarize(text: String) -> String uses llm_call:
    "Summarize the following in one sentence: {text}"
```

## How interpolation works

The body is one template string. Parameters interpolate with
`{param}` — any declared parameter, not just strings. Non-string
values render as their JSON form, so an `Int` parameter needs no
conversion:

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

prompt classify_priority(score: Int) -> String uses llm_call:
    "The urgency score is {score}. Reply with one word: low, medium, or high."
```

The body is a template, not an expression — `"Score is " + score`
is not a prompt body. Everything the model should see goes inside
the one template string.

## Typed return values

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

type Decision:
    refund: Bool
    reason: String

prompt decide(ticket: String) -> Decision uses llm_call:
    "Given this support ticket: {ticket} — decide whether to refund. Reply as JSON with fields refund and reason."
```

The runtime derives a JSON schema from the `Decision` type, sends it
with the request (schema-constrained decoding on providers that
support it), and parses the response into a `Decision` value. Parse
failure is a typed error, not a panic.

For per-struct decoders the compiler emits at codegen time, see the
slice that landed this surface
([Phase 20n-C](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/phases/phase-20n-open-gap-implementation.md)).

## Multi-message prompts

> **Planned — lands in slice 46b of the Language completeness
> track.** Today a prompt renders as a single user-role message;
> there is no system-prompt or role-block surface yet. First-class
> conversation history follows in slice 46c.

```corvid-planned
prompt ask(question: String) -> String uses llm_call:
    system: "You are a careful, terse assistant."
    user: "{question}"
```

## Provider routing

Which LLM serves the prompt is decided by the typed model-routing
substrate at deploy time, not by hardcoding the provider in the
prompt body. See
[`docs/internals/effect-spec/13-model-substrate-shipped.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/internals/effect-spec/13-model-substrate-shipped.md).

## Budgets

A prompt's `cost` dimension flows into the enclosing agent's budget.
A prompt that runs ten times in a loop counts ten times.

## Replay

Every prompt invocation is recorded with input, output, model,
latency, and cost. `corvid replay` reproduces the run from the
recording without hitting the provider. `corvid eval --swap-model`
re-runs the prompt against a different model and diffs the result
against the baseline.
