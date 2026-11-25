# Making HTTP requests

> **Status:** B05 (2026-11-25). Every HTTP request goes through a
> `connector`. The `http_get` and `http_post_json` tools this page
> once documented are replaced by connectors; their three
> RuntimeChecked guarantees now govern every connector request.
> `std/http.cor` keeps the request and response envelope types, the
> pure envelope-builder agents, and the built-in network effects
> `http_egress_get` and `http_egress_post`.

## Quick reference

```corvid
effect http_read:
    cost: $0.0

type FetchError:
    | Unreachable(reason: String)

type Event:
    kind: String

type Receipt:
    id: String

connector web:
    hosts: allowed_hosts
    timeout: 10s
    redirects: allowed_hosts
    modes: [real, replay]
    operation page(url: String) -> Result<Response<String>, FetchError> uses http_read:
        GET url
        response: text
        on failure -> Unreachable
    operation publish(url: String, event: Event) -> Result<Receipt, FetchError> dangerous uses http_egress_post:
        POST url body event
        on failure -> Unreachable

agent status_of(url: String) -> Int uses http_read:
    return match page(url):
        Ok(answer) -> answer.status()
        Err(e) -> 0
```

A connector calling one API declares `base_url: "https://api.example.com"` and gives each operation a path; `hosts: allowed_hosts`, as here, lets each call name its URL. Either way the request runs through the always-on SSRF block and the `[http] allow` list below, takes at most its declared `timeout`, and follows only the redirects its `redirects:` policy allows. With `on failure -> Unreachable`, a refused URL, a timeout, a network error, an unmapped error status or a body that does not match the declared type is an `Err(Unreachable(reason))` value; without it, the failure stops the program. `response: text` reads a page as a `String` instead of JSON, and `Response<Body>` gives the status and headers. The full grammar is in [connector declarations](../grammar.md#connector-declarations); the design is in the [HTTP client proposal](../../proposals/http-client.md).

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

A connector operation is a tool, so it is non-deterministic. The
checker rejects calls inside a `@deterministic` body — the existing
`decl_replayability.rs::is_deterministic_compatible` rule that
treats every tool call as non-deterministic by declaration kind.
Programs that need both deterministic logic and HTTP egress factor
the egress into a separate, non-deterministic agent.

## Replay quarantine

When a Corvid program runs in Substitute-mode replay (replaying a
recorded trace), every request honors the quarantine the
`HttpClient::quarantine` hook provides. Two layers:

1. **Low-level `HttpClient::send`** returns
   `QuarantineViolation { surface: "http", .. }` if called during
   replay. The detail names the method + URL so the operator can
   see exactly what was refused.
2. **Dispatch path**: a connector operation's call goes through the
   replay-substitution path FIRST. Reads and writes both substitute
   from the recorded trace or diverge; they never reach the live
   network.

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
| `io_source.http_quarantine_on_replay` | RuntimeChecked | Connector calls during replay substitute from the trace or diverge; never reach the network. |

The `@deterministic`-rejection property is covered by the
existing `replay.deterministic_pure_path` guarantee — it applies
to all tool calls regardless of effect.

## Worked example — webhook fan-out with allowlist scoping

```corvid
type Event:
    kind: String

type Receipt:
    id: String

type DeliveryError:
    | Undelivered(reason: String)

connector hooks:
    hosts: allowed_hosts
    timeout: 5s
    redirects: never
    modes: [real, replay]
    operation notify(url: String, event: Event) -> Result<Receipt, DeliveryError> dangerous uses http_egress_post:
        POST url body event
        on failure -> Undelivered
```

With `corvid.toml`:

```toml
[http]
allow = ["hooks.example.com", "audit.example.com"]
```

`notify("https://hooks.example.com/new-order", event)` succeeds;
`notify("https://hooks.attacker.example/...", event)` returns
`Err(Undelivered(reason))` whose reason names the unlisted host and
the configured allowlist. Even if a configuration mistake adds
`127.0.0.1` to the allowlist, calls to loopback are still refused by
the structural SSRF block.

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
- `std/http.cor` — the envelope types, envelope-builder agents and
  built-in network effects.


### Deadlines in a served route

Add the destination host to `[http] allow` in the project manifest. Under `redirects: allowed_hosts`, a redirect destination is checked against that policy too, and the trace records each attempt and hop with its elapsed time.

`return try wait_for_reply() timeout 300` gives the call a 300 ms deadline. If it expires, the server returns HTTP 504 with error code `handler_timeout` and cancels the pending request. Closing the client connection also cancels an outstanding request. For a stream, the start deadline covers waiting for the first chunk.
