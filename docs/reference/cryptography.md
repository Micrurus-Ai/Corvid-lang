# Cryptography

`Bytes` holds a byte string: a digest, a MAC tag, a signature, a key or a binary payload. The SHA-2 hashes, HMAC and Ed25519 signature verification are methods on it. Every `Bytes` method is pure: it runs in `@deterministic` agents, is not traced, and gives the same result on every run and in replay. `Secret` holds sensitive text, such as a signing key, a password or an API token, that the program can use but never reveal; see [Secrets](#secrets). `std/crypto` issues random tokens as secrets and hashes passwords with Argon2id at a declared cost; see [Random tokens](#random-tokens) and [Passwords](#passwords).

Status: B04 in progress. `Bytes`, SHA-2 hashing, HMAC, Ed25519 verification, `Secret`, random tokens and password hashing run on the interpreter. `Bytes`, hashing, HMAC with a `Bytes` key and Ed25519 verification also compile to native code, and so do reading, holding and comparing a `Secret` (see [Native code](#native-code)). Ed25519 signing is not offered yet. [Proposal](../proposals/cryptography.md)

```corvid
agent content_hash(body: String) -> String:
    return body.utf8().sha256().to_hex()

agent digest_matches(body: String, received_hex: String) -> Bool:
    return match Bytes.from_hex(received_hex):
        Ok(received) -> body.utf8().sha256() == received
        Err(problem) -> false
```

`content_hash("abc")` returns `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad`. `digest_matches` compares the computed digest with the received one in constant time, because that is what `==` does on `Bytes`.

## Bytes from text and back

Text becomes bytes only through `utf8()`, and bytes become text only through a named encoding, so a program always says which encoding it means.

| Call | Result |
| --- | --- |
| `text.utf8()` | `Bytes`, the UTF-8 encoding of the text |
| `bytes.to_utf8()` | `Result<String, String>`; `Err` when the bytes are not UTF-8 |
| `bytes.length()` | `Int`, the number of bytes. `"café".utf8().length()` is 5. |
| `bytes.to_hex()` | `String`, two lowercase digits per byte |
| `bytes.to_base64()` | `String`, the standard alphabet with `=` padding (RFC 4648 section 4) |
| `bytes.to_base64_url()` | `String`, the URL-safe alphabet without padding (RFC 4648 section 5) |
| `Bytes.from_hex(text)` | `Result<Bytes, String>`; either case |
| `Bytes.from_base64(text)` | `Result<Bytes, String>` |
| `Bytes.from_base64_url(text)` | `Result<Bytes, String>` |

The decoders are the ones the [text decoders](text-and-encoding.md#encoding) use, with the same rules and messages: padding is optional, anything else non-canonical is an error, and the error names the first problem and its position. They differ only in what they return: `Bytes.from_hex("ff")` is `Ok`, where `"ff".decode_hex()` is an `Err`, because the byte `ff` is not UTF-8 text.

| Call | Result |
| --- | --- |
| `Bytes.from_hex("abc")` | `Err("not hex: 3 digits cannot encode whole bytes; hex needs two digits per byte")` |
| `Bytes.from_base64("Zm9v!A==")` | ``Err("not base64: unexpected character `!` at position 4")`` |
| `Bytes.from_base64_url("ab+c")` | ``Err("not URL-safe base64: unexpected character `+` at position 2")`` |
| `Bytes.from_hex("61ff")` then `to_utf8()` | `Err("the bytes are not UTF-8 text (byte 1 starts an invalid sequence)")` |

## Hashing

| Call | Result |
| --- | --- |
| `bytes.sha256()` | `Bytes`, 32 bytes |
| `bytes.sha384()` | `Bytes`, 48 bytes |
| `bytes.sha512()` | `Bytes`, 64 bytes |

The digests are FIPS 180-4's. Hashing text is written `body.utf8().sha256()`, so the encoding being hashed is visible. MD5 and SHA-1 are not offered, because they are broken for the uses people reach for them in.

## HMAC

| Call | Result |
| --- | --- |
| `bytes.hmac_sha256(key)` | `Bytes`, the 32-byte HMAC-SHA256 tag of the bytes |
| `bytes.hmac_sha512(key)` | `Bytes`, the 64-byte HMAC-SHA512 tag |

The tags are RFC 2104's HMAC over FIPS 180-4's hashes, and match RFC 4231's test vectors. The key is `Bytes` or a `Secret`:

- **A `Secret`** keys the HMAC with the UTF-8 bytes of its text. The text never becomes a value: the HMAC reads it inside the cryptography library, and the program sees only the tag.
- **`Bytes`** keys it with those bytes, for a key the program holds as bytes, such as `Bytes.from_hex(text)?`.
- **A `String` is refused**, with a hint to read the key with `secret(name)`: a key typed into text is one step from a log, and the type keeps it out of every trace. A key held as text that is not a secret is written `key.utf8()`.

Like every `Bytes` method, HMAC is pure: allowed in `@deterministic` agents and never traced, so a trace holds neither the key nor a record of the call. Compare a computed tag with a received one using `==`, which is constant-time.

### A key written in base64

Some providers write the signing key in base64 after a prefix: a Standard Webhooks secret (used by Svix and others) is `whsec_` followed by the base64 of the key's bytes, and the HMAC is keyed with the decoded bytes, not the text. `decode_base64_after` turns such a secret into the key, which is still a `Secret`:

```corvid
key = secret("STANDARD_WEBHOOK_SECRET")?.decode_base64_after("whsec_")?
```

`secret.decode_base64_after(prefix) -> Result<Secret, String>` strips `prefix` and decodes the rest as standard base64 (padding optional). It is `Err` when the secret does not start with the prefix (``the secret does not start with `whsec_` ``), when the rest is not base64 (``the secret is not base64 after `whsec_` ``) or when no key is left (``the secret holds no key after `whsec_` ``). The messages never quote the secret: unlike `Bytes.from_base64`, they name neither the character nor its position. The prefix must be a string literal, because a prefix computed at run time could test the secret's text one guess at a time; a literal states only the provider's public format. For a secret that is base64 with no prefix, write `decode_base64_after("")`.

A decoded key is bytes, not text, so it has no form a tool could receive: passing one to a tool stops the run with an error that says so. Use it for `hmac_sha256` and `hmac_sha512` and compare it with `==`.

### Verify a webhook

A payment provider signs each webhook request with an HMAC of its raw body under a key it shares with the receiver. Stripe sends `Stripe-Signature: t=<timestamp>,v1=<hex>`, where the signed payload is the timestamp, a `.` and the raw body, and the key is the signing secret's text (`whsec_...`, used as written):

```corvid
import "./std/secrets" use secret

agent verify_stripe(signature_header: String, body: String) -> Result<Bool, String>:
    key = secret("STRIPE_WEBHOOK_SECRET")?
    timestamp = ""
    for part in signature_header.split(","):
        if part.starts_with("t="):
            timestamp = part.substring(2, part.length())
    if timestamp == "":
        return Err("the signature header has no timestamp")
    expected = (timestamp + "." + body).utf8().hmac_sha256(key)
    for part in signature_header.split(","):
        if part.starts_with("v1="):
            matched = match Bytes.from_hex(part.substring(3, part.length())):
                Ok(received) -> received == expected
                Err(problem) -> false
            if matched:
                return Ok(true)
    return Ok(false)
```

A Standard Webhooks request sends `webhook-id`, `webhook-timestamp` and `webhook-signature: v1,<base64>` (space-separated when there are several); the signed payload is the id, the timestamp and the body joined by `.`, keyed by the decoded key:

```corvid
agent verify_standard_webhook(message_id: String, timestamp: String, signature_header: String, body: String) -> Result<Bool, String>:
    encoded = secret("STANDARD_WEBHOOK_SECRET")?
    key = encoded.decode_base64_after("whsec_")?
    expected = (message_id + "." + timestamp + "." + body).utf8().hmac_sha256(key)
    for entry in signature_header.split(" "):
        if entry.starts_with("v1,"):
            matched = match Bytes.from_base64(entry.substring(3, entry.length())):
                Ok(received) -> received == expected
                Err(problem) -> false
            if matched:
                return Ok(true)
    return Ok(false)
```

Both programs are the acceptance test, run through the driver against recorded requests whose signatures were computed outside Corvid: the recorded requests verify, and a changed body byte, timestamp, message id, signature or key is rejected. The trace of a verification holds `<redacted>` for the secret read and nothing for the HMAC. Every request is checked in full: `==` on `Bytes` takes the same time whichever byte differs, so a forged signature cannot be guessed byte by byte.

These programs check the signature only. A complete verifier also refuses a timestamp too far from the current time, so a recorded request cannot be replayed later, and reads the body exactly as it arrived. A reusable verifier, its declared tolerance, and raw request bodies in routes come with verified webhooks (B16).

## Signatures

| Call | Result |
| --- | --- |
| `message.verify_ed25519(signature, public_key)` | `Bool`: whether `signature` is the signer's Ed25519 signature of the bytes |

All three are `Bytes`: the signed message, the 64-byte signature and the signer's 32-byte public key. An Ed25519 signature (RFC 8032) is made with a private key the receiver never sees and checked with the public key, so anyone can verify a release, a signed token or a webhook from a provider that signs with a public key, and nobody but the signer can make one. Like every `Bytes` method it is pure: allowed in `@deterministic` agents, never traced, and the same in replay.

```corvid
agent signed_by_publisher(body: String, signature_base64: String) -> Result<Bool, String>:
    signature = Bytes.from_base64(signature_base64)?
    public_key = Bytes.from_hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")?
    return Ok(body.utf8().verify_ed25519(signature, public_key))
```

The answer is `false` for a forged or altered signature, a changed message or another signer's key, and also for a signature that is not 64 bytes or a public key that is not 32 bytes or not a point on the curve. A malformed signature verifies nothing, so it gets the same answer as a forged one; a decoding mistake in the hex or base64 is still an `Err` from `Bytes.from_hex` or `from_base64`. A 64-byte key is usually a private key with its public half appended, as some tools print it: pass only the public half.

**Verification is strict.** RFC 8032 lets verifiers differ on edge cases, and the permissive reading accepts signatures nobody made: with a public key of small order, such as the identity point, a signature of 63 zero bytes after a single `01` verifies for every message. Corvid refuses a public key or a signature point R of small order, and an S that is not reduced below the group order, so a valid signature cannot be altered into a second valid one. This is the `verify_strict` check of the `ed25519-dalek` library. A signature from any conforming signer verifies; RFC 8032's test vectors do.

### Verify a signed release

`corvid release` signs its manifest as a DSSE envelope: the signature covers `DSSEv1 <type length> <type> <payload length> <payload>`, so it cannot be moved to a payload of another type. Corvid checks one with string methods and `verify_ed25519`:

```corvid
fn pre_authentication_encoding(payload_type: String, payload: String) -> Bytes:
    type_length = payload_type.utf8().length().to_string()
    payload_length = payload.utf8().length().to_string()
    return ("DSSEv1 " + type_length + " " + payload_type + " " + payload_length + " " + payload).utf8()

@deterministic
agent verify_release(payload_type: String, payload_base64: String, signature_base64: String, public_key_hex: String) -> Result<Bool, String>:
    if payload_type != "application/vnd.corvid.release.manifest.v1+json":
        return Ok(false)
    payload = Bytes.from_base64(payload_base64)?.to_utf8()?
    signature = Bytes.from_base64(signature_base64)?
    public_key = Bytes.from_hex(public_key_hex)?
    return Ok(pre_authentication_encoding(payload_type, payload).verify_ed25519(signature, public_key))
```

A driver test signs a release manifest with the release tooling's own signing function and runs this program: the signed release verifies, and a changed manifest byte, another manifest's signature, a changed signature character, another payload type and another signer's key are rejected, each with the same answer as the tooling's verifier. The lengths are byte counts, read from `utf8()`. The manifest is JSON text, so the encoding is built as a string; a binary payload would need byte concatenation, which `Bytes` does not offer.

Signing is not offered: a private key is a long-lived credential whose storage needs its own design.

## Comparing bytes

`==` and `!=` on `Bytes` are constant-time: comparing two byte strings of the same length takes the same time whichever bytes differ, so comparing a computed signature with a received one cannot leak the signature byte by byte. Byte strings of different lengths are unequal at once, which reveals only their lengths. The comparison is the same wherever it happens: between two values, between records or lists that hold bytes, and in `list.contains(bytes)`.

Every other operator is a compile error that says why:

| Expression | Error hint |
| --- | --- |
| `a < b`, `<=`, `>`, `>=` | byte strings have no order, because an ordered comparison stops at the first differing byte |
| `a + b` | `Bytes` supports only `==` and `!=` |
| `a == "text"`, `"digest: " + a` | bytes and text do not mix; convert with `utf8()` or a named encoding |

## JSON

In JSON, at routes, tools, prompts, stores and `to_json`, a `Bytes` value is a string in unpadded base64url, such as `"3q2-7w"` for the bytes `de ad be ef`. Decoding accepts the string with or without `=` padding and refuses anything else, naming the field. OpenAPI and prompt schemas publish it as `type: string` with `contentEncoding: base64url`, and the TypeScript client and the SDKs hold it as a string. A table column of type `Bytes` is a compile error until B09 maps its storage; store `to_base64_url()` in a `String` column and read it back with `Bytes.from_base64_url(text)`.

A tool that takes or returns `Bytes` receives and returns that base64url text, and the trace records it.

## Native code

A compiled program holds a `Bytes` value as its unpadded base64url text, the text its JSON form carries, and every method is one call into the runtime, which computes with `corvid-crypto`, the code the interpreter calls. Both tiers return the same digests, tags, verdicts and error messages, and `to_json` and `from_json` read and write the same text. `==` and `!=` compare the two texts in constant time; the text is one to one with the bytes, so that is the same answer the interpreter's byte comparison gives, without decoding a digest first. Records, lists, options and results holding bytes compare the same way. An entry agent may return `Bytes`, printed as `Bytes(<hex>)` on both tiers; it cannot take one from the command line, so take a `String` and decode it with `Bytes.from_hex(text)` or `Bytes.from_base64_url(text)`.

These run on the interpreter, chosen automatically by `corvid run` and refused by name in a native build:

- **A secret passed to a tool of the program's own, or to `hash_password`, and a secret that keys an HMAC, `verify_password` or `decode_base64_after`.** Native code holds a secret as the runtime's opaque object: `secret(name)` and `random_token(bytes)` run through the runtime, which records their results as `<redacted>`; a secret can sit in a local, a record, a list, an option or a result; and `==` and `!=` compare secrets in constant time, through the same `corvid_crypto::Secret` the interpreter uses. A native call to a tool of the program's own records its arguments without redaction, so a program that passes a secret to one runs on the interpreter, and so does one that hashes a password or keys an HMAC with a secret, until native secret cryptography lands (native dispatch N3b and N3c). To verify natively today, key the HMAC with `Bytes`.
- **A tool or prompt whose signature holds `Bytes`**, as for the B06 types: native tool and prompt bridges have no `Bytes` form.
- **`Bytes` as a map value**, since native maps hold `String`, `Int` and record values, and **`list.contains(bytes)`**, since native code compiles `contains` on no list yet.

## Secrets

A `Secret` is text the program needs but must never expose: a webhook signing key, a password, an API token. `secret(name)` from `std/secrets` reads one from the environment:

```corvid
import "./std/secrets" use secret

tool sign_webhook(key: Secret, body: String) -> String

agent signature(body: String) -> Result<String, String>:
    key = secret("WEBHOOK_KEY")?
    return Ok(sign_webhook(key, body))

agent token_matches(supplied: Secret) -> Result<Bool, String>:
    expected = secret("API_TOKEN")?
    return Ok(supplied == expected)
```

`secret(name) -> Result<Secret, String>` is `Err` when the variable is missing, unreadable or the name is empty, because there is no secret to return. `secret_read(name)` still returns the text as a `String` for a program that must handle it; prefer `secret`.

### What a program can do with a secret

- **Key an HMAC.** `body.utf8().hmac_sha256(key)` keys the HMAC with the secret's bytes; the program sees only the tag (see [HMAC](#hmac)). `key.decode_base64_after("whsec_")` turns a base64 signing secret into the key it encodes, which is again a `Secret`.
- **Check it against a stored hash.** `password.verify_password(hash)` is a `Bool` (see [Passwords](#passwords)), and `hash_password(password, owasp_2023())` makes the hash to store.
- **Compare two.** `==` and `!=` between two secrets are constant-time, with the same comparison as `Bytes`: secrets of the same length take the same time whichever characters differ, and secrets of different lengths are unequal at once, which reveals only the lengths. Records and lists holding secrets, and `list.contains(secret)`, compare the same way.
- **Pass one to a tool.** A tool may take a `Secret` parameter, alone or inside a record, a list or another type argument. The tool's implementation receives the text. Every record of the call holds `<redacted>` in that position (see [Traces and replay](#traces-and-replay)).
- **Hold one.** A secret can sit in a local, a record field, a list or a `Result`, and travel between agents.

Displaying a secret, in a run's printed result, the REPL, a debugger or a Rust panic, shows `<secret>`.

### What the compiler refuses

Each of these is a compile error that names the place and says what to write instead.

| Written | Why it is refused |
| --- | --- |
| `key.to_string()`, `key.to_json()`, any method on a secret but `decode_base64_after` and `verify_password` | No method returns a secret's text; `decode_base64_after` returns another secret and `verify_password` a `Bool` |
| `key + "x"`, `"Bearer " + key`, `key == "text"` | A secret and text do not mix: the result would be ordinary text holding the secret |
| `a < b` and the other orderings, `a + b`, arithmetic | Secrets have no order; an ordered comparison stops at the first differing character |
| `Credentials.from_json(text)`, `creds.to_json()` with a `Secret` field | A secret has no JSON form, in a record field too |
| A route path, query, body or response holding a secret | A route's inputs and response are JSON a client sees |
| A store field holding a secret | A store keeps its fields as JSON |
| A table column holding a secret | A stored secret could be read back. To check a value later, store `hash_password(value, owasp_2023())` in a `String` column and check it with `value.verify_password(hash)`; to use the value again it needs an encrypted column, which is not built yet (tracked in B09). Never store the text |
| A prompt parameter or result holding a secret, including a function a model can call | No secret may reach a model, and a model cannot produce one |
| A connector operation's parameter or result holding a secret | A connector's credential is declared once, as its `auth`, and never travels in its operations |
| `ask(question, Secret)`, `choose(secrets)` | A person's answer is recorded, and `choose` shows its options |
| `agent main` taking or returning a secret | `main` is run from the command line, which prints its result and passes arguments as text |

A record holding a secret counts wherever the secret would: `Credentials` with a `password: Secret` field is refused as a route body, a prompt parameter or a table column, and the message names the field. A literal is never a secret: `sign_webhook("whsec_...", body)` is a type error, so a key cannot be written into source by mistake.

`corvid run` also refuses to run a file whose only agent returns a secret, because it would print the result.

### Traces and replay

A tool call whose declared parameters hold a `Secret` is built twice. The *recorded* arguments have `<redacted>` at every secret position, and everything that records or compares the call uses them: the `ToolCall` trace event, approvals, the step debugger, replay, and the observation and OpenTelemetry exports built from the trace. The *revealed* arguments carry the text and go only to the tool's implementation. The text is read from a `Secret` in one place in the interpreter, when it builds the revealed arguments.

```text
{"type":"tool_call","tool":"sign_webhook","args":["<redacted>","order.paid"],...}
```

`secret(name)` and `random_token(bytes)` record their result as `<redacted>` too, with no characters kept.

**A tool that echoes a secret back.** If a tool's result or error message contains the text of a secret it was given, the record is scrubbed and the program's value is not. The program receives the tool's real result or error, unchanged. Only the records of the call are scrubbed: the `ToolResult` trace event and the step debugger's view of the result have every occurrence of the text replaced by `<redacted>`. A scrubbed `ToolResult` is marked inside its payload, as the tool-error record already is, so the trace schema does not change:

```text
{"kind":"tool_result","tool":"echo_key","result":{"__corvid_secret_scrubbed__":"key=<redacted>"}}
{"kind":"tool_result","tool":"fail","result":{"__corvid_secret_scrubbed__":true,"__corvid_tool_error__":"tool `fail` failed: bad key <redacted>"}}
```

Once the program holds the echoed text, it is an ordinary `String`. Wherever the program sends it, it is recorded like any other text, including the run's own recorded result if an agent returns it. A tool that returns a secret's text has turned it into plain text, so keep such tools out of a program that must never record the secret.

**Replaying a scrubbed result.** Replay cannot reproduce text the trace never held. When it substitutes a marked result, it returns the recorded, scrubbed form (`key=<redacted>`, or the scrubbed error) and prints a warning that names the tool:

```text
warning: replaying tool `echo_key` with its recorded result, in which a secret the tool echoed was replaced by `<redacted>`; the live run received the real result, so this replay can differ from it from here on
```

The warning is kept for `Runtime::replay_secret_scrub_warnings()`, as the tz-database warning is for `replay_zone_rules_warnings()`. If the scrubbed value changes what the run computes, replay also reports the divergence where the recorded run's events stop matching. It never differs from the recording silently.

Replay re-reads every secret from the environment rather than from the trace, as `secret_read` does. Because the replayed run's arguments are also `<redacted>` at the secret's position, a recorded call matches whatever secret the replay read: a rotated key still replays, and the trace never needs, or holds, the text.

### The application contract

The application contract publishes no secret. A public type's field that holds a `Secret` is left out of the contract, so the TypeScript client and the SDKs have no such field, and a public agent whose signature holds one is server-only and not listed. The ABI descriptor names a `Secret` parameter's type, so a host sees the declaration, but no value of it crosses the boundary.

### Not yet

Native code cannot yet pass a secret to a tool of the program's own or key cryptography with one (see [Native code](#native-code)). Secrets come from the environment (`secret(name)`) and from `random_token(bytes)`; a request body cannot carry one yet, so a login form's password cannot reach an agent through a route (see [Passwords](#passwords)). A tool's own logging of the text it receives is outside what Corvid can see. A key made by `decode_base64_after` cannot be passed to a tool.

## Random tokens

`random_token(bytes)` from `std/crypto` draws `bytes` random bytes from the operating system's secure generator and returns them as a `Secret` holding their URL-safe base64 text without padding, so the token travels in a URL, a header or a cookie unchanged:

```corvid
import "./std/crypto" use random_token

agent new_session_key() -> Secret:
    return random_token(32)
```

`random_token(32)` is 43 characters and holds 256 bits. `bytes` must be from 16 to 1024. Fewer than 16 bytes (128 bits) is refused, because a shorter token can be guessed by an attacker with enough attempts, and more than 1024 is refused as a mistake, such as a length in bits or characters written where bytes are meant. Either stops the call with an error that says so.

Because a token is a `Secret`, everything [Secrets](#secrets) says applies: the program can compare it with `==` in constant time, hash it, check it with `verify_password` and pass it to a tool, and it can never be printed, turned into text, stored or sent to a model. Its recorded `ToolResult` is `<redacted>`:

```text
{"kind":"tool_call","tool":"random_token","args":[32],...}
{"kind":"tool_result","tool":"random_token","result":"<redacted>",...}
```

`random_token` is a tool, as `uuid_v4` is, because it draws entropy: a `@deterministic` agent cannot call it.

### Replaying a token

The trace never holds a token, so replay has nothing to substitute. Refusing to replay would make every program that issues a session or an API key unreplayable, so replay draws a fresh token and prints a warning:

```text
warning: replay drew a fresh token for `random_token`, because the recorded token was never stored in the trace; anything this run computes from the token's value can differ from the recording from here on
```

The warning is kept for `Runtime::replay_redrawn_token_warnings()`. A token only ever reaches positions every record holds as `<redacted>`, so a later call that takes it, such as `hash_password(key, owasp_2023())` or the tool that delivers it, still matches its recording and is substituted: a replayed issuance returns the recorded hash. Where the fresh token changes what the run computes, such as `key.verify_password(hash)` against a hash recorded from the old token, replay reports the divergence at the first event that no longer matches, here the run's result. It never differs from the recording silently.

### Showing a new key to its owner once

A key the program issues must reach its owner exactly once, and never a trace. Deliver it through a tool that takes a `Secret`: the tool receives the text, every record of the call holds `<redacted>`, and the program keeps only the hash.

```corvid
import "./std/crypto" use random_token, hash_password, owasp_2023

tool email_api_key(owner: String, key: Secret) -> Bool

agent issue_api_key(owner: String) -> String:
    key = random_token(32)
    sent = email_api_key(owner, key)
    return hash_password(key, owasp_2023())
```

A route cannot return a secret, so a key cannot yet be shown once in an HTTP response, as a "copy your new key" page needs. That needs the response to be written without being recorded, a contract marker so the client shows the value once and never caches it, and the frontend binding; it is a roadmap item.

## Passwords

`hash_password(password, cost)` from `std/crypto` hashes a password, or any secret the program only needs to check later such as an API key, with Argon2id (RFC 9106) under a fresh 16-byte random salt. It returns the hash as a PHC string, which records the algorithm, its version, the cost and the salt:

```text
$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>
```

`password.verify_password(hash)` on a `Secret` says whether the secret is the password the hash was made from. It reads the cost and the salt from the hash, so a hash made at any cost verifies, and compares the computed hash with the stored one in constant time.

### The cost is declared

Argon2id's memory, iterations and lanes trade login latency against the cost of guessing passwords from a stolen database, and there is more than one reasonable setting, so the program states which one it uses. `owasp_2023()` is the OWASP Password Storage Cheat Sheet's 2023 minimum for Argon2id: 19 MiB (19456 KiB) of memory, 2 iterations and 1 lane.

```corvid
hash = hash_password(password, owasp_2023())
stronger = hash_password(password, PasswordCost(65536, 3, 1))
```

`PasswordCost(memory_kib, iterations, lanes)` declares another cost. Any value may be higher than the minimum. A cost weaker than every setting OWASP lists as equal to the minimum (47104 KiB with 1 iteration, 19456 KiB with 2, 12288 KiB with 3, 9216 KiB with 4, 7168 KiB with 5) stops the call with an error that names them:

```text
tool `hash_password` failed: a password cost of 4096 KiB and 1 iteration is below the OWASP 2023 minimum for Argon2id; use owasp_2023() (19456 KiB, 2 iterations, 1 lane) or a setting OWASP lists as equal: 47104 KiB with 1 iteration, 12288 KiB with 3, 9216 KiB with 4 or 7168 KiB with 5
```

A cost above 4 GiB of memory is refused too, because a few concurrent logins would exhaust a server. Raising the cost later needs no migration: old hashes record the cost they were made at and keep verifying, and a program can rehash a user's password at the new cost when they next log in.

The runtime's API-key store (`corvid auth keys`) hashes with the same function at the same declared cost, `owasp_2023()`, so the language and the runtime hash one way. Keys it stored before this slice used the same setting and keep verifying.

### What is recorded

`hash_password` is a tool, because it draws the salt. Its password argument is a `Secret`, so every record of the call holds `<redacted>` in its place, and the trace never holds the password. Its result, the hash, is a verifier rather than the password, so it is recorded and replay substitutes it like any tool result:

```text
{"kind":"tool_call","tool":"hash_password","args":["<redacted>",{"iterations":2,"lanes":1,"memory_kib":19456}],...}
{"kind":"tool_result","tool":"hash_password","result":"$argon2id$v=19$m=19456,t=2,p=1$...",...}
```

`verify_password` is a pure method: allowed in `@deterministic` agents, never traced, and the same on every run and in replay. It is `false` for a hash that is malformed, made by another algorithm (argon2i, argon2d, bcrypt) or another Argon2 version, or that claims more than 4 GiB of memory: a hash it cannot read verifies nothing. A program that must tell a corrupted stored hash from a wrong password can check the stored text with `starts_with("$argon2id$")` before verifying.

### Log in

A sign-up stores the hash and a log-in checks a password against it:

```corvid
import "./std/crypto" use hash_password, owasp_2023

tool save_password_hash(user: String, hash: String) -> Bool
tool stored_password_hash(user: String) -> Option<String>

agent sign_up(user: String, password: Secret) -> Bool:
    return save_password_hash(user, hash_password(password, owasp_2023()))

agent log_in(user: String, password: Secret) -> Bool:
    return match stored_password_hash(user):
        Some(hash) -> password.verify_password(hash)
        None -> false
```

A table can hold the hash in a `String` column, never the password. Both agents take the password as a `Secret`, and today one arrives from `secret(name)` or `random_token`, or from another agent. A route's body cannot hold a secret yet, so a login form cannot send its password field to `log_in` directly; that needs the request body's password field to be a `Secret` the server reads without recording, with the masked form input from the frontend roadmap. Both are roadmap items.

The driver test `crates/corvid-driver/tests/tokens_and_passwords.rs` runs these steps from Corvid source: a password read with `secret(name)` is hashed, the right password verifies and a wrong one does not, the trace holds neither the password nor an issued key, a replayed issuance returns the recorded hash with one re-drawn-token warning, and a replay whose result depends on the token reports the divergence.

`verify_password` runs Argon2id on the interpreter's own thread, tens of milliseconds at the OWASP cost; `hash_password` runs on a blocking worker. Moving verification off the request thread under heavy concurrent load is a roadmap item.
