//! The builtin-method table (slice 45c) — the single source of truth
//! for methods on built-in receiver types (`String`, `List<T>`,
//! `Int`, `Float`, `Option<T>`, `Result<T,E>`, `Grounded<T>`, …).
//!
//! Three consumers share this table so a method exists exactly once:
//!
//! 1. The **checker** (`check_method_call`) looks up the signature,
//!    checks arity + argument types, and returns the result type.
//! 2. The **IR lowerer** re-derives the same lookup from the
//!    receiver's checked type and lowers to
//!    `IrExprKind::BuiltinMethod { kind, .. }`.
//! 3. The **interpreter** executes on the `BuiltinMethodKind` enum —
//!    one match arm per method.
//!
//! Adding a method = one arm in `builtin_method` + one interpreter
//! arm + tests. The table is a FUNCTION rather than a static map so
//! generic receivers work naturally (`List<T>.first() -> Option<T>`
//! computes its return type from the receiver's element type).
//!
//! Method batches ship in dedicated slices: strings (45d),
//! conversions (45e), lists (45f), Option/Result helpers (45l),
//! `Grounded<T>` named unwraps (44f addendum). This slice ships the
//! machinery plus one pilot: `String.length()`.
//!
//! Grounded receivers are NOT auto-unwrapped by this table in 45c —
//! `Grounded<String>.length()` stays an error until the contagion
//! rule for method calls is decided alongside the Grounded unwrap
//! batch.

use crate::types::Type;
use corvid_ast::Effect;
use serde::{Deserialize, Serialize};

/// Stable identity of a builtin method — the contract between the
/// checker, the lowerer, and the interpreter.
///
/// String-method semantics (slice 45d), decided once here:
/// - Indices and lengths count Unicode scalar values (Python's
///   `len(str)`), never UTF-8 bytes.
/// - `to_upper`/`to_lower` are full Unicode case mappings.
/// - `split` with an empty separator TRAPS at runtime (Python-like;
///   Rust's empty-pattern split yields surprising empty pieces).
/// - `replace` replaces ALL occurrences (Python/Rust behavior).
/// - `substring(start, end)` clamps out-of-range indices to the
///   string bounds and returns "" when start >= end (Python slice
///   behavior); negative indices are not supported in 45d.
/// - There is no `chars()` — strings are already `for c in s`
///   iterable at the statement level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltinMethodKind {
    /// `String.length() -> Int` — the number of Unicode scalar
    /// values (code points), matching Python's `len(str)`, not the
    /// UTF-8 byte count.
    StringLength,
    /// `String.to_upper() -> String` — Unicode uppercase mapping.
    StringToUpper,
    /// `String.to_lower() -> String` — Unicode lowercase mapping.
    StringToLower,
    /// `String.trim() -> String` — strip Unicode whitespace from
    /// both ends.
    StringTrim,
    /// `String.contains(needle: String) -> Bool`.
    StringContains,
    /// `String.starts_with(prefix: String) -> Bool`.
    StringStartsWith,
    /// `String.ends_with(suffix: String) -> Bool`.
    StringEndsWith,
    /// `String.split(sep: String) -> List<String>` — empty separator
    /// traps at runtime.
    StringSplit,
    /// `String.replace(from: String, to: String) -> String` — every
    /// occurrence.
    StringReplace,
    /// `String.substring(start: Int, end: Int) -> String` — Unicode
    /// scalar indices, clamped to bounds; "" when start >= end.
    StringSubstring,

    // ----- conversions (slice 45e) --------------------------------
    //
    // Number→String rendering is Python-style: a Float ALWAYS shows
    // a decimal point or exponent (`42.0` → "42.0", never "42") so
    // string output round-trips visibly typed — these strings feed
    // LLM prompts and JSON. `to_int_truncated` truncates toward
    // zero and TRAPS on NaN / out-of-i64-range (the always-checked
    // arithmetic rule; no silent wrapping). The parse methods trim
    // ASCII whitespace first (Python's `int(" 42 ")` convenience)
    // and return `Result<_, String>` with the offending input named
    // in the Err.
    /// `Int.to_string() -> String`.
    IntToString,
    /// `Float.to_string() -> String` — always shows `.` or exponent.
    FloatToString,
    /// `Bool.to_string() -> String` — "true" / "false".
    BoolToString,
    /// `Int.to_float() -> Float` — exact for |n| <= 2^53, nearest
    /// otherwise.
    IntToFloat,
    /// `Float.to_int_truncated() -> Int` — toward zero; traps on
    /// NaN or out-of-range.
    FloatToIntTruncated,
    /// `String.parse_int() -> Result<Int, String>`.
    StringParseInt,
    /// `String.parse_float() -> Result<Float, String>`.
    StringParseFloat,

    // ----- lists (slice 45f) --------------------------------------
    //
    // `append`, `reverse`, and `sort` mutate IN PLACE and return
    // `Nothing` — coherent with the 45b reference-semantics decision
    // and Python's list API. `sort` is only offered where the
    // element type has a natural order (Int / Float / String; the
    // table returns no signature otherwise, so unsortable element
    // types get the standard "no builtin method" diagnostic).
    // Floats sort by IEEE total order (NaN last). `slice` clamps
    // like `substring` and returns a NEW list. `join` is offered on
    // `List<String>` only. `first`/`last` return `Option<T>` — the
    // first generic-return methods, computed from the receiver's
    // element type (the reason the table is a function).
    /// `List<T>.length() -> Int`.
    ListLength,
    /// `List<T>.append(item: T) -> Nothing` — in place.
    ListAppend,
    /// `List<T>.contains(item: T) -> Bool` — structural equality.
    ListContains,
    /// `List<T>.first() -> Option<T>`.
    ListFirst,
    /// `List<T>.last() -> Option<T>`.
    ListLast,
    /// `List<T>.slice(start: Int, end: Int) -> List<T>` — clamped,
    /// new list.
    ListSlice,
    /// `List<T>.reverse() -> Nothing` — in place.
    ListReverse,
    /// `List<T>.sort() -> Nothing` — in place; Int/Float/String
    /// elements only (table-gated).
    ListSort,
    /// `List<String>.join(sep: String) -> String`.
    ListJoin,

    // ----- maps (slice 45g) --------------------------------------
    //
    // `m[k] = v` is the insert-or-update path (place assignment);
    // `get` is the method spelling of the `m[k]` Option read.
    // `remove` returns the removed value as Option<V>. `keys` /
    // `values` return snapshot lists in insertion order.
    /// `Map<K,V>.length() -> Int`.
    MapLength,
    /// `Map<K,V>.get(key: K) -> Option<V>`.
    MapGet,
    /// `Map<K,V>.contains_key(key: K) -> Bool`.
    MapContainsKey,
    /// `Map<K,V>.keys() -> List<K>` — insertion order.
    MapKeys,
    /// `Map<K,V>.values() -> List<V>` — insertion order.
    MapValues,
    /// `Map<K,V>.remove(key: K) -> Option<V>`.
    MapRemove,

    // ----- higher-order list methods (slice 45j) ------------------
    //
    // The lambda-taking batch. `map`'s result element type and
    // `fold`'s accumulator type cannot come from the receiver alone,
    // so the CHECKER refines these signatures while checking the
    // arguments in order: `fold`'s `init` argument fixes `U` in both
    // the lambda parameter and the result; `map`'s checked lambda
    // return type fixes the result element type. The interpreter
    // applies the closure once per element, left to right; `any` /
    // `all` short-circuit.
    /// `List<T>.map(f: (T) -> U) -> List<U>`.
    ListMap,
    /// `List<T>.filter(pred: (T) -> Bool) -> List<T>`.
    ListFilter,
    /// `List<T>.fold(init: U, f: (U, T) -> U) -> U`.
    ListFold,
    /// `List<T>.any(pred: (T) -> Bool) -> Bool` — short-circuits.
    ListAny,
    /// `List<T>.all(pred: (T) -> Bool) -> Bool` — short-circuits.
    ListAll,

    // ----- math (slice 45m) ----------------------------------------
    //
    // Pure numeric methods under the always-checked rule: Int
    // methods TRAP instead of wrapping (`abs` on i64::MIN, `pow`
    // overflow or negative exponent); Float->Int conversions
    // (`floor`/`ceil`/`round`) trap on NaN or out-of-i64-range,
    // exactly like `to_int_truncated`. `round` is half-AWAY-FROM-
    // ZERO (2.5 -> 3, -2.5 -> -3) — deliberately not Python's
    // half-to-even, which surprises far more readers than it
    // helps. Float `min`/`max` follow IEEE/Rust: a NaN operand
    // loses. `sqrt` of a negative traps (a silent NaN would poison
    // downstream arithmetic invisibly). All are pure — replay
    // stays deterministic; the effectful numeric source (`random`)
    // lives in std/random.cor as a TOOL.
    /// `Int.abs() -> Int` — traps on i64::MIN.
    IntAbs,
    /// `Int.min(other: Int) -> Int`.
    IntMin,
    /// `Int.max(other: Int) -> Int`.
    IntMax,
    /// `Int.pow(exp: Int) -> Int` — checked; traps on overflow or
    /// a negative exponent (use `to_float().pow(...)` for roots).
    IntPow,
    /// `Float.abs() -> Float`.
    FloatAbs,
    /// `Float.min(other: Float) -> Float` — NaN operand loses.
    FloatMin,
    /// `Float.max(other: Float) -> Float` — NaN operand loses.
    FloatMax,
    /// `Float.pow(exp: Float) -> Float`.
    FloatPow,
    /// `Float.sqrt() -> Float` — traps on negative input.
    FloatSqrt,
    /// `Float.floor() -> Int` — traps on NaN / out-of-range.
    FloatFloor,
    /// `Float.ceil() -> Int` — traps on NaN / out-of-range.
    FloatCeil,
    /// `Float.round() -> Int` — half away from zero; traps on
    /// NaN / out-of-range.
    FloatRound,

    // ----- Option / Result ergonomics (slice 45l) -----------------
    //
    // The point-of-use shorthands: defaulting an absent Option,
    // asking which side a Result landed on, and converting between
    // the two envelopes without a full `match`. `ok_or`'s error
    // type comes from its argument and `map_err`'s from its
    // lambda's checked return type — the same sequential signature
    // refinement `map`/`fold` use. All are pure; `map_err` applies
    // its closure at most once (only on the Err side).
    /// `Option<T>.unwrap_or(default: T) -> T`.
    OptionUnwrapOr,
    /// `Option<T>.is_some() -> Bool`.
    OptionIsSome,
    /// `Option<T>.is_none() -> Bool`.
    OptionIsNone,
    /// `Option<T>.ok_or(err: E) -> Result<T, E>` — Some(v) becomes
    /// Ok(v), None becomes Err(err).
    OptionOkOr,
    /// `Result<T, E>.unwrap_or(default: T) -> T`.
    ResultUnwrapOr,
    /// `Result<T, E>.is_ok() -> Bool`.
    ResultIsOk,
    /// `Result<T, E>.is_err() -> Bool`.
    ResultIsErr,
    /// `Result<T, E>.map_err(f: (E) -> F) -> Result<T, F>` — Ok
    /// passes through untouched; the closure runs only on Err.
    ResultMapErr,

    /// `range(start: Int, end: Int) -> List<Int>` — the free builtin
    /// function (half-open, step 1, empty when start >= end). Lowered
    /// through the BuiltinMethod IR with `start` as the receiver so
    /// no new IR variant is needed.
    RangeIntList,
    /// `form_for(target: String, initial: Record, submit_label: String) -> String`
    RouteForm,
    BrowserFields,

    // ----- HTTP-boundary uploads (slice 52c-2) --------------------
    //
    // Read accessors on an `Upload<Format>` request body. The value
    // is materialised by `corvid serve` from the multipart request
    // (after accepted-MIME + max-size enforcement), so the handler
    // reads its parts through these methods.
    /// `Upload<Format>.text() -> String` — the upload bytes decoded
    /// as UTF-8 (lossy for non-UTF-8 content).
    UploadText,
    /// `Upload<Format>.bytes() -> List<Int>` — the raw upload bytes,
    /// each 0–255.
    UploadBytes,
    /// `Upload<Format>.filename() -> String` — the client-supplied
    /// file name (may be empty).
    UploadFilename,
    /// `Upload<Format>.content_type() -> String` — the part's MIME
    /// type as received.
    UploadContentType,
    /// `Upload<Format>.size() -> Int` — the byte length of the upload.
    UploadSize,

    // ----- Response<Body> accessors (B05) -------------------------
    /// `Response<Body>.status() -> Int` — the HTTP status.
    ResponseStatus,
    /// `Response<Body>.body() -> Body` — the decoded body.
    ResponseBody,
    /// `Response<Body>.header(name) -> Option<String>` — the first
    /// response header with that name, compared without case.
    ResponseHeader,

    // ----- typed JSON (B01) ---------------------------------------
    //
    // One public JSON mapping (`corvid_vm::public_json`) serves HTTP
    // bodies, the published OpenAPI schemas and these methods, so a
    // value a route accepts is a value `from_json` accepts. Decoding
    // is called on a TYPE (`Order.from_json(text)`); the lowered
    // receiver is the text and the target type is the call's checked
    // `Result<T, JsonError>` type. All three are pure, so replay stays
    // deterministic.
    /// `T.from_json(text: String) -> Result<T, JsonError>` — keys `T`
    /// does not declare are skipped, as external APIs add fields.
    JsonDecode,
    /// `T.from_json_strict(text: String) -> Result<T, JsonError>` —
    /// the first undeclared key is an error.
    JsonDecodeStrict,
    /// `T.to_json() -> String` — compact JSON text. Traps on a
    /// non-finite Float, which has no JSON form. A receiver that
    /// contains `Tainted` data returns `Tainted<String>`.
    JsonEncode,

    // ----- text encoding and UUIDs (B03) --------------------------
    //
    // Pure String methods over the text's UTF-8 bytes, implemented
    // once in `corvid-text` for both tiers. Encoders never fail;
    // decoders return `Result<String, String>` naming the first
    // problem and its character position, and also fail when the
    // decoded bytes are not UTF-8. Decoders accept base64 with or
    // without padding and nothing else that is non-canonical.
    /// `String.encode_base64() -> String` — RFC 4648 standard
    /// alphabet, padded.
    StringEncodeBase64,
    /// `String.decode_base64() -> Result<String, String>`.
    StringDecodeBase64,
    /// `String.encode_base64_url() -> String` — RFC 4648 URL-safe
    /// alphabet, unpadded.
    StringEncodeBase64Url,
    /// `String.decode_base64_url() -> Result<String, String>`.
    StringDecodeBase64Url,
    /// `String.encode_hex() -> String` — lowercase.
    StringEncodeHex,
    /// `String.decode_hex() -> Result<String, String>` — either case.
    StringDecodeHex,
    /// `String.encode_url_component() -> String` — percent-encodes
    /// every byte outside the RFC 3986 unreserved set.
    StringEncodeUrlComponent,
    /// `String.decode_url_component() -> Result<String, String>` —
    /// `+` stays `+`.
    StringDecodeUrlComponent,
    /// `String.parse_uuid() -> Result<String, String>` — the canonical
    /// lowercase hyphenated form of any common UUID spelling.
    StringParseUuid,

    // ----- regular expressions (B03) ------------------------------
    //
    // The pattern (always the first argument) must be a string literal;
    // the checker validates it with `corvid_text::regex::check_pattern`,
    // the parser the runtime compiles it with, so no invalid pattern
    // reaches either tier. Positions count characters. All are pure.
    /// `String.contains_regex(pattern: String) -> Bool`.
    StringContainsRegex,
    /// `String.find_regex(pattern: String) -> Option<RegexMatch>`.
    StringFindRegex,
    /// `String.find_all_regex(pattern: String) -> List<RegexMatch>`.
    StringFindAllRegex,
    /// `String.replace_regex(pattern: String, replacement: String) ->
    /// String` with a literal replacement: a template whose `$1` and
    /// `${name}` references the checker validated.
    StringReplaceRegex,
    /// The same call with a replacement computed at run time, which is
    /// inserted exactly as written. The IR lowerer selects this kind
    /// through [`refine_builtin_method_kind`]; the table never returns
    /// it.
    StringReplaceRegexVerbatim,
    /// `String.split_regex(pattern: String) -> List<String>` — the
    /// checker refuses patterns that can match the empty string.
    StringSplitRegex,

    // ----- exact decimals (B06) ------------------------------------
    //
    // Every Decimal method calls `corvid_decimal`, the code the
    // checker parses literals with, so the tiers cannot disagree on a
    // digit. Results are exact or the call traps; only the rounding
    // methods drop digits, and each names its mode.
    /// `Decimal.round_<mode>(places: Int) -> Decimal` — exactly
    /// `places` fractional digits, rounded with `mode`.
    DecimalRound(corvid_decimal::RoundingMode),
    /// `(a / b).round_<mode>(places)` on `Decimal` values. Not in the
    /// method table: IR lowering fuses a rounded `/` into this kind
    /// with `a` as the receiver and `b`, `places` as the arguments, so
    /// the quotient is rounded once and never exists unrounded.
    DecimalDivideRound(corvid_decimal::RoundingMode),
    /// `Decimal.abs() -> Decimal`.
    DecimalAbs,
    /// `Decimal.scale() -> Int` — the number of fractional digits.
    DecimalScale,
    /// `Decimal.to_string() -> String` — plain notation with every
    /// fractional digit the value has (`2.50`), never an exponent.
    DecimalToString,
    /// `Decimal.to_float() -> Float` — the nearest Float; the only
    /// lossy direction, named so the loss is visible.
    DecimalToFloat,
    /// `Decimal.to_int_truncated() -> Int` — toward zero; traps out
    /// of the Int range.
    DecimalToIntTruncated,
    /// `Int.to_decimal() -> Decimal` — exact.
    IntToDecimal,
    /// `Float.to_decimal() -> Decimal` — the shortest decimal that
    /// reads back as the same Float (the digits `to_string` shows);
    /// traps on NaN and infinity.
    FloatToDecimal,
    /// `String.parse_decimal() -> Result<Decimal, String>` — trims
    /// whitespace like `parse_int`; the Err names the problem and its
    /// position.
    StringParseDecimal,

    // ----- exact time (B06) ----------------------------------------
    //
    // Every rule is `corvid_time`'s, the code the native runtime will
    // call. Readings truncate toward zero and trap outside the Int
    // range; nothing reads the clock here (that is the `time_now` tool).
    /// `Duration.whole_<unit>() -> Int` — whole units, truncated toward zero.
    DurationWhole(corvid_time::TimeUnit),
    /// `Duration.abs() -> Duration`.
    DurationAbs,
    /// `Duration.to_string() -> String` — ISO 8601 (`PT1H30M`).
    DurationToString,
    /// `Instant.epoch_milliseconds() -> Int` — rounded toward negative
    /// infinity, as JavaScript's `Date` reads it.
    InstantEpochMilliseconds,
    /// `Instant.to_string() -> String` — RFC 3339 in UTC.
    InstantToString,
    /// `Instant.from_epoch_milliseconds(n: Int) -> Instant`, a method
    /// of the type itself; traps outside years 1 to 9999.
    InstantFromEpochMilliseconds,
    /// `String.parse_duration() -> Result<Duration, String>` — ISO 8601.
    StringParseDuration,
    /// `String.parse_instant() -> Result<Instant, String>` — RFC 3339.
    StringParseInstant,
    /// `Instant.in_zone(zone: TimeZone) -> ZonedTime` — always succeeds.
    InstantInZone,
    /// `ZonedTime.<field>() -> Int` — `year`, `month`, `day`, `hour`,
    /// `minute`, `second`, `nanosecond`, `weekday` (1 = Monday) and
    /// `offset_seconds`, from `corvid_time::ZonedField`.
    ZonedTimeField(corvid_time::ZonedField),
    /// `ZonedTime.time_zone() -> TimeZone` (`zone` is a keyword).
    ZonedTimeZone,
    /// `ZonedTime.instant() -> Instant`.
    ZonedTimeInstant,
    /// `ZonedTime.add_days(n: Int) -> ZonedTime` — the same local time
    /// on another calendar day.
    ZonedTimeAddDays,
    /// `ZonedTime.add_months(n: Int) -> ZonedTime` — clamped to the
    /// month's last day.
    ZonedTimeAddMonths,
    /// `ZonedTime.add_years(n: Int) -> ZonedTime`.
    ZonedTimeAddYears,
    /// `ZonedTime.with_zone(zone: TimeZone) -> ZonedTime` — the same
    /// instant in another zone.
    ZonedTimeWithZone,
    /// `ZonedTime.to_string() -> String` — RFC 9557.
    ZonedTimeToString,
    /// `TimeZone.to_string() -> String` — the IANA name.
    TimeZoneToString,
    /// `String.parse_time_zone() -> Result<TimeZone, String>`.
    StringParseTimeZone,
    /// `String.parse_zoned_time() -> Result<ZonedTime, String>` — RFC 9557.
    StringParseZonedTime,
    /// `String.parse_local_time(zone: TimeZone) -> Result<ZonedTime,
    /// String>` — a wall-clock time; a skipped or repeated local time
    /// is an `Err` naming both offsets.
    StringParseLocalTime,

    // ----- currency formatting (B06) -------------------------------
    //
    // Every rule is `corvid_currency`'s, over Unicode CLDR data.
    // Formatting never rounds: an amount with more digits than the
    // currency shows traps and asks for a declared rounding first.
    /// `Decimal.format_currency(currency: Currency, locale: Locale) ->
    /// String` — `$1,234.50` in en-US, `1.234,50 €` in de-DE.
    DecimalFormatCurrency,
    /// `Currency.minor_digits() -> Int` — 2 for USD, 0 for JPY.
    CurrencyMinorDigits,
    /// `Currency.to_string() -> String` — the ISO 4217 code.
    CurrencyToString,
    /// `Locale.to_string() -> String` — the canonical BCP 47 tag.
    LocaleToString,
    /// `String.parse_currency() -> Result<Currency, String>`.
    StringParseCurrency,
    /// `String.parse_locale() -> Result<Locale, String>`.
    StringParseLocale,

    // ----- byte strings and hashing --------------------------------
    //
    // Every rule is `corvid_crypto`'s. Encodings are always explicit:
    // text becomes bytes only through `utf8()`, and bytes become text
    // only through a named encoding.
    /// `String.utf8() -> Bytes` — the string's UTF-8 encoding.
    StringUtf8,
    /// `Bytes.to_utf8() -> Result<String, String>` — the Err names the
    /// first byte that starts an invalid sequence.
    BytesToUtf8,
    /// `Bytes.length() -> Int` — the number of bytes.
    BytesLength,
    /// `Bytes.to_hex() -> String` — two lowercase digits per byte.
    BytesToHex,
    /// `Bytes.to_base64() -> String` — the standard alphabet, padded.
    BytesToBase64,
    /// `Bytes.to_base64_url() -> String` — the URL-safe alphabet,
    /// unpadded; also the JSON form.
    BytesToBase64Url,
    /// `Bytes.from_hex(text: String) -> Result<Bytes, String>`, a method
    /// of the type itself; either case.
    BytesFromHex,
    /// `Bytes.from_base64(text: String) -> Result<Bytes, String>`;
    /// padding optional.
    BytesFromBase64,
    /// `Bytes.from_base64_url(text: String) -> Result<Bytes, String>`;
    /// padding optional.
    BytesFromBase64Url,
    /// `Bytes.sha256() -> Bytes` — the 32-byte digest.
    BytesSha256,
    /// `Bytes.sha384() -> Bytes` — the 48-byte digest.
    BytesSha384,
    /// `Bytes.sha512() -> Bytes` — the 64-byte digest.
    BytesSha512,
    /// `Bytes.hmac_sha256(key) -> Bytes` — the 32-byte HMAC-SHA256 tag.
    /// The key is `Bytes` or a `Secret` (see [`is_hmac_key_type`]).
    BytesHmacSha256,
    /// `Bytes.hmac_sha512(key) -> Bytes` — the 64-byte HMAC-SHA512 tag;
    /// the same keys.
    BytesHmacSha512,
    /// `Bytes.verify_ed25519(signature: Bytes, public_key: Bytes) ->
    /// Bool` — whether `signature` is the receiver's Ed25519 signature
    /// under `public_key`, verified strictly; `false` for a signature
    /// that is not 64 bytes or a key that is not 32.
    BytesVerifyEd25519,
    /// `Secret.decode_base64_after(prefix: String) -> Result<Secret,
    /// String>` — the key written in base64 after a literal prefix, as
    /// a new secret.
    SecretDecodeBase64After,
    /// `Secret.verify_password(hash: String) -> Bool` — whether the
    /// secret is the password the Argon2id PHC string was made from;
    /// `false` for a malformed hash. Pure, and constant-time.
    SecretVerifyPassword,
}

impl BuiltinMethodKind {
    /// The B03 text methods that take no arguments and return
    /// `Result<String, String>`: every decoder and `parse_uuid`.
    pub fn is_text_decoder(self) -> bool {
        use BuiltinMethodKind::*;
        matches!(
            self,
            StringDecodeBase64
                | StringDecodeBase64Url
                | StringDecodeHex
                | StringDecodeUrlComponent
                | StringParseUuid
        )
    }

    /// The B03 text encoders: no arguments, `String` result.
    pub fn is_text_encoder(self) -> bool {
        use BuiltinMethodKind::*;
        matches!(
            self,
            StringEncodeBase64 | StringEncodeBase64Url | StringEncodeHex | StringEncodeUrlComponent
        )
    }

    /// The B06 methods that take or produce a `Decimal`.
    pub fn is_decimal(self) -> bool {
        use BuiltinMethodKind::*;
        matches!(
            self,
            DecimalRound(_)
                | DecimalDivideRound(_)
                | DecimalAbs
                | DecimalScale
                | DecimalToString
                | DecimalToFloat
                | DecimalToIntTruncated
                | IntToDecimal
                | FloatToDecimal
                | StringParseDecimal
        )
    }

    /// The B06 methods that take or produce a `Duration` or an `Instant`.
    pub fn is_time(self) -> bool {
        use BuiltinMethodKind::*;
        matches!(
            self,
            DurationWhole(_)
                | DurationAbs
                | DurationToString
                | InstantEpochMilliseconds
                | InstantToString
                | InstantFromEpochMilliseconds
                | StringParseDuration
                | StringParseInstant
                | InstantInZone
                | ZonedTimeField(_)
                | ZonedTimeZone
                | ZonedTimeInstant
                | ZonedTimeAddDays
                | ZonedTimeAddMonths
                | ZonedTimeAddYears
                | ZonedTimeWithZone
                | ZonedTimeToString
                | TimeZoneToString
                | StringParseTimeZone
                | StringParseZonedTime
                | StringParseLocalTime
        )
    }

    /// The B06 currency-formatting methods.
    pub fn is_currency(self) -> bool {
        use BuiltinMethodKind::*;
        matches!(
            self,
            DecimalFormatCurrency
                | CurrencyMinorDigits
                | CurrencyToString
                | LocaleToString
                | StringParseCurrency
                | StringParseLocale
        )
    }

    /// The byte-string, hashing, HMAC, signature, secret-key and password
    /// methods.
    pub fn is_crypto(self) -> bool {
        use BuiltinMethodKind::*;
        matches!(
            self,
            StringUtf8
                | BytesToUtf8
                | BytesLength
                | BytesToHex
                | BytesToBase64
                | BytesToBase64Url
                | BytesFromHex
                | BytesFromBase64
                | BytesFromBase64Url
                | BytesSha256
                | BytesSha384
                | BytesSha512
                | BytesHmacSha256
                | BytesHmacSha512
                | BytesVerifyEd25519
                | SecretDecodeBase64After
                | SecretVerifyPassword
        )
    }

    /// The HMAC methods, whose key is `Bytes` or a `Secret`.
    pub fn is_hmac(self) -> bool {
        matches!(self, BuiltinMethodKind::BytesHmacSha256 | BuiltinMethodKind::BytesHmacSha512)
    }

    /// The B03 regular-expression methods, whose first argument is a
    /// pattern literal.
    pub fn is_regex(self) -> bool {
        use BuiltinMethodKind::*;
        matches!(
            self,
            StringContainsRegex
                | StringFindRegex
                | StringFindAllRegex
                | StringReplaceRegex
                | StringReplaceRegexVerbatim
                | StringSplitRegex
        )
    }
}

/// Whether a value of type `ty` can key an HMAC: `Bytes`, or a
/// `Secret`, which keys it with its bytes without revealing them. The
/// method table can give a parameter only one type, so the HMAC
/// entries name `Bytes` and the checker accepts either through this
/// function.
pub fn is_hmac_key_type(ty: &Type) -> bool {
    ty.is_assignable_to(&Type::Bytes) || ty.is_assignable_to(&Type::Secret)
}

/// The rounding mode a `Decimal` rounding method names:
/// `round_half_even` names `HalfEven`. The checker, the method table
/// and IR lowering all read method names through this one function.
pub fn decimal_rounding_mode(method: &str) -> Option<corvid_decimal::RoundingMode> {
    method.strip_prefix("round_").and_then(corvid_decimal::RoundingMode::from_suffix)
}

/// The unit a `Duration` reading names: `whole_seconds` names `Seconds`.
pub fn duration_reading_unit(method: &str) -> Option<corvid_time::TimeUnit> {
    method.strip_prefix("whole_").and_then(corvid_time::TimeUnit::from_name)
}

/// A method called on a compiler-known type name rather than a value:
/// `Instant.from_epoch_milliseconds(n)`, `Bytes.from_hex(text)`. The
/// checker and IR lowering both look the call up here.
pub fn builtin_type_method(receiver: &Type, name: &str) -> Option<BuiltinMethodSig> {
    use BuiltinMethodKind::*;
    let sig = |kind, params, ret| Some(BuiltinMethodSig { kind, params, ret });
    let bytes_result = || Type::Result(Box::new(Type::Bytes), Box::new(Type::String));
    match (receiver, name) {
        (Type::Instant, "from_epoch_milliseconds") => {
            sig(InstantFromEpochMilliseconds, vec![Type::Int], Type::Instant)
        }
        (Type::Bytes, "from_hex") => sig(BytesFromHex, vec![Type::String], bytes_result()),
        (Type::Bytes, "from_base64") => sig(BytesFromBase64, vec![Type::String], bytes_result()),
        (Type::Bytes, "from_base64_url") => sig(BytesFromBase64Url, vec![Type::String], bytes_result()),
        _ => None,
    }
}

/// The compiler-known type a bare type name denotes when it is a
/// method receiver (`Instant` in `Instant.from_epoch_milliseconds`,
/// `Bytes` in `Bytes.from_hex`).
pub fn builtin_type_receiver(
    target: &corvid_ast::Expr,
    bindings: &std::collections::HashMap<corvid_ast::Span, corvid_resolve::Binding>,
) -> Option<Type> {
    let corvid_ast::Expr::Ident { name, .. } = target else { return None };
    match bindings.get(&name.span)? {
        corvid_resolve::Binding::BuiltIn(corvid_resolve::BuiltIn::Instant) => Some(Type::Instant),
        corvid_resolve::Binding::BuiltIn(corvid_resolve::BuiltIn::Bytes) => Some(Type::Bytes),
        _ => None,
    }
}

/// The source digits of a number literal, which `corvid_decimal` reads
/// when the checker types the literal `Decimal`. The checker and the IR
/// lowerer both read literal digits here.
pub fn number_literal_text(value: &corvid_ast::Literal) -> Option<String> {
    match value {
        corvid_ast::Literal::Int(n) => Some(n.to_string()),
        corvid_ast::Literal::Float { text, .. } => Some(text.clone()),
        _ => None,
    }
}

/// The value of a string-literal argument, which is the only form a
/// regex pattern or a replacement template may take. The checker and
/// the IR lowerer both decide literal-ness here.
pub fn string_literal_argument(arg: &corvid_ast::Expr) -> Option<&str> {
    match arg {
        corvid_ast::Expr::Literal { value: corvid_ast::Literal::String(text), .. } => Some(text),
        _ => None,
    }
}

/// The kind a call executes as, given its arguments, when the table's
/// kind depends on them: `replace_regex` with a replacement computed at
/// run time inserts it verbatim instead of expanding `$` references.
pub fn refine_builtin_method_kind(kind: BuiltinMethodKind, args: &[corvid_ast::Expr]) -> BuiltinMethodKind {
    match kind {
        BuiltinMethodKind::StringReplaceRegex
            if args.get(1).is_some_and(|replacement| string_literal_argument(replacement).is_none()) =>
        {
            BuiltinMethodKind::StringReplaceRegexVerbatim
        }
        other => other,
    }
}

/// The fields of the compiler-known `RegexMatch` record, in display
/// order. The checker types `m.start` from this list and both tiers
/// build `RegexMatch` values from it.
pub fn regex_match_fields() -> [(&'static str, Type); 4] {
    [
        ("text", Type::String),
        ("start", Type::Int),
        ("end", Type::Int),
        ("groups", Type::List(Box::new(Type::Option(Box::new(Type::String))))),
    ]
}

/// The type id runtime `RegexMatch` records carry, reserved like
/// [`JSON_ERROR_TYPE_ID`].
pub const REGEX_MATCH_TYPE_ID: corvid_resolve::DefId = corvid_resolve::DefId(u32::MAX - 1);

/// The type id runtime `Response` records carry, reserved like
/// [`JSON_ERROR_TYPE_ID`].
pub const RESPONSE_TYPE_ID: corvid_resolve::DefId = corvid_resolve::DefId(u32::MAX - 2);

/// The fields of a runtime `Response` record, in order: the status, the
/// response headers as `[name, value]` pairs with lower-cased names, and
/// the decoded body. Programs read them through the accessors, never as
/// fields; the interpreter builds and reads records from this one list.
pub const RESPONSE_FIELDS: [&str; 3] = ["status", "headers", "body"];

/// The fields of the compiler-known `JsonError` record, in display
/// order. The checker types `error.path` from this list and the
/// runtime builds `JsonError` values from it.
pub fn json_error_fields() -> [(&'static str, Type); 4] {
    [
        ("path", Type::String),
        ("expected", Type::String),
        ("found", Type::String),
        ("message", Type::String),
    ]
}

/// The type id runtime `JsonError` records carry. `JsonError` is
/// compiler-known, so it has no declaration id; this one is reserved.
pub const JSON_ERROR_TYPE_ID: corvid_resolve::DefId = corvid_resolve::DefId(u32::MAX);

/// The type id a table write's conflict carries at run time (B09).
/// `Conflict` is compiler-known, so it has no declaration id. Its
/// variants are [`crate::ConflictCase::ALL`], each case's index its
/// variant index.
pub const CONFLICT_TYPE_ID: corvid_resolve::DefId = corvid_resolve::DefId(u32::MAX - 3);

/// Every type id reserved for a compiler-known record, by name. A new
/// compiler-known record takes the next free id below these and joins
/// this list; the test beside it fails if two share an id, which would
/// make the interpreter mistake one record for the other.
pub const RESERVED_TYPE_IDS: [(&str, corvid_resolve::DefId); 4] = [
    ("JsonError", JSON_ERROR_TYPE_ID),
    ("RegexMatch", REGEX_MATCH_TYPE_ID),
    ("Response", RESPONSE_TYPE_ID),
    ("Conflict", CONFLICT_TYPE_ID),
];

/// The ids native code gives the record it holds a `Response<Body>`
/// as, one per distinct body type (native dispatch N4). The band sits
/// below the reserved ids above and far above any id the resolver
/// assigns, which counts up from zero.
pub const RESPONSE_RECORD_IDS: std::ops::Range<u32> = (u32::MAX - 4 - 65_536)..(u32::MAX - 4);

#[cfg(test)]
mod reserved_type_id_tests {
    use super::{RESERVED_TYPE_IDS, RESPONSE_RECORD_IDS};

    #[test]
    fn the_response_record_band_holds_no_reserved_id() {
        for (name, id) in RESERVED_TYPE_IDS {
            assert!(!RESPONSE_RECORD_IDS.contains(&id.0), "`{name}`'s id {id:?} is inside the Response record band");
        }
        assert!(RESPONSE_RECORD_IDS.start > 1 << 24, "the band must sit far above resolver-assigned ids");
    }

    #[test]
    fn every_reserved_type_id_is_distinct() {
        for (i, (name, id)) in RESERVED_TYPE_IDS.iter().enumerate() {
            for (other, other_id) in &RESERVED_TYPE_IDS[i + 1..] {
                assert_ne!(id, other_id, "`{name}` and `{other}` share the reserved type id {id:?}");
            }
        }
    }
}

/// Look up `T.name(...)` where the receiver is the type `target`
/// itself rather than a value. Only the JSON decoders are type
/// methods; whether `target` has a JSON form is checked by the caller,
/// which can see record fields.
pub fn type_method(target: &Type, name: &str) -> Option<BuiltinMethodSig> {
    let kind = match name {
        "from_json" => BuiltinMethodKind::JsonDecode,
        "from_json_strict" => BuiltinMethodKind::JsonDecodeStrict,
        _ => return None,
    };
    Some(BuiltinMethodSig {
        kind,
        params: vec![Type::String],
        ret: Type::Result(Box::new(target.clone()), Box::new(Type::JsonError)),
    })
}

/// A builtin method's checked signature for a CONCRETE receiver
/// type. Parameter types are the declared types of the call's
/// arguments (the receiver is not included).
#[derive(Debug, Clone)]
pub struct BuiltinMethodSig {
    pub kind: BuiltinMethodKind,
    pub params: Vec<Type>,
    pub ret: Type,
}

/// `Result<String, String>`, the result of every text decoder.
fn text_result() -> Type {
    Type::Result(Box::new(Type::String), Box::new(Type::String))
}

/// Look up `receiver.name(...)` in the builtin-method table.
/// Returns `None` when no builtin method matches — the caller falls
/// through to extend-method dispatch or its existing diagnostics.
pub fn builtin_method(receiver: &Type, name: &str) -> Option<BuiltinMethodSig> {
    use BuiltinMethodKind::*;
    let sig = |kind, params, ret| Some(BuiltinMethodSig { kind, params, ret });
    let func = |params: Vec<Type>, ret: Type| Type::Function {
        params,
        ret: Box::new(ret),
        effect: Effect::Safe,
    };
    match (receiver, name) {
        (Type::String, "length") => sig(StringLength, vec![], Type::Int),
        (Type::String, "to_upper") => sig(StringToUpper, vec![], Type::String),
        (Type::String, "to_lower") => sig(StringToLower, vec![], Type::String),
        (Type::String, "trim") => sig(StringTrim, vec![], Type::String),
        (Type::String, "contains") => sig(StringContains, vec![Type::String], Type::Bool),
        (Type::String, "starts_with") => sig(StringStartsWith, vec![Type::String], Type::Bool),
        (Type::String, "ends_with") => sig(StringEndsWith, vec![Type::String], Type::Bool),
        (Type::String, "split") => sig(
            StringSplit,
            vec![Type::String],
            Type::List(Box::new(Type::String)),
        ),
        (Type::String, "replace") => sig(
            StringReplace,
            vec![Type::String, Type::String],
            Type::String,
        ),
        (Type::String, "substring") => {
            sig(StringSubstring, vec![Type::Int, Type::Int], Type::String)
        }
        (Type::Int, "to_string") => sig(IntToString, vec![], Type::String),
        (Type::Float, "to_string") => sig(FloatToString, vec![], Type::String),
        (Type::Bool, "to_string") => sig(BoolToString, vec![], Type::String),
        (Type::Int, "to_float") => sig(IntToFloat, vec![], Type::Float),
        (Type::Int, "to_decimal") => sig(IntToDecimal, vec![], Type::Decimal),
        (Type::Float, "to_decimal") => sig(FloatToDecimal, vec![], Type::Decimal),
        (Type::String, "parse_decimal") => sig(
            StringParseDecimal,
            vec![],
            Type::Result(Box::new(Type::Decimal), Box::new(Type::String)),
        ),
        (Type::String, "parse_duration") => sig(
            StringParseDuration,
            vec![],
            Type::Result(Box::new(Type::Duration), Box::new(Type::String)),
        ),
        (Type::String, "parse_instant") => sig(
            StringParseInstant,
            vec![],
            Type::Result(Box::new(Type::Instant), Box::new(Type::String)),
        ),
        (Type::Duration, "abs") => sig(DurationAbs, vec![], Type::Duration),
        (Type::Duration, "to_string") => sig(DurationToString, vec![], Type::String),
        (Type::Duration, method) if duration_reading_unit(method).is_some() => sig(
            DurationWhole(duration_reading_unit(method).expect("checked by the guard")),
            vec![],
            Type::Int,
        ),
        (Type::Instant, "epoch_milliseconds") => sig(InstantEpochMilliseconds, vec![], Type::Int),
        (Type::Instant, "to_string") => sig(InstantToString, vec![], Type::String),
        (Type::Instant, "in_zone") => sig(InstantInZone, vec![Type::TimeZone], Type::ZonedTime),
        (Type::ZonedTime, method) if corvid_time::ZonedField::from_name(method).is_some() => sig(
            ZonedTimeField(corvid_time::ZonedField::from_name(method).expect("checked by the guard")),
            vec![],
            Type::Int,
        ),
        (Type::ZonedTime, "time_zone") => sig(ZonedTimeZone, vec![], Type::TimeZone),
        (Type::ZonedTime, "instant") => sig(ZonedTimeInstant, vec![], Type::Instant),
        (Type::ZonedTime, "add_days") => sig(ZonedTimeAddDays, vec![Type::Int], Type::ZonedTime),
        (Type::ZonedTime, "add_months") => sig(ZonedTimeAddMonths, vec![Type::Int], Type::ZonedTime),
        (Type::ZonedTime, "add_years") => sig(ZonedTimeAddYears, vec![Type::Int], Type::ZonedTime),
        (Type::ZonedTime, "with_zone") => sig(ZonedTimeWithZone, vec![Type::TimeZone], Type::ZonedTime),
        (Type::ZonedTime, "to_string") => sig(ZonedTimeToString, vec![], Type::String),
        (Type::TimeZone, "to_string") => sig(TimeZoneToString, vec![], Type::String),
        (Type::String, "parse_time_zone") => sig(
            StringParseTimeZone,
            vec![],
            Type::Result(Box::new(Type::TimeZone), Box::new(Type::String)),
        ),
        (Type::String, "parse_zoned_time") => sig(
            StringParseZonedTime,
            vec![],
            Type::Result(Box::new(Type::ZonedTime), Box::new(Type::String)),
        ),
        (Type::String, "parse_local_time") => sig(
            StringParseLocalTime,
            vec![Type::TimeZone],
            Type::Result(Box::new(Type::ZonedTime), Box::new(Type::String)),
        ),
        (Type::Decimal, "format_currency") => sig(
            DecimalFormatCurrency,
            vec![Type::Currency, Type::Locale],
            Type::String,
        ),
        (Type::Currency, "minor_digits") => sig(CurrencyMinorDigits, vec![], Type::Int),
        (Type::Currency, "to_string") => sig(CurrencyToString, vec![], Type::String),
        (Type::Locale, "to_string") => sig(LocaleToString, vec![], Type::String),
        (Type::String, "parse_currency") => sig(
            StringParseCurrency,
            vec![],
            Type::Result(Box::new(Type::Currency), Box::new(Type::String)),
        ),
        (Type::String, "parse_locale") => sig(
            StringParseLocale,
            vec![],
            Type::Result(Box::new(Type::Locale), Box::new(Type::String)),
        ),
        (Type::String, "utf8") => sig(StringUtf8, vec![], Type::Bytes),
        (Type::Bytes, "to_utf8") => sig(BytesToUtf8, vec![], text_result()),
        (Type::Bytes, "length") => sig(BytesLength, vec![], Type::Int),
        (Type::Bytes, "to_hex") => sig(BytesToHex, vec![], Type::String),
        (Type::Bytes, "to_base64") => sig(BytesToBase64, vec![], Type::String),
        (Type::Bytes, "to_base64_url") => sig(BytesToBase64Url, vec![], Type::String),
        (Type::Bytes, "sha256") => sig(BytesSha256, vec![], Type::Bytes),
        (Type::Bytes, "sha384") => sig(BytesSha384, vec![], Type::Bytes),
        (Type::Bytes, "sha512") => sig(BytesSha512, vec![], Type::Bytes),
        (Type::Bytes, "hmac_sha256") => sig(BytesHmacSha256, vec![Type::Bytes], Type::Bytes),
        (Type::Bytes, "hmac_sha512") => sig(BytesHmacSha512, vec![Type::Bytes], Type::Bytes),
        (Type::Bytes, "verify_ed25519") => sig(BytesVerifyEd25519, vec![Type::Bytes, Type::Bytes], Type::Bool),
        (Type::Secret, "decode_base64_after") => sig(
            SecretDecodeBase64After,
            vec![Type::String],
            Type::Result(Box::new(Type::Secret), Box::new(Type::String)),
        ),
        (Type::Secret, "verify_password") => sig(SecretVerifyPassword, vec![Type::String], Type::Bool),
        (Type::Decimal, "abs") => sig(DecimalAbs, vec![], Type::Decimal),
        (Type::Decimal, "scale") => sig(DecimalScale, vec![], Type::Int),
        (Type::Decimal, "to_string") => sig(DecimalToString, vec![], Type::String),
        (Type::Decimal, "to_float") => sig(DecimalToFloat, vec![], Type::Float),
        (Type::Decimal, "to_int_truncated") => sig(DecimalToIntTruncated, vec![], Type::Int),
        (Type::Decimal, method) if decimal_rounding_mode(method).is_some() => sig(
            DecimalRound(decimal_rounding_mode(method).expect("checked by the guard")),
            vec![Type::Int],
            Type::Decimal,
        ),
        (Type::Float, "to_int_truncated") => sig(FloatToIntTruncated, vec![], Type::Int),
        (Type::String, "parse_int") => sig(
            StringParseInt,
            vec![],
            Type::Result(Box::new(Type::Int), Box::new(Type::String)),
        ),
        (Type::String, "parse_float") => sig(
            StringParseFloat,
            vec![],
            Type::Result(Box::new(Type::Float), Box::new(Type::String)),
        ),
        (Type::String, "encode_base64") => sig(StringEncodeBase64, vec![], Type::String),
        (Type::String, "encode_base64_url") => sig(StringEncodeBase64Url, vec![], Type::String),
        (Type::String, "encode_hex") => sig(StringEncodeHex, vec![], Type::String),
        (Type::String, "encode_url_component") => {
            sig(StringEncodeUrlComponent, vec![], Type::String)
        }
        (Type::String, "decode_base64") => sig(StringDecodeBase64, vec![], text_result()),
        (Type::String, "decode_base64_url") => sig(StringDecodeBase64Url, vec![], text_result()),
        (Type::String, "decode_hex") => sig(StringDecodeHex, vec![], text_result()),
        (Type::String, "decode_url_component") => {
            sig(StringDecodeUrlComponent, vec![], text_result())
        }
        (Type::String, "parse_uuid") => sig(StringParseUuid, vec![], text_result()),
        (Type::String, "contains_regex") => sig(StringContainsRegex, vec![Type::String], Type::Bool),
        (Type::String, "find_regex") => sig(
            StringFindRegex,
            vec![Type::String],
            Type::Option(Box::new(Type::RegexMatch)),
        ),
        (Type::String, "find_all_regex") => sig(
            StringFindAllRegex,
            vec![Type::String],
            Type::List(Box::new(Type::RegexMatch)),
        ),
        (Type::String, "replace_regex") => sig(
            StringReplaceRegex,
            vec![Type::String, Type::String],
            Type::String,
        ),
        (Type::String, "split_regex") => sig(
            StringSplitRegex,
            vec![Type::String],
            Type::List(Box::new(Type::String)),
        ),
        (Type::List(_), "length") => sig(ListLength, vec![], Type::Int),
        (Type::List(elem), "append") => sig(ListAppend, vec![(**elem).clone()], Type::Nothing),
        (Type::List(elem), "contains") => sig(ListContains, vec![(**elem).clone()], Type::Bool),
        (Type::List(elem), "first") => sig(ListFirst, vec![], Type::Option(elem.clone())),
        (Type::List(elem), "last") => sig(ListLast, vec![], Type::Option(elem.clone())),
        (Type::List(elem), "slice") => sig(
            ListSlice,
            vec![Type::Int, Type::Int],
            Type::List(elem.clone()),
        ),
        (Type::List(_), "reverse") => sig(ListReverse, vec![], Type::Nothing),
        (Type::List(elem), "sort")
            if matches!(**elem, Type::Int | Type::Float | Type::String) =>
        {
            sig(ListSort, vec![], Type::Nothing)
        }
        (Type::List(elem), "join") if matches!(**elem, Type::String) => {
            sig(ListJoin, vec![Type::String], Type::String)
        }
        (Type::List(elem), "map") => sig(
            ListMap,
            vec![func(vec![(**elem).clone()], Type::Unknown)],
            Type::List(Box::new(Type::Unknown)),
        ),
        (Type::List(elem), "filter") => sig(
            ListFilter,
            vec![func(vec![(**elem).clone()], Type::Bool)],
            Type::List(elem.clone()),
        ),
        (Type::List(elem), "fold") => sig(
            ListFold,
            vec![
                Type::Unknown,
                func(vec![Type::Unknown, (**elem).clone()], Type::Unknown),
            ],
            Type::Unknown,
        ),
        (Type::List(elem), "any") => sig(
            ListAny,
            vec![func(vec![(**elem).clone()], Type::Bool)],
            Type::Bool,
        ),
        (Type::List(elem), "all") => sig(
            ListAll,
            vec![func(vec![(**elem).clone()], Type::Bool)],
            Type::Bool,
        ),
        (Type::Int, "abs") => sig(IntAbs, vec![], Type::Int),
        (Type::Int, "min") => sig(IntMin, vec![Type::Int], Type::Int),
        (Type::Int, "max") => sig(IntMax, vec![Type::Int], Type::Int),
        (Type::Int, "pow") => sig(IntPow, vec![Type::Int], Type::Int),
        (Type::Float, "abs") => sig(FloatAbs, vec![], Type::Float),
        (Type::Float, "min") => sig(FloatMin, vec![Type::Float], Type::Float),
        (Type::Float, "max") => sig(FloatMax, vec![Type::Float], Type::Float),
        (Type::Float, "pow") => sig(FloatPow, vec![Type::Float], Type::Float),
        (Type::Float, "sqrt") => sig(FloatSqrt, vec![], Type::Float),
        (Type::Float, "floor") => sig(FloatFloor, vec![], Type::Int),
        (Type::Float, "ceil") => sig(FloatCeil, vec![], Type::Int),
        (Type::Float, "round") => sig(FloatRound, vec![], Type::Int),
        (Type::Option(inner), "unwrap_or") => {
            sig(OptionUnwrapOr, vec![(**inner).clone()], (**inner).clone())
        }
        (Type::Option(_), "is_some") => sig(OptionIsSome, vec![], Type::Bool),
        (Type::Option(_), "is_none") => sig(OptionIsNone, vec![], Type::Bool),
        (Type::Option(inner), "ok_or") => sig(
            OptionOkOr,
            vec![Type::Unknown],
            Type::Result(inner.clone(), Box::new(Type::Unknown)),
        ),
        (Type::Result(ok, _), "unwrap_or") => {
            sig(ResultUnwrapOr, vec![(**ok).clone()], (**ok).clone())
        }
        (Type::Result(_, _), "is_ok") => sig(ResultIsOk, vec![], Type::Bool),
        (Type::Result(_, _), "is_err") => sig(ResultIsErr, vec![], Type::Bool),
        (Type::Result(ok, err), "map_err") => sig(
            ResultMapErr,
            vec![func(vec![(**err).clone()], Type::Unknown)],
            Type::Result(ok.clone(), Box::new(Type::Unknown)),
        ),
        (Type::Map(_, _), "length") => sig(MapLength, vec![], Type::Int),
        (Type::Map(k, v), "get") => sig(MapGet, vec![(**k).clone()], Type::Option(v.clone())),
        (Type::Map(k, _), "contains_key") => {
            sig(MapContainsKey, vec![(**k).clone()], Type::Bool)
        }
        (Type::Map(k, _), "keys") => sig(MapKeys, vec![], Type::List(k.clone())),
        (Type::Map(_, v), "values") => sig(MapValues, vec![], Type::List(v.clone())),
        (Type::Map(k, v), "remove") => {
            sig(MapRemove, vec![(**k).clone()], Type::Option(v.clone()))
        }

        // ----- Upload<Format> accessors (slice 52c-2) -------------
        (Type::Upload(_), "text") => sig(UploadText, vec![], Type::String),
        (Type::Upload(_), "bytes") => {
            sig(UploadBytes, vec![], Type::List(Box::new(Type::Int)))
        }
        (Type::Upload(_), "filename") => sig(UploadFilename, vec![], Type::String),
        (Type::Upload(_), "content_type") => sig(UploadContentType, vec![], Type::String),
        (Type::Upload(_), "size") => sig(UploadSize, vec![], Type::Int),

        // ----- Response<Body> accessors (B05) ---------------------
        (Type::Response(_), "status") => sig(ResponseStatus, vec![], Type::Int),
        (Type::Response(body), "body") => sig(ResponseBody, vec![], (**body).clone()),
        (Type::Response(_), "header") => {
            sig(ResponseHeader, vec![Type::String], Type::Option(Box::new(Type::String)))
        }

        // Every receiver is offered `to_json`; the checker then
        // refuses types with no JSON form (it can see record fields,
        // this table cannot) and marks a Tainted-bearing result.
        (_, "to_json") => sig(JsonEncode, vec![], Type::String),

        _ => None,
    }
}
