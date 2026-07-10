# Errors and Result

Every `corvid`-tagged block compiles through the real driver in CI;
Planned blocks name the slice that implements them.

## The two kinds of failure

Corvid distinguishes:

- **Recoverable failures** — represented as `Result<T, E>`. Callers
  propagate with `?` or retry with `try … on error retry …`.
- **Unrecoverable failures** — runtime traps (integer overflow,
  division by zero, out-of-range indexing). They abort the agent run
  with a typed error, and replay reproduces them deterministically.

## `Result<T, E>` and `?`

Construct with `Ok(x)` / `Err(e)`. Postfix `?` unwraps an `Ok` or
propagates the `Err` early to a caller whose return type is itself a
`Result`:

```corvid
agent check_amount(amount: Float) -> Result<Float, String>:
    if amount > 100.0:
        return Err("amount exceeds the auto-approve limit")
    return Ok(amount)

agent apply_fee(amount: Float) -> Result<Float, String>:
    approved = check_amount(amount)?
    return Ok(approved + 2.5)
```

The executing stdlib surfaces use the same envelope: `json_parse`
returns `Result<JsonValue, String>`, typed decoders return
`Result<T, String>`, and malformed input flows through `?` instead of
crashing (see [Talking to the outside world](./18-talking-to-the-outside-world.md)).

## Retrying a fallible call

The `try … retry` expression wraps any call with bounded retries and
mandatory backoff (base delay in milliseconds):

`try … retry` applies only to `Result`- or `Option`-typed expressions:

```corvid
effect flaky_effect:
    cost: $0.001
    reversible: true

tool fetch_remote(id: String) -> Result<String, String> uses flaky_effect

agent robust(id: String) -> Result<String, String>:
    value = try fetch_remote(id) on error retry 3 times backoff exponential 250
    return value
```

## Branching on errors

`match` destructures a Result — branching on WHICH error occurred
is first-class (compiled in CI):

```corvid
agent check_amount(amount: Float) -> Result<Float, String>:
    if amount > 100.0:
        return Err("over limit")
    return Ok(amount)

agent describe(amount: Float) -> String:
    return match check_amount(amount):
        Ok(v) -> "approved: " + v.to_string()
        Err(msg) -> "rejected: " + msg
```

> **Planned — the `unwrap_or` / `is_ok` / `map_err` method
> shorthands land in slice 45l:**

```corvid-planned
x = parse_int(s).unwrap_or(0)
```

## Typed error enums

Sum types and `match` make typed error enums fully usable — declare
the failure shapes, return them in `Err`, and branch on exactly
which one occurred (compiled in CI). The stdlib's `Result<_,
String>` convention predates these and migrates surface-by-surface.

```corvid
type RefundError:
    | InvalidAmount(amount: Float)
    | CustomerNotFound(id: String)
    | OverDailyLimit

agent refund_check(amount: Float) -> Result<Float, RefundError>:
    if amount <= 0.0:
        return Err(InvalidAmount(amount))
    if amount > 1000.0:
        return Err(OverDailyLimit)
    return Ok(amount)

agent explain(amount: Float) -> String:
    return match refund_check(amount):
        Ok(v) -> "refunding " + v.to_string()
        Err(InvalidAmount(a)) -> "invalid amount: " + a.to_string()
        Err(CustomerNotFound(id)) -> "no such customer: " + id
        Err(_) -> "over the daily limit"
```

Exhaustiveness over NESTED patterns is conservative in v1: arms
like `Err(InvalidAmount(_))` don't compose into full `Err`
coverage, so a nested match ends with an `Err(_)` catch-all.

## Runtime traps

Integer overflow, division by zero, and modulo by zero are checked in
every build mode and trap with a typed runtime error — there is no
saturating or wrapping mode. Out-of-range list indexing traps at
runtime. Traps abort the agent run; the durable job runner records
the trap and the trace, and replay reproduces it deterministically.

Use `Result` for everything the caller could reasonably handle: LLM
output of unexpected shape, network failures, malformed user input.
Traps are for states the program must never reach.
