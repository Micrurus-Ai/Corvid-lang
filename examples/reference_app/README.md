# Reference application — the Phase 52 continuous fixture

One Corvid file that grows into a complete AI backend, one slice at a
time. It is the acceptance target for **Phase 52 — The Complete
Application Runtime**: every slice extends [src/main.cor](src/main.cor)
and proves its new runtime capability against it, with no special
cases. When Contract Closure is enforced (slice 52b), this app fails to
start the moment a runtime path lags the contract it advertises.

## Current surface (slices 52a–52c-2)

A pure in-memory orders API exercising every HTTP shape the runtime
executes end-to-end through the interpreter:

| Route                     | Shape             | Notes                                   |
| ------------------------- | ----------------- | --------------------------------------- |
| `GET /orders/{id}`        | path parameter    | `path.id`                               |
| `GET /orders`             | typed query       | `query.status`, `query.limit`           |
| `POST /orders`            | typed body        | `body.item`, `body.quantity`            |
| `GET /orders/activity`    | `Stream<T>` → SSE | one `data:` event per yield             |
| `GET /orders/page`        | `Page<Item>`      | `{items, next_cursor, has_more}` envelope |
| `POST /orders/import`     | `Upload<Csv>`     | multipart; `body.filename()`/`size()`   |

Run it:

```
corvid serve examples/reference_app/src/main.cor --listen 127.0.0.1:8531

curl -s localhost:8531/orders/order-42
# {"id":"order-42","status":"open","total":42.0}

curl -s 'localhost:8531/orders?status=open&limit=5'
# {"count":5,"orders":[{"id":"order-1","status":"open"}]}

curl -s -X POST localhost:8531/orders -d '{"item":"widget","quantity":3}'
# {"accepted":true,"id":"order-new"}
```

Malformed boundary input is a structured `400`, never a `500`:

```
curl -s 'localhost:8531/orders?status=open&limit=notanumber'
# {"detail":"`notanumber` is not an Int","error":"invalid_query"}
```

The live backend also serves its own contract at
`/.well-known/corvid` and `/openapi.json`.
