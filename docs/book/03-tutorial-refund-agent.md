# Tutorial: Refund Agent

## What you'll build

A customer-support agent that:

1. Reads a customer's support ticket.
2. Decides whether to refund based on a policy document (RAG-grounded).
3. Asks for human approval before issuing the refund.
4. Issues the refund through a payment connector.
5. Writes an audit log of the decision.

By the end you'll see every Corvid invention in one program: effects,
approvals, `Grounded<T>`, prompts with budgets, agent loops, and the
replay surface.

## Step 1 — Project skeleton

```sh
corvid new refund-agent
cd refund-agent
```

Add the example policy doc:

```sh
mkdir data
echo "Refunds approved up to $100 within 30 days of purchase." > data/policy.txt
```

## Step 2 — Declare the effects

`src/main.cor`:

```corvid
effect retrieval_effect:
    cost: $0.001
    latency: fast
    confidence: 0.95
    data: grounded

effect llm_decision:
    cost: $0.02
    latency: medium
    confidence: 0.9
    trust: model_only

effect refund_effect:
    cost: $100.00
    latency: medium
    trust: supervisor_required
    reversible: false
    data: external_action
```

Three named effects, each with the dimensions that matter for that kind
of work. The compiler will use these to decide which calls need
approvals, which calls need grounding, and what the budget ceiling is for
the agent.

## Step 3 — Write the retrieval tool

```corvid-fragment
tool fetch_policy(path: String) -> Grounded<String> uses retrieval_effect
```

The tool is signature-only; the host's retrieval implementation
returns the document and the runtime attaches provenance (source
path, retrieval timestamp, content hash). `Grounded<String>` is not
the same type as `String` — you cannot pass a plain string where a
grounded one is expected; the compiler refuses (see
**[Grounded](/docs/grounded)** for the full asymmetric rule).

## Step 4 — Write the decision prompt

```corvid-fragment
prompt decide_refund(ticket: String, policy: Grounded<String>) -> Bool uses llm_decision:
    cites policy strictly
    "Given the policy: {policy} — decide whether to refund this ticket: {ticket}. Reply yes or no."
```

The body is a single template string; `{policy}` and `{ticket}`
interpolate (grounded parameters interpolate with their provenance
attached). `cites policy strictly` requires the model's answer to
carry text evidence from that source — the model sees the source, the
trace records the source, the auditor can prove the decision was
grounded.

## Step 5 — Write the refund tool

```corvid-fragment
tool refund(amount: Float, customer_id: String) -> String dangerous uses refund_effect
```

This is a real side-effect tool: a signature-only declaration whose
implementation the host provides through registered-tool dispatch (a
`tools.py` function, a Rust FFI cdylib, or an executing stdlib tool).
The `dangerous` marker is the compile-time approve gate — the compiler
requires an `approve` token before any reachable call site. The effect
row's `trust: supervisor_required` and `reversible: false` record the
trust tier, which feeds `@trust(...)` constraints and runtime approval
routing.

## Step 6 — Compose the agent

The full program so far (this exact block is compiled in CI):

```corvid
effect retrieval_effect:
    cost: $0.001
    latency: fast
    confidence: 0.95
    data: grounded

effect llm_decision:
    cost: $0.02
    latency: medium
    confidence: 0.9
    trust: model_only

effect refund_effect:
    cost: $100.00
    latency: medium
    trust: supervisor_required
    reversible: false
    data: external_action

tool fetch_policy(path: String) -> Grounded<String> uses retrieval_effect

prompt decide_refund(ticket: String, policy: Grounded<String>) -> Bool uses llm_decision:
    cites policy strictly
    "Given the policy: {policy} — decide whether to refund this ticket: {ticket}. Reply yes or no."

tool refund(amount: Float, customer_id: String) -> String dangerous uses refund_effect

agent handle_refund(ticket: String, customer_id: String) -> String:
    policy = fetch_policy("data/policy.txt")
    should_refund = decide_refund(ticket, policy)
    if should_refund:
        approve Refund(50.0, customer_id)
        return refund(50.0, customer_id)
    return "Refund denied per policy."
```

Try compiling this:

```sh
corvid check src/main.cor
```

## Step 7 — Watch the compiler catch the obvious bugs

Remove the `approve` line:

```corvid-fragment
    if should_refund:
        return refund(50.0, customer_id)
```

```text
[E0101] error: dangerous tool `refund` called without a prior `approve`
    ╭─[src/main.cor:25:16]
    │
 25 │         return refund(50.0, customer_id)
    │                ────────────┬────────────
    │                            ╰───────────── this call needs prior approval
    │
    │ Help: add `approve Refund(arg1, arg2)` on the line before this call
────╯

1 error(s) found.
```

Restore the `approve`. Now try laundering provenance: change
`decide_refund` to take a plain `String` policy and mark the agent
`@grounded_pure` (the strict provenance mode):

```corvid-fragment
prompt decide_refund(ticket: String, policy: String) -> Bool uses llm_decision:
    "Given the policy: {policy} — refund this ticket: {ticket}?"

@grounded_pure
agent handle_refund(ticket: String, customer_id: String) -> String:
    policy = fetch_policy("data/policy.txt")
    should_refund = decide_refund(ticket, policy)
    ...
```

```text
error: agent `handle_refund` is marked `@grounded_pure` but silently
coerces a `Grounded<T>` into a non-grounded slot at `Grounded<String>`
    │ Help: preserve the `Grounded<T>` wrapper — return / parameter /
    │ field types should match the source's grounding, not strip it.
```

A strict agent refuses to drop provenance silently. (Without
`@grounded_pure`, the coercion is permitted and recorded — the
reverse direction, forging a `Grounded<String>` from a plain string,
is refused in every mode. See **[Grounded](/docs/grounded)**.)

## Step 8 — Run with budgets

Add a budget annotation to the agent. Set it BELOW the worst case
first, to watch the budget checker refuse:

```corvid-fragment
@budget($0.50)
@max_steps(5)
agent handle_refund(ticket: String, customer_id: String) -> String:
    ...
```

```text
[E0250] error: effect constraint violated in agent `handle_refund`
    │ @budget($0.50)
    │ static worst-case cost exceeds the declared budget
```

The composed worst case includes the $100.00 `refund_effect`, so a
$0.50 budget is refused at compile time. Raise it to cover the worst
case:

```corvid-fragment
@budget($150.00)
@max_steps(5)
agent handle_refund(ticket: String, customer_id: String) -> String:
    ...
```

Now the agent compiles with a proven cost ceiling. At runtime, a
budget violation additionally aborts the agent with a typed error
before an over-budget call goes out to the provider.

## Step 9 — Replay

Every run produces a trace.

```sh
corvid run src/main.cor
corvid trace list
corvid replay <trace-id>
```

Replay is byte-identical: same prompts, same model responses (cached
from the original run), same tool calls (intercepted, not re-executed
unless you ask). This is what gives you "what changed?" in seconds when
a model upgrade lands.

```sh
corvid eval --swap-model gpt-5 --source src/main.cor target/trace
```

Diffs the new model's behavior against the recorded baseline.

## What you just shipped

A real customer-support agent with five compile-time guarantees:

1. The refund tool (marked `dangerous`) cannot be called without an
   explicit approval.
2. The policy carries provenance end-to-end: it can never be forged
   from model output, and under `@grounded_pure` it can never be
   silently dropped.
3. The agent has a hard cost ceiling, proven against the composed
   worst case at compile time (E0250 when exceeded).
4. The trace is replayable.
5. A model upgrade is a diff, not an outage.

Read **[Effects](/docs/effects)** to understand the dimension algebra in
depth, or jump to **[Reference apps](/docs/reference-apps)** to see this
pattern applied at production scale.
