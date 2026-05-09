# Recipes

## RAG pattern

A retrieval-grounded Q&A agent:

```corvid
import "@stdlib/retrieval" as rag

effect retrieval_effect:
    cost: $0.001
    latency: fast
    confidence: 0.95
    data: grounded

effect llm_decision:
    cost: $0.02
    latency: medium

prompt fetch_passage(question: String) -> Grounded<String> uses retrieval_effect:
    @retrieve_top_k(question, k: 3)

prompt answer(question: String, context: Grounded<String>) -> String uses llm_decision:
    "Question: " + question +
    "\n\nContext: " + context.unwrap_with_citation() +
    "\n\nAnswer concisely with [citation]."

@budget($0.05)
agent rag_qa(question: String) -> String:
    let context = fetch_passage(question)
    return answer(question, context)
```

Properties:
- The answer always includes a citation (the `Grounded<T>` flow forces it).
- The audit log shows which passage backed each answer.
- A model upgrade is `corvid eval --swap-model gpt-5` away.

## Approval-gated tool pattern

```corvid
effect refund_effect:
    cost: $100.00
    trust: supervisor_required
    reversible: false

tool refund(amount: Float, id: String) -> String uses refund_effect:
    @host.payment.refund(id, amount)

@budget($0.10)
agent process_refund(ticket: Ticket) -> Result<String, String>:
    if not ticket.eligible_for_refund():
        return Err("not eligible per policy")
    approve Refund(ticket.amount, ticket.customer_id)
    return Ok(refund(ticket.amount, ticket.customer_id))
```

If you remove the `approve`, it doesn't compile. If you add
`await_approval`, an operator approves through the approval product
surface before the call goes out.

## Multi-step agent with checkpoints

```corvid
@replayable
@max_steps(10)
@max_wall_time(5m)
agent draft_and_send(brief: String) -> SendResult uses email_effect, llm_effect:
    let outline = generate_outline(brief)
    checkpoint outline                     # durable checkpoint
    let draft = expand_outline(outline)
    checkpoint draft
    await_approval EmailDraft(hash: hash(draft))
    approve EmailSend("review@example.com")
    return email_send("review@example.com", draft)
```

`checkpoint` writes the named value to durable state. After a crash,
the agent resumes from the last checkpoint with the recorded value.

## Provider routing

```corvid
import "@stdlib/model" as model

effect cheap_classification:
    cost: $0.001
    latency: fast
    confidence: 0.85

effect quality_summarization:
    cost: $0.05
    latency: medium
    confidence: 0.95

prompt classify(text: String) -> Category uses cheap_classification:
    requires model.cost <= $0.005
    "Classify: " + text

prompt summarize(text: String) -> String uses quality_summarization:
    requires model.confidence >= 0.9
    "Summarize: " + text
```

The `requires` clauses constrain which models the substrate can pick
at deploy time. A configuration that violates the constraint is
rejected before the binary ships.

## Local model fallback

```corvid
prompt local_summarize(text: String) -> String uses cheap_summarization:
    requires model.local
    "Summarize: " + text

prompt cloud_summarize(text: String) -> String uses quality_summarization:
    requires model.provider in ["openai", "anthropic"]
    "Summarize: " + text

agent summarize_with_fallback(text: String) -> String:
    let result = try local_summarize(text) on error retry 1 times
    if result.is_err():
        return cloud_summarize(text)
    return result.unwrap()
```

## Audit log per-decision

```corvid
import "@stdlib/db" as db
import "@stdlib/observability" as obs

agent process_refund_audited(ticket: Ticket) -> Result<String, String>:
    let trace_id = obs.current_trace_id()
    if not eligible(ticket):
        db.audit_log.write({
            actor: "system",
            action: "refund_denied",
            metadata: { customer_id: ticket.customer_id, reason: "ineligible" },
            trace_id,
        })
        return Err("ineligible")
    approve Refund(ticket.amount, ticket.customer_id)
    let receipt_id = refund(ticket.amount, ticket.customer_id)
    db.audit_log.write({
        actor: "system",
        action: "refund_issued",
        cost_cents: (ticket.amount * 100.0).to_int(),
        metadata: { customer_id: ticket.customer_id, receipt_id },
        trace_id,
    })
    return Ok(receipt_id)
```
