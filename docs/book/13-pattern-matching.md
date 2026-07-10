# Pattern matching

Every `corvid`-tagged block compiles through the real driver in CI;
the deliberately-failing block is pinned to keep failing.

## `match` form

`match` is an expression: arms are `pattern -> expr`, tried in
order. It is **exhaustiveness-checked** — the compiler refuses a sum
match that doesn't cover every variant (or carry a catch-all).

```corvid
type Status:
    | Pending
    | Approved(approver: String)
    | Denied(reason: String, code: Int)

agent describe(s: Status) -> String:
    return match s:
        Pending -> "waiting"
        Approved(who) -> "approved by " + who
        Denied(reason, code) -> "denied: " + reason + " #" + code.to_string()
```

## Patterns

```corvid
type Status:
    | Pending
    | Approved(approver: String)
    | Denied(reason: String, code: Int)

type Claim:
    urgent: Bool
    amount: Float

agent tour(n: Int, c: Claim, o: Option<Int>, s: Status) -> String:
    # literal arms + guards + wildcard
    size = match n:
        0 -> "zero"
        x if x > 100 -> "big"
        _ -> "small"

    # record destructure: literal fields, shorthand binding, `..` rest
    claim = match c:
        Claim { urgent: true, amount } -> "urgent " + amount.to_string()
        Claim { .. } -> "routine"

    # Option / Result are just sum types underneath
    count = match o:
        Some(x) -> x
        None -> 0

    # binding the matched value with `@`
    kind = match s:
        v @ Approved(_) -> "an approval"
        other -> "something else"

    return size + claim + count.to_string() + kind
```

A bare name in a pattern is a **unit-variant test** when it resolves
to a variant (`Pending`), and a **binding** otherwise (`other`).
Guarded arms never count toward exhaustiveness — a guard can fail.

## Exhaustiveness

This block is compiled in CI and pinned to KEEP failing:

```corvid-error
type Status:
    | Pending
    | Approved(approver: String)
    | Denied(reason: String)

agent main(s: Status) -> String:
    return match s:
        Pending -> "waiting"
        Approved(who) -> who
```

```text
error: non-exhaustive match: missing variant(s) `Denied`
    Help: add the missing arm(s), or a final `_ -> ...` catch-all
```

`Option` needs `Some(_)` + `None`; `Result` needs `Ok(_)` + `Err(_)`;
`Bool` needs `true` + `false`; every other scrutinee type needs a
catch-all arm (`_` or a binding).

## Destructuring bindings and parameters (Planned — with 45n)

Keyword-free destructuring in statement position shares the
`Type { ... }` surface with 45n's named struct literals and ships
alongside them:

```corvid-planned
Decision { refund, amount, .. } = compute_decision(ticket)
```

## What this replaced

Until this slice, the shipped consumption story for Option/Result
was `?` propagation only — an `Err` could be forwarded but never
inspected. `match` closes that gap: branching on WHICH error
occurred, defaulting a `None` at the point of use, and modelling
state machines over sum types are all first-class now.
