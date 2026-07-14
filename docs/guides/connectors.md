# Connectors

Phase 41 ships six production connectors via the
`corvid-connector-runtime` crate. Each connector runs in three
modes (mock / replay / real) that share the same typed surface;
the runtime enforces the mode boundary so a replay session
cannot accidentally make a real provider call.

The source-level `import "@connector/gmail" as gmail` /
`gmail.recent(...)` ergonomic surface is post-v1.0 sugar (tracked
at `35V2-P41-I-post-v1.0-connector-syntax-sugar`); today
connectors enter Corvid through `tool` declarations whose
host-side implementation calls into `corvid-connector-runtime`.
Same pattern as persistence + Python FFI.

## What ships

Six connectors, each tested in all three modes:

| Connector | Read | Write | OAuth | Webhooks |
|---|---|---|---|---|
| Gmail | `gmail_recent`, `gmail_search` | `gmail_send` (dangerous) | ✓ | n/a |
| MS365 | `ms365_recent` | `ms365_send` (dangerous) | ✓ (tenant-aware) | n/a |
| Calendar | availability, event read | event create/update/cancel (dangerous + approval-gated for external invites) | ✓ | n/a |
| Slack | channel + DM metadata | post messages (dangerous) | ✓ | ✓ HMAC-SHA256 + 5-min replay window |
| Linear + GitHub | issue read + search | issue create / update / comment (dangerous) | ✓ | ✓ HMAC-SHA256 |
| Local files | indexed folder read | provenance-preserving snippet write (approval-gated) | n/a | n/a |

## The three modes

Every connector exposes:

- **Mock** — in-memory deterministic responses. No network. Used
  by default in CI + by `cargo test`. Token scope + tenant
  enforcement still applies at the runtime boundary.
- **Replay** — serves recorded responses from a JSONL trace. No
  network. Write operations refuse and surface
  `ConnectorRuntimeError::ReplayWriteQuarantined` — the
  `connector.replay_quarantine` row guarantees a replay session
  cannot escape into real-mode writes.
- **Real** — live API calls behind `CORVID_PROVIDER_LIVE=1` +
  per-provider credential env vars. The shared `ReqwestRealClient`
  honours `Retry-After` on 429/5xx via
  `parse_retry_after_header` and surfaces the typed
  `ConnectorRuntimeError::RateLimited { retry_after_ms }`
  variant.

## Using a connector from Corvid

Today connectors enter through `tool` declarations:

```corvid
effect gmail_read_effect:
    cost: $0.001

effect gmail_send_effect:
    cost: $0.002
    trust: human_required
    reversible: false

tool gmail_recent(user_id: String, since: String) -> String uses gmail_read_effect
tool gmail_send(user_id: String, message: String) -> Nothing dangerous uses gmail_send_effect

agent send_reply(user_id: String, reply: String) -> Nothing uses gmail_send_effect:
    approve GmailSend(user_id, reply)
    gmail_send(user_id, reply)
```

The host-side implementation of `gmail_recent` + `gmail_send`
calls into `corvid_connector_runtime::gmail`; the typechecker
enforces the effect row + the approve boundary at the Corvid
layer.

## Grounding connector reads today

The `Grounded<T>` wrap does not need the post-v1.0 `connector`
syntax: declare the connector-backed tool with an effect whose
`data` dimension is `grounded`, and every call site's return is
auto-wrapped today —

```corvid
effect gmail_read:
    data: grounded

tool gmail_recent(query: String) -> String uses gmail_read

agent latest_thread(query: String) -> Grounded<String>:
    thread = gmail_recent(query)
    return thread
```

`thread` is `Grounded<String>`; the provenance flows to the declared
`Grounded` return with no annotation, and a `Grounded` return
without a grounded source is a typecheck error. Passing a grounded
value where a plain `T` is expected is the deliberate legacy
coercion — it typechecks, and lowering emits an explicit
provenance-discard node (verified across all four tiers by the
`legacy_grounded_coercion` corpus fixture), so the drop is tracked
rather than silent. A strict no-implicit-strip mode and
connector-side default grounding ride the post-v1.0
`connector ... grounded` syntax; today the shipped examples carry
provider record ids explicitly (`provenance_id` fields, the
files-connector pattern), and this opt-in effect pattern layers the
typed wrap on top.

## CLI

```sh
corvid connectors list                              # shipped connectors + modes + scopes + rate limits
corvid connectors check                             # validate every shipped manifest
corvid connectors check --live                      # also detect drift against the real provider
                                                    # (CORVID_PROVIDER_LIVE=1; the --live drift
                                                    # path is filed as launch-readiness
                                                    # 35V2-P41-D-LR-connector-drift-narration)
corvid connectors run <connector> <op> [--mode=mock|replay|real]
corvid connectors oauth <provider>                  # OAuth2 token lifecycle (issue / refresh)
corvid connectors verify-webhook --provider github|slack|linear --signature <sig>
                                                    # HMAC-SHA256 webhook signature verification
```

## OAuth setup

```sh
export CORVID_GMAIL_OAUTH_CLIENT_ID=<id>
export CORVID_GMAIL_OAUTH_CLIENT_SECRET=<secret>
corvid connectors oauth google
```

This walks the PKCE-required OAuth code flow + stores the
refresh token Argon2id-hashed (see `auth.api_key_at_rest_hashed`
+ `auth.oauth_pkce_required`). Refresh-token rotation is
automatic; a revoked refresh token surfaces as
`BearerTokenError::Revoked` on the next refresh attempt.

## Webhook signature verification

```sh
# GitHub webhook (X-Hub-Signature-256: sha256=<hex>)
corvid connectors verify-webhook --provider github \
    --signature "$X_HUB_SIGNATURE_256" \
    --body @webhook-body.json \
    --secret-env GITHUB_WEBHOOK_SECRET

# Slack webhook (v0:<ts>:<body> with 5-min replay window)
corvid connectors verify-webhook --provider slack \
    --signature "$X_SLACK_SIGNATURE" \
    --timestamp "$X_SLACK_REQUEST_TIMESTAMP" \
    --body @webhook-body.json \
    --secret-env SLACK_WEBHOOK_SECRET
```

Exit 0 on a valid signature, exit 1 on mismatch / stale
timestamp / malformed header. Comparison is constant-time; the
five-minute window catches Slack-style replay attacks.

## Adversarial corpus (7 named threats, 14 tests)

The Phase 41 audit verified all 7 named threats are covered.
Tests live in `crates/corvid-connector-runtime/tests/threat_corpus.rs`:

| Threat | Tests |
|---|---|
| token-scope escalation | `t1_github_rejects_unauthorised_scope` + per-provider variants |
| cross-tenant message access | `t2_*_rejects_missing_tenant` |
| refresh-token replay after revocation | `t3_oauth_refresh_after_revocation_marks_store_revoked` |
| malformed JSON body | `t4_github_search_missing_required_field` + `t4_github_write_unknown_kind_refused` |
| 429/5xx retries with Retry-After | `t5_rate_limited_propagates_retry_after_ms` + `t5_retry_after_parser_handles_seconds_form` |
| expired OAuth state | `t6_expired_oauth_access_triggers_refresh` |
| webhook signature forgery | `t7_github_webhook_forgery_rejected` + per-provider variants |

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| Scope minimum enforced before HTTP call | `connector.scope_minimum_enforced` | RuntimeChecked | `crates/corvid-connector-runtime/src/runtime.rs` |
| Rate limit honours provider Retry-After | `connector.rate_limit_respects_provider` | RuntimeChecked | `crates/corvid-connector-runtime/src/real_client.rs` |
| Webhook HMAC-SHA256 + replay window | `connector.webhook_signature_verified` | RuntimeChecked | `crates/corvid-connector-runtime/src/webhook_verify.rs` |
| Replay refuses write operations | `connector.replay_quarantine` | RuntimeChecked | `crates/corvid-connector-runtime/src/runtime.rs` |
| Connector write requires approval (typecheck) | `connector.write_requires_approval` | OutOfScope | gated on `35V2-P41-I-post-v1.0-connector-syntax-sugar` |
| `--live` drift detected against real provider | `connector.contract_drift_detected` | OutOfScope | gated on `35V2-P41-D-LR-connector-drift-narration` |
