# Keys and encryption at rest

**Built:** K1, K2 and K3 on 2026-11-27; K4 to K6 are on the roadmap.
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

### Build plan for K2 and K3 (2026-11-27)

K2 alone would let a program declare `Secret` columns the runtime stores unencrypted, so K2 and K3 ship as one slice. Frontend development agreed (review item 8): keep secret columns out of forms, client schemas and live row projections, and keep public row types usable without the secret.

1. **Grammar:** `encryption: key(secret("NAME"))` in a `database` block, as `DatabaseDecl.encryption`. `previous:` is refused by name until K4.
2. **Checker:**
   - A `Secret` column is allowed only in a table whose database declares a key. The existing `SecretTableColumn` error names the item to add.
   - A `Secret` column is refused as the key, in `unique`, in an index, and in a `where`, `order_by` or aggregate.
   - A `live` on a table with a `Secret` column is refused, since a live row is sent to a browser.
3. **Storage:** a new `ColumnKind::Sealed`, stored as `SEALED TEXT` on SQLite and as a `corvid_sealed` text domain on Postgres, so the schema's round trip stays exact. The value is `corvid:v1:<key id>:<base64url>` from `corvid_crypto::DataKey::seal`.
4. **Where encryption happens:** in the interpreter's table bridge, `corvid-vm/src/interp/tables.rs`. `value_row` seals each `Secret` field with the associated data `table/column/key`, and `row_value` opens it. The runtime and the trace therefore only ever see ciphertext, and a replay opens recorded rows the same way.
5. **The key:** `Runtime::data_key(database)` reads the named secret once and caches the `DataKey`. A missing key, or one that is not 32 bytes of base64, is a typed error naming the secret. `corvid serve` reads every key its source's databases declare before it binds, so it refuses to start. `corvid run`, and a database imported from another module, read the key at the first row that needs it, with the same error.
6. **Refusals:** a row sealed under another key id, or one that fails to open (moved to another row or column, or tampered with), is a typed error naming the table, column and key id, never a silent `None`.
7. **Tests:** sealing and opening on SQLite and Postgres; the stored text is ciphertext; a value copied to another row fails; the trace holds no plaintext; replay with the key reads the rows and replay without it is refused; each checker refusal.

## Slices

- [x] K1. `corvid-crypto` gains XChaCha20-Poly1305 sealing bound to associated data, with known-answer tests and a test that a ciphertext moved to another row fails. Built 2026-11-27 as `corvid_crypto::DataKey` (`seal`, `open`, `sealed_key_id`, `generate_base64`), checked against draft-irtf-cfrg-xchacha's published vector, with the key's bytes wiped on drop and its `Debug` showing only the key id.
- [x] K2. `encryption: key(secret(...))` parsed and checked; a `Secret` column allowed only under it; an encrypted column refused in `where` and `order_by`. Built 2026-11-27 with K3, following the plan above. The refusals also cover grouping, `unique` and `live`. One rule, `ColumnKind::not_comparable`, now gives every comparison refusal its reason.
- [x] K3. Writes encrypt and reads decrypt on both engines, the trace holds ciphertext, and replay without the key is a typed refusal. Built 2026-11-27. Tests: `crates/corvid-driver/tests/tables_sealed_columns.rs`, on SQLite and Postgres, and `corvid-runtime`'s `data_keys` unit test.
- [ ] K4. `previous:` keys, `corvid migrate rekey`, and `corvid keys new`.
- [ ] K5. Durable sign-in tokens (user credentials C3) stored under the identity database's key.
- [ ] K6. Native code, with native tables.

## Non-scope

- Encrypting whole databases or files (the database's own encryption, or the disk's).
- Key services (KMS, HSM): the owner's decision, with room left in the `key(...)` form.
- Ed25519 signing keys: they will load through the same `secret(...)` path, in the cryptography proposal's own slice.
