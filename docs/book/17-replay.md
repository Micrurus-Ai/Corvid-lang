# Replay

## What replay does

Use a recording to rerun application code against a known sequence of model and tool responses. For example, a recorded function-calling prompt can be replayed without setting the original provider's API key.

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

An unpinned prompt uses the model from the recorded exchange. If the application sends different arguments, replay reports a mismatch.

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
