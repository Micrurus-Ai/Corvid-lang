# Compile-time guarantees

## What this page is

The catalog of properties the Corvid compiler enforces at compile time.
Every `Static` row in this list is the kind of bug that other
languages cannot catch until production.

## How to read this

Each guarantee has an id (e.g. `approval.dangerous_call_requires_token`),
a class (Static / RuntimeChecked / OutOfScope), a phase (TypeCheck /
Resolve / IrLower / Codegen / Runtime), and a positive + adversarial
test reference. The full machine-readable list is `corvid contract list`.

```sh
corvid contract list                        # human-readable
corvid contract list --format=json          # machine-readable
corvid claim --explain approval.dangerous_call_requires_token
```

## Headline guarantees

- `approval.dangerous_call_requires_token` — every call to a tool whose
  effect row carries `trust: supervisor_required` must be preceded by
  a matching `approve` in lexical scope.
- `approval.token_lexical_only` — an `approve` token authorizes only
  calls in its lexical scope. Body-wide approves are caught and
  reported with this id.
- `grounded.no_silent_unwrap` — a `Grounded<T>` cannot be passed where
  a `T` is expected. Unwrap is one of three named methods, each
  recorded in the audit log.
- `effect_row.budget_within_limit` — when an agent has `@budget($X)`,
  the composed worst-case cost of its call graph must be ≤ `$X`.
- `effect_row.composition_well_formed` — composed effect rows obey
  the dimension algebra (cost adds, confidence multiplies, trust takes
  the strictest, etc.).
- `signed_cdylib.descriptor_match` — a signed cdylib build refuses to
  emit if the in-binary ABI descriptor doesn't match what
  `corvid-abi-verify` reconstructs from source.

## Full list

```sh
corvid contract list
```

The full machine-readable registry lives in
[`crates/corvid-guarantees/src/registry.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-guarantees/src/registry.rs).
Every row carries positive and adversarial test references; every
referenced test exists and runs in CI. The drift-gate test
(`every_test_ref_resolves_to_a_real_test_function`) catches divergence
between the registry and the test surface on every push.

## Honest classification

Some properties that look like compile-time guarantees are actually
RuntimeChecked or OutOfScope. The registry says so explicitly with an
`out_of_scope_reason` field. We do not optimistically tag a property
as Static if the enforcement mechanism is actually a runtime check or
is subsumed by another guarantee. Phase 35V's verification round
caught four such cases and downgraded them honestly.

## What's NOT compile-time-guaranteed

- Cryptographic primitive correctness (we use ed25519, SHA-256, DSSE
  as standardized primitives, we do not redesign them).
- Compiler toolchain compromise (rustc, the linker, etc. are trusted).
- Signing-key compromise (host responsibility, see
  [`docs/security/model.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/security/model.md)).
- Formal mechanized proof of the type system (post-v1.0 research).

The
[security model doc](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/security/model.md)
enumerates the trusted computing base and the threat model.
