# Effects

## What is an effect

An effect is a typed description of what a function does to the outside
world. In Corvid, effects are the load-bearing primitive that lets the
compiler reason about cost, trust, latency, reversibility, data
provenance, and confidence — all the things that matter for AI code but
that conventional type systems can't see.

## The six dimensions

Every effect carries values along these six dimensions:

| Dimension | Type | Examples |
|---|---|---|
| `cost` | `Money` | `$0.005`, `$50.00` |
| `latency` | `Latency` | `fast`, `medium`, `slow` |
| `confidence` | `Float` (0..1) | `0.9`, `0.95` |
| `trust` | `Trust` | `model_only`, `supervisor_required`, `verified_chain` |
| `reversible` | `Bool` | `true`, `false` |
| `data` | `Data` | `grounded`, `synthetic`, `external_action` |

You don't have to specify all six on every effect. Unspecified
dimensions take a per-dimension default that the compiler can warn
about if your call graph composes them in surprising ways.

## Declaring effects

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

effect refund_effect:
    cost: $100.00
    trust: supervisor_required
    reversible: false
```

## Using effects

```corvid
prompt summarize(text: String) -> String uses llm_call:
    "Summarize: " + text

tool refund(amount: Float, id: String) -> String uses refund_effect:
    @host.payment.refund(id, amount)
```

## Composition

When an agent calls a prompt and a tool, its effect row is the union:

```corvid
agent main() -> String:
    summary = summarize("hello")        # uses llm_call
    return refund(10.0, "cust_1")       # uses refund_effect
    # main's effect row: { llm_call, refund_effect }
```

The compiler computes this automatically. You can write the union
explicitly if you want to lock the agent's effect surface.

## Why this matters

Each dimension drives a real compiler behavior:

- `cost` — composed across calls; if the agent has a `@budget` annotation,
  the compiler proves the worst-case cost is within budget.
- `trust: supervisor_required` — every call site must be preceded by an
  `approve` token. See **[Approve](/docs/approve)**.
- `reversible: false` — the call is treated as committed at the moment of
  invocation; replay does not re-execute it by default.
- `data: grounded` — the return value must be wrapped in `Grounded<T>`.
  See **[Grounded](/docs/grounded)**.
- `data: external_action` — the call is recorded in the audit log with
  a special "side effect on the world" tag.
- `confidence` — composed across calls; agents whose composed confidence
  falls below a threshold can be configured to escalate or refuse.

## Effects vs. types

Other languages give you types. Corvid gives you types AND effects. A
function's type tells you what it returns; its effect row tells you what
it costs, who can authorize it, what data it produces, and whether it is
reversible. Both are checked at compile time.

## Deeper material

- The full effect algebra:
  [`docs/internals/effect-spec/02-composition-algebra.md`](https://github.com/Corvid-lang/Corvid-lang/blob/main/docs/internals/effect-spec/02-composition-algebra.md)
- Typing rules:
  [`docs/internals/effect-spec/03-typing-rules.md`](https://github.com/Corvid-lang/Corvid-lang/blob/main/docs/internals/effect-spec/03-typing-rules.md)
