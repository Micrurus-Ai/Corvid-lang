# Errors and Result

## The two kinds of failure

Corvid distinguishes:

- **Recoverable failures** — represented as `Result<T, E>`. Callers
  can match on the result, propagate with `?`, or convert.
- **Unrecoverable failures** — panics. They abort the agent run with
  a typed crash report. The replay tool reproduces the panic
  deterministically.

## `Result<T, E>` and `?`

```corvid
fn fetch_user(id: String) -> Result<User, FetchError> uses http_effect:
    let resp = http.get("/users/" + id)?
    if resp.status != 200:
        return Err(FetchError::Status(resp.status))
    return Ok(resp.parse_json::<User>()?)
```

`?` propagates the `Err` early. The caller's return type must be a
`Result` whose error type can be constructed from the propagated
error. Conversion rules use the explicit `From` trait — there are no
implicit conversions.

## Match for branching

```corvid
match fetch_user("u1"):
    Ok(user)              -> render(user)
    Err(FetchError::Status(404)) -> render_not_found()
    Err(other)            -> render_error(other)
```

## When to panic

Panics are appropriate when:

- The program has reached a state it must never reach (precondition
  violation in pure code).
- A required environment resource is missing at startup (`expect()`
  with a message is fine).

Panics are inappropriate when:

- An LLM call returns an unexpected shape (use `Result<T, E>`).
- A network call fails (use `Result<T, E>`).
- User input is malformed (use `Result<T, E>`).

## `expect()` and `unwrap()`

```corvid
let x: Int = parse_int("42").unwrap()              # panics on Err
let y: Int = parse_int(s).expect("config corrupt") # panics with message
```

Use `expect()` over `unwrap()` — it forces you to write a message
that the auditor reads when the panic fires.

## Panic safety in agents

When an agent panics, the durable job runner records the panic, the
trace, and (if `@retry` is set) requeues the job. Idempotency keys
prevent duplicate side effects. The `@max_steps` annotation prevents
infinite-panic-and-retry loops.

## Error types

A common pattern:

```corvid
type RefundError:
    | InvalidAmount(amount: Float)
    | CustomerNotFound(id: String)
    | PaymentFailed(provider_message: String)
    | OverDailyLimit

fn refund_logic(amount: Float, id: String) -> Result<Unit, RefundError>:
    if amount <= 0.0:
        return Err(RefundError::InvalidAmount(amount))
    ...
```

`corvid audit` prints which error variants are reachable from each
agent's call graph.
