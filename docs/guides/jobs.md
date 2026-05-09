# Jobs and schedules

## What `std.jobs` gives you

A durable job runner with:

- Multi-worker async pool (`corvid jobs run --workers=N`).
- Lease-based exclusivity (no two workers run the same job).
- Idempotency keys (no double-side-effect under concurrent retry).
- Retry/backoff/dead-letter queue.
- Cron schedules with DST-aware timezone handling.
- Step checkpoints for durable agent runs (resume after crash).
- Approval-wait state (pause, await operator approval, resume).

## Defining a job

```corvid
@budget($0.20)
@retry(max_attempts: 3, backoff: exponential(base: 30s, cap: 5m))
@idempotency(key: brief.user_id)
@replayable
@max_steps(20)
job daily_brief(user_id: String) uses email_effect, summary_effect:
    let inbox = gmail.recent(user_id, since: yesterday())
    let summary = summarise(inbox)
    approve SendBrief(user_id, summary)
    gmail.send(user_id, summary)
```

## Scheduling

```corvid
schedule "0 8 * * *" zone "America/New_York" -> daily_brief(every_user())
```

The cron expression supports the standard 5-field shape. The `zone`
clause is required for any schedule that should respect a specific
timezone (DST-aware).

## Running the runner

```sh
corvid jobs run --queue=default --workers=4
```

The runner polls the queue, leases jobs, executes them with the
configured concurrency, and handles retries and DLQ.

## Operations

```sh
corvid jobs schedule list
corvid jobs inspect <id>
corvid jobs explain <id>             # AI-assisted root-cause from typed trace
corvid jobs dlq triage               # AI-assisted DLQ pattern clustering
corvid jobs retry <id>
corvid jobs cancel <id>
corvid jobs export-trace <id>
corvid jobs pause --queue=default
corvid jobs drain --workers=all
```

## Approval-wait

```corvid
job send_marketing(campaign_id: String) uses email_effect:
    let preview = generate_preview(campaign_id)
    await_approval CampaignSend(campaign_id, preview_hash: preview.hash)
    approve EmailBlast(campaign_id)
    email_blast(campaign_id)
```

`await_approval` pauses the job, persists state, and resumes when an
operator approves through `corvid approvals approve <id>`. The
approval timeout is configurable; expired approvals transition the
job to a terminal state.

## Idempotency under concurrency

The 4-concurrent-worker idempotency test (Phase 38L) is the
canonical contract: 100 jobs sharing one idempotency key processed
by 4 workers simultaneously result in exactly one ran (enforced at
the SQL layer by a partial UNIQUE INDEX). Build your jobs assuming
this guarantee.

## Crash recovery

```sh
# kill -9 the worker process mid-step
# restart `corvid jobs run`
# the job's lease expires, the next worker takes the lease, the run
# resumes from the last step checkpoint
```

The crash-recovery integration test (Phase 38L) asserts byte-exact
resume with the LLM call counter unchanged.

## DST cron correctness

`chrono-tz` is the timezone backend. Spring-forward at `02:30
America/New_York`: a cron that fires at `02:30` on the spring-forward
day fires according to the documented `fire_once_on_recovery` policy.
Fall-back at `01:30`: fires exactly once, not twice.
