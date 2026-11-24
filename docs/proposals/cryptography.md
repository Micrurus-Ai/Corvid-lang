# Proposal: cryptography (B04)

Status: **accepted, in progress.** Slices are tracked under "Task progress" in [the backend and AI roadmap](../backend-ai-roadmap.md).
Owner: backend and AI development, macOS session ([parallel backend sessions](../internals/parallel-backend-sessions.md)). It adds no parser rule, so it needs no frontend agreement.

## Problem

Every backend verifies something cryptographic within its first month: a payment provider's webhook signature (HMAC-SHA256 over the raw body), a password at login (Argon2id), a signed token or release (Ed25519), a content hash (SHA-256). Python reaches for `hashlib`, `hmac`, `secrets` and `argon2-cffi`; TypeScript for `crypto` and `bcrypt`. Corvid has none of them in the language. Release tooling signs with Ed25519 and API keys are hashed with Argon2id, but a program cannot hash, sign, verify or compare a digest.

Two hazards come with the feature and are what Corvid should remove:

- **Timing leaks.** Comparing a computed signature to a received one with ordinary equality returns early on the first differing byte, which leaks the signature byte by byte. Every webhook guide says "use a constant-time compare", and many programs still use `==`.
- **Secrets in logs.** `std/secrets` returns a secret as a plain `String`. Its own documentation names the residual channel: a secret passed on to another tool is recorded in that tool's trace. A webhook signing key or a password that reaches a trace, a prompt or a log is a breach.

## What exists today

A survey before writing any code:

- The workspace already depends on `sha2`, `hmac`, `argon2`, `ed25519-dalek`, `subtle` and `rand_core` (the runtime, the ABI tooling and the CLI), so B04 adds no new cryptography library.
- `corvid-runtime/src/auth/api_keys.rs` hashes API keys with `Argon2::default()`: Argon2id, 19 MiB, 2 iterations, 1 lane, the OWASP 2023 minimum, chosen silently.
- `std/secrets.cor` reads environment secrets; the trace records a redacted copy, and replay re-reads the environment rather than substituting.
- B03 deferred byte encoders and decoders to B04, which decides how Corvid represents bytes. `Upload.bytes()` returns `List<Int>`.
- `std/random.cor` and `std/uuid.cor` model entropy as traced tools.

## Design

### Two new types

| Type | Meaning |
| --- | --- |
| `Bytes` | An immutable byte string: digests, MAC tags, signatures, keys, binary payloads |
| `Secret` | Sensitive text, such as a signing key or a password, that the program can use but never reveal |

**`Bytes`.** A digest is bytes, not text, and treating it as text is how encodings get mixed up. `text.utf8()` gives a string's UTF-8 bytes. `Bytes.from_hex(text)`, `Bytes.from_base64(text)` and `Bytes.from_base64_url(text)` return `Result<Bytes, String>`, and `bytes.to_hex()`, `bytes.to_base64()` and `bytes.to_base64_url()` return text, through B03's `corvid-text` codecs. `bytes.length()` and `bytes.to_utf8()` (a `Result`) complete the set. **Equality on `Bytes` is always constant-time** (it depends only on the lengths): the comparison the webhook guides tell people to remember is simply what `==` does, so a program cannot get it wrong. `Bytes` has no ordering. Its JSON form is base64url text.

**`Secret`.** A `Secret` holds text the program needs but must never expose:

- It cannot be printed, concatenated, converted to a `String`, encoded as JSON, stored in a table or passed to a prompt. Each of these is a compile error that says why.
- A tool may take a `Secret` parameter. Its trace records `<redacted>` in that position, which closes the residual channel `std/secrets` documents. Replay re-reads the secret from the environment, as `secret_read` does today.
- `std/secrets` gains `secret(name) -> Result<Secret, String>`, which reads the environment the way `secret_read` does. `secret_read` stays for programs that need the text, with its documentation pointing to `secret`.
- `Secret` values compare with constant-time `==`.

### Hashing, MACs and signatures

All of these are pure and deterministic, so they are builtin methods, not traced, and allowed in `@deterministic` code:

| Call | Result |
| --- | --- |
| `bytes.sha256()`, `bytes.sha384()`, `bytes.sha512()` | `Bytes` |
| `bytes.hmac_sha256(key)`, `bytes.hmac_sha512(key)` with `key: Secret` or `Bytes` | `Bytes` |
| `message.verify_ed25519(signature, public_key)` with three `Bytes` | `Bool` |

The methods are on `Bytes`, so hashing text is written `body.utf8().sha256()`: the encoding is visible, which is the point. MD5 and SHA-1 are not offered; they are broken for the uses people reach for them in. Ed25519 *signing* is not in this task, because a signing key is a long-lived credential whose storage deserves its own design; verification covers webhooks, tokens and releases signed elsewhere.

### Random tokens and passwords

These use entropy, so they are tools, traced like `uuid_v4`:

| Call | Result |
| --- | --- |
| `random_token(bytes: Int) -> Secret` in `std/crypto` | A secret of `bytes` random bytes from the operating system, written base64url |
| `hash_password(password: Secret, cost: PasswordCost) -> String` in `std/crypto` | An Argon2id hash in PHC format, which records its own parameters and salt |
| `password.verify_password(hash)` on a `Secret` | `Bool`; pure, reads the parameters from the hash, constant time |

- **A token is a `Secret`,** so it never lands in a trace: replay generates a fresh one and the trace records `<redacted>`. A program that must show a token once, such as an API key at creation, reveals it with the explicit `token.reveal_once()`, which is allowed only as the value of a route response and is recorded in the trace as a revealed secret, never its text.
- **The password-hashing cost is declared.** Argon2id's memory, iterations and lanes trade login latency against attack cost, and OWASP lists several equivalent settings, so under the "no hidden defaults" rule the call names its cost: `hash_password(password, owasp_2023())`. `std/crypto` exports `owasp_2023()` (19 MiB, 2 iterations, 1 lane) and the `PasswordCost` record for other values. The hash string records the parameters, so verification never needs them and a later cost increase verifies old hashes. The API-key store moves to the same function with the same declared cost, so the language and the runtime hash one way.

### One implementation for every tier

A new crate, `corvid-crypto`, holds every rule: hashing, HMAC, Ed25519 verification, constant-time comparison, Argon2id hashing and verification, and token generation from operating-system entropy. The checker, the interpreter, the native runtime and the API-key store call it, as `corvid-text` serves B03 and `corvid-decimal` serves B06.

### Across the tiers

Answered under the project's three-tier rule.

- **What the backend knows that the frontend and AI can use.** Which values are secrets, declared by type. The application contract marks a `Secret` field as one that never leaves the server, so the TypeScript client and the SDKs have no field to read, and a form field bound to a `Secret` is a password input that never echoes.
- **What the UI gets for free.** A login form whose password field is typed `Secret` is masked, is never logged and never prefills; the browser's password-manager hints come from the same declaration.
- **What an assistant can safely do.** Nothing with a secret's text: a prompt or a model-visible action that would receive a `Secret` is a compile error, so no secret can reach a model. An assistant can still call a declared action that uses a secret on the server, such as "verify this webhook", and sees only the `Bool`.

### Not an invention, except `Secret`

Hashing, HMAC, signatures and password hashing are commodity features with the papercuts removed (constant-time `==`, encodings made visible, no broken hashes, declared cost). `Secret`, a type the compiler keeps out of traces, prompts, logs, JSON and the client contract, is a Corvid-specific guarantee, so its slice ships with the invention contract: a catalog entry, a `corvid tour` demo and a proof-matrix row.

## Syntax alternatives considered

Recorded under the syntax simplicity rule.

- **Constant-time comparison.** `constant_time_equals(a, b)` is the form other libraries offer, and people forget it. Making `==` on `Bytes` and `Secret` constant-time removes the decision.
- **Hashing text.** `text.sha256()` would hide which encoding is hashed; `text.utf8().sha256()` costs six characters and makes the bytes explicit.
- **Password cost.** A default cost is the hidden policy the project rule forbids; a cost argument named `owasp_2023()` reads as the policy it is.

## Slices

1. **`Bytes` and hashing on the interpreter.** The `corvid-crypto` crate, the `Bytes` type with constant-time equality, the conversions (including B03's deferred byte codecs) and SHA-2.
2. **`Secret` on the interpreter.** The type and its compile errors, `secret(name)`, redaction of `Secret` tool arguments in traces, refusal at prompts, JSON, tables and the client contract, and the invention contract.
3. **HMAC and the webhook acceptance test.** `hmac_sha256` and `hmac_sha512`; verify a recorded payment-provider webhook signature and reject a tampered body.
4. **Tokens and passwords.** `random_token`, `hash_password` with a declared cost, `verify_password`, and the API-key store moved onto `corvid-crypto`.
5. **Ed25519 verification.**
6. **Native code** for all of the above, with parity tests.

## Deferred

- **Ed25519 signing and key storage**, which need a design for long-lived keys.
- **Raw request bodies in routes**, needed to verify a live webhook over HTTP; B16 (verified webhooks for a payment provider) owns it.
- **Encryption** (AES-GCM, ChaCha20-Poly1305), until a backend example needs it.
