# Text and encoding

Corvid's text methods encode and decode text, and parse UUIDs. They are builtin `String` methods, so they need no import. They are pure: they run in `@deterministic` agents, are not traced, and give the same result on every run and in replay.

Status: B03 in progress. Encoding and UUID parsing run on the interpreter. Native code, regular expressions and UUID generation follow. [Proposal](../proposals/b03-text-and-encoding.md)

## Encoding

```corvid
agent basic_auth(user: String, password: String) -> String:
    return "Basic " + (user + ":" + password).encode_base64()

agent read_token(payload: String) -> Result<String, String>:
    return payload.decode_base64_url()
```

| Method | Result |
| --- | --- |
| `text.encode_base64()` | `String`. The standard alphabet with `=` padding (RFC 4648 section 4). |
| `text.decode_base64()` | `Result<String, String>` |
| `text.encode_base64_url()` | `String`. The URL-safe alphabet (`-` and `_`) without padding (RFC 4648 section 5), as JSON Web Tokens use. |
| `text.decode_base64_url()` | `Result<String, String>` |
| `text.encode_hex()` | `String`. Two lowercase digits per byte. |
| `text.decode_hex()` | `Result<String, String>`. Accepts either case. |
| `text.encode_url_component()` | `String`. Every byte except `A-Z a-z 0-9 - . _ ~` becomes `%XX`. |
| `text.decode_url_component()` | `Result<String, String>` |

Each method works on the UTF-8 bytes of the text, so `"café".encode_hex()` is `"636166c3a9"`.

A decoder returns `Err` with a message naming the first problem and its position. Positions count characters, like `length()` and `substring`:

| Call | Result |
| --- | --- |
| `"Zm9v!A==".decode_base64()` | ``Err("not base64: unexpected character `!` at position 4")`` |
| `"4g".decode_hex()` | ``Err("not hex: unexpected character `g` at position 1")`` |
| `"100%".decode_url_component()` | ``Err("not URL-encoded: `%` at position 3 must be followed by two hex digits")`` |
| `"/w==".decode_base64()` | `Err("the decoded bytes are not UTF-8 text (byte 0 starts an invalid sequence)")` |

The decoders follow these rules:

- **Padding is optional.** Both base64 decoders accept input with or without `=` padding. Anything else non-canonical is an error, including whitespace, characters from the other alphabet and unused bits that are not zero.
- **The result must be text.** A decoder returns `Err` when the decoded bytes are not valid UTF-8.
- **`+` stays `+`.** `decode_url_component` decodes only `%XX` escapes. A `+` means a space only in HTML form bodies, and `corvid serve` decodes those before a handler runs.

`encode_url_component` keeps only the RFC 3986 unreserved characters, so its output is safe in a path segment, a query value or a form body. It is Python's `urllib.parse.quote(text, safe="")`. JavaScript's `encodeURIComponent` also leaves `!'()*` unescaped.

Bytes that are not text, such as cryptographic digests, cannot be encoded yet. Byte encoders arrive with B04, which decides how Corvid represents bytes.

## UUIDs

```corvid
agent order_key(raw: String) -> Result<String, String>:
    return raw.parse_uuid()
```

`text.parse_uuid()` returns `Result<String, String>`. On success the value is the canonical form: 36 lowercase characters hyphenated 8-4-4-4-12, such as `f81d4fae-7dec-11d0-a765-00a0c91e6bf6`. A UUID in Corvid is a `String` in this form, so two equal UUIDs are always equal strings.

`parse_uuid` accepts these spellings, in either case, after trimming surrounding whitespace:

- `f81d4fae-7dec-11d0-a765-00a0c91e6bf6`
- `f81d4fae7dec11d0a76500a0c91e6bf6`
- `{f81d4fae-7dec-11d0-a765-00a0c91e6bf6}`
- `urn:uuid:f81d4fae-7dec-11d0-a765-00a0c91e6bf6`

It accepts every version and variant, including the nil UUID. Anything else returns an `Err` that quotes the input, such as ``Err("not a UUID: `nope` (expected 32 hex digits, optionally hyphenated 8-4-4-4-12)")``.

## One implementation

Every rule on this page lives in one Rust crate, `corvid-text`. The interpreter calls it now, and native code will call the same functions, so the two tiers cannot disagree on an encoding.
