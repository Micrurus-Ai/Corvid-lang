# Keys and encryption at rest

**Status:** proposal, 2026-11-27. It adds an item to the `database` block, which is in the shared grammar reference, so it is item 8 of [the review brief](frontend-review-2026-11-27.md) and is built after one working day without objection.
**Owner:** backend (Claude Code).
**Unblocks:** a `Secret` table column (B09), durable sign-in tokens ([user credentials](user-credentials.md) C3), and the key handling Ed25519 signing will reuse ([cryptography](cryptography.md), deferred work).

## The problem

A program that must use a secret again later has nowhere safe to keep it. It might be a customer's API key for a service it calls on their behalf, a webhook signing secret per tenant, or a person's sign-in token. A `Secret` column is refused today, correctly, because a plain column would put the text at rest. Python and TypeScript developers reach for an encryption library and a key in an environment variable, and get the details wrong:
- a nonce reused;
- a ciphertext that can be copied from one row to another and still decrypt;
- a key with no way to rotate;
- a plaintext copy left in a log.

## The design

```corvid
database billing:
    engine: postgres
    location: env("BILLING_DATABASE_URL")
    encryption: key(secret("BILLING_DATA_KEY"))

table Customer:
    key id: Int
    name: String
    stripe_key: Secret
```

- `encryption: key(secret("NAME"))` gives the database one data key, read from the secret store like every other secret, never written in source. Without the item, nothing is encrypted and a `Secret` column stays refused, with the error naming the item to add. There is no default key.
- **A `Secret` column becomes legal exactly when its database declares a key.** A `Secret` can only ever be stored encrypted, so the source does not say `encrypted` again; the compiler knows. Reading the row gives back a `Secret`, which keeps every rule it has today: it cannot be printed, returned from a route or sent to a model.
- **How it is stored:** XChaCha20-Poly1305 with a fresh random 24-byte nonce per write. The associated data is the table, the column and the row's key, so a ciphertext copied to another row or column fails to decrypt instead of being read as the other value. The stored text is `corvid:v1:<key id>:<base64url of nonce and ciphertext>`, where the key id is derived from the key and says which key wrote it.
- **Rotation:** `encryption: key(secret("BILLING_DATA_KEY"), previous: secret("BILLING_DATA_KEY_2026"))` reads rows written under either key and writes under the current one. `corvid migrate rekey` re-encrypts every row still under a previous key and reports how many, after which `previous:` can go. A row under a key the program does not have is a typed refusal naming the key id, never a silent `None`.
- **Queries:** an encrypted column cannot be compared or ordered in a `where`, because the database only holds ciphertext. That is a compile error naming the column; look the row up by another column instead.
- **The key:** 32 random bytes, base64. `corvid keys new` prints one to paste into the secret store. A key of the wrong length is a refusal to start. The key's own text never enters a trace or a log, as for any `Secret`.
- **Traces and replay:** table reads are recorded as the database returned them, so a trace holds the ciphertext, never the text. Decryption happens after the read, in the interpreter, so a replay needs the key in its secret store. Without it, the replayed read is the same typed refusal.

## Three tiers

- **Backend:** one declared key per database, authenticated encryption bound to its row, rotation and a rekey command, with no new syntax on the column.
- **Frontend:** a `Secret` field is never sent to a browser, so nothing changes. Later, a masked input for setting one (already on the roadmap) can write into such a column.
- **AI:** an assistant never sees a `Secret`. An action can use a customer's stored key to call a service on their behalf, and the trace shows the call without the key.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| `stripe_key: Secret encrypted` | Says twice what the type already guarantees; a `Secret` column could never be anything but encrypted. |
| AES-256-GCM | Equally sound, but its 12-byte random nonce limits how many writes one key can take safely; XChaCha20's 24-byte nonce removes that limit. |
| A key per column | More keys to store and rotate for no gain at this stage; the associated data already separates columns. |
| A cloud key service (KMS) | A billed service and the owner's decision; the `key(...)` form leaves room for `kms("...")` later without changing columns. |
| Encrypting other columns (`ssn: String encrypted`) now | Useful for personal data, and a later slice. It needs the same query refusal, and a decision on whether such values may be shown. |

## Slices

- [ ] K1. `corvid-crypto` gains XChaCha20-Poly1305 sealing bound to associated data, with known-answer tests and a test that a ciphertext moved to another row fails.
- [ ] K2. `encryption: key(secret(...))` parsed and checked; a `Secret` column allowed only under it; an encrypted column refused in `where` and `order_by`.
- [ ] K3. Writes encrypt and reads decrypt on both engines, the trace holds ciphertext, and replay without the key is a typed refusal.
- [ ] K4. `previous:` keys, `corvid migrate rekey`, and `corvid keys new`.
- [ ] K5. Durable sign-in tokens (user credentials C3) stored under the identity database's key.
- [ ] K6. Native code, with native tables.

## Non-scope

- Encrypting whole databases or files (the database's own encryption, or the disk's).
- Key services (KMS, HSM): the owner's decision, with room left in the `key(...)` form.
- Ed25519 signing keys: they will load through the same `secret(...)` path, in the cryptography proposal's own slice.
