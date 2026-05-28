# Finance Operations Agent Security Model

- The app provides operational summaries and executes operations a human
  approved. It never gives regulated financial advice — every surface is
  "do the thing the human decided", not "recommend what to do".
- Every dangerous financial operation is gated by a developer-authored,
  compiler-enforced approval contract. The developer owns the flow
  (role, cost ceiling, data class, irreversibility, expiry); Corvid
  enforces the `approve <Label>` boundary and never decides the flow:
  - `SubmitPaymentIntent` — Admin, irreversible (money leaves).
  - `CancelSubscription` — Reviewer (stop a recurring charge).
  - `DisputeTransaction` — Reviewer (file a provider dispute).
  - `ExportFinancialReport` — Admin, irreversible (financial data leaves
    the tenant boundary; redaction policy hash required).
  - `ScheduleRecurringPayment` — Admin, irreversible (commits future
    money movement).
- Calling any dangerous tool without its prior `approve` boundary fails
  `corvid check` with `E0101`; the `adversarial/ungated_*.cor` fixtures
  plus `autonomous_payment.json` are the named-threat corpus.
- Mock fixtures use fingerprints for sensitive explanations; raw
  financial detail is never committed.
- Execution of any financial write without approval is out of scope and
  rejected at compile time.
