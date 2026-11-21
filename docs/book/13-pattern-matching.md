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

With a user-defined variant, bind payloads in declaration order. An arm such as `Found(item, weight) if weight > 1 -> item` returns the enclosed record when the guard succeeds. A later `Found(_, _)` arm handles the other weights. This works on both CLI targets, including when an imported function created the value.

String comparisons also work directly in guards and arm expressions. For example, `Some(s) if enabled and s == "ready" -> s.length()` checks the string only when `enabled` is true. Both CLI targets skip calls on the right of a false `and` or a true `or`.

A match can also perform an action. This loop tries each input once. A successful parse calls `record_number`, and a failed parse keeps the original text. Both arms run in the interpreter and the native compiler.

```corvid
agent record_number(numbers: List<Int>, number: Int) -> Nothing:
    numbers.append(number)

agent collect(inputs: List<String>, numbers: List<Int>, rejected: List<String>) -> Nothing:
    for text in inputs:
        match text.parse_int():
            Ok(number) -> record_number(numbers, number)
            Err(_) -> rejected.append(text)
```

An arm can use calls inside arithmetic, for example `Some(n) -> score(n) + bonus(n) * 2`. Only the selected arm runs. Its calls execute from left to right, and multiplication has its usual precedence.

In a native `Result` match, calls can take pattern bindings, surrounding locals, literals, simple scalar expressions, and the results of other agent calls. For example, `Ok(number) -> record_number(numbers, normalize(number))` calls `normalize` once when parsing succeeds. The selected arm evaluates arguments from left to right and can return a value, including a string or list.

Both CLI targets support nested `Some`, `None`, `Ok` and `Err` patterns. A guard runs only after its pattern matches. If it returns false, matching continues with the next arm. This distinguishes a present negative number from either kind of missing value.

```corvid
agent classify(value: Option<Option<Int>>) -> Int:
    return match value:
        Some(Some(n)) if n > 0 -> n
        Some(Some(_)) -> -1
        Some(None) -> -2
        _ -> -3
```

## Patterns

`List.first()` returns `Some` with the first item, or `None` for an empty list. In both native and interpreted Corvid, either arm of this two-arm match can return a value or call a helper.

```corvid
agent first_label(labels: List<String>) -> String:
    return match labels.first():
        Some(label) -> label
        None -> "Untitled"
```

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

## Destructuring bindings

Keyword-free destructuring in statement position shares the
`Type { ... }` surface with named struct literals (compiled in
CI). The pattern must be IRREFUTABLE — bare names, `field: name`
renames, and `..` only; anything refutable belongs in `match`:

```corvid
type Decision:
    refund: Bool
    amount: Float

agent apply(d: Decision) -> Float:
    Decision { refund, amount, .. } = d
    if refund:
        return amount
    return 0.0
```

## What this replaced

Until this slice, the shipped consumption story for Option/Result
was `?` propagation only — an `Err` could be forwarded but never
inspected. `match` closes that gap: branching on WHICH error
occurred, defaulting a `None` at the point of use, and modelling
state machines over sum types are all first-class now.
