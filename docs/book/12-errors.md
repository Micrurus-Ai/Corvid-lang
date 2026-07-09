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

> **Planned.** Inspecting WHICH error occurred at the point of use
> needs `match` (slice 45i) or the `Result` helper methods
> `is_ok` / `unwrap_or` / `map_err` (slice 45l). Today an `Err` can
> be propagated (`?`), retried (`try … retry`), or compared whole
> (`==`) — it cannot be destructured. This is a known blocker the
> Language completeness track closes.

```corvid-planned
match fetch_user("u1"):
    Ok(user)   -> render(user)
    Err(other) -> render_error(other)

x = parse_int(s).unwrap_or(0)
```

## Typed error enums

> **Planned — slice 45h (sum types) + 45i (match).** Today `E` is a
> `String` in practice (the stdlib convention), because a typed error
> variant could be constructed but never inspected. Sum-type error
> enums become useful the moment `match` lands:

```corvid-planned
type RefundError:
    | InvalidAmount(amount: Float)
    | CustomerNotFound(id: String)
    | PaymentFailed(provider_message: String)
    | OverDailyLimit
```

## Runtime traps

Integer overflow, division by zero, and modulo by zero are checked in
every build mode and trap with a typed runtime error — there is no
saturating or wrapping mode. Out-of-range list indexing traps at
runtime. Traps abort the agent run; the durable job runner records
the trap and the trace, and replay reproduces it deterministically.

Use `Result` for everything the caller could reasonably handle: LLM
output of unexpected shape, network failures, malformed user input.
Traps are for states the program must never reach.
