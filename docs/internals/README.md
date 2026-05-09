# Internals

Spec-level material. Read these when you are contributing to the
compiler, designing a connector, debugging an ABI mismatch, or
auditing the language's formal claims.

## Pages

- [Effect spec](effect-spec/) — the formal effect algebra,
  composition rules, typing rules, dimensions, grounding,
  confidence gates, cost budgets, streaming, model substrate,
  verification, replay.
- [Bundle format](bundle-format.md) — the published Corvid bundle
  layout for reproducible builds.
- [WASM ABI](wasm-abi.md) — the bare `(ptr, len)` UTF-8 ABI,
  multi-value returns, manifest kind discriminator.
- [Host compliance](host-compliance.md) — host-side requirements
  that runtime enforcement assumes.
- [Testing primitives](testing-primitives.md) — `test`, `eval`,
  `fixture`, `mock` internals.
- [Package manager scope](package-manager-scope.md) — what the
  package manager covers and doesn't.

## See also

- [Reference: grammar](../reference/grammar.md) — formal EBNF.
- [Reference: lexer rules](../reference/lexer-rules.md) — line
  continuation behavior.
- [Reference: core semantics](../reference/core-semantics.md) —
  registry-derived spec.
