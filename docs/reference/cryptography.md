# Cryptography

`Bytes` holds a byte string: a digest, a MAC tag, a signature, a key or a binary payload. The SHA-2 hashes are methods on it. Everything on this page is pure: it runs in `@deterministic` agents, is not traced, and gives the same result on every run and in replay.

Status: B04 in progress. `Bytes` and SHA-2 hashing run on the interpreter. A program that uses `Bytes` runs on the interpreter until the native cryptography slice, and a native build that refuses one names that slice. `Secret`, HMAC, random tokens, password hashing and Ed25519 verification come in later slices. [Proposal](../proposals/cryptography.md)

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
