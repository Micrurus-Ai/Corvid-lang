# Personal Executive Agent — per-app side-by-side

## Headline

For an executive assistant that triages inbox threads, drafts replies,
schedules and preps meetings, extracts tasks, and gates every external
write (send-follow-up-email, edit-calendar-event, edit-task-item,
send-chat-message, external-calendar-share) behind a typed approval,
Corvid declares the governance in the language and rejects any ungated
send/edit at typecheck. The FastAPI + LangChain and Next.js +
Vercel-AI-SDK baselines wire the same Gmail/Calendar/Slack orchestration
plus approval + audit across separate libraries with hand-written glue.

## Reproduce

Corvid governance surface lives in
[`examples/backend/personal_executive_agent/src/main.cor`](../../examples/backend/personal_executive_agent/src/main.cor).
Countable by inspection:

```bash
m=examples/backend/personal_executive_agent/src/main.cor
grep -c 'trust: human_required' $m   # external-write effects
grep -c 'dangerous uses' $m          # dangerous tool declarations
grep -c '^    approve ' $m           # compiler-enforced approve gates
grep -c 'agent permission_for_' $m   # typed permission per dangerous tool
```

`cargo test -p corvid-cli --test serve_smoke` proves the source runs as
an HTTP service. Durable-job + observability latency reference
[`jobs_durability.md`](./jobs_durability.md) and
[`observability.md`](./observability.md).

## Side-by-side (sketch)

### Corvid

```corvid
effect calendar_share:
    cost: $0.02
    trust: human_required
    data: external

tool external_calendar_share(req: CalendarShareRequest)
    -> CalendarShareReceipt dangerous uses calendar_share

agent execute_approved_calendar_share(req: CalendarShareRequest)
    -> CalendarShareReceipt uses calendar_share:
    approve ExternalCalendarShare(req)
    return external_calendar_share(req)
```

The compiler rejects any reachable path to `external_calendar_share`
(or `send_follow_up_email`, `edit_calendar_event`, `edit_task_item`,
`send_chat_message`) lacking its `approve` boundary — `E0101`, pinned
by `adversarial/ungated_send.cor` and `ungated_share.cor`. The four
durable jobs (daily brief, meeting prep, inbox triage, follow-up) are
`@replayable` and budget-bounded; none of them sends or edits anything
— external writes happen only on the approval-gated routes.

### Python (FastAPI + LangChain + custom approval) — bounty-open

FastAPI routes; LangChain for triage/draft; google-api-python-client +
slack-sdk for the connectors; a Pydantic approval model + policy; an
explicit audit middleware; Celery for the four scheduled jobs. The
"no scheduled job can send/edit without approval" invariant is a
runtime convention. Submission lands under
`runs/python/personal_executive_agent/`.

### TypeScript (Next.js + Vercel AI SDK + zod) — bounty-open

Next.js handlers; Vercel AI SDK for draft generation; googleapis +
@slack/web-api connectors; zod approval contracts; BullMQ for the
jobs; the reachability/approval check is TypeScript code. Submission
lands under `runs/typescript/personal_executive_agent/`.

## Governance line count

| Implementation | Governance surface | Governance lines | Notes |
|---|---|---|---|
| Corvid | 4 human_required effects, 5 dangerous tools, 5 approve gates, 5 approval contracts, 5 typed permissions, `approval_surface_valid` | ~84 | language-level + compiler-checked |
| Python (FastAPI + LangChain + Celery) | same intent across Pydantic + middleware + Celery | bounty-open | reachability is a runtime convention |
| TypeScript (Next.js + Vercel AI SDK + BullMQ) | same intent across zod + handlers + BullMQ | bounty-open | reachability is a runtime convention |

## What Corvid wins on

- **Reachability at typecheck.** Any of the 5 sends/edits reached
  without an `approve` boundary fails to compile (`E0101`).
- **Scheduled jobs cannot act.** The 4 cron jobs carry only
  read/observe effects; the type system guarantees a job cannot reach a
  `dangerous` write — no "auto-send" foot-gun.
- **Approval contracts are typed records** (role, ceiling, data class,
  irreversibility, expiry), not runtime conventions.
- **One source → HTTP service + worker + eval corpus.**

## What Corvid does not claim

- **Connector breadth** (every Gmail/Calendar/Slack capability) is not
  shipped day one.
- **Raw request throughput** is not the moat.
- **The `bounty-open` cells are not yet measured.**
