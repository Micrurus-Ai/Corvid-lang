# Replay

## What replay does

Every Corvid agent run records a deterministic trace. `corvid replay`
re-executes that trace without hitting the LLM provider or the
side-effect tools — it serves cached prompt responses, intercepts
tool calls, and produces byte-identical output.

This is what gives you "what changed?" answerable in seconds when a
model upgrade lands, a tool API changes, or a customer reports a bug.

## Recording

The interpreter writes JSONL traces under `target/trace/`. For a scaffolded project this directory is at the project root.

```sh
corvid run --target interp src/main.cor "input"
corvid trace list                    # show recent traces
```

## Playing

```sh
corvid replay target/trace/<trace-id>.jsonl --source src/main.cor
```

Output is byte-identical to the original run, with a faster wall-clock
because no network calls happen.

## Inspecting a recording

```sh
corvid trace show target/trace/<trace-id>.jsonl
corvid trace dag target/trace/<trace-id>.jsonl
```

Use `show` to inspect the events in order. Use `dag` to print a Graphviz graph of recorded provenance edges.

## Reusing a recording in a test

Copy the JSONL file beside the test source and declare a test with `from_trace "recording.jsonl"`. The test can call the application and assert on its return value. It can also check events with assertions such as `assert called classify`. Each statistical repetition starts at the beginning of the recording. See [Testing](15-testing.md) for a complete example.

## Comparing models

To send the recorded prompts to a configured candidate model and inspect changes across a directory of recordings, use the model migration command.

```sh
corvid eval --swap-model candidate-model --source src/main.cor target/trace
```

For a single recording, use `corvid replay target/trace/<trace-id>.jsonl --source src/main.cor --model candidate-model`. These comparisons require access to the candidate model. Plain replay uses the recorded responses.
