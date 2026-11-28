# Refinements on parameters

Status: proposed 2026-11-27 (macOS session), posted for frontend review. The grammar is shared, so building waits one working day for an objection. Roadmap: the B04 item to refuse a literal `random_token` size and `PasswordCost` below the OWASP floor at compile time.

## The weakness

A refinement bounds a record field (`limit: Int where between(1, 50)`), but
not a parameter. So a bound that belongs to a function, tool or action input
can only be checked by hand inside the body, at run time:

```corvid
token = random_token(8)   # compiles; fails when it runs: 8 bytes is too few
```

`std:crypto` knows `random_token` needs 16 to 1024 bytes, but it cannot say
so in its signature. Neither can an action whose `limit` must be 1 to 50.
Each says it in a runtime check, which the frontend's form, the OpenAPI
document and an assistant's tool schema never see.

## Design

A parameter takes the refinements a field takes:

```corvid
public tool random_token(bytes: Int where between(16, 1024)) -> Secret uses crypto_random

action search(query: String where len_between(1, 200), limit: Int where between(1, 50)) -> List<Book>:
    ...
```

```text
param ::= IDENT ':' type_ref refinement? ownership?
```

1. **A literal argument out of range is a compile error** at the call,
   naming the parameter, the bound and the value: `random_token(8)` gives
   ``bytes` is 8, outside `between(16, 1024)``. This is the rule constructors
   already follow for refined fields (Refine-1).
2. **A computed argument is checked when the call starts**, on both tiers,
   with the message a refined field gives. A tool's arguments are checked
   before the tool runs, so its implementation never sees an out-of-range
   value.
3. **The bound is part of the signature**, so it reaches every tier from the
   one declaration:
   - the application contract and OpenAPI add `minimum`/`maximum` or
     `minLength`/`maxLength`;
   - the generated TypeScript client and a form bound to an action validate
     it before sending;
   - an assistant's tool schema carries it.
4. **Cost analysis reads it.** A loop bounded by a refined parameter
   (`for i in range(limit)`) is charged at the bound's maximum, as a refined
   field's `take(n)` already is.

`PasswordCost`'s fields take refinements too: `memory_kib: Int where
between(7168, 4194304)`, and `iterations` and `lanes` at least 1. That
catches a cost that is wrong in one dimension at compile time. The OWASP
rule relates memory to iterations (47104 KiB with 1 pass, 19456 with 2,
and so on), which a per-field bound cannot state. It stays a runtime check,
with its existing message.

## Alternatives

- **A checker rule for the two std declarations.** This is a hidden
  special case that only std could use, against "power from composition".
- **Evaluate a validator function at compile time** (`@validated_by(f)`).
  It can state cross-field rules like OWASP's, but it is a much larger
  feature (compile-time evaluation), and it is invisible to the contract and
  the frontend, which cannot run Corvid.
- **Refinement types** (`type TokenSize = Int where between(16, 1024)`).
  This needs a newtype and its conversions, while aliases are transparent
  today. A bound written where it is used reads more plainly.

## Three tiers

- **Backend:** literal arguments are refused at compile time, and computed
  ones are checked on entry on both tiers.
- **Frontend:** the bound reaches the contract, so a generated form and
  client validate before a request is sent, from the same declaration.
- **Assistant:** an action's tool schema carries `minimum`/`maximum`, so
  a model is told the range instead of discovering it from a refusal.

## Status

PR-1, PR-2, PR-3's backend half and PR-4 built 2026-11-28, after frontend development approved (item 5 of the review brief). `std:crypto`'s bounds move to PR-2: declared now, they would keep every native program that uses `random_token` on the interpreter, since PR-1 refuses refined parameters natively until native code checks them.

## Slices

1. PR-1. The parser and checker: refinements on parameters, literal
   arguments checked at compile time, and computed ones checked on entry on
   the interpreter. `std:crypto` declares `random_token`'s bound, and
   `PasswordCost` declares its field bounds.
2. PR-2. Native code checks a refined parameter on entry with the
   interpreter's message.
3. PR-3. The contract, OpenAPI, the TypeScript client and the form binding
   carry the bound.
4. PR-4. Cost analysis bounds loops by a refined parameter.

## Non-scope

- Cross-field rules such as OWASP's memory and iterations.
- One-sided bounds (`at_least(n)`). A large upper bound stands in for one
  until a program needs it.
- Refinements on return types.
