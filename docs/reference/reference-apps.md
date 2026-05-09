# Reference apps

## What ships

The `examples/` directory contains canonical Corvid programs that
exercise every shipped invention end-to-end:

- `refund_bot.cor` — approval-gated refund agent with budgets.
- `rag_qa_bot.cor` — grounded retrieval Q&A.
- `support_escalation_bot.cor` — multi-step support agent with loop
  bounds and human approval.
- `code_review_agent.cor` — code-review agent with effect rows on
  filesystem and version-control reads.
- `provider_routing_demo.cor` — typed model substrate routing.
- `local_model_demo.cor` — local model via the model substrate.

## How to run

```sh
git clone https://github.com/Micrurus-Ai/Corvid-lang
cd Corvid-lang/examples
corvid run refund_bot.cor
```

Each example has tests, evals, traces, and benchmark notes.

## What each one teaches

| Example | Inventions exercised |
|---|---|
| refund_bot | approve, budget, audit log |
| rag_qa_bot | Grounded<T>, citation, retrieval effect |
| support_escalation_bot | loop bounds, await_approval, durable jobs |
| code_review_agent | filesystem effect, multi-prompt composition |
| provider_routing_demo | model substrate, swap_model eval |
| local_model_demo | local model substrate, no-network compile |

## The benchmark archives

Phase 33N ships side-by-side benchmark runners against Python and
TypeScript implementations of `refund_bot`, `rag_qa_bot`, and
`support_escalation_bot`. Results live in
[`benches/moat/RESULTS.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/benches/moat/RESULTS.md).
