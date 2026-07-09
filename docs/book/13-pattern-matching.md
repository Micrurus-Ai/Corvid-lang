# Pattern matching

> **Planned — this entire chapter describes designed syntax that is
> not yet implemented.** User-declared sum types land in slice 45h
> and the `match` form + patterns + destructuring land in slice 45i
> of the Language completeness track (see `ROADMAP.md`). Until those
> ship, this chapter is the design document for that work; the
> section at the bottom shows what to use today.

## `match` form (Planned — 45i)

```corvid-planned
match value:
    Pattern1 -> expr1
    Pattern2 -> expr2
    _        -> default
```

`match` is exhaustive — the compiler refuses to emit if a sum-type
match doesn't cover every variant or have a wildcard.

## Patterns (Planned — 45i)

```corvid-planned
# literal
match n:
    0     -> "zero"
    1     -> "one"
    _     -> "many"

# record destructure
match decision:
    Decision { refund: true, amount, .. }   -> "refund approved"
    Decision { refund: false, reason, .. }  -> "denied: " + reason

# sum type (declared per slice 45h)
match status:
    Pending          -> "waiting"
    Approved(who)    -> "by " + who
    Denied(reason)   -> reason

# Option / Result (just sum types underneath)
match find(xs, n):
    Some(x) -> "got one"
    None    -> "no match"

# binding the matched value
match status:
    s @ Approved(_) -> log(s)
    other           -> log(other)

# guard
match x:
    n if n > 100 -> "big"
    n if n > 10  -> "medium"
    _            -> "small"
```

## Destructuring in `let` and parameters (Planned — 45i, on the 45a `let` surface)

```corvid-planned
let Decision { refund, amount, .. } = compute_decision(ticket)

fn handle(Decision { refund, amount, reason }: Decision) -> String:
    ...
```

## Exhaustiveness (Planned — 45i)

```corvid-planned
match status:
    Pending       -> "waiting"
    Approved(who) -> "by " + who
    # error: missing variant `Denied`
```

Add the missing arm or a `_ -> ...` catch-all.

## What ships today

Until `match` lands, the shipped consumption story is postfix `?`
propagation plus `if`/`else` branching. `?` unwraps an `Ok`/`Some`
or propagates the `Err`/`None` to a caller whose return type carries
it:

```corvid
type Decision:
    refund: Bool
    amount: Float
    reason: String

agent compute_decision(amount: Float) -> Result<Decision, String>:
    if amount > 100.0:
        return Err("above the auto-approve limit")
    return Ok(Decision(true, amount, "policy match"))

agent handle(amount: Float) -> Result<String, String>:
    decision = compute_decision(amount)?
    if decision.refund:
        return Ok("refund approved: " + decision.reason)
    return Ok("denied: " + decision.reason)
```

Branching on which *error* occurred (rather than just propagating
it) and defaulting a `None` at the point of use both need `match` /
`unwrap_or` — that's exactly the gap slices 45i and 45l close.
