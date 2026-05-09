# Prompts

## What a prompt is

A prompt is a function whose body is a string template, whose return
type is a typed value (or an `Option<T>` / `Result<T,E>` if the parse
might fail), and whose effect row carries the LLM call's cost,
latency, and confidence.

```corvid
prompt summarize(text: String) -> String uses llm_call:
    "Summarize the following in one sentence: " + text
```

## How interpolation works

Variables in the prompt body interpolate. Strings concatenate with `+`.
There is no implicit string conversion — non-string values must be
converted explicitly:

```corvid
prompt classify_priority(score: Int) -> String uses llm_call:
    "Score is " + score.to_string() + ". What priority?"
```

## Typed return values

```corvid
struct Decision:
    refund: Bool
    reason: String

prompt decide(ticket: String) -> Decision uses llm_call:
    "Given ticket: " + ticket + "\n\nReply as JSON {refund, reason}."
```

The runtime asks the model to emit JSON matching the `Decision` schema
and parses the response into a `Decision` value. Parse failure is a
typed error.

For per-struct decoders the compiler emits at codegen time, see the
slice that landed this surface
([Phase 20n-C](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/phases/phase-20n-open-gap-implementation.md)).

## Multi-message prompts

```corvid
prompt ask(question: String) -> String uses llm_call:
    system: "You are a careful, terse assistant."
    user: question
```

Each role gets its own template. The runtime renders the messages,
sends them, parses the response.

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
