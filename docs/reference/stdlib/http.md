# `std.http` — Executing HTTP-client surface

> **Status:** Phase 33S2 (closed 2026-06-09). Two tools execute
> real HTTP requests; three RuntimeChecked guarantees govern
> their behavior. Earlier slices shipped the envelope types and
> envelope-builder agents only; 33S2 promotes the module to
> executing. Since the stdlib Result-envelope migration, both
> tools return `Result<HttpResponseEnvelope, String>` —
> recoverable failures are Err values.

## Quick reference

```corvid
import "./std/http" use http_get, http_post_json

agent fetch_status(url: String) -> Result<Int, String>:
    response = http_get(url)?
    return Ok(response.status)

agent publish_event(url: String, event_json: String) -> Result<Int, String>:
    response = http_post_json(url, event_json)?
    return Ok(response.status)
```

When this agent runs through `corvid run`, both calls flow through
the configured `[http] allow` allowlist (after the always-on SSRF
block) and return a `Result`-wrapped `HttpResponseEnvelope`. A
policy refusal (SSRF block, missing/unlisted allowlist entry) or a
transport failure (DNS, connect, timeout) is an `Err(String)`
VALUE naming the cause. **An error HTTP STATUS (4xx/5xx) is still
`Ok`** — the request succeeded at the transport layer; inspect
`response.status` (or `http_ok`) to branch on it:

```corvid
public type HttpResponseEnvelope:
    status: Int
    body: String
    attempts: Int
    elapsed_ms: Int
    effect_meta: EffectEnvelope
```

## The two tools

### `http_get(url: String) -> Result<HttpResponseEnvelope, String> uses http_egress_get`

Performs an HTTP GET. The `http_egress_get` effect carries
`io_source: net.egress` and `reversible: true` — composes
correctly with `@reversible` constraints elsewhere in the call
graph.

### `http_post_json(url: String, body: String) -> Result<HttpResponseEnvelope, String> uses http_egress_post`

Performs an HTTP POST with the supplied UTF-8 body and
`Content-Type: application/json`. The `http_egress_post` effect
carries `io_source: net.egress` and `reversible: false` so callers
can reason about side effects in trace + budget constraints.

For composing requests with custom timeouts, retry policies, or
header sets, the envelope-builder agents in `std/http.cor`
(`http_request_get`, `http_request_post_json`, `http_with_retry`,
`http_with_timeout`, `http_ok`) construct `HttpRequestEnvelope`
values — those are still pure agents that BUILD a request; the
two `tool` declarations above PERFORM the request.

## Security model — `[http] allow` + always-on SSRF

The executing HTTP-client surface combines TWO independent
properties on every URL:

### 1. Structural SSRF block — **always on**

Every URL whose host lexically resolves to a private range is
refused, regardless of allowlist contents. The hosts blocked:

| range | examples |
|---|---|
| RFC1918 private IPv4 | `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` |
| Loopback IPv4 | `127.0.0.0/8` |
| Link-local IPv4 | `169.254.0.0/16` |
| Unspecified IPv4 | `0.0.0.0/8` |
| Loopback IPv6 | `::1`, `[::1]:port` |
| Link-local IPv6 | `fe80::/10` |
| ULA IPv6 | `fc00::/7` |
| Unspecified IPv6 | `::` |
| DNS alias | `localhost` (case-insensitive) |

A misconfigured allowlist that contains `127.0.0.1` STILL gets
refused — the SSRF block is the security FLOOR, not a
configurable setting. There is no `--allow-private-egress` flag
and no env override; the rule lives in
`HttpEgressPolicy::check`'s structural-properties section and
can only be relaxed by patching the runtime source.

### 2. `[http] allow` allowlist — **required, fail-closed**

The executing HTTP-client surface refuses to operate without an
explicit `[http] allow` list declared in `corvid.toml`. The
`corvid new`-scaffolded project carries `[http] allow = []` so
the security boundary is visible from day one; users opt in to
each host explicitly.

#### corvid.toml

```toml
[http]
allow = ["api.example.com", "api.anthropic.com"]
```

Comparison is exact-host, case-insensitive. No subdomain
wildcards (yet — `*.example.com` is a planned future-scope
extension, not in 33S2).

#### Env override

```sh
CORVID_HTTP_ALLOW=api.example.com,api.anthropic.com corvid run src/main.cor
```

Comma-separated host list; whitespace trimmed; empty entries
stripped. Precedence: `CORVID_HTTP_ALLOW` wins over
`corvid.toml`; whitespace-only env values fall back to
`corvid.toml` (so `CORVID_HTTP_ALLOW=" "` doesn't accidentally
clear an allowlist).

### What's rejected

Every rejection below arrives as an `Err` VALUE the program
observes — the boundary is just as hard, but the caller keeps
control (retry, fall back, log, or `?`-propagate).

- **Missing `[http] allow`** (no section, empty list, or unset
  env): every call fails closed with an Err diagnostic naming the
  missing config + the `CORVID_HTTP_ALLOW` env pathway + the
  fail-closed contract from the 33S0 security model.
- **Host not in allowlist**: the Err diagnostic names the
  requested URL, the parsed host, and the configured allowlist
  so the operator can see exactly which entry is missing.
- **Private / loopback / link-local URL**: the SSRF block fires
  before the allowlist is consulted; the Err diagnostic names
  "SSRF" and "structural property" so the operator understands
  the floor.

## Determinism

Both tools are non-deterministic. The checker rejects calls
inside a `@deterministic` body — this is the existing
`decl_replayability.rs::is_deterministic_compatible` rule that
treats every tool call as non-deterministic by declaration kind.
Programs that need both deterministic logic AND HTTP egress must
factor egress into a separate, non-deterministic agent.

## Replay quarantine

When a Corvid program runs in Substitute-mode replay (replaying a
recorded trace), the executing HTTP-client surface honors the
same quarantine the existing `HttpClient::quarantine` hook
provides. Two layers:

1. **Low-level `HttpClient::send`** returns
   `QuarantineViolation { surface: "http", .. }` if called during
   replay. The detail names the method + URL so the operator can
   see exactly what was refused.
2. **Dispatch path**
   (`Runtime::call_tool("http_get" | "http_post_json", ...)`) goes
   through the replay-substitution path FIRST. POST and GET both
   substitute from the recorded trace OR diverge — they never
   reach the live network.

Together the two layers prove the network is provably untouched
during replay, regardless of allowlist contents (a fully-
configured allowlist does not bypass the quarantine).

## Guarantees

Three RuntimeChecked rows in the canonical guarantee registry
([`docs/reference/core-semantics.md`](../core-semantics.md)):

| id | class | enforces |
|---|---|---|
| `io_source.http_ssrf_structural_block` | RuntimeChecked | Private / loopback / link-local hosts refused regardless of allowlist. Structural floor. |
| `io_source.http_allowlist_enforcement` | RuntimeChecked | URL host must appear in `[http] allow`; missing allowlist fails closed. |
| `io_source.http_quarantine_on_replay` | RuntimeChecked | POST / GET dispatch during replay substitutes from trace or diverges; never reaches the network. |

The `@deterministic`-rejection property is covered by the
existing `replay.deterministic_pure_path` guarantee — it applies
to all tool calls regardless of effect.

## Worked example — webhook fan-out with allowlist scoping

```corvid
import "./std/http" use http_post_json, http_ok

agent notify_webhook(url: String, event_json: String) -> Result<Bool, String>:
    response = http_post_json(url, event_json)?
    return Ok(http_ok(response))
```

With `corvid.toml`:

```toml
[http]
allow = ["hooks.example.com", "audit.example.com"]
```

`notify_webhook("https://hooks.example.com/new-order", "...")`
succeeds; `notify_webhook("https://hooks.attacker.example/...", "...")`
returns `Err` from the dispatch boundary with a diagnostic naming
the unlisted host and the configured allowlist. Even if a future
configuration mistake adds `127.0.0.1` to the allowlist, calls to
loopback are still refused by the structural SSRF block.

## Related references

- [`core-semantics.md`](../core-semantics.md) — full guarantee
  registry, including the three `io_source.http_*` rows.
- [`inventions.md`](../inventions.md) — invention proof matrix.
- `corvid tour --topic http-client` — runnable demo (offline; the
  topic source compiles through the driver's tour-compile gate).
- `crates/corvid-runtime/src/http.rs` — `HttpEgressPolicy` source
  + inline guarantee anchors (`GUARANTEE_ID_IO_SOURCE_HTTP_*`).
- `crates/corvid-driver/src/run.rs::load_http_egress_policy` —
  CLI loader: env > corvid.toml > unset.
- `std/http.cor` — the tool declarations + envelope types.
