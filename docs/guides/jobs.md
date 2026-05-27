# Jobs and schedules

Durable background work in Corvid is built on top of two shipped
surfaces:

- **`agent`** — the language-level declaration for any callable that
  composes effects and returns a typed value. The same `agent` you
  write for an interactive call is what runs as a background job; the
  durability + scheduling happens at the queue layer, not the source
  layer.
- **`schedule`** — the language-level cron trigger that points at an
  agent call.

There is no separate `job` keyword. Treating background work as
`agent` + `schedule` keeps one safety story (effects, approvals,
budgets, replay) instead of two.

## What `std.jobs` gives you

A durable job runner with:

- Multi-worker async pool (`corvid jobs run --source <path>.cor --workers=N`).
- Lease-based exclusivity (no two workers run the same job).
- Idempotency keys (no double-side-effect under concurrent retry).
- Retry / backoff / dead-letter queue (configured per enqueue, not as
  source attributes — see "Configuring retries" below).
- Cron schedules with DST-aware timezone handling.
- Step checkpoints for durable agent runs (resume after crash).
- Approval-wait state via `corvid jobs wait-approval` (pause, await
  operator approval, resume).

## Defining the job body

```corvid
effect email_effect:
    cost: $0.05

effect summary_effect:
    confidence: 0.85

tool gmail_recent(user_id: String, since: String) -> String uses email_effect
tool gmail_send(user_id: String, message: String) -> Nothing dangerous uses email_effect
prompt summarise(text: String) -> String uses summary_effect:
    "Summarise: {text}"

@replayable
@budget($0.20)
agent daily_brief(user_id: String) -> String uses email_effect, summary_effect:
    inbox = gmail_recent(user_id, "yesterday")
    summary = summarise(inbox)
    approve GmailSend(user_id, summary)
    gmail_send(user_id, summary)
    return summary
```

`@replayable` lets `corvid replay` reproduce the run deterministically;
`@budget` is the compile-time cost ceiling. Both are stable AgentAttribute
surfaces shipped since Phase 21.

## Scheduling

```corvid skip
# Snippet (visibly opted out of the docs-as-code drift gate
# because it relies on names declared in the previous block).
schedule "0 8 * * *" zone "America/New_York" -> daily_brief("user_123") uses email_effect, summary_effect
```

The cron expression supports the standard 5-field shape. The `zone`
clause is required for any schedule that should respect a specific
timezone (DST-aware via `chrono-tz`). The effect row at the schedule
declaration must match what the called agent uses — the resolver
checks this.

A self-contained version that compiles cleanly:

```corvid
effect email_effect:
    cost: $0.05

effect summary_effect:
    confidence: 0.85

tool gmail_recent(user_id: String, since: String) -> String uses email_effect
tool gmail_send(user_id: String, message: String) -> Nothing dangerous uses email_effect
prompt summarise(text: String) -> String uses summary_effect:
    "Summarise: {text}"

@replayable
@budget($0.20)
agent daily_brief(user_id: String) -> String uses email_effect, summary_effect:
    inbox = gmail_recent(user_id, "yesterday")
    summary = summarise(inbox)
    approve GmailSend(user_id, summary)
    gmail_send(user_id, summary)
    return summary

schedule "0 8 * * *" zone "America/New_York" -> daily_brief("user_123") uses email_effect, summary_effect
```

## Running the runner

```sh
corvid jobs run --source app.cor --queue=default --workers=4
```

The runner polls the queue, leases jobs, compiles the supplied source
to resolve agent bodies, executes them with the configured concurrency,
and handles retries and DLQ. `--source` is required: a production
`corvid jobs run` without compiled source would mark jobs `succeeded`
without doing any work — a silent durable-state lie. For test-mode job
lifecycle without executing agent bodies, use `corvid jobs run-one`.

## Configuring retries, idempotency, and concurrency

Retry policy, idempotency keys, and concurrency limits are runtime
configuration, not source-level attributes. They attach to a job at
enqueue time (via the host API) or at the queue level (via `corvid
jobs limit`).

```sh
# Set queue-wide concurrency.
corvid jobs limit set --queue=default --max-concurrent=4

# Inspect current limits.
corvid jobs limit list
```

The idempotency key is per-enqueue: when two enqueue calls share a
non-null key, exactly one row exists in the queue (enforced by a
partial UNIQUE INDEX at the SQL layer; see "Idempotency under
concurrency" below). Retry budget is per-enqueue too — the runner
escalates to the dead-letter queue when the budget is exhausted.

> Source-level `@retry(...)`, `@idempotency(...)`, and `await_approval`
> keyword surfaces are post-v1.0 ergonomic improvements; the runtime
> behaviour they would surface is already shipped through the CLI +
> host API.

## Operations

```sh
corvid jobs schedule list             # list durable cron schedules + last fire
corvid jobs inspect <id>              # one job + its operational metadata
corvid jobs retry <id>                # requeue a terminal or delayed job
corvid jobs cancel <id>               # cancel a job
corvid jobs export-trace <id>         # redacted JSON trace for one job
corvid jobs pause --queue=default     # pause leasing new work
corvid jobs resume --queue=default    # resume leasing work
corvid jobs drain --workers=all       # pause + release active leases
corvid jobs dlq list                  # inspect terminally failed jobs
corvid jobs checkpoint list <id>      # durable step checkpoints for a job
corvid jobs loop status               # bounded agent-loop usage per job
```

`corvid jobs explain <id>` (AI-assisted root-cause from typed trace)
and `corvid jobs dlq triage` (AI-assisted DLQ pattern clustering)
are launch-readiness items — they land alongside the v1.0 launch
artifacts. The trace + DLQ surfaces both ship today; only the
AI-helper layer is the deferred bit.

## Approval-wait

For agents that include an `approve` step, the runner pauses the
job at the approve boundary and routes it to the approval queue.
Use `corvid jobs wait-approval` to inspect the pause, and `corvid
jobs approval approve <id>` / `corvid jobs approval deny <id>` to
resolve.

```sh
corvid jobs wait-approval                   # lease + pause next approval-bound job
corvid jobs approvals                       # list jobs paused on approval
corvid jobs approval approve <id>           # approve a paused job; runner resumes it
corvid jobs approval deny <id>              # deny + transition to terminal state
```

The approval lives in the same `approve` flow the language already
ships for interactive agents — there is one approval story, not two.
Expired approvals (timeout configured at enqueue) transition the job
to a terminal state.

## Idempotency under concurrency

The 4-concurrent-worker idempotency test
(`crates/corvid-runtime/tests/durability_corpus.rs::t38l_d1_four_workers_collapse_to_one_row`)
is the canonical contract: 100 jobs sharing one idempotency key
processed by 4 workers simultaneously result in exactly one running
(enforced at the SQL layer by a partial UNIQUE INDEX on the queue
table). Build your jobs assuming this guarantee.

## Crash recovery

```sh
# kill -9 the worker process mid-step
# restart `corvid jobs run --source app.cor`
# the job's lease expires, the next worker takes the lease, the run
# resumes from the last step checkpoint
```

The crash-recovery integration test
(`crates/corvid-runtime/tests/durability_corpus.rs::t38l_d3_checkpoints_survive_unclean_shutdown`)
asserts the property using a runtime-drop surrogate for SIGKILL;
the literal subprocess-SIGKILL test is a post-v1.0 hardening item.

## DST cron correctness

`chrono-tz` is the timezone backend. Spring-forward at `02:30
America/New_York`: a cron that fires at `02:30` on the spring-forward
day fires according to the documented `fire_once_on_recovery` policy.
Fall-back at `01:30`: fires exactly once, not twice. Both properties
are tested in
`crates/corvid-runtime/tests/durability_corpus.rs::t38l_d2_dst_*`.

## Replay quarantine

A job that ran in production produced a typed trace. `corvid jobs replay
--source <path>.cor --job <job_id>` reproduces the run from the trace
through the same queue runtime that recorded it. During the replay the
runtime quarantines every side-effect surface — LLM adapter calls
refuse with `QuarantineViolation { surface: "llm", .. }`, outbound HTTP
refuses with `"http"`, application store writes refuse with `"store"`,
and file writes refuse with `"io"`. Recorded calls substitute from the
trace; unrecorded ones fail closed. The durable queue uses raw SQLite
and the trace writer uses its own writer, so queue-internal bookkeeping
and trace recording are unaffected. See guarantee
`jobs.replayable_side_effects` (RuntimeChecked, shipped in
audit-correction track `35V2-P38-C-replay-quarantine`).
