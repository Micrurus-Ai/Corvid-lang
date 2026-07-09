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

## Step 3 — Write the retrieval prompt

```corvid
prompt fetch_policy() -> Grounded<String> uses retrieval_effect:
    @retrieve("data/policy.txt")
```

`Grounded<String>` is not the same type as `String`. A grounded value
carries provenance: which source produced it, when, and a hash. You
cannot accidentally pass a grounded value where a model-only string is
expected — the compiler refuses.

## Step 4 — Write the decision prompt

```corvid
prompt decide_refund(
    ticket: String,
    policy: Grounded<String>,
) -> Bool uses llm_decision:
    "Given the policy: " + policy.unwrap_with_citation() +
    "\n\nDecide whether to refund this ticket: " + ticket +
    "\n\nReply 'yes' or 'no'."
```

`policy.unwrap_with_citation()` produces a string that includes the
citation. The model sees the source. The trace records the source. The
auditor can prove the decision was grounded.

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

```corvid
agent handle_refund(ticket: String, customer_id: String) -> String:
    policy = fetch_policy()
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

```corvid
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

Restore the `approve`. Now make `decide_refund` use the policy without
unwrapping it (i.e., treat it as a regular `String`):

```corvid
prompt decide_refund(ticket: String, policy: String) -> Bool ...
```

```
error[E0412]: type mismatch
  --> src/main.cor:30:36
   |
30 |     should_refund = decide_refund(ticket, policy)
   |                                           ^^^^^^ expected `String`, found `Grounded<String>`
   |
   = help: use `policy.unwrap_with_citation()` to expose the value
           with provenance, or `policy.unwrap_discarding_sources()`
           to drop provenance (loses audit guarantees).
   = guarantee: grounded.no_silent_unwrap
```

The compiler refuses to silently drop provenance. If you really want
to discard it, you write that explicitly with
`unwrap_discarding_sources()` and the audit log will record that you
did.

## Step 8 — Run with budgets

Add a budget annotation to the agent:

```corvid
@budget($0.50)
@max_steps(5)
agent handle_refund(ticket: String, customer_id: String) -> String:
    ...
```

Now the compiler knows the agent's cost ceiling. At runtime, a budget
violation aborts the agent with a typed error before the over-budget
call goes out to the provider.

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

1. The refund tool cannot be called without an explicit approval.
2. The policy passed to the model carries provenance the auditor can verify.
3. The agent has a hard cost ceiling.
4. The trace is replayable.
5. A model upgrade is a diff, not an outage.

Read **[Effects](/docs/effects)** to understand the dimension algebra in
depth, or jump to **[Reference apps](/docs/reference-apps)** to see this
pattern applied at production scale.
