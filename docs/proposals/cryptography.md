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

The hex and base64 codecs are the exception: they stay in `corvid-text`, which gained byte variants of its codecs (slice 1), and its text codecs are now built on them. `corvid-crypto` calls those, so `Bytes.from_hex(text)` and `text.decode_hex()` share one decoder, one strictness rule and one set of messages.

### Across the tiers

Answered under the project's three-tier rule.

- **What the backend knows that the frontend and AI can use.** Which values are secrets, declared by type. The application contract marks a `Secret` field as one that never leaves the server, so the TypeScript client and the SDKs have no field to read, and a form field bound to a `Secret` is a password input that never echoes.
- **What the UI gets for free.** A login form whose password field is typed `Secret` is masked, is never logged and never prefills; the browser's password-manager hints come from the same declaration.
- **What an assistant can safely do.** Nothing with a secret's text: a prompt or a model-visible action that would receive a `Secret` is a compile error, so no secret can reach a model. An assistant can still call a declared action that uses a secret on the server, such as "verify this webhook", and sees only the `Bool`.

### Not an invention, except `Secret`

Hashing, HMAC, signatures and password hashing are commodity features with the papercuts removed (constant-time `==`, encodings made visible, no broken hashes, declared cost). `Secret`, a type the compiler keeps out of traces, prompts, logs, JSON and the client contract, is a Corvid-specific guarantee, so its slice ships with the invention contract: a catalog entry, a `corvid tour` demo and a proof-matrix row.

## As built

Slice 1 (`Bytes` and hashing on the interpreter) differs from the design above in these details:

- **`+` is refused, as is mixing with text.** The design named only the missing ordering. `a + b` on byte strings is a compile error (no slice needs concatenation, and `+` must not quietly join bytes and text), and so is `bytes == "text"` or `"digest: " + bytes`; the hint names `utf8()` and the encodings.
- **Constant-time everywhere a value compares.** The interpreter's value equality calls `Bytes::constant_time_eq`, so a record or list holding bytes, and `list.contains(bytes)`, compare in constant time too, not only a bare `==`.
- **Decoder messages.** `Bytes.from_hex`, `from_base64` and `from_base64_url` return the text decoders' messages unchanged. `to_utf8()` says `the bytes are not UTF-8 text (byte N starts an invalid sequence)`.
- **JSON schema.** OpenAPI and prompt schemas publish `type: string`, `contentEncoding: base64url` (JSON Schema 2020-12's keyword for binary data in a string) and a pattern of the URL-safe alphabet with optional padding.
- **Table columns.** A `Bytes` column was refused, with the B06 types, until B09 mapped its storage. B09 P8c stores it as `BLOB` on SQLite and `bytea` on Postgres; see [binary values](../reference/tables.md#binary-values).
- **Native code.** Any program that mentions `Bytes`, in a declared field, a signature or an expression, runs on the interpreter until the native slice, which must keep `==` constant-time rather than reuse native string equality. Native code's list of text-held scalars leaves `Bytes` out for the same reason.
- **`Upload.bytes()`** still returns `List<Int>`; moving it to `Bytes` changes existing programs and is a roadmap item.

Slice 2 (`Secret` on the interpreter) differs from the design above in these details:

- **Display and the trace use different markers.** A displayed secret (a printed result, the REPL, a Rust panic) shows `<secret>`; a recorded one, in a tool argument or a result, is `<redacted>`, the marker the trace redaction already used. `secret` records `<redacted>` whole, where `secret_read` keeps the last two characters.
- **Redaction happens where trace payloads are built.** The interpreter's `value_to_json`, from which every trace event, approval, step event, replay comparison and rendered prompt is built, writes `<redacted>` for a secret. A separate `tool_argument_json` writes the text and is called only when a tool's declared parameters hold a `Secret`, for the implementation's arguments. The runtime keeps the two forms apart (`corvid_runtime::secret_arguments`), so the `ToolCall` event, replay and approvals see only the recorded form.
- **Replay compares the redacted form.** A recorded call matches a replayed one because both hold `<redacted>` at the secret's position, so a redacted position matches any secret and a rotated key still replays. Replay re-reads secrets and now steps past their recorded calls, which also fixes `secret_read` inside a longer recorded run.
- **A tool that echoes a secret is scrubbed in the record only.** Every occurrence of a secret's text in a tool's result or error message becomes `<redacted>` in the recorded `ToolResult` (marked `__corvid_secret_scrubbed__` inside the payload, with no schema change) and in the step debugger's view. The program receives the real result unchanged. Replay substitutes the scrubbed form and warns, naming the tool.
- **More refusals than the design named.** Besides printing, conversion, JSON, tables and prompts, the checker refuses a secret in a route, a store, a connector operation (whose credential is its `auth`), `ask` and `choose`, and `main`'s signature, each with its own hint. A function a model can call through a prompt parameter may not take one either. `corvid run` refuses a file whose only agent returns a secret.
- **The contract leaves secrets out rather than marking them.** A public type's secret field and a public agent whose signature holds a secret are not in the application contract, so the client and SDKs have no such field. The masked password input the design describes needs a contract marker agreed with the frontend roadmap; it is a roadmap item.
- **Only `secret` produces one.** JSON cannot mint a `Secret`, so a user tool, a model, a request body or a resumed job cannot return one; `secret(name)` is decoded on its own path. A table column holding a secret names both ways forward: `hash_password(...)` (slice 4) for values that are only checked, and an encrypted column (B09) for values that must be used again.
- **`Secret` as a user type name.** A declared `type Secret` now names the builtin; the two programs in the repository that declared one (a tour and a `serve` test) use `Classified`.

Slice 3 (HMAC and the webhook acceptance test) differs from the design above in these details:

- **One method per hash, two key types.** The builtin-method table gives a parameter one type, so `hmac_sha256` and `hmac_sha512` name `Bytes`, and a checker rule (`checker/crypto_arguments.rs`, through `is_hmac_key_type` in the table's module) accepts `Bytes` or `Secret`. The interpreter passes `corvid_crypto::HmacKey::Bytes` or `HmacKey::Secret` by the key's value, and the secret's bytes are read only inside `corvid-crypto`. A `String` key is refused with a hint to read it with `secret(name)`.
- **A base64 key is still a `Secret`.** Standard Webhooks (Svix) keys the HMAC with the bytes written in base64 after `whsec_`, while Stripe uses the `whsec_...` text as written. `secret.decode_base64_after("whsec_")` returns `Result<Secret, String>`, the one method on `Secret`. Its messages never quote the secret (no character, no position), an empty key is an `Err`, and the prefix must be a string literal, so it cannot be used to test the secret's text one guess at a time. `corvid_crypto::Secret` now holds bytes and remembers whether they are text: a decoded key is not, so `expose_text` returns `None` for it, and passing one to a tool stops the run with an error rather than handing over the bytes in an encoding the tool never asked for. Base64 decoding of the key is not constant-time; it runs on the provider's configuration, once per verification.
- **The acceptance test is Corvid source run through the driver** (`crates/corvid-driver/tests/webhook_verification.rs`): a recorded Stripe request and a recorded Standard Webhooks request verify, and a changed body byte, timestamp, message id, signature or key is rejected. The expected signatures were computed outside Corvid (Python's `hmac`, checked with `openssl`). The headers are parsed with string methods; a reusable parser, a declared timestamp tolerance and raw request bodies are B16's.

Slice 4 (tokens and passwords) differs from the design above in these details:

- **Replay draws a fresh token and warns.** The design said replay generates a fresh token; it also says so. A token is never recorded, so there is nothing to substitute, and refusing to replay would make every program that issues a session or an API key unreplayable. Replay steps past the recorded call, draws a new token, and prints a warning kept for `Runtime::replay_redrawn_token_warnings()`. A token reaches only positions every record holds as `<redacted>`, so a later `hash_password(key, ...)` or a delivery tool still matches and is substituted, and a replayed issuance returns the recorded hash. Where the fresh token changes the run (`verify_password` against a hash recorded from the old token), replay reports the divergence at the first event that differs.
- **The token's size is bounded.** `random_token(bytes)` takes 16 to 1024 bytes. Sixteen bytes (128 bits) is the security floor; more than 1024 is treated as a mistake, such as a length in bits. Either stops the call with an error, because the result is a `Secret`, not a `Result`: a bad size is a defect in the program, and the common call stays `random_token(32)`.
- **The cost floor is OWASP's table, not one setting.** `PasswordCost` is refused when it is weaker than every setting OWASP lists as equal to the minimum (47104 KiB with 1 iteration, 19456 with 2, 12288 with 3, 9216 with 4, 7168 with 5); any dimension may be higher. More than 4 GiB of memory is refused, and verification refuses a stored hash that claims more, so a planted hash cannot exhaust the server. The refusal happens when the hash is made (a tool error), not at compile time; refusing a literal below the floor at compile time is a roadmap item.
- **`owasp_2023()` is a `fn` in `std/crypto`.** It is pure, so `@deterministic` agents can call it. Its numbers are written in Corvid source and in `corvid_crypto::PasswordCost::OWASP_2023`; a driver test asserts the two are equal, as is the API-key store's `API_KEY_HASH_COST`.
- **`verify_password` returns `Bool`.** A hash that is malformed, from another algorithm or Argon2 version, or over the memory limit verifies nothing, so it is `false`. `Result<Bool, String>` would make every login a `match` for a case that, when it happens, is a corrupted database rather than a user error. `corvid_crypto::verify_password` itself returns the error, which the API-key store keeps reporting as an invalid stored hash.
- **Only Argon2id.** Verification refuses argon2i, argon2d and other schemes rather than accept whatever the hash names.
- **Hashing and verification run on a blocking worker.** `hash_password` is a tool, run with `spawn_blocking`. `verify_password` is a pure method; the interpreter runs it on the blocking pool from its async dispatch (2026-11-26), so a check does not stall other work on the same thread.
- **`reveal_once()` is deferred.** A token that no one can see is useless for issuing a key, so a key is shown to its owner through a tool that takes a `Secret` (an email, a notification, a host's one-time display): the tool receives the text, every record holds `<redacted>`. Showing it once in a route response needs the serve runtime to write a response without recording it, a contract marker so the client shows the value once, and the frontend's binding; none exists, and a `reveal_once()` that the checker accepted but the runtime could not keep out of a trace would be the hidden degradation the project rule forbids. It is a roadmap item, as is a `Secret` request-body field for a login form.
- **The API-key store moved.** `hash_api_key_secret` and `verify_api_key_secret` call `corvid_crypto` with `API_KEY_HASH_COST = PasswordCost::OWASP_2023`, the setting `Argon2::default()` chose silently. Stored hashes record their parameters, so keys stored before the move verify, which a runtime test proves with a hash made by the old path. `std/uuid` and `std/random` now draw entropy through `corvid_crypto::fill_random` too.

Slice 5 (Ed25519 verification) differs from the design above in these details:

- **Strict verification.** RFC 8032 leaves verifiers room on edge cases, and the permissive check accepts signatures no one made: under a public key of small order (the identity point among them) the 64 bytes `01 00 ... 00` verify for every message, which a unit test shows `ed25519-dalek`'s `verify` accepting. `corvid_crypto::verify_ed25519` calls `verify_strict`, which refuses a public key or a signature point R of small order and, like the permissive check, an S not reduced below the group order. Every RFC 8032 section 7.1 Ed25519 vector (TEST 1, 2, 3, 1024 and SHA(abc)) verifies; no conforming signer produces a signature the strict check refuses.
- **Wrong lengths are `false`.** A signature that is not 64 bytes, a key that is not 32 bytes or not a curve point verifies nothing, so the answer is the one a forgery gets. Considered: `Result<Bool, String>`, which would make every check a `match` with two failure branches that a caller treats alike (and an attacker controls the signature's length, so a length error is not a program defect); a trap on a wrong-length key, which is closer to a configuration mistake but would let a malformed input stop the agent; and fixed-length `Signature` and `PublicKey` types, which would catch a swapped argument at compile time but add two types before any program needs them. The decoding step (`Bytes.from_hex`, `from_base64`) is already a `Result`, so a garbled header is still reported.
- **One implementation, not yet one caller.** Five places in the release and package tooling verify Ed25519 with `ed25519-dalek`'s permissive `verify`: `corvid-abi`'s `verify_envelope` (which `ops_show` calls), `corvid-cli`'s `trace_diff/signing.rs` and `bundle_cmd/attest.rs`, and `corvid-driver`'s `dimension_artifact.rs` and `package_registry.rs`. None is a plain verify function that could move: each is one library call inside its own envelope parsing and error messages, taking an already-parsed key. Moving them onto `corvid_crypto::verify_ed25519` changes what they accept (strict instead of permissive) in files other sessions own, so it is a roadmap item rather than part of this slice. The driver test instead signs a release manifest with the tooling's own `corvid_abi::sign_envelope` and requires Corvid's answer and `corvid_abi::verify_envelope`'s to agree on the genuine release and on five forgeries.
- **A DSSE envelope is built as text.** A signed release covers `DSSEv1 <type length> <type> <payload length> <payload>`; the test builds it with string methods and `utf8()`, which works because a manifest is JSON text. A binary payload needs byte concatenation, which `Bytes` refuses (`+` was left out in slice 1); a byte-joining form is a roadmap item for when a program needs it.

Slice 6 (native code) differs from the design above in these details:

- **`Bytes` is its base64url text.** Native code keeps every value in one 8-byte slot, so a byte string is a reference-counted Corvid String holding the unpadded base64url text `to_base64_url()` writes, its JSON form, as B06 holds a `Decimal`. That reuses string reference counting, record and list slots, option and result payloads, JSON and the trace unchanged; `to_base64_url()` returns the value itself. Every other method is one call into `corvid-runtime/src/ffi_bridge/bytes.rs`, which decodes with `corvid_crypto::Bytes::from_base64_url` and computes with `corvid-crypto`, so the tiers share every digest, tag, verdict and message. One table in the code generator (`bytes_methods.rs`) names each symbol; the imports, the lowering, the tier picker and native match arms read it. A dedicated byte buffer was considered; the runtime has none, and it would need its own retain, release, list, record, option, result, JSON and trace paths for no gain in what a program can do.
- **`==` compares the texts in constant time, never as strings.** The text is one to one with the bytes (the decoder refuses stray bits in the last character and the encoder never pads), so `corvid_crypto::Bytes::canonical_texts_equal`, `constant_time_eq` over the two texts, gives exactly the byte comparison's answer, and a unit test checks the two agree. Comparing the texts avoids decoding a computed digest first, which would branch on its characters. Structural equality gains an `EqualityLayout::Bytes` kind that calls the same function, so records, lists, options and results holding bytes compare in constant time too. A native decode canonicalizes padded JSON input (`"3q2-7w=="`) by re-encoding it, so every native value is canonical text.
- **Printing.** An entry agent that returns `Bytes` prints `Bytes(<hex>)`, written by the runtime from `corvid_crypto::Bytes`'s `Debug` form, the text the interpreter prints. An entry parameter of type `Bytes` is refused, as for the B06 types: neither tier reads one from the command line.
- **`Secret` stays on the interpreter.** A secret comes only from a tool (`secret(name)`, `random_token(bytes)`), and native binaries do not dispatch the standard library's tools: a native tool call is a direct call into a linked `#[tool]` library, whose trace and replay paths have no position to redact, no revealed-argument form and no echo scrubbing. Making `Secret` native would therefore compile only code no native program can reach with a secret in hand, and could not be tested on both tiers. A program that mentions `Secret` anywhere keeps running on the interpreter, which keeps every guarantee; the tier picker routes it there and a native build refuses it with one shared reason (`corvid_codegen_cl::secret_boundary`). That covers `hash_password`, `verify_password`, `decode_base64_after` and an HMAC keyed by a `Secret`. Native `Secret` needs native dispatch of the standard library's tools through the runtime's recording first; it is a roadmap item.
- **Tool and prompt boundaries.** A tool or prompt whose signature holds `Bytes` runs on the interpreter, as for the B06 types (`text_scalar_boundary.rs`): a native tool would receive the base64url String with nothing to check it.
- **Parity tests** (`crates/corvid-codegen-cl/tests/parity/crypto.rs`) run each program on both tiers with balanced allocations: the FIPS 180-4 digests, round trips and decoder messages, equality inside records, lists, options and results, every RFC 4231 HMAC vector used by `corvid-crypto`'s tests, the recorded Stripe and Standard Webhooks requests with the key held as `Bytes`, the RFC 8032 vectors and six forgeries, JSON, a printed result, and the refusal of a secret. The webhook program reads its header as the driver's programs do, rebinding the timestamp inside a loop and returning from inside the loop that finds a signature; writing these tests found a native leak and double free in those two shapes, fixed since in the ownership pass.

## Syntax alternatives considered

Recorded under the syntax simplicity rule.

- **Constant-time comparison.** `constant_time_equals(a, b)` is the form other libraries offer, and people forget it. Making `==` on `Bytes` and `Secret` constant-time removes the decision.
- **Hashing text.** `text.sha256()` would hide which encoding is hashed; `text.utf8().sha256()` costs six characters and makes the bytes explicit.
- **Password cost.** A default cost is the hidden policy the project rule forbids; a cost argument named `owasp_2023()` reads as the policy it is.
- **Reading encoded bytes (slice 1).** `text.decode_hex_bytes()` would sit next to `text.decode_hex()` and differ by a suffix that is easy to miss; `Bytes.from_hex(text)` names the type it produces, reads like `Instant.from_epoch_milliseconds(n)`, and keeps the `String` methods about text.
- **Text to bytes (slice 1).** `text.to_bytes()` hides the encoding and `Bytes.from_utf8(text)` is longer for the most common call; `text.utf8()` is short and says which encoding. The reverse is `to_utf8()`, a `Result` because not every byte string is text.
- **Reading a secret (slice 2).** `secret(name)` beside `secret_read(name)` rather than `read_secret(name)` or `Secret.from_env(name)`: the module is already named for secrets, `secret` reads as the thing it returns, and keeping the environment out of the name leaves room for other sources behind the same call.
- **Comparing secrets (slice 2).** `==`, as for `Bytes`, rather than `a.matches(b)`: there is one equality, and it is the safe one.
- **Length (slice 1).** `length()`, as on `String`, `List` and `Map`, rather than `size()` or `len()`: one name for every collection.
- **HMAC (slice 3).** `body.utf8().hmac_sha256(key)`, a method on the bytes being authenticated like `sha256()`, rather than `hmac_sha256(key, body)` or `Hmac.sha256(key).update(body).finish()`: a webhook check is one call, and the message's encoding is visible. One method takes a `Bytes` or a `Secret` key rather than `hmac_sha256_secret(key)` beside `hmac_sha256(key)`, because which kind of key it is never changes what the call means.
- **A base64 signing key (slice 3).** `secret.decode_base64_after("whsec_")` returning a `Secret`. Considered: `key.without_prefix("whsec_").decode_base64()`, two `Result`s for one step that every Standard Webhooks program takes; a `Bytes` result (`Bytes.from_base64(...)` on the text), which would make the key printable; a separate `SigningKey` type, which would catch passing a decoded key to a tool at compile time but would add a second secret type to every boundary rule; and an `hmac_sha256` option that decodes the key itself, which hides the decoding inside each call. The shipped form is one call that names what it does, and its result stays a secret.

- **A token (slice 4).** `random_token(32)`, a number of bytes, rather than `random_token()` with a fixed size (one size does not fit a session id, a key and a CSRF value, and a hidden size is a hidden policy), `random_token(bits: 256)` (no named arguments, and bytes match `Bytes.length()`), or `Secret.random(32)` (the tool is in `std/crypto` with its effect, as `uuid_v4` is in `std/uuid`). The result is a `Secret` rather than `Bytes`, which a program could print, and rather than `Result<Secret, String>`, because a size outside the bounds is a defect, not a condition to handle.
- **A password hash (slice 4).** `hash_password(password, owasp_2023())`: considered `password.hash_password(owasp_2023())`, a method that would have to be a tool, and methods are pure; `hash_password(password)` with a default cost, the hidden policy the rule forbids; and `Argon2id(memory: ..., ...).hash(password)`, which names the algorithm at every call for no gain. The cost is a plain record, so a stronger one is `PasswordCost(65536, 3, 1)` with no new syntax.
- **Checking a password (slice 4).** `password.verify_password(hash)` rather than `verify_password(password, hash)` (a method on the secret reads as the question it asks and sits with `==`) or `hash.verify(password)` (a method on `String` that takes a `Secret` would put a cryptography method on every string). The name repeats `password` so the call is unambiguous next to `verify_ed25519`.
- **Verifying a signature (slice 5).** `message.verify_ed25519(signature, public_key)`, as the design wrote it: a method on the signed bytes like `sha256()` and `hmac_sha256(key)`, named for the algorithm because a verifier must know which one it checks. Considered: `verify_ed25519(message, signature, public_key)`, a free function whose three `Bytes` arguments are easier to swap; `public_key.verify(message, signature)`, which reads well but puts a generic `verify` on every `Bytes` value and hides the algorithm; and `Ed25519.verify(...)`, a namespace for one call. The result is a `Bool` (see As built).

## Slices

1. **`Bytes` and hashing on the interpreter.** The `corvid-crypto` crate, the `Bytes` type with constant-time equality, the conversions (including B03's deferred byte codecs) and SHA-2.
2. **`Secret` on the interpreter.** The type and its compile errors, `secret(name)`, redaction of `Secret` tool arguments in traces, refusal at prompts, JSON, tables and the client contract, and the invention contract.
3. **HMAC and the webhook acceptance test.** `hmac_sha256` and `hmac_sha512`; verify a recorded payment-provider webhook signature and reject a tampered body.
4. **Tokens and passwords.** `random_token`, `hash_password` with a declared cost, `verify_password`, and the API-key store moved onto `corvid-crypto`.
5. **Ed25519 verification.** Strict `verify_ed25519` on `Bytes`, RFC 8032's vectors, and a signed release verified through the driver.
6. **Native code** for all of the above, with parity tests.

## Deferred

- **Ed25519 signing and key storage**, which need a design for long-lived keys.
- **Raw request bodies in routes**, needed to verify a live webhook over HTTP; B16 (verified webhooks for a payment provider) owns it.
- **Encryption** (AES-GCM, ChaCha20-Poly1305), until a backend example needs it.
