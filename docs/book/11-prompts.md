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

Role blocks give a prompt a real message structure — `system:`
sets behavior, `user:` (and `assistant:`, for few-shot pairs)
carry the conversation. Each block's template interpolates
`{param}` exactly like the single-template form, and each provider
receives its native shape (Anthropic: top-level `system` +
messages array; OpenAI: role-tagged array; Gemini:
`systemInstruction` + contents). At least one non-`system` message
is required. First-class conversation history follows in slice
46c. (Compiled in CI:)

```corvid
effect llm_call:
    cost: $0.01
    reversible: true

prompt ask(question: String) -> String uses llm_call:
    system: "You are a careful, terse assistant."
    user: "{question}"

agent main(q: String) -> String:
    return ask(q)
```

## Conversation history

A parameter typed `List<AiMessage>` IS the history surface — no new
syntax. Its messages splice between the declaration's `system:`
blocks and the current turn, in list order (compiled in CI):

```corvid
effect llm_call:
    cost: $0.01
    reversible: true

type AiMessage:
    role: String
    content: String

prompt chat(history: List<AiMessage>, question: String) -> String uses llm_call:
    system: "You are a careful assistant."
    user: "{question}"

agent converse() -> String:
    turns = [
        AiMessage("user", "What is Corvid?"),
        AiMessage("assistant", "An AI-native language."),
    ]
    return chat(turns, "Who makes it?")
```

Rules: one history parameter per prompt; `{history}` in a template
is a compile error (history splices structurally — interpolating it
as JSON is almost always a bug); roles are validated at dispatch
(`system` / `user` / `assistant`). The history list rides the trace
as an ordinary argument, so replay matching is exact.

**Context windows.** Declare `context_window: N` on a model and the
runtime drops history messages OLDEST-FIRST — whole messages, never
split, never the system blocks or the current turn — until the
request fits `N` minus the completion reserve (`max_tokens` or its
default estimate). Truncation is a pure function of its inputs, so
a replayed run truncates identically. If the request still doesn't
fit with all history dropped, the call fails with a typed error
instead of a silent provider rejection. Full design:
`docs/meta/46c-conversation-history-design.md`.

## Sampling parameters

`temperature`, `top_p`, and `max_tokens` live in two places, with a
clear precedence (compiled in CI):

```corvid
model precise:
    capability: expert
    temperature: 0.1
    top_p: 0.9
    max_tokens: 512

prompt classify(text: String) -> String:
    route:
        true -> precise
    with temperature 0.7
    "Classify {text}"
```

The model declaration sets the model's defaults; a per-prompt
`with temperature 0.7` overrides them for that prompt only.
Anything left unset falls through to the provider's default. The
resolved values ride the request to all four provider adapters and
are recorded in the trace's `llm_call` event, so a replayed run
documents exactly which sampling produced the recorded response.
Ranges are compile-checked: temperature 0..=2, top_p 0..=1,
max_tokens a positive integer.

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
