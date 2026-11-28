# Verified webhooks (limits plan row 8, B16)

**Status:** built 2026-11-27, W1 to W3 together. Frontend development agreed on 2026-11-27 (review item 3), asking that the contract carry a verified-route marker and that generated clients, the console and assistant discovery filter on it. A `verified` clause the server did not enforce would be a declared policy that does not run, so the three slices shipped as one.
**Owner:** backend (Claude Code). No frontend half: a webhook is a provider calling the backend.
**Roadmap:** [B16](../backend-ai-roadmap.md) verified webhooks; [limits plan](removing-limits.md) row 8.

## The problem

Payment and SaaS providers (Stripe, GitHub, Svix-style Standard Webhooks, Shopify) call an application with a signed request: an HMAC of the exact body bytes, usually with a timestamp to stop replays. Corvid has the cryptography (HMAC-SHA256 with a `Secret` key, constant-time `Bytes` comparison), and a test verifies a recorded Stripe and Standard Webhooks request in Corvid code. A route cannot use it safely today:

1. **The raw body is gone.** A route's `body T` is decoded JSON; the signature is over the bytes the provider sent, and re-encoding the decoded value does not reproduce them.
2. **Verification is optional code.** A route that forgets to call the check, checks after a write, or compares with `==` on text accepts forged events. Nothing in the source says the route is a webhook.
3. **The replay window is a hidden choice.** A timestamp tolerance is a security policy; written in handler code it is easy to leave out.

## The proposal

```corvid
type PaymentEvent:
    id: String
    type: String

server payments:
    route POST "/webhooks/stripe" body PaymentEvent verified stripe(secret: "STRIPE_WEBHOOK_SECRET", within: 5 minutes) -> json Nothing:
        return record_payment(body)
```

`verified <scheme>(secret: "<ENV NAME>", within: <duration>)` is one clause on a route. Before the body is decoded, `corvid serve`:

1. reads the raw bytes and the scheme's headers;
2. refuses a timestamp outside `within` of the server clock (`401 webhook_expired`);
3. computes the scheme's HMAC over the scheme's signed payload with the named secret, and compares in constant time with each signature the header carries, so a provider rolling its key keeps working (`401 webhook_signature_invalid` when none matches);
4. only then decodes the body as `PaymentEvent` and runs the handler, which never sees an unverified request.

The handler's `body` is the decoded type as for any route. A route that needs the exact bytes writes `body Bytes`, and receives them unchanged.

| Scheme | Signed payload | Header |
| --- | --- | --- |
| `stripe` | `timestamp + "." + body` | `Stripe-Signature: t=...,v1=...` (several `v1`) |
| `standard_webhooks` | `id + "." + timestamp + "." + body` | `webhook-id`, `webhook-timestamp`, `webhook-signature: v1,<base64> ...`; the secret's `whsec_` prefix and base64 are read as the spec says |
| `github` | `body` (no timestamp) | `X-Hub-Signature-256: sha256=...` |

GitHub signs no timestamp, so `within` is refused for `github` and the route must say `replays: accepted` instead, naming the risk the source takes on; every other scheme requires `within`.

### Consequential choices, all in source

- `within` is required wherever the scheme signs a time, between `30 seconds` and `1 hour`. There is no default replay window.
- The secret is named, read with `secret(name)` at startup; a missing secret refuses to start, as a connector credential does. It never appears in a trace or an error.
- A verified route takes no `requires`: the provider is not a signed-in person. A route with both is a compile error, since one would always be refused.

### Three tiers

- **Backend:** the clause, the verifier, startup closure for the secret, the trace (`webhook.verified` with scheme and event id, never the secret or signature).
- **Frontend:** nothing to call. The generated TypeScript client and the dev console leave verified routes out, since no browser can sign as the provider; OpenAPI documents the signature header so a provider's tooling can test.
- **AI:** a verified route is never offered to an assistant. An event the handler records is ordinary typed state the application's actions and assistants already see.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| A `verify_stripe(body, header)` library function | Leaves verification optional and lets the handler run first; the clause makes it impossible to reach the handler unverified. |
| `body Bytes` only, verification in code | Solves the raw-body gap but not the forgetting; kept as the escape hatch for unlisted schemes. |
| A default 5-minute window | A replay window is security policy; the source states it. |
| An annotation, `@webhook(stripe, ...)` | Routes already read left to right as what they take and who may call; `verified stripe(...)` sits where `requires` sits, and says the same kind of thing. |

### Decided when building (2026-11-27)

- **One verifier.** `corvid-connector-runtime` already had a webhook verifier for GitHub, Slack and Linear. It is now the only one: it gained Stripe and Standard Webhooks, and `corvid serve` and `corvid connectors verify-webhook` both call it. Strengthening it was its own commit first. Its 5-minute window when none was given was a hidden default and is gone. It had its own copy of the constant-time compare, over hex text, so uppercase hex failed. It tried only one signature per header, so a provider rolling its key broke.
- **Five schemes, not three.** Slack and Linear were already implemented and tested in the verifier, so the clause accepts them too. Linear signs no time in its header, so it takes `replays: accepted`, like GitHub.
- **Signature before timestamp.** The proposal listed the timestamp check first. The verifier checks the signature first, so a request that is not from the provider learns only that its signature is bad, never whether its time would have been fresh.
- **The provider's undeclared fields are skipped.** An ordinary route refuses keys its body type does not declare, because the contract publishes `additionalProperties: false` to the app's own clients. A provider's payload is defined by the provider and grows without notice, so a verified route reads the declared fields and skips the rest, as `T.from_json` does for an outside API.
- **The route shape.** A verified route takes no `requires`, is not GET or DELETE, and takes no `Upload`: its body is buffered so the exact bytes can be checked.
- **The console and the assistant endpoint** list only agents and actions, never routes, so they cannot expose a verified route. The TypeScript client, which generates route calls, filters on the contract's `verified` marker.

## Slices

- [x] W1. Grammar and checking: the `verified` clause, scheme names, `within` bounds and the `replays: accepted` rule for schemes that sign no time, refused with `requires` (E0718); the contract (`verified`, with scheme, signature headers and window) and OpenAPI (`x-corvid-verified`, the header parameters, a 401) mark the route; `body Bytes` on any route is the raw body, published as `application/octet-stream`.
- [x] W2. The verifier in `corvid serve` for every scheme, before decoding, with key-rolling signatures and the startup secret check (a missing or unusable secret refuses to start). A recorded Stripe request and a recorded Standard Webhooks request verify, and every tampered field is refused (`webhook_verify.rs`). Served deliveries are refused for a changed body, a changed timestamp, a missing header and another key, and the recorded 2024 Stripe request is refused as expired (`serve_webhooks.rs`).
- [x] W3. The TypeScript client leaves verified routes out; the trace records `webhook.verified` and `webhook.refused` with the route, scheme and delivery id, never the secret or signature.

## Non-scope

- Sending webhooks (signing outgoing requests).
- Schemes beyond the five (Shopify, Twilio): each is a row in the scheme table, added with a recorded request.
- Native code: `corvid build` servers do not serve verified routes yet. The native server is Desktop's area.
- Idempotent handling of a provider's retries: the handler records the event id; a declared deduplication store is later work.
