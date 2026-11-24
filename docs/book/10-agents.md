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

## Your first agent in the reading list

Open shelf 2 and choose **Suggest a title**. The task is to recommend a different book for that shelf. To run the exercise with repeatable answers, follow the [packaged reading-list trial](19-browser-views.md#try-a-typed-suggestion).

The reading-list authoring prototype takes a shelf and returns a suggestion. This excerpt uses the `Shelf`, `BookSuggestion`, `recommend` and `lookup_shelf` declarations from the example application linked below.

```corvid-fragment
agent suggest_title(shelf: Shelf) -> BookSuggestion:
    title = ask recommend(shelf.title)
        allow lookup_shelf once
        within 30 seconds

    return BookSuggestion(shelf, title)
```

The model may suggest a title immediately or read one title from the shelf snapshot. A missing shelf, a second lookup, or a blank suggestion ends the operation with an error. Both model calls and the optional lookup share the deadline. The app shows the proposed title in an editable preview. The shelf changes when the reader chooses **Apply suggestion**.

The compiler accepts this form directly. In `examples/book_suggestions`, run `corvid check` and `corvid build src/main.cor --target web`. The named prompt takes two strings (the input and lookup context) and returns a record whose `action` is `Suggest(String)` or `Lookup(Int)`. The allowed lookup accepts an integer and returns `Result<String, String>`. The enclosing agent's concise return type names the success value. Its callers receive a `Result` and can handle lookup failures. With an explicit `Result` signature, write ordinary `Ok` and `Err` returns. [Native agent and form route](../../examples/book_suggestions/src/main.cor)

Edit the prototype and run `python scripts/prototype_reading_list_ask.py --write` from the checkout root. Run it without `--write` to check that the generated application matches. The generated agent exposes errors through `Result<BookSuggestion, String>`. The preview also keeps the lookup context for replay. This older generator preserves the traced reading-list fixture. [Prototype](../../examples/reading_list/suggest_title.cor.in) and [runnable application](../../examples/reading_list/src/main.cor)

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

## Durability

Agents tagged `@replayable` and run via the durable job runner survive
process restart. Their step checkpoints, tool-call results, and
approval-wait state persist. See **[Jobs](/docs/jobs)** for the
runner story.

## Examples in the wild

The reference apps under `examples/backend/` ship canonical agent
shapes. Read them.

## Returning a stream

An agent can produce elements with `yield` or forward an existing stream with `return`. A caller iterates over either stream in the same way.

```corvid
agent numbers() -> Stream<Int>:
    yield 2
    yield 5

agent forwarded() -> Stream<Int>:
    return numbers()
```
