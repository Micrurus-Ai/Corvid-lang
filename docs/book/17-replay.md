# Replay

## What replay does

Every Corvid agent run records a deterministic trace. `corvid replay`
re-executes that trace without hitting the LLM provider or the
side-effect tools — it serves cached prompt responses, intercepts
tool calls, and produces byte-identical output.

This is what gives you "what changed?" answerable in seconds when a
model upgrade lands, a tool API changes, or a customer reports a bug.

## Recording

Every `corvid run` records by default. Traces persist under
`.corvid/replay/` (configurable in `corvid.toml`).

```sh
corvid run src/main.cor "input"
corvid trace list                    # show recent traces
```

## Playing

```sh
corvid replay <trace-id>
```

Output is byte-identical to the original run, with a faster wall-clock
because no network calls happen.

## Inspecting

```sh
corvid trace dag <trace-id>          # provenance DAG
corvid trace open <trace-id>         # opens in the trace viewer
```

The trace DAG shows every prompt input/output, every tool call, every
approve, every grounded unwrap, and the budget consumed at each step.

## Eval and model swap

```sh
corvid eval --swap-model gpt-5 \
            --source src/main.cor \
            target/refund_traces
```

Re-runs the saved traces against gpt-5 instead of the recorded model
and produces a diff: which prompts changed answer, which budgets were
exceeded, which tool sequences differed.

```sh
corvid eval --swap-model gpt-5 --report=json > swap_report.json
```

Useful in CI: a model upgrade is a diff to review, not an outage to
debug.

## Replay quarantine

Connectors carry replay-quarantine tags. Replaying a trace that
contains a connector call does not re-execute the connector — the
recorded response is served. The quarantine fires for every connector
type; the test corpus exercises each.

If you genuinely want the connector to be hit during replay (e.g., for
load testing), pass `--live`:

```sh
corvid replay <trace-id> --live
```

The audit log records that the replay went live.

## Determinism guarantees

A trace replays byte-identically given:

- The same source code (same hash).
- The same deps (same `corvid.lock`).
- The same recorded model responses.
- The same recorded host call responses.

If any input differs (source change, dep upgrade, tool API change),
the replay diverges and the diff is the answer to "what changed?".

## Trace storage

```toml
[replay]
storage = ".corvid/replay"           # local
retention_days = 30                  # auto-prune older
sign = true                          # ed25519-sign each trace
```

For long-term retention, configure remote storage (S3-compatible)
under `[replay.remote]`. The traces are signed; tampering is detected
on replay.
