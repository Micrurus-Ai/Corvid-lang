# Grounded

Every `corvid`-tagged block compiles through the real driver in CI;
the deliberately-failing block is pinned to keep failing.

## What `Grounded<T>` is

`Grounded<T>` is a type that wraps a value with provenance. A value of
type `Grounded<String>` is not the same type as `String` — and the
compiler enforces the boundary asymmetrically, in the direction that
matters for safety:

- **Ungrounded → grounded slot: always a compile error.** You cannot
  pass a plain `String` where `Grounded<String>` is expected. A model
  output can never masquerade as sourced data.
- **Grounded → ungrounded slot: permitted by default** (the "legacy
  coercion" — the wrapper drops silently, and the typechecker records
  the site). Mark the agent `@grounded_pure` and every such site
  becomes a compile error.

This is how Corvid prevents "the model said X, and we treated X as
fact": sourced data is a different type, and strict agents refuse to
launder it away.

## How you produce a grounded value

A tool whose effect carries `data: grounded` returns `Grounded<T>` —
the runtime attaches the provenance (source, timestamp, content hash)
when the host's retrieval implementation returns:

```corvid
effect retrieval:
    data: grounded

tool fetch_policy(path: String) -> Grounded<String> uses retrieval
```

## Grounded values flow through prompts

A prompt can take and return grounded values; `{param}` interpolation
works on them, and `cites <param> strictly` requires the model's
answer to carry text evidence from that source:

```corvid
effect retrieval:
    data: grounded

effect llm_call:
    cost: $0.02
    latency: medium
    confidence: 0.9

tool fetch_policy(path: String) -> Grounded<String> uses retrieval

prompt answer(ctx: Grounded<String>) -> Grounded<String> uses llm_call:
    cites ctx strictly
    "Answer from the policy: {ctx}"

agent cited(path: String) -> Grounded<String>:
    return answer(fetch_policy(path))
```

## What the compiler always refuses

Passing an ungrounded value into a grounded slot:

```corvid-error
effect retrieval:
    data: grounded

effect llm_call:
    cost: $0.02
    latency: medium
    confidence: 0.9

tool fetch_policy(path: String) -> Grounded<String> uses retrieval

prompt cite_answer(ctx: Grounded<String>) -> Grounded<String> uses llm_call:
    cites ctx strictly
    "Answer from {ctx}"

agent main() -> Grounded<String>:
    plain = "not grounded"
    return cite_answer(plain)
```

```text
[E0208] error: type mismatch in argument 1 to `cite_answer`
    │ Help: change the value to produce a `Grounded<String>`, or update the signature
```

## Strict mode: `@grounded_pure`

By default, passing a `Grounded<String>` into a plain `String` slot
coerces silently (the legacy rule). Mark the agent `@grounded_pure`
and the compiler refuses every laundering shape:

```corvid-error
effect retrieval:
    data: grounded

effect llm_call:
    cost: $0.02
    latency: medium
    confidence: 0.9

tool fetch_policy(path: String) -> Grounded<String> uses retrieval

prompt decide(policy: String) -> Bool uses llm_call:
    "Decide from policy: {policy}"

@grounded_pure
agent main() -> Bool:
    policy = fetch_policy("data/policy.txt")
    return decide(policy)
```

```text
error: agent `main` is marked `@grounded_pure` but silently coerces a
`Grounded<T>` into a non-grounded slot at `Grounded<String>`
    │ Help: preserve the `Grounded<T>` wrapper — return / parameter /
    │ field types should match the source's grounding, not strip it.
```

`@grounded_pure` composes through the call graph the same way
`@deterministic` does: a `@grounded_pure` agent may only call agents
that are themselves `@grounded_pure`.

## Named unwrap methods

> **Planned — rides the builtin-method machinery (slice 45c).** The
> named consumption methods (`unwrap_with_citation()`, `value()`,
> `unwrap_discarding_sources()`) are designed so every provenance
> drop is explicit and audit-visible, but method calls on built-in
> types are not implemented yet. Today the only unwrap is the silent
> legacy coercion described above — which is exactly why
> `@grounded_pure` exists as the strict boundary until the named
> methods land.

```corvid-planned
g.unwrap_with_citation()         # String including the citation marker
g.value()                        # bare T, but the trace records the unwrap
g.unwrap_discarding_sources()    # bare T, audit log records the discard
```

## Why this is the moat

In every other language, "the model said X" and "the document said X"
are the same `String`. The auditor cannot tell which one made the
decision. In Corvid they are different types: sourced data can never
be forged from model output (always enforced), and strict agents
(`@grounded_pure`) additionally refuse to drop provenance silently.
