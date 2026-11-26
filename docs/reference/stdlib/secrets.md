# `std.secrets` — Executing secret access with a replay-safe trace contract

> **Status:** executing. Two tools read environment-backed secrets
> through `secrets::SecretRuntime`; one RuntimeChecked guarantee
> (`secrets.trace_never_carries_value`) governs what reaches traces.

## Quick reference

```corvid
import "std:secrets" use secret

tool call_service(key: Secret) -> Bool

agent main() -> Result<Bool, String>:
    key = secret("EXAMPLE_SERVICE_KEY")?
    return Ok(call_service(key))
```

### `secret(name: String) -> Result<Secret, String> uses secrets_read`

Reads the environment variable `name` as a `Secret`: text the program
can compare with `==` (in constant time) and pass to the tools that
need it, and that the compiler keeps out of every place its text could
leave the program. An empty name, an unreadable variable and a missing
variable are `Err`, because there is no secret to return. A tool that
takes a `Secret` receives the text, and every record of the call holds
`<redacted>` in that position. [Cryptography: secrets](../cryptography.md#secrets)
has the rules.

### `secret_read(name: String) -> Result<SecretReadEnvelope, String> uses secrets_read`

Reads the environment variable `name` and returns its text as a plain
`String` in the envelope. Prefer `secret(name)`: a `String` can reach a
log, a prompt or another tool's trace, which a `Secret` cannot. A MISSING secret is `Ok` with
`present: false` — absence is a modeled state, not an error. Err is
reserved for recoverable failures (empty name, unreadable variable).

### Constructing example values

`secret_present(name: String, value: String)` and `secret_missing(name: String)` both return a `SecretReadEnvelope` without reading the environment. The first keeps the supplied value and sets `present` to true. The second sets `present` to false and uses an empty string for `value`. Use `present` to distinguish a missing value from a present empty string.

## The trace contract (the invention)

Three properties hold together, and each is test-pinned:

1. **The program receives the real value** — `envelope.value` is the
   live secret, because programs need it to work.
2. **The trace never does** — the recorded ToolResult carries a
   redacted copy. For `secret_read`, `value` is replaced by the
   `<redacted:XY>` marker (final two characters — enough to
   correlate, never enough to recover) and `value_redacted: true`.
   For `secret`, the result is `<redacted>` with no characters kept.
3. **Replay re-reads the live environment** — Substitute-mode replay
   re-executes the env read instead of substituting (there is
   nothing usable in the trace to substitute). The same
   read-passthrough rule `db_query` follows: env reads are
   process-internal inputs and cannot escape. If the environment
   differs at replay time, the run diverges HONESTLY instead of
   replaying a value the trace never stored. Replay steps past the
   recorded read, so the calls after it stay aligned with the trace.

## Forwarding a secret to another tool

A secret read with `secret` and passed into another tool's arguments
is recorded as `<redacted>` in that tool's trace events, and replay
matches the redacted argument against whatever secret it re-reads.
The text of a secret read with `secret_read` is an ordinary `String`,
so a program that forwards it records it in that tool's events; use
`secret` for any value that must stay out of traces.

## Related references

- [`core-semantics.md`](../core-semantics.md) — the
  `secrets.trace_never_carries_value` guarantee row.
- `corvid tour --topic secrets` and `corvid tour --topic replay-safe-secrets` — runnable demos.
- [`cryptography.md`](../cryptography.md#secrets) — the `Secret` type.
- `crates/corvid-runtime/src/secrets.rs` — `SecretRuntime` + the
  guarantee anchor.
- `std/secrets.cor` — the tool declaration + envelope.
