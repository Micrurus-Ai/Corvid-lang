# Backend basics

## What the rendered backend gives you

`corvid build --target=server` produces a Cargo project that compiles
to a production-shape axum 0.7 binary with:

- Routes — declared in Corvid source as `server` blocks.
- Middleware pipeline — auth, rate-limit, tracing-headers, CORS,
  compression, request logging — wired in declared order.
- Graceful shutdown via tokio oneshot.
- Handler isolation: each request runs in a subprocess with a
  per-handler timeout (504 on exceed).
- Body-limit enforcement (413 on exceed).
- `/healthz`, `/readyz`, `/metrics` endpoints generated from runtime
  state.

## Declaring a server

```corvid
server refund_api:
    route "/refund"        -> handle_refund
    route "/refund/status" -> handle_status
    route "/healthz"       -> healthz       # auto-generated, override-able
```

## Writing a handler

```corvid
struct RefundRequest:
    customer_id: String
    amount: Float

struct RefundResponse:
    ok: Bool
    receipt_id: String

@budget($0.50)
@max_wall_time(10s)
agent handle_refund(req: RefundRequest) -> RefundResponse uses http_effect, refund_effect:
    approve Refund(req.amount, req.customer_id)
    let receipt = refund(req.amount, req.customer_id)
    return RefundResponse { ok: true, receipt_id: receipt.id }
```

The handler agent's effect row drives middleware behavior:

- `refund_effect` carries `trust: supervisor_required`, so the
  middleware enforces the host-side approval check.
- `@max_wall_time` sets the per-request handler-isolation timeout.

## Building and running

```sh
corvid build src/main.cor --target=server
cd target/server
cargo run --release
```

The server listens on `0.0.0.0:8080` by default; override with
`PORT=9090 cargo run`.

## Configuration

```toml
# corvid.toml
[server]
port = 8080
body_limit_bytes = 1048576           # 1 MiB
handler_timeout_seconds = 30
graceful_shutdown_seconds = 10

[server.cors]
allowed_origins = ["https://app.example.com"]

[server.rate_limit]
requests_per_minute = 1000
burst = 100

[server.auth]
jwt_jwks_url = "https://auth.example.com/.well-known/jwks.json"
required_claims = ["sub", "tenant"]
```

## Observability

Every request emits structured logs and OTel spans. The `/metrics`
endpoint exposes Prometheus-format counters: requests by route, by
status, by effect-row, plus handler-isolation timeouts and body-limit
hits.

See **[Observability](/docs/observability)**.

## Scope boundaries

Out of scope for v1.0:

- Multi-process worker pool (current shape: one server process,
  subprocess-spawned handlers per request).
- Custom middleware injection from Corvid source (current set is
  fixed by the renderer).
