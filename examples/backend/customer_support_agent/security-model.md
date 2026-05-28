# Customer Support Agent Security Model

- Draft replies are policy-grounded: every `SupportDraftReply` carries a
  `PolicyCitation` with a `provenance_id`, and the triage/draft agents
  fail their contract if the citation is missing. The agent answers
  from policy, not from imagination.
- Every dangerous support operation is gated by a developer-authored,
  compiler-enforced approval contract. The developer owns the flow
  (role, cost ceiling, data class, irreversibility, expiry); Corvid
  enforces the `approve <Label>` boundary and never decides the flow:
  - `SendSupportReply` — Reviewer, customer data, irreversible (a sent
    reply cannot be unsent; must be policy-grounded).
  - `IssueSupportRefund` — Admin, financial, irreversible (money leaves).
  - `EscalateTicket` — Reviewer, customer data (reversible: de-escalate).
  - `CloseTicket` — Reviewer, customer data (reversible: reopen).
  - `ApplyAccountCredit` — Admin, financial, irreversible (goodwill
    credit is money-equivalent).
- Calling any dangerous tool without its prior `approve` boundary fails
  `corvid check` with `E0101`; the `adversarial/ungated_*.cor` fixtures
  plus `ungrounded_reply.json` are the named-threat corpus.
- The three durable cron jobs (`sla_breach_scan`, `nightly_csat_rollup`,
  `policy_reindex`) carry only read effects — they observe, aggregate,
  and refresh the policy corpus. They can never send a reply, issue a
  refund, or change ticket state autonomously.
- Customer identifiers are represented as fingerprints in committed
  fixtures; raw customer detail is never committed.
