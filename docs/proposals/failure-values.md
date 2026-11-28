# The fallback sees which failure happened: `else failure -> ...`

**Status:** proposal, 2026-11-28. Shared grammar: needs a working day without objection from frontend development before it is built (AGENTS.md, "Grammar shared with the frontend").
**Owner:** backend (Claude Code, Mac session).
**Roadmap:** "The fallback sees which failure happened" under `try ... else` in [backend-ai-roadmap](../backend-ai-roadmap.md); parent [recoverable failures](recoverable-failures.md).

## The problem

`try ... else` gives a fallback, but the fallback cannot tell what went wrong. A search page should say "Search is taking too long, try again" after a timeout, and "Search is unavailable" when the database is down. An agent should retry later after a rate limit, but not after a refused key. Today the program cannot tell these apart, so every failure gets the same message, and the failure's reason reaches only the trace.

## The design, in a realistic program

```corvid
type Results:
    titles: List<String>
    notice: String

tool find_titles(term: String) -> List<String>

agent search(term: String) -> Results:
    return try Results(find_titles(term), "") timeout 2000 else failure -> Results([], notice_for(failure))

agent notice_for(failure: Failure) -> String:
    return match failure.kind:
        Timeout -> "Search is taking too long. Try again in a moment."
        Unusable -> "Search returned something we could not read."
        _ -> "Search is unavailable right now."
```

`else failure -> <fallback>` names the failure the fallback answers. The plain `else <fallback>` form is unchanged, and remains the short form when the reason does not matter.

### `Failure`

A built-in record, the same on every tier:

| Field | Type | Meaning |
| --- | --- | --- |
| `kind` | `FailureKind` | What failed |
| `call` | `String` | The tool, connector or prompt that failed, by its declared name |
| `message` | `String` | The runtime's message, with any secret the call carried replaced by `<redacted>` |

`FailureKind` is a built-in enum with one variant per kind of outage the one outage rule (`RuntimeError::is_outage`) already distinguishes:

| Variant | Raised when |
| --- | --- |
| `Tool` | A tool or connector failed |
| `Model` | A model call failed |
| `Database` | A table or database operation failed |
| `Timeout` | A `timeout` expired |
| `Unusable` | An answer or reply did not fit its declared type |

A program matches on it like any enum, so a new kind added later is a compile error at every exhaustive `match`, the hardest-work rule's typed failure.

## Rules

- `failure` is bound only inside the fallback, like a pattern binding in a `match` arm.
- The message is the scrubbed form a trace records: a tool that echoed a secret it was given cannot hand the secret's text to a fallback that might show it to a person or a model.
- Replay serves the recorded failure, kind and call included, so a replayed run takes the same branch of the fallback. Failures are recorded with their kind from this slice on. A trace written before it replays a caught failure as `Tool`, and the replay warns once.
- Cost analysis is unchanged: the fallback is one path, as before.

## Simpler spellings considered

- `else (failure) -> ...` adds parentheses that say nothing a first-time reader needs; one name before `->` already reads as "the failure, then".
- `else as failure: ...` reuses `as` with a meaning it does not have elsewhere in Corvid.
- `catch failure -> ...` is a second keyword for the clause `else` already introduces.
- Binding a name implicitly (`else -> failure.message`) hides where `failure` came from, against "no hidden" and "plain words".

`else failure -> expr` reads as "otherwise, given the failure, this", and reuses the `name -> expr` shape of a `match` arm.

## Across the tiers

- **Backend:** a route answers each kind of outage with the right status and message (504 for `Timeout`, 502 for `Tool`), from one place.
- **Frontend:** a screen's loader shows a message that matches what happened, and the published contract carries `FailureKind`, so a client renders the same notices.
- **AI:** an agent's tool call gets the failure as a typed value, so an assistant can decide to retry a `Timeout` and stop on `Unusable`, through declared actions. `Failure` holds no secret, so it is safe to pass to a model.

## Slices

- [ ] FV-1. Parser, checker and interpreter: `else name -> expr`, `Failure` and `FailureKind`; failures recorded with their kind and call, and replayed with them.
- [ ] FV-2. Native code: the held outage carries its kind and call, and the fallback reads them through the runtime.
- [ ] FV-3. The contract publishes `Failure` and `FailureKind` for clients.

## Not in scope

- A failure the program raises itself. `Err` already does that, and stays the way to fail on purpose.
- Bugs and decisions: they are never caught, so they never become a `Failure`.
