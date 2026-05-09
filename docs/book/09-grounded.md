# Grounded

## What `Grounded<T>` is

`Grounded<T>` is a type that wraps a value with provenance. A value of
type `Grounded<String>` is not interchangeable with a value of type
`String` — the compiler refuses silent conversion in either direction.
This is how Corvid prevents "the model said X, and we treated X as
fact" without anyone tagging it as a model output.

## How you produce a grounded value

```corvid
prompt fetch_policy() -> Grounded<String> uses retrieval_effect:
    @retrieve("data/policy.txt")
```

The `@retrieve(...)` host call returns a value annotated with the
source path, retrieval timestamp, and content hash. The runtime
attaches that provenance; the type system propagates it.

## How you consume a grounded value

Three ways, each named to make the audit log obvious:

```corvid
g.unwrap_with_citation()         # String including the citation marker
g.value()                        # bare T, but the trace records the unwrap
g.unwrap_discarding_sources()    # bare T, audit log records the discard
```

`unwrap_with_citation()` is the default — the model sees the source,
the trace records the source, the auditor can prove the decision was
grounded.

`value()` gives you the bare value but the trace records that you
unwrapped without citation. Useful for cases where the model already
saw the source via another path.

`unwrap_discarding_sources()` is the explicit "I am dropping
provenance" path. The audit log records the discard; the auditor can
review whether it was justified.

## What the compiler refuses

```corvid
prompt decide(policy: String) -> Bool uses llm_call:
    ...

agent main():
    policy = fetch_policy()        # Grounded<String>
    decide(policy)                 # error: expected String, got Grounded<String>
```

```
error[E0412]: type mismatch
  = guarantee: grounded.no_silent_unwrap
```

You either change `decide`'s signature to take `Grounded<String>`, or
you unwrap explicitly with one of the three named methods. Silent loss
of provenance does not happen.

## Composing grounded values

```corvid
fn combine(a: Grounded<String>, b: Grounded<String>) -> Grounded<String>:
    Grounded.merge(a, b)
```

`Grounded.merge` produces a value whose provenance is the union of the
inputs' provenances. The trace shows both sources.

## Why this is the moat

In every other language, "the model said X" and "the document said X"
are the same `String`. The auditor cannot tell which one made the
decision. In Corvid, they are different types, and the compiler will
not let you mix them up.
