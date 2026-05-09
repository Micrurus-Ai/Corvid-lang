# Observability

## What `std.observability` gives you

- OpenTelemetry SDK conformant span emission for every prompt, tool,
  agent, approve, grounded unwrap, and budget event.
- A lineage graph for every agent run.
- Structured logs with automatic redaction of secrets and tokens.
- Operator runbooks for common incident types.

## Span emission

Every Corvid construct emits spans automatically:

- `prompt.<name>` — input, output, model, latency, cost.
- `tool.<name>` — input args, return, host call latency.
- `agent.<name>` — composed effect row, total cost, total wall time.
- `approve.<action>` — when fired, what arguments.
- `grounded.unwrap.<which>` — `with_citation` / `value` / `discarding_sources`.
- `budget.<event>` — `consumed`, `exceeded`, `reset`.

Configure the OTel exporter in `corvid.toml`:

```toml
[observability]
otlp_endpoint = "http://otel-collector:4317"
service_name = "my-app"
service_version = "0.1.0"
sample_rate = 1.0

[observability.redaction]
patterns = ["api[_-]?key", "secret", "token", "bearer"]
```

## Lineage graphs

```sh
corvid trace dag <trace-id>
```

Emits a graph showing:
- Which prompt called which tool.
- Which grounded value flowed into which prompt.
- Which approve token authorized which call.
- Which budget event constrained which step.

The graph is the input to `corvid jobs explain` (AI-assisted
root-cause).

## Structured logs

```corvid
import "@stdlib/observability" as obs

agent handle(req: Request) -> Response uses obs_effect:
    obs.log.info("request received", { path: req.path, customer_id: req.customer_id })
    let result = process(req)
    obs.log.info("request processed", { result_kind: result.kind() })
    return result
```

Log lines are JSON. Token-shaped values are redacted automatically.
The redaction pattern list is in the config.

## Operator runbooks

The Phase 40 operator runbook ships canonical playbooks:
- "Why is this agent over budget?"
- "Why is this approval pending past SLO?"
- "Why is this DLQ growing?"
- "Why did this model swap break X?"

Each runbook is a `.md` file in
[`docs/operations/operator-runbooks.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/operations/operator-runbooks.md)
with concrete CLI commands and diff-against-baseline patterns.
