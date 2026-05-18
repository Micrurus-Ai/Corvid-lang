# Auth and approvals

The auth + approval product surface ships in v1.0 as a CLI + host
API. Source-level `auth Name:` / `tenant Org { ... }` / `approval
Name:` / `@requires` / `@approval` keywords are post-v1.0 ergonomic
sugar (tracked at slice `35V2-P39-I-post-v1.0-auth-syntax-sugar`);
the runtime behaviour they would surface ships today through the
CLI + host API + `agent` declarations + `approve` boundary.

## What ships today

The runtime auth surface:

- **JWT verification** (`crates/corvid-runtime/src/jwt_verify/`) —
  real RSA signature checks, `kid` resolution against a JWKS, `alg`
  enforcement (refuses `none`), expiry validation, configurable
  `iss` / `aud` matching. Backed by `jsonwebtoken`. Adversarial
  tests for `kid` downgrade, `alg: none` injection, expired tokens,
  missing signatures all green.
- **API key store** (`crates/corvid-runtime/src/auth/api_keys.rs`)
  — Argon2id-hashed at rest, scope fingerprint, tenant scoping,
  revocation + rotation. Plaintext leaves Corvid memory exactly
  once (at issue time, echoed to the operator).
- **OAuth callback state** (`crates/corvid-runtime/src/auth/oauth.rs`)
  — PKCE-required, single-use, tenant-scoped, expiry-bound. Used
  by Phase 41 connectors (Gmail, Slack, MS365, etc.).
- **Session store** (`crates/corvid-runtime/src/auth/sessions.rs`)
  — rotation on privilege change, expiry, revocation, cross-tenant
  refusal.
- **Approval queue** (`crates/corvid-runtime/src/approval_queue.rs`)
  — create / list / inspect / approve / deny / expire / comment /
  delegate / batch, with audit + trace events on every transition.

## CLI

Initialise the auth + approval stores (idempotent — safe to re-run
on every deploy):

```sh
corvid auth migrate
```

API keys for service actors:

```sh
corvid auth keys issue --tenant-id <tenant> --service-actor-id <actor> --raw-key <key>
corvid auth keys rotate --tenant-id <tenant> --service-actor-id <actor> --old-key <hash> --raw-key <new>
corvid auth keys revoke --tenant-id <tenant> --service-actor-id <actor> --key-hash <hash>
```

Approval queue management:

```sh
corvid approvals queue --tenant <tenant>            # list pending approvals
corvid approvals inspect <id>                        # one approval + audit trail
corvid approvals approve <id> --actor-id <reviewer>  # approve
corvid approvals deny <id> --actor-id <reviewer> --reason <text>
corvid approvals expire <id>                         # expire a stale approval
corvid approvals comment <id> --actor-id <reviewer> --text <text>
corvid approvals delegate <id> --to-actor-id <other>
corvid approvals batch --approval-ids <id1,id2,...> --tenant-id <tenant> --actor-id <reviewer>
corvid approvals export --tenant <tenant> --since <iso-timestamp>
```

The auth + approval CLI is the operator surface; agent code
interacts with auth + approvals through the existing `approve`
boundary keyword + the runtime's actor / session / tenant context
threading.

## Approvals in agents (the shipped `approve` keyword)

Approve before a dangerous tool fires:

```corvid
effect email_effect:
    cost: $0.05

tool send_email(to: String, body: String) -> Nothing dangerous uses email_effect

agent send(to: String, body: String) -> Nothing uses email_effect:
    approve SendEmail(to, body)
    send_email(to, body)
```

The compiler rejects `send_email(to, body)` without a matching
`approve SendEmail(...)` on the reachable path. The approval flows
into the queue when the agent runs; an operator approves via
`corvid approvals approve <id>`; the agent's run resumes (or
transitions to terminal on deny).

The `await_approval` source keyword that would let the agent pause
and persist its own checkpoint mid-step is post-v1.0 syntax sugar
— today the runtime achieves the same persistence by composing the
existing `approve` keyword with the durable job runner (`corvid
jobs run --queue=...`).

## Tenant isolation

Every actor record carries a `tenant_id`. The approval queue
refuses cross-tenant approve / deny operations (`tenant
mismatch` error). The API key + session stores enforce the same.
Adversarial tests:
`crates/corvid-runtime/src/approval_queue.rs::approval_bypass_rejects_tenant_crossing_actor`
+ `crates/corvid-runtime/src/auth/sessions.rs::session_runtime_rejects_expired_revoked_and_cross_tenant_sessions`
+ `crates/corvid-runtime/src/auth/api_keys.rs::api_key_runtime_rejects_wrong_tenant_revoked_expired_and_user_actors`.

The source-level `tenant Org { ... }` block that would let the
typechecker refuse a cross-tenant value at compile time is post-
v1.0; until then the runtime's reject-at-call-time check is the
guarantee `tenant.cross_tenant_compile_error` row tracks (currently
OutOfScope with reason naming the post-v1.0 slice).

## Audit + replay

Every approval transition writes a trace + audit event the
compliance review consumes. Replay an approval session
deterministically:

```sh
corvid approvals export --tenant <tenant> --since 2026-01-01T00:00:00Z > approvals.jsonl
corvid replay approvals.jsonl
```

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| API keys hashed at rest | `auth.api_key_at_rest_hashed` | RuntimeChecked | `crates/corvid-runtime/src/auth/api_keys.rs` |
| JWT kid rotation enforced | `auth.jwt_kid_rotation` | RuntimeChecked | `crates/corvid-runtime/src/jwt_verify/` |
| OAuth PKCE required | `auth.oauth_pkce_required` | RuntimeChecked | `crates/corvid-runtime/src/auth/oauth.rs` |
| Dangerous tool requires approve | `approval.dangerous_call_requires_token` | Static | `crates/corvid-types/src/checker/` |
| Session rotation on privilege change | `auth.session_rotation_on_privilege_change` | OutOfScope | gated on `35V2-P39-D-LR-session-rotation-hook` (launch-readiness) |
| CSRF double-submit | `auth.csrf_double_submit` | OutOfScope | gated on `35V2-P39-C-LR-csrf-middleware` |
| Cross-tenant compile error | `tenant.cross_tenant_compile_error` | OutOfScope | gated on `35V2-P39-I-post-v1.0-auth-syntax-sugar` |
| Approval policy clause static check | `approval.policy_clause_static_check` | OutOfScope | gated on `35V2-P39-I` |
| Approval batch equivalence typed | `approval.batch_equivalence_typed` | OutOfScope | gated on `35V2-P39-I` |
| Approval confused-deputy typecheck | `approval.confused_deputy_typecheck` | OutOfScope | gated on `35V2-P39-J-LR-role-coverage-reachability` |
