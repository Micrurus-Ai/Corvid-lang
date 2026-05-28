# Finance Operations Agent — per-app side-by-side

## Headline

For a finance operations agent that aggregates read-only financial data,
surfaces operational summaries, and gates five money/data operations
(submit-payment-intent, cancel-subscription, dispute-transaction,
export-financial-report, schedule-recurring-payment) behind typed
approvals — while never giving regulated advice — Corvid encodes both
the approval flow and the non-advice posture in the shape of the
surface, enforced at compile time. The FastAPI + LangChain and Next.js
baselines re-assert the same approval + audit + "no advice" discipline
as runtime conventions across separate libraries.

## Reproduce

Corvid governance surface lives in
[`examples/backend/finance_operations_agent/src/main.cor`](../../examples/backend/finance_operations_agent/src/main.cor).

```bash
m=examples/backend/finance_operations_agent/src/main.cor
grep -c 'trust: human_required' $m   # external-write effects
grep -c 'dangerous uses' $m          # dangerous tool declarations
grep -c '^    approve ' $m           # compiler-enforced approve gates
grep -c 'agent permission_for_' $m   # typed permission per dangerous tool
```

`cargo test -p corvid-cli --test serve_smoke` proves the source serves
`/schema` (which reports `non_advice: true`).

## Side-by-side (sketch)

### Corvid

```corvid
effect payment_write:
    cost: $0.02
    trust: human_required
    data: financial

tool submit_payment_intent(req: PaymentIntentRequest)
    -> PaymentIntentReceipt dangerous uses payment_write

agent execute_approved_payment_intent(req: PaymentIntentRequest)
    -> PaymentIntentReceipt uses payment_write:
    approve SubmitPaymentIntent(req)
    return submit_payment_intent(req)
```

The compiler rejects any reachable path to a financial write lacking its
`approve` (`E0101`, pinned by the 4 `ungated_*.cor` fixtures +
`autonomous_payment.json`). The non-advice posture is structural: there
is no advisory tool to call, and the 3 cron jobs (balance sync, anomaly
scan, renewal check) carry only read effects, so the type system
guarantees no scheduled path moves money or recommends an action.

### Python (FastAPI + LangChain + custom approval) — bounty-open

FastAPI routes; Stripe/Plaid SDKs for balances + payments; a Pydantic
approval model with role + ceiling + irreversibility; SQLAlchemy
approval + audit tables; Celery for the scans. "No advisory output" and
"no autonomous payment" are review-time disciplines, not type facts.
Submission lands under `runs/python/finance_operations_agent/`.

### TypeScript (Next.js + Vercel AI SDK + zod) — bounty-open

Next.js handlers; stripe-node + Plaid clients; zod approval contracts;
the non-advice + segregation-of-duties checks are TypeScript code.
Submission lands under `runs/typescript/finance_operations_agent/`.

## Governance line count

| Implementation | Governance surface | Governance lines | Notes |
|---|---|---|---|
| Corvid | 5 human_required effects, 5 dangerous tools, 5 approve gates, 5 approval contracts (Admin/irreversible for money/data egress; Reviewer/reversible for cancel/dispute), 5 typed permissions | ~88 | role/irreversibility gradient is language-level |
| Python (FastAPI + LangChain + Celery) | same intent across Pydantic + middleware | bounty-open | non-advice + reachability are runtime conventions |
| TypeScript (Next.js + Vercel AI SDK) | same intent across zod + handlers | bounty-open | non-advice + reachability are runtime conventions |

## What Corvid wins on

- **Reachability at typecheck.** No financial write executes without its
  `approve` boundary (`E0101`).
- **Non-advice is structural.** No advisory surface exists; the cron
  jobs cannot move money (read-only effects). The compiler + trace log
  prove it — it is not a disclaimer.
- **Role/irreversibility gradient is typed**: Admin + irreversible for
  payment/export/recurring; Reviewer + reversible for cancel/dispute.
- **One source → HTTP service + worker + eval corpus.**

## What Corvid does not claim

- **Provider breadth** (every bank aggregator / payment rail) is not
  shipped day one.
- **Raw throughput** is not the moat.
- **The `bounty-open` cells are not yet measured.**
