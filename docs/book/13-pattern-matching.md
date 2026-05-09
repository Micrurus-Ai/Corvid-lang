# Pattern matching

## `match` form

```corvid
match value:
    Pattern1 -> expr1
    Pattern2 -> expr2
    _        -> default
```

`match` is exhaustive — the compiler refuses to emit if a sum-type
match doesn't cover every variant or have a wildcard.

## Patterns

```corvid
# literal
match n:
    0     -> "zero"
    1     -> "one"
    _     -> "many"

# struct
match decision:
    Decision { refund: true, amount, ... }  -> "refund " + amount.to_string()
    Decision { refund: false, reason }      -> "denied: " + reason

# sum type
match status:
    Pending          -> "waiting"
    Approved(who)    -> "by " + who
    Denied(reason)   -> reason

# Option / Result (just sum types underneath)
match find(xs, n):
    Some(x) -> "got " + x.to_string()
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

## Destructuring in `let` and parameters

```corvid
let Decision { refund, amount, .. } = compute_decision(ticket)

fn handle(Decision { refund, amount, reason }: Decision) -> String:
    ...
```

## Exhaustiveness

```corvid
match status:
    Pending       -> "waiting"
    Approved(who) -> "by " + who
    # error: missing variant `Denied`
```

Add the missing arm or a `_ -> ...` catch-all.
