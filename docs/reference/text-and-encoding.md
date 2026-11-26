# Text and encoding

Corvid's text methods encode and decode text, parse UUIDs and match regular expressions. They are builtin `String` methods, so they need no import. They are pure: they run in `@deterministic` agents, are not traced, and give the same result on every run and in replay.

Status: B03 in progress. Everything here runs on the interpreter. Encoding, UUID parsing and every regex method (`contains_regex`, `find_regex`, `find_all_regex`, `replace_regex`, `split_regex`) also compile to native code, with the same results and messages; native code holds a `RegexMatch` as the same record the interpreter does. `uuid_v4` and `uuid_v7` run in native code through the runtime, so a native run records and replays them as the interpreter does. [Proposal](../proposals/text-and-encoding.md)

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

Bytes that are not text, such as cryptographic digests, are a `Bytes` value, with the same encoders and decoders as `bytes.to_hex()` and `Bytes.from_hex(text)`. See [Cryptography](cryptography.md).

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

### Generating UUIDs

```corvid
import "std:uuid" use uuid_v7

agent new_order_id() -> String:
    return uuid_v7()
```

| Tool | Result |
| --- | --- |
| `uuid_v4()` | A random UUID (RFC 9562 version 4). |
| `uuid_v7()` | A time-ordered UUID (RFC 9562 version 7): the Unix time in milliseconds, then random bits. Later UUIDs sort after earlier ones, which keeps database indexes compact. UUIDs from the same millisecond are ordered randomly. |

Both return the canonical form and use the `uuid_generate` effect. Generation is not pure, so these are tools in `std/uuid` rather than builtin methods: each call is traced, replay returns the recorded UUID, and the compiler refuses them inside `@deterministic` agents. The random bits come from the operating system's entropy source. Native programs call them through the runtime, with the same trace and replay.

## Regular expressions

```corvid
agent order_numbers(text: String) -> List<String>:
    numbers: List<String> = []
    for m in text.find_all_regex("#(\\d+)"):
        numbers.append(m.groups[1].unwrap_or(""))
    return numbers

agent us_to_iso(text: String) -> String:
    return text.replace_regex("(?<month>\\d{2})/(?<day>\\d{2})/(?<year>\\d{4})", "${year}-${month}-${day}")
```

| Method | Result |
| --- | --- |
| `text.contains_regex(pattern)` | `Bool`: whether the pattern matches anywhere in the text |
| `text.find_regex(pattern)` | `Option<RegexMatch>`: the first match |
| `text.find_all_regex(pattern)` | `List<RegexMatch>`: every match, left to right, without overlaps |
| `text.replace_regex(pattern, replacement)` | `String`: every match replaced |
| `text.split_regex(pattern)` | `List<String>`: the text between matches |

### The compiler checks every pattern

The pattern must be a string literal, and the compiler parses it. An invalid pattern is a compile error naming the problem and its position in the pattern:

```text
invalid regular expression in `find_regex`: unclosed group at position 0
```

A pattern held in a variable is a compile error too, because the compiler could not check it. To search for text computed at run time, use `contains`, which needs no escaping.

Corvid strings treat `\` as an escape, so a pattern doubles its backslashes: write `"\\d+"` for `\d+`.

### Syntax

Patterns use the syntax of Rust's `regex` crate, which is the familiar Perl and Python syntax: classes such as `[a-z]` and `\d`, groups, alternation, `*`, `+`, `?` and `{n,m}` repetition, anchors, `(?i)` for case-insensitive matching, and named groups written `(?<name>...)` or `(?P<name>...)`. Backreferences such as `\1` and lookaround such as `(?=...)` are not supported, and the compiler says so. In exchange, matching takes time proportional to the length of the text for every pattern, so no pattern or input can stall a server.

Matching is Unicode-aware. `.` matches one character, and `\w`, `\d` and `\s` include letters, digits and spaces from every script. `(?i)` folds case with Unicode's simple case folding.

### `RegexMatch`

`find_regex` and `find_all_regex` return `RegexMatch` records. `RegexMatch` can be named in a signature, such as `agent describe(m: RegexMatch) -> String`.

| Field | Type | Meaning |
| --- | --- | --- |
| `text` | `String` | The matched text. |
| `start` | `Int` | The position of the first matched character. |
| `end` | `Int` | The position just after the match. |
| `groups` | `List<Option<String>>` | The capture groups. `groups[0]` is the whole match, `groups[1]` is the first group, and so on. A group that took no part in the match is `None`. |

Positions count characters, like `length()` and `substring`, so `text.substring(m.start, m.end)` is always `m.text`. A group's index is the number its replacement reference uses: `groups[1]` and `$1` are the same group.

A `RegexMatch` has no JSON form, because its positions point into one particular text. Copy the fields you need into a record you declare to send or store them.

### Replacements

In a replacement written as a string literal:

- `$1` or `${1}` inserts group 1, and `$0` inserts the whole match.
- `$name` or `${name}` inserts the named group.
- `$$` inserts a dollar sign.

The compiler checks every reference. A group the pattern does not define is a compile error, and so is a `$` that starts no reference. `$1a` is read as a group named `1a`, so the compiler suggests `${1}a` to follow group 1 with the letter `a`.

A replacement computed at run time, such as a parameter, is inserted exactly as written, with no `$` expansion. Data from a user can never be read as a template:

```corvid
agent mask(text: String, replacement: String) -> String:
    return text.replace_regex("\\d+", replacement)
```

`mask("pin 1234", "$1")` returns `"pin $1"`.

### Splitting

`split_regex` keeps empty pieces, as `split` does: `"a, b,,c".split_regex(",\\s*")` is `["a", "b", "", "c"]`. A pattern that can match empty text, such as `"\\s*"`, is a compile error, because splitting on an empty match has no single sensible result. Require at least one character instead.

## One implementation

Every rule on this page lives in one Rust crate, `corvid-text`. The interpreter calls it directly, and native code calls the same functions through the runtime, so the two tiers cannot disagree on an encoding or a match. Parity tests run each method on both tiers and compare the results, including error messages and positions. The compiler checks patterns and replacements with the same crate, so a pattern it accepts is a pattern the runtime compiles.
