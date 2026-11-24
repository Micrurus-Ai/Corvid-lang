# Cryptography

`Bytes` holds a byte string: a digest, a MAC tag, a signature, a key or a binary payload. The SHA-2 hashes are methods on it. Every `Bytes` method is pure: it runs in `@deterministic` agents, is not traced, and gives the same result on every run and in replay. `Secret` holds sensitive text, such as a signing key, a password or an API token, that the program can use but never reveal; see [Secrets](#secrets).

Status: B04 in progress. `Bytes`, SHA-2 hashing and `Secret` run on the interpreter. A program that uses `Bytes` or `Secret` runs on the interpreter until the native cryptography slice, and a native build that refuses one names that slice. HMAC, random tokens, password hashing and Ed25519 verification come in later slices. [Proposal](../proposals/cryptography.md)

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

- **Compare two.** `==` and `!=` between two secrets are constant-time, with the same comparison as `Bytes`: secrets of the same length take the same time whichever characters differ, and secrets of different lengths are unequal at once, which reveals only the lengths. Records and lists holding secrets, and `list.contains(secret)`, compare the same way.
- **Pass one to a tool.** A tool may take a `Secret` parameter, alone or inside a record, a list or another type argument. The tool's implementation receives the text. Every record of the call holds `<redacted>` in that position (see [Traces and replay](#traces-and-replay)).
- **Hold one.** A secret can sit in a local, a record field, a list or a `Result`, and travel between agents.

Displaying a secret, in a run's printed result, the REPL, a debugger or a Rust panic, shows `<secret>`.

### What the compiler refuses

Each of these is a compile error that names the place and says what to write instead.

| Written | Why it is refused |
| --- | --- |
| `key.to_string()`, `key.to_json()`, any method on a secret | A secret has no methods, so none can return its text |
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

Native code refuses `Secret` until the native cryptography slice. Secrets come only from the environment. A tool's own logging of the text it receives is outside what Corvid can see. HMAC keyed by a `Secret`, `random_token` and `verify_password` are later B04 slices.
