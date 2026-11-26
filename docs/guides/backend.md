# Backend basics

Phase 36 ships the backend server tier. `corvid build
--target=server` renders a production-shape axum 0.7 binary from
a Corvid `server` declaration. The source-level surface is the
shipped subset of `server { ... route ... }` shape that the
existing reference apps under `examples/backend/` use; richer
ergonomic forms (custom middleware blocks, per-route effect
constraints, source-level CORS / rate-limit configuration) are
post-v1.0.

## What the rendered backend gives you

- Routes declared in Corvid source as `server` blocks.
- The middleware pipeline: tracing-headers, body-limit
  enforcement (413 on exceed), handler-isolation timeout (504 on
  exceed), graceful shutdown via tokio oneshot.
- Generated `/healthz` + `/readyz` endpoints from runtime state.
- Per-route effect-row enforcement: dangerous routes require the
  caller's auth context to carry the configured permission +
  approve boundary (see [`auth.md`](./auth.md)).

## Declaring a server

The canonical shape, copy-pasted from
`examples/backend/refund_api/src/refund_api.cor`:

```corvid
type RefundRequest:
    order_id: String
    amount: Float
    reason: String

type RefundResponse:
    receipt_id: String
    status: String

type RefundStatus:
    service: String
    mode: String

effect transfer_money:
    cost: $0.05
    trust: human_required
    reversible: false
    data: financial

tool issue_refund(req: RefundRequest) -> String dangerous uses transfer_money

agent approve_refund(req: RefundRequest) -> RefundResponse uses transfer_money:
    approve IssueRefund(req)
    receipt_id = issue_refund(req)
    return RefundResponse(receipt_id, "approved")

agent refund_status() -> RefundStatus:
    return RefundStatus("refund_api", "contract")

identity users:
    provider google
    provisioning:
        first_login: invited
        tenant: from_invitation
    roles:
        finance_reviewer: "approvals.decide"

server refund_api:
    route GET "/status" -> json RefundStatus:
        return refund_status()
    @approval(role: "finance_reviewer", risk: "financial_refund", data: "financial", expires_ms: 600000, max_cost_usd: $2500.0, irreversible: true)
    route POST "/refunds" body RefundRequest -> json RefundResponse uses transfer_money requires authenticated:
        return approve_refund(body)
```

Notes on the shape:

- `route GET "/status" -> json RefundStatus: ...` — `GET` is the
  HTTP method, `/status` is the path, `json RefundStatus` is the
  return content-type + return type. The body is an inline
  expression that returns the typed value.
- `route POST "/refunds" body RefundRequest -> json RefundResponse uses transfer_money: ...`
  — `body RefundRequest` parses the request body as the typed
  shape; `uses transfer_money` declares the effect row the route
  carries; the body block can reference the parsed body as
  `body`.
- `route GET "/refunds/{refund: Int}" -> json RefundStatus: ...` — a
  `{...}` part of the path is a parameter, read as `path.refund`.
  `{refund: Int}` makes it an `Int`; `Float`, `Bool`, `Decimal`,
  `Instant`, `Duration`, a time zone, currency, locale, `Bytes`, or a
  type of plain variants (`{status: Status}`, read by the variant's
  name) work the same way. `{refund}` alone is a `String`. A request
  whose segment is not its type is answered with a 400 naming the
  parameter before the handler runs. The contract and the generated
  clients carry the type, and `live` paths read parameters the same way.
- `query Search` reads the query string into the record `Search`, with
  the same types per field, plus `Option` (left out of the URL when
  absent, `None` in the handler) and `List` (the name repeated:
  `?status=Open&status=Closed`). A value is read exactly as it would be
  in a JSON body, so `?total=19.90` keeps its digits.

The typechecker enforces:

- The declared `uses` row matches the body's actual effect
  composition (otherwise the route fails to compile).
- A route that can reach `approve` is authenticated and declares its
  complete reviewer/risk/data/expiry/cost/reversibility policy.
- Dangerous tools called from the body have a matching `approve`
  in lexical scope (otherwise the route fails to compile under
  `approval.dangerous_call_requires_token`).
- Body types are `serde`-roundtrippable (otherwise the codegen
  fails).

### JSON bodies and replies

A `body` route reads ordinary JSON, and a `json` route replies with
it, following the schema published at `/openapi.json`:

| Corvid | JSON |
| --- | --- |
| `Option<T>` field | the value, `null`, or the key left out |
| `Result<T, E>` | `{"tag": "ok", "ok": …}` or `{"tag": "err", "err": …}`; `tag` is optional in a request |
| sum type | `{"tag": "variant", "type": "Outcome", "variant": "Changed", "fields": [2, "Dune"]}`; only `variant` and `fields` are required in a request |
| `Grounded<T>` | `{"value": …, "sources": [...]}` in a reply; it cannot be sent in a request |

A request body is checked against its declared type before the
handler runs. A key the type does not declare, a value of the wrong
kind, a missing field or a refinement violation returns HTTP 400 with
the location of the first problem:

```json
{
  "error": "invalid_body",
  "path": "lines[0].quantity",
  "expected": "Int",
  "found": "a string",
  "detail": "at `lines[0].quantity`: expected Int, found a string"
}
```

Route types do not need to be `public` to appear in the published
schema. The [JSON reference](../reference/stdlib/json.md#public-json-mapping)
defines the complete mapping.

## Building and running

```sh
corvid build src/main.cor --target=server
cd target/server
cargo run --release
```

The rendered server listens on `0.0.0.0:8080` by default;
override with `PORT=9090 cargo run`.

The output tree (cargo-rendered axum project) lives under
`target/server/`. Each reference app under `examples/backend/`
ships its own pre-rendered server tree so operators can inspect
the shape before running `corvid build` themselves.

## Operational concerns

| Concern | Where it lives |
|---|---|
| Auth + sessions + API keys | [`auth.md`](./auth.md) — `corvid auth migrate` + `corvid auth keys issue/revoke/rotate` |
| Persistence + migrations | [`persistence.md`](./persistence.md) — `corvid migrate up/down/status` |
| Background jobs + cron | [`jobs.md`](./jobs.md) — `corvid jobs run --source <path>.cor --workers=N` |
| Observability + traces + evals | [`observability.md`](./observability.md) — `corvid observe list/show/explain` |
| Connectors (Gmail / Slack / etc.) | [`connectors.md`](./connectors.md) |
| Deploy + signed attestation | the deploy + release CLI; see `corvid deploy package` + `corvid release` |

## Scope boundaries — out of scope for v1.0

- Multi-process worker pool (current shape: one server process
  per binary; horizontal scaling lives at the operator's
  load-balancer tier).
- Custom middleware injection from Corvid source (the shipped
  middleware pipeline is the renderer-fixed set; per-app custom
  middleware is post-v1.0).
- Per-route source-level CORS / rate-limit / body-limit
  configuration (today these are middleware-pipeline-fixed; the
  per-route ergonomic surface is post-v1.0).
- WebSockets, server-sent events, long-poll HTTP streaming (the
  v1.0 backend serves request-response HTTP; streaming
  primitives are post-v1.0).

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| Dangerous route handler requires approve | `approval.dangerous_call_requires_token` | Static | `crates/corvid-types/src/checker/` |
| Effect row propagates through route body | `effect_row.body_completeness` | Static | `crates/corvid-types/src/effects.rs` |
| Cross-tenant value refused at compile time | `tenant.cross_tenant_compile_error` | OutOfScope | gated on `35V2-P39-I-post-v1.0-auth-syntax-sugar` |
| Routes carry signed claim manifest | `claim.audit_runnable_artifacts` (43V) | RuntimeChecked | `crates/corvid-cli/src/claim_cmd.rs` |
| Lineage IDs per request | `observability.lineage_completeness` | RuntimeChecked | `crates/corvid-runtime/src/lineage.rs` |
