# Cryptography

`Bytes` holds a byte string: a digest, a MAC tag, a signature, a key or a binary payload. The SHA-2 hashes and HMAC are methods on it. Every `Bytes` method is pure: it runs in `@deterministic` agents, is not traced, and gives the same result on every run and in replay. `Secret` holds sensitive text, such as a signing key, a password or an API token, that the program can use but never reveal; see [Secrets](#secrets).

Status: B04 in progress. `Bytes`, SHA-2 hashing, HMAC and `Secret` run on the interpreter. A program that uses `Bytes` or `Secret` runs on the interpreter until the native cryptography slice, and a native build that refuses one names that slice. Random tokens, password hashing and Ed25519 verification come in later slices. [Proposal](../proposals/cryptography.md)

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
- **Compare two.** `==` and `!=` between two secrets are constant-time, with the same comparison as `Bytes`: secrets of the same length take the same time whichever characters differ, and secrets of different lengths are unequal at once, which reveals only the lengths. Records and lists holding secrets, and `list.contains(secret)`, compare the same way.
- **Pass one to a tool.** A tool may take a `Secret` parameter, alone or inside a record, a list or another type argument. The tool's implementation receives the text. Every record of the call holds `<redacted>` in that position (see [Traces and replay](#traces-and-replay)).
- **Hold one.** A secret can sit in a local, a record field, a list or a `Result`, and travel between agents.

Displaying a secret, in a run's printed result, the REPL, a debugger or a Rust panic, shows `<secret>`.

### What the compiler refuses

Each of these is a compile error that names the place and says what to write instead.

| Written | Why it is refused |
| --- | --- |
| `key.to_string()`, `key.to_json()`, any method on a secret but `decode_base64_after` | No method returns a secret's text; the one method returns another secret |
| `key + "x"`, `"Bearer " + key`, `key == "text"` | A secret and text do not mix: the result would be ordinary text holding the secret |
| `a < b` and the other orderings, `a + b`, arithmetic | Secrets have no order; an ordered comparison stops at the first differing character |
| `Credentials.from_json(text)`, `creds.to_json()` with a `Secret` field | A secret has no JSON form, in a record field too |
| A route path, query, body or response holding a secret | A route's inputs and response are JSON a client sees |
| A store field holding a secret | A store keeps its fields as JSON |
| A table column holding a secret | A stored secret could be read back. To check a value later, store its hash with `hash_password(...)`, which arrives in a later cryptography slice; to use the value again it needs an encrypted column, which is not built yet (tracked in B09). Never store the text |
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

`secret(name)` records its result as `<redacted>` too, with no characters kept.

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

Native code refuses `Secret` until the native cryptography slice. Secrets come only from the environment. A tool's own logging of the text it receives is outside what Corvid can see. A key made by `decode_base64_after` cannot be passed to a tool. `random_token` and `verify_password` are later B04 slices.
