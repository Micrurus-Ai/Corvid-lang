# Approve

## What approve is

`approve` is a compile-time token. When a tool is declared with the
`dangerous` marker, the compiler refuses to emit a binary unless every
reachable call site is preceded by an `approve` of the matching shape.
The effect row's `trust:` dimension records the trust tier the call
carries (it feeds `@trust(...)` constraints and runtime approval
routing); the compile-time approve gate is the `dangerous` marker on
the tool declaration.

## The shape

```corvid-fragment
approve <PascalCaseAction>(<args matching the call>)
```

By convention the action name matches the tool name in PascalCase, but
the compiler's rule is purely structural: the arguments to `approve`
must match the arguments to the protected call.

## Example

```corvid-fragment
agent main() -> String:
    approve Refund(50.0, "cust_123")
    return refund(50.0, "cust_123")
```

If you remove the `approve` line, the compiler fails:

```text
[E0101] error: dangerous tool `refund` called without a prior `approve`
    │ Help: add `approve Refund(arg1, arg2)` on the line before this call

1 error(s) found.
```

## Lexical scope

`approve` is lexically scoped. An `approve` inside an `if` branch
authorizes only the calls in that branch. An `approve` inside a loop
authorizes only the calls in that iteration of the loop body.

```corvid-fragment
agent main() -> String:
    if needs_refund:
        approve Refund(50.0, "cust_123")
        return refund(50.0, "cust_123")     # ok
    return refund(50.0, "cust_123")          # error: no approve in scope
```

## Why lexical, not body-wide

A body-wide approve would mean "any call to this tool anywhere in the
agent body is fine," which defeats the purpose. The lexical rule means
your audit log shows exactly which control-flow path produced each
authorized call.

There is one body-wide token concept used internally
(`approval.token_lexical_only`) that lets the typechecker discriminate
between "you forgot the approve" (no approve seen anywhere in this
body) and "you put the approve in the wrong scope" (approve seen
elsewhere but not in scope here). The two diagnostics fire with
different `guarantee_id`s so you can tell at a glance which mistake
you made.

## At runtime

`approve` is compile-time only. There is no runtime `approve` machinery
to bypass — the binary either carries the proof or it doesn't.

## With human approval

For workflows where a human approves a specific run (not just the code
path), use `await_approval`:

```corvid-fragment
agent main() -> String:
    await_approval RefundDecision(amount: 50.0, customer: "cust_123")
    approve Refund(50.0, "cust_123")
    return refund(50.0, "cust_123")
```

`await_approval` pauses the run, persists state, and resumes when an
operator approves through the approval product surface (Phase 39).

## Adversarial coverage

The compiler's approval rule is tested against four classes of
attempted bypass:

1. `@approve` re-export through an aliased import.
2. Effect row downgrade via type erasure.
3. Reachability laundering through a higher-order function.
4. Body-wide approval inserted via macro or codegen.

The corpus that exercises these mutations lives in
`crates/corvid-types/tests/source_bypass_corpus.rs`. Each mutation is
caught with the matching `guarantee_id`.
