# Auth and approvals

## What `std.auth` gives you

- JWT verification with real signature checks, `kid` resolution,
  JWKS fetch, and `alg` enforcement (refuses `none`).
- API key generation, rotation, and revocation.
- OAuth flows for connector authentication.
- The approval product surface: `corvid approvals` CLI, pending list,
  approve/deny.

## CLI

```sh
corvid auth keys generate            # generate a new API key
corvid auth keys rotate              # rotate signing keys
corvid auth jwt verify --jwks <url>  # verify a JWT against a JWKS

corvid approvals pending             # list pending approval requests
corvid approvals approve <id>        # approve
corvid approvals deny <id> --reason  # deny with reason
```

## JWT verification in code

```corvid
import "@stdlib/auth" as auth

@budget($0.001)
agent verify_request(token: String) -> Result<Claims, AuthError> uses auth_effect:
    let verifier = auth.jwt_verifier({
        jwks_url: "https://auth.example.com/.well-known/jwks.json",
        issuer: "https://auth.example.com",
        audience: "my-app",
    })
    return verifier.verify(token)
```

The verifier:
- Refuses `alg: none` (always).
- Refuses tokens whose `kid` doesn't resolve in the JWKS.
- Refuses missing signatures, malformed signatures, expired tokens.
- Refuses tokens whose `iss` or `aud` doesn't match the configured
  values.

## OAuth flow

```corvid
let oauth = auth.oauth_provider({
    provider: "google",
    client_id_env: "GOOGLE_CLIENT_ID",
    client_secret_env: "GOOGLE_CLIENT_SECRET",
    scopes: ["email", "https://www.googleapis.com/auth/gmail.readonly"],
})

# Step 1: redirect URL
let url = oauth.authorize_url(state: "csrf-token-123")

# Step 2: in the callback handler
let token = oauth.exchange_code(code, state)?
db.tokens.put_encrypted("google_oauth", customer_id, token)
```

OAuth refresh-token rotation is automatic and uses encrypted storage.

## Approval product surface

The approval product is the operator-facing UI/CLI for `await_approval`
states. From an agent:

```corvid
agent send_email(to: String, body: String) uses email_effect:
    await_approval EmailSend(to: to, body_hash: hash(body))
    approve EmailDispatch(to)
    email_send(to, body)
```

The operator sees the pending approval in `corvid approvals pending`,
reviews the body hash + metadata, and approves or denies. The
agent resumes (or transitions to terminal) accordingly.

## Tenant isolation

The job queue + approval queue are tenant-scoped by default. A query
like `corvid approvals pending --tenant=customer-A` only returns
approvals scoped to that tenant.
