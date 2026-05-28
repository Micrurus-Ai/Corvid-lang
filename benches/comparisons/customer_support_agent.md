# Customer Support Agent — per-app side-by-side

## Headline

For a support agent that triages tickets, drafts policy-grounded
replies, tracks SLAs, and gates five operations (send-support-reply,
issue-support-refund, escalate-ticket, close-ticket,
apply-account-credit) behind typed approvals — where every reply must
cite policy — Corvid encodes the grounding requirement and the approval
flow in the language, enforced at compile time. The FastAPI + LangChain
and Next.js baselines reconstruct the same RAG-grounded reply +
approval + audit discipline as runtime conventions.

## Reproduce

Corvid governance surface lives in
[`examples/backend/customer_support_agent/src/main.cor`](../../examples/backend/customer_support_agent/src/main.cor).

```bash
m=examples/backend/customer_support_agent/src/main.cor
grep -c 'trust: human_required' $m   # external-write effects
grep -c 'dangerous uses' $m          # dangerous tool declarations
grep -c '^    approve ' $m           # compiler-enforced approve gates
grep -c 'agent permission_for_' $m   # typed permission per dangerous tool
```

`cargo test -p corvid-cli --test serve_smoke` proves the source serves
its `/schema` route.

## Side-by-side (sketch)

### Corvid

```corvid
effect support_write:
    cost: $0.01
    trust: human_required
    data: customer

tool send_support_reply(req: SupportReplySendRequest)
    -> SupportReplySendReceipt dangerous uses support_write

agent draft_policy_grounded_reply_mock() -> SupportDraftReply
    uses ticket_read, policy_search, support_ai:
    triage = triage_ticket_mock()
    return SupportDraftReply(triage.ticket.id, "sha256:support-draft",
        triage.citation, "drafted", "SendSupportReply", triage.grounded)

agent execute_approved_support_reply(req: SupportReplySendRequest)
    -> SupportReplySendReceipt uses support_write:
    approve SendSupportReply(req)
    return send_support_reply(req)
```

The compiler rejects any reachable send/refund/escalate/close/credit
lacking its `approve` (`E0101`, pinned by the 5 `ungated_*.cor`
fixtures). The policy-grounding posture is structural: a
`SupportDraftReply` carries a `PolicyCitation`, and the triage/draft
contract fails if the citation has no provenance (`ungrounded_reply.json`
names the threat). The 3 cron jobs (SLA scan, CSAT rollup, policy
reindex) carry only read effects.

### Python (FastAPI + LangChain + custom approval) — bounty-open

FastAPI routes; LangChain RAG over the policy corpus; Zendesk/Intercom
SDKs; a Pydantic approval model; SQLAlchemy approval + audit tables;
Celery for SLA scans. "Every reply cites policy" + "no reply/refund
without approval" are runtime conventions. Submission lands under
`runs/python/customer_support_agent/`.

### TypeScript (Next.js + Vercel AI SDK + zod) — bounty-open

Next.js handlers; Vercel AI SDK over the policy corpus; zod approval
contracts; the grounding + approval checks are TypeScript code.
Submission lands under `runs/typescript/customer_support_agent/`.

## Governance line count

| Implementation | Governance surface | Governance lines | Notes |
|---|---|---|---|
| Corvid | 5 human_required effects, 5 dangerous tools, 5 approve gates, 5 approval contracts (Admin for refund/credit; Reviewer for reply/escalate/close), 5 typed permissions | ~88 | grounding + role gradient are language-level |
| Python (FastAPI + LangChain + Celery) | same intent across Pydantic + middleware + RAG | bounty-open | grounding + reachability are runtime conventions |
| TypeScript (Next.js + Vercel AI SDK) | same intent across zod + handlers | bounty-open | grounding + reachability are runtime conventions |

## What Corvid wins on

- **Reachability at typecheck.** No customer-facing write executes
  without its `approve` boundary (`E0101`).
- **Policy grounding is structural.** A draft carries its
  `PolicyCitation`; an ungrounded draft fails its contract rather than
  shipping — the baselines re-check at runtime if at all.
- **Role gradient is typed**: Admin for money (refund/credit), Reviewer
  for customer-facing/reversible ops.
- **One source → HTTP service + worker + eval corpus.**

## What Corvid does not claim

- **Ticketing/RAG connector breadth** is not shipped day one.
- **Raw reply-generation latency** is not the moat (model time is the
  baseline either way).
- **The `bounty-open` cells are not yet measured.**
