# Proposal: text and encoding (B03)

Status: **accepted, in progress.** Slices are tracked under "Task progress" in [the backend and AI roadmap](../backend-ai-roadmap.md).
Owner: backend and AI development. No parser rule changes, so no frontend agreement is needed. The one syntax idea it raises, raw string literals, is deferred to an agreement with frontend development (see [Deferred](#deferred)).

## Problem

A backend developer moving from Python or TypeScript reaches for four things in the first week: regular expressions (`re`, `RegExp`), base64 and hex (`base64`, `binascii`, `Buffer`), URL component encoding (`urllib.parse.quote`, `encodeURIComponent`) and UUIDs (`uuid`). Corvid has none of them. Today a program must declare a host tool for each, which loses purity: a tool call is refused inside `@deterministic` code, is traced, and does not compile to native code without a tools library.

## What exists today

A survey before writing any code:

- `corvid-runtime` already depends on `base64` (for its own wire formats), and `corvid-driver` and `corvid-abi` on `hex`. None of it is reachable from Corvid source.
- `corvid-runtime` hand-rolls a lowercase hex encoder (`hex_lower` in `llm_dispatch.rs`) for approval-token digests. It is internal, encodes bytes rather than text, and has no decoder, so it is left alone.
- `regex` and `uuid` are not dependencies of any Corvid crate. `regex` is in the lockfile through other crates.
- String methods live in one table, `corvid_types::builtin_methods`, which the checker, the IR lowerer, the interpreter and the native code generator all read. Shared runtime semantics for strings live in `corvid_runtime::string` (`substring`, `parse_int`), which both tiers call.
- `std/random.cor` and `std/time.cor` model nondeterminism as tools, so replay substitutes recorded results.

## Design

### Methods, not module functions

Encoding, regular expressions and UUID parsing are pure functions of their input, so they are **builtin String methods**, next to `replace` and `parse_int`. They need no import, run inside `@deterministic` code, are not traced, and replay cannot diverge on them. UUID generation is not pure, so it is a **tool** in a new `std/uuid.cor`, which makes it traced and replay-substituted exactly like `random_float`.

### One implementation for the checker and both tiers

A new crate, `corvid-text`, holds every rule: the base64, hex and URL encoders and decoders, UUID parsing and formatting, and regular expression compilation and matching. The checker calls it to validate pattern literals, and the interpreter and the native runtime call it to execute. An invalid pattern therefore cannot pass the checker and then fail at run time, and the two tiers cannot disagree on any encoding, because there is only one copy. `corvid-types` cannot depend on `corvid-runtime` (the runtime depends on the types), which is why the code is a separate crate rather than a runtime module. The precedent is `corvid-public-json`.

### Encoding

| Method | Result |
| --- | --- |
| `text.encode_base64()` | `String`, the standard alphabet with `=` padding (RFC 4648 section 4) |
| `text.decode_base64()` | `Result<String, String>` |
| `text.encode_base64_url()` | `String`, the URL-safe alphabet without padding (RFC 4648 section 5), as JSON Web Tokens use |
| `text.decode_base64_url()` | `Result<String, String>` |
| `text.encode_hex()` | `String`, lowercase |
| `text.decode_hex()` | `Result<String, String>`, either case |
| `text.encode_url_component()` | `String`, every byte outside the RFC 3986 unreserved set (`A-Z a-z 0-9 - . _ ~`) percent-encoded |
| `text.decode_url_component()` | `Result<String, String>` |

Decisions:

- **The bytes are the UTF-8 bytes of the text.** Encoders never fail. A decoder returns `Err` when the input is malformed or when the decoded bytes are not UTF-8, and the message says which, with the position of the first bad character. Bad input is never a trap.
- **Decoders accept input with or without padding.** Padding carries no information, and APIs disagree on it. Every other deviation, including a character outside the alphabet, whitespace and non-zero trailing bits, is an error, so one byte string has exactly one accepted encoding apart from padding.
- **`decode_url_component` leaves `+` as `+`.** `+` means a space only in HTML form bodies (`application/x-www-form-urlencoded`), which `corvid serve` decodes before a handler runs; treating it as a space here would corrupt base64 and phone numbers in query values. This is Python's `unquote`, not `unquote_plus`.
- **The URL encoder uses the RFC 3986 unreserved set**, not JavaScript's wider `encodeURIComponent` set, so the output is safe in every URL component, including paths and `application/x-www-form-urlencoded` bodies. This is Python's `quote(text, safe="")`.
- **No byte-list variants yet.** Binary payloads, such as the HMAC digests B04 produces, need a representation for bytes. `Upload.bytes()` returns `List<Int>`, but a list of integers admits values outside 0 to 255, so encoding one would need either a trap or a `Result` on an encoder. B04 decides the byte representation and adds the byte variants with it.

### UUIDs

| Call | Result |
| --- | --- |
| `text.parse_uuid()` | `Result<String, String>`: the canonical lowercase hyphenated form |
| `uuid_v4()` from `std/uuid` | `String`, a random UUID (RFC 9562 version 4) |
| `uuid_v7()` from `std/uuid` | `String`, a time-ordered UUID (RFC 9562 version 7) |

Decisions:

- **A UUID is a `String` in canonical form**, not a new type. Every database, JSON API and log already treats UUIDs as strings, `parse_uuid` is the validation point, and equal UUIDs compare equal as strings because the form is canonical. A distinct `Uuid` type would need a JSON form, a database mapping and an SDK mapping, and would buy only a check the parse already gives.
- **`parse_uuid` accepts the four common spellings**: hyphenated, 32 hex digits without hyphens, braced (`{...}`) and `urn:uuid:`, in either case, after trimming whitespace as `parse_int` does. It accepts any version and variant, because a parser that rejected the nil UUID or version 1 identifiers from older systems would fail on real data.
- **Generation is a tool**, so it is traced, replay returns the recorded UUID, and the checker refuses it inside `@deterministic` code. The effect is `uuid_generate`. Version 7 is offered because it sorts by creation time, which keeps database indexes compact, the reason B09 tables want it. The random bits come from the operating system's entropy source, as `random_int` does.

### Regular expressions

| Method | Result |
| --- | --- |
| `text.contains_regex(pattern)` | `Bool`: whether the pattern matches anywhere |
| `text.find_regex(pattern)` | `Option<RegexMatch>`: the first match |
| `text.find_all_regex(pattern)` | `List<RegexMatch>`: every non-overlapping match, left to right |
| `text.replace_regex(pattern, replacement)` | `String`: every match replaced, as `replace` replaces every occurrence |
| `text.split_regex(pattern)` | `List<String>`: the text between matches |

`RegexMatch` is a compiler-known record, as `JsonError` is:

| Field | Type | Meaning |
| --- | --- | --- |
| `text` | `String` | the matched text |
| `start` | `Int` | where the match starts |
| `end` | `Int` | where it ends, exclusive |
| `groups` | `List<Option<String>>` | capture groups; `groups[0]` is the whole match, and a group that did not take part is `None` |

Decisions:

- **The pattern must be a string literal, and the compiler checks it.** An invalid pattern is a compile error naming the problem and its position, such as `unclosed group`. A literal pattern is how almost every program uses regular expressions, and checking it removes the most common regex failure: a pattern that breaks only when the code path finally runs. A pattern computed at run time is refused with a message saying so; see [Deferred](#deferred).
- **The syntax is the Rust `regex` crate's**, which is the Perl and Python syntax most developers know, without backreferences and lookaround. In exchange, matching time is linear in the text length, so no pattern and no input can hang a server (no catastrophic backtracking). Named groups are written `(?<name>...)` or `(?P<name>...)`.
- **Unicode is on, and positions count characters.** `.` matches one Unicode scalar value, `\w`, `\d` and `\s` are Unicode classes, and `start` and `end` count Unicode scalar values like `length()` and `substring`, so `text.substring(m.start, m.end) == m.text` always holds. Both tiers convert from byte offsets in `corvid-text`. Case-insensitive matching (`(?i)`) uses Unicode simple case folding, which differs from `to_upper()`'s full mapping only for characters such as `ß` whose uppercase form is more than one character.
- **`groups[i]` is group `i`**, so the index in code is the number written in the replacement (`$1`). Python's `groups()` omits group 0, which makes `groups()[0]` group 1 and invites off-by-one errors.
- **The replacement is checked too, and only a literal is a template.** In a literal replacement, `$1`, `${1}`, `$name` and `${name}` insert a group and `$$` inserts `$`. The compiler rejects a reference to a group the pattern does not have, which also catches the classic mistake `$1a` (read as a group named `1a`) and suggests `${1}a`. A replacement computed at run time is inserted exactly as written, with no `$` expansion. User data therefore can never be read as a template, the same rule that keeps SQL parameters apart from SQL text.
- **`split_regex` refuses a pattern that can match the empty string** at compile time, as `split("")` is refused at run time: splitting on an empty match has no single sensible answer. `find_all_regex` and `replace_regex` allow empty matches, with the `regex` crate's rule that an empty match never occurs immediately after the previous match.
- **Compiled patterns are cached** per process by pattern text. Patterns are literals, so the cache is bounded by the program's source.
- **Named-group access is not a `RegexMatch` method yet.** See [Deferred](#deferred).

### Replay, effects and purity

Every method above is pure and deterministic: the same receiver and arguments give the same result on both tiers, so replay never records or substitutes them, and they are allowed in `@deterministic` code. Only `uuid_v4` and `uuid_v7` are effects, and they are tools.

### Native code

Both tiers call `corvid-text`. Native code calls it through `extern "C"` functions in `corvid-runtime`, as `parse_int` and `replace` already do. `RegexMatch` becomes an ordinary native record before ownership analysis, as `JsonError` does, so field access, reference counting and equality need no new machinery. Parity tests run every method on both tiers and compare results and allocation counts.

### Not an invention

These are commodity features. They follow the design principle's rule for commodity features: the best known semantics with the papercuts removed (compile-time pattern checking, character positions, no template expansion of run-time text, linear-time matching, Result-returning decoders). They get reference documentation and tests but no invention-catalog entry or tour topic, because no other language would find them unfamiliar.

## Slices

1. **Encoding and UUID parsing on the interpreter.** The `corvid-text` crate, the eight encoding methods and `parse_uuid`.
2. **Regular expressions on the interpreter.** Compile-time pattern and replacement checks, the five methods and `RegexMatch`.
3. **UUID generation.** `std/uuid.cor` with `uuid_v4` and `uuid_v7`, traced and replay-substituted.
4. **Native encoding and UUID parsing**, with parity tests.
5. **Native regular expressions**, with parity tests.

## Deferred

- **Patterns built at run time.** They need a value that proves a pattern was validated, such as `Regex.parse(text) -> Result<Regex, String>`, so that methods taking it cannot fail. That is a new value type for every layer, including the native runtime, and no current backend example needs one. Programs that search for user-supplied text use `contains`, which needs no escaping.
- **Named-group lookup with a compile-time name check.** `m.group("year")` could be checked against the pattern's group names, but only if `RegexMatch` carried them in its type. Until then, named groups work in replacements and by number in `groups`.
- **Raw string literals** (`r"\d+"`). Patterns currently double their backslashes (`"\\d+"`). A raw string literal is a lexical rule shared by every part of the language, including frontend code, so it is agreed with frontend development first.
- **Byte-list encoders and decoders.** These land with B04, which decides how Corvid represents bytes.
- **Native UUID generation.** Native binaries do not yet dispatch any stdlib tool (`random_int` and `time_now_utc` are interpreter-only), so `uuid_v4` and `uuid_v7` follow them when native stdlib tools arrive.
