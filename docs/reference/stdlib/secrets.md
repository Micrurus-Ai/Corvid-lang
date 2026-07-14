# `std.secrets` — Executing secret access with a replay-safe trace contract

> **Status:** executing. One tool reads environment-backed secrets
> through `secrets::SecretRuntime`; one RuntimeChecked guarantee
> (`secrets.trace_never_carries_value`) governs what reaches traces.

## Quick reference

```corvid
import "./std/secrets" use secret_read

agent main() -> Result<String, String>:
    key = secret_read("ANTHROPIC_API_KEY")?
    if not key.present:
        return Err("set ANTHROPIC_API_KEY")
    return Ok(key.value)
```

### `secret_read(name: String) -> Result<SecretReadEnvelope, String> uses secrets_read`

Reads the environment variable `name`. A MISSING secret is `Ok` with
`present: false` — absence is a modeled state, not an error. Err is
reserved for recoverable failures (empty name, unreadable variable).

## The trace contract (the invention)

Three properties hold together, and each is test-pinned:

1. **The program receives the real value** — `envelope.value` is the
   live secret, because programs need it to work.
2. **The trace never does** — the recorded ToolResult carries a
   redacted copy: `value` replaced by the `<redacted:XY>` marker
   (final two characters — enough to correlate, never enough to
   recover) and `value_redacted: true`.
3. **Replay re-reads the live environment** — Substitute-mode replay
   re-executes the env read instead of substituting (there is
   nothing usable in the trace to substitute). The same
   read-passthrough rule `db_query` follows: env reads are
   process-internal inputs and cannot escape. If the environment
   differs at replay time, the run diverges HONESTLY instead of
   replaying a value the trace never stored.

## Residual channel — explicit non-scope

A secret the program forwards into ANOTHER tool's arguments (an HTTP
header, a request body) is recorded by that tool's own trace events.
The structural fix — an opaque `SecretHandle` value that never
serializes (the `DbHandle` pattern), accepted by consuming surfaces —
is the tracked post-v1.0 deepening. Until then: pass secrets to as
few tools as possible, and prefer host-side injection for transport
credentials.

## Related references

- [`core-semantics.md`](../core-semantics.md) — the
  `secrets.trace_never_carries_value` guarantee row.
- `corvid tour --topic replay-safe-secrets` — runnable demo.
- `crates/corvid-runtime/src/secrets.rs` — `SecretRuntime` + the
  guarantee anchor.
- `std/secrets.cor` — the tool declaration + envelope.
