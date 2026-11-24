# Proposal: decimal money, durations and time zones (B06)

Status: **accepted, in progress.** Slices are tracked under "Task progress" in [the backend and AI roadmap](../backend-ai-roadmap.md).
Owner: backend and AI development, macOS session ([parallel backend sessions](../internals/parallel-backend-sessions.md)). This proposal changes no parser rule, so it needs no frontend agreement. The lexer keeps each float literal's source digits, which changes no accepted program.

## Problem

A backend developer moving from Python or TypeScript expects to find `decimal.Decimal` or Decimal.js for money, `timedelta` or `Temporal.Duration` for elapsed time, and `zoneinfo` or `Temporal.ZonedDateTime` for local time. Corvid has none of them:

- Money is a `Float`, so `0.1 + 0.2 != 0.3`, and a price list summed in a loop drifts by fractions of a cent. Nothing in the language stops an author from storing a price as a `Float`.
- Durations are bare `Int` milliseconds. Nothing distinguishes a duration from a timestamp, a count or a price, so passing seconds where milliseconds are expected compiles.
- Corvid has no way to ask what time it is in Berlin, or to add one calendar day to a meeting across a daylight-saving change. An app that schedules anything for people has to call a host tool.

## What exists today

I surveyed the codebase before writing any code:

- `std/time.cor` offers `time_now_utc() -> TimeInstant`, where `TimeInstant` is a record with `epoch_ms: Int` and `iso: String`. It also offers `time_monotonic_ms`, `time_parse_iso` and `time_format_iso`. All four are tools. The two conversions are tools only because they need a date library, and they declare the `time_wall` effect even though they are pure. The reference page says that durations are plain `Int` milliseconds.
- `corvid-runtime` already depends on `chrono` and `chrono-tz`. The queue scheduler (`queue/schedule.rs`) uses them to run cron schedules in IANA time zones. That makes `chrono-tz` the time zone database Corvid already ships, and the new types use the same one, so a schedule and a program cannot disagree about when a zone's daylight-saving change happens.
- No Corvid crate depends on a decimal library.
- The checker infers types bottom-up. A literal has its own type wherever it appears, so `x: Decimal = 19.99` cannot make `19.99` a decimal without bidirectional checking, which no other part of the checker needs.
- `Int` already widens to `Float` (`Type::is_assignable_to`, and the `+ - * / %` rules in `checker/ops.rs`). Builtin methods live in one table, `corvid_types::builtin_methods`, which the checker, the IR lowerer, the interpreter and the native code generator all read.
- B03 set the precedent for rules that both the checker and the runtime need. `corvid-text` holds the rules for validating regular expression literals at compile time and for executing them on both tiers.

## Design

### Five new primitive types

| Type | Meaning | Runtime value |
| --- | --- | --- |
| `Decimal` | An exact base-10 number | a coefficient of at most 38 digits and a scale from 0 to 28 |
| `Duration` | An exact elapsed time | signed 64-bit nanoseconds, about ±292 years |
| `Instant` | An exact point on the UTC timeline | signed 64-bit nanoseconds since the Unix epoch |
| `TimeZone` | An IANA time zone | the zone's name, resolved against the bundled database |
| `ZonedTime` | An instant viewed in a time zone, with its local date, time and offset | an `Instant` and a `TimeZone` |

These are `Type` variants, like `Int` and `String`, and not records. Each has its own JSON form, equality and ordering, and its arithmetic needs operator rules a record cannot have.

### Rules shared by the checker and both tiers

A new crate, `corvid-decimal`, holds decimal parsing, arithmetic, rounding and formatting. A second new crate, `corvid-time`, holds durations, instants, zone lookup and calendar arithmetic, and wraps `chrono` and `chrono-tz`. The checker calls each crate to validate literals, and the interpreter and the native runtime call the same crate to execute. As with `corvid-text`, they are separate crates because `corvid-types` cannot depend on `corvid-runtime`. The queue scheduler moves its zone lookup onto `corvid-time` in the same slice that adds time zones, so Corvid keeps one copy of the zone rules.

### Decimal

Amended on 2026-11-24 under the project's syntax simplicity rule, before slice 1 landed. The first draft spelled constants `Decimal("19.99")` and division `a.divide_half_even(b, places)`; [Syntax alternatives](#syntax-alternatives-considered) records why both were replaced.

**Construction.**

| Form | Result |
| --- | --- |
| A number literal where a `Decimal` is meant | `Decimal`, read exactly from the digits written: `price: Decimal = 19.99`, `Line("Coffee", 4.50, 2)` for a `Decimal` field, `price * 1.08`. |
| `text.parse_decimal()` | `Result<Decimal, String>`, for text that arrives at run time. It trims whitespace as `parse_int` does. |
| `n.to_decimal()` on an `Int` | `Decimal`. This conversion is exact. |
| `x.to_decimal()` on a `Float` | `Decimal`. It takes the shortest decimal that reads back as the same `Float`, the digits `to_string` shows, so `0.1.to_decimal()` is `0.1`. It traps on NaN and infinity, as `to_int_truncated` does. |
| `d.to_float()` | `Float`. The name says the conversion is lossy, and it is the only way from `Decimal` to `Float`. |

**Literals take their type from where they are used.** A literal made only of numbers, signs and arithmetic (`19.99`, `-0.50`, `0.1 + 0.2`) is a `Decimal` when its slot is declared `Decimal` (a binding, parameter, field, list item or return) or when the other operand of its operator is a `Decimal`. Anywhere else it is a `Float` or an `Int`, as before. The checker already passes the expected type into every slot, so this is one rule in the literal case and one in the operator rule. The lexer keeps each float literal's source digits, so `2.50` keeps its scale and a long fraction keeps every digit; a literal with no exact `Decimal` value is a compile error. An `Int` variable in a `Decimal` slot is a type error whose hint names `to_decimal()`, because the interpreter does not convert values at slots (a gap the existing `Int`-to-`Float` widening has too; see the roadmap).

**Arithmetic.**

- `+`, `-`, `*`, `%`, unary `-` and the comparisons work between two `Decimal` values, or between a `Decimal` and an `Int`, because converting an `Int` is exact. So `price * quantity` and `total - 1` work.
- **Mixing `Decimal` and `Float` is a compile error.** The message names `to_decimal()` and `to_float()`. `Decimal` is never assignable to `Float`, and `Float` is never assignable to `Decimal`.
- **Division is written with its rounding:** `(total / 3).round_half_even(2)`. A `Decimal` `/` is accepted only as the receiver of a rounding method, and IR lowering fuses the two into one operation that computes the exact quotient and rounds it once. Any other `/` on decimals, including `total /= 3`, is a compile error that shows this form. This is the B06 acceptance's "declared rounding". A silent default precision, such as the 28 digits Python's default context uses, would be a hidden rounding policy on money.
- **Results are exact or they trap.** Addition, subtraction and multiplication either produce the exact result or trap, following the rule that arithmetic is always checked. A value holds at most 38 significant digits and at most 28 after the point; a product that needs more than 28 fractional digits traps rather than rounding silently. A program that multiplies rates repeatedly rounds as it goes, which it has to do in any exact system.
- **Scale is kept and equality is numeric.** `1.50 + 1.25` is `2.75`, and `2.50` prints as `2.50`, as in Python. `1.0 == 1.00` is `true`, and both values are the same map key.
- Taint and grounding pass through these operators as they do for `Int`, because the operator rules run inside the existing contagion wrapper in `check_binop`.

**Rounding.** The rounding mode is part of the method name, so a reader sees the policy where it is applied and needs nothing imported:

| Method | Mode |
| --- | --- |
| `d.round_half_even(places)` | Ties to even (banker's rounding) |
| `d.round_half_up(places)` | Ties away from zero (school rounding) |
| `d.round_toward_zero(places)` | Truncate |
| `d.round_away_from_zero(places)` | Any remainder moves away from zero |
| `d.round_floor(places)` | Toward negative infinity |
| `d.round_ceiling(places)` | Toward positive infinity |

`places` is the number of fractional digits in the result, from 0 to 28. A rounding result always has exactly `places` fractional digits, so `2.5` rounded with `round_half_even(2)` is `2.50`, the scale a price display wants. Division by a zero value traps, as it does for `Int`, and a literal zero divisor is a compile error. A computed `places` outside 0 to 28 traps, and a literal one is a compile error.

**Other methods.** `d.abs()`, `d.scale()` (the number of fractional digits), `d.to_string()` (plain notation, never an exponent) and `d.to_int_truncated()`, which traps outside the `Int` range.

**Currency formatting.** `d.format_currency(currency, locale)` returns a `String` (amended under the syntax simplicity rule before slice 4 landed):

- `currency` is a `Currency` and `locale` is a `Locale`, two small types. A string literal where one is meant is one, checked against the bundled CLDR data, as zones are: `total.format_currency("EUR", "de-DE")`. A currency CLDR does not name, or a locale whose language CLDR has no data for, is a compile error. Text that arrives at run time goes through `text.parse_currency()` or `text.parse_locale()`, which return a `Result`. The draft's second method, `format_currency_checked`, is gone, because the types carry the check. A region or script without its own data falls back within its language, as CLDR intends (`de-LU` formats as German), but never to the root locale.
- `currency.minor_digits()` gives the digits the currency shows (2 for USD, 0 for JPY, 3 for KWD), so a program rounds with `amount.round_half_even(currency.minor_digits())`.
- **There is no default locale.** Whether `1234.5` prints as `$1,234.50` or `1.234,50 $` is visible to every customer, so the author declares the locale. A process-wide locale picked up from the host, as C's `setlocale` does, would make output depend on the machine.
- **Formatting never rounds.** An amount shows at least the currency's minor-unit digits, padded with zeros. An amount with more digits than the currency shows, such as `1.005` in USD, traps with a message that asks for a declared rounding first. CLDR formatters round to the minor unit, and a formatter that rounded would introduce a second, hidden rounding policy into an invoice. The draft showed the extra digits (`$1.005`) instead, but no CLDR pattern writes money that way, so the trap is the honest choice.
- The locale data is CLDR through ICU4X, the Unicode Consortium's Rust implementation, compiled into the binary. ICU4X's currency formatter lives in `icu_experimental`, whose API is not yet stable, so it is pinned exactly (`=0.7.0`) and used only from the `corvid-currency` crate. A probe binary that formats in every CLDR locale, with the currency names and likely-subtags data the checks need, is 1.7 MB in a release build. ICU4X covers every locale in CLDR, not a hand-picked subset.

### Duration

Amended on 2026-11-24 under the syntax simplicity rule, before slice 2 landed.

**Construction.** A duration is written as a number and a unit: `timeout = 30 seconds`, `deadline = now + 2 hours`, `1.5 seconds`. A computed duration multiplies one, as in `attempts * 30 seconds`, so the literal is the only way to construct a duration and there is no second spelling such as `30.seconds()`. The unit words, singular or plural, are nanoseconds, microseconds, milliseconds, seconds, minutes and hours. They are contextual, not reserved, and they match the bounded ask's existing `within 30 seconds`. A number followed by a word is a parse error today, so no program changes meaning. The literal is shared grammar, so it waits for agreement with frontend development and lands as slice 2b. Until then a duration comes from `instant - instant` or from `text.parse_duration()`, which reads ISO 8601 (`PT1H30M`) into a `Result`.

**There is no day.** In a time zone, a day is 23, 24 or 25 hours, and "one day" in a product requirement almost always means the calendar day. `30 days` will be a compile error that names both meanings: `24 hours` for exactly 24 hours, and `zoned.add_days(n)` for the same local time on another day. The same applies to weeks, months and years, and `parse_duration` already rejects `P1D` with that message. Python's `timedelta(days=1)` and Java's `Duration.ofDays` both blur this distinction.

**Arithmetic.** `Duration` supports `+` and `-` between durations, `*` by an `Int` on either side, `/` by an `Int` (truncating toward zero to the nanosecond, as `Int` division does), unary `-`, `abs()` and comparisons. `Duration + Int` is a compile error whose hint says a bare number has no unit, the papercut that bare-`Int` milliseconds kept. The readings `d.whole_nanoseconds()`, `whole_microseconds()`, `whole_milliseconds()`, `whole_seconds()`, `whole_minutes()` and `whole_hours()` return an `Int`, truncated toward zero; the name says so. `d.to_string()` prints the ISO 8601 form.

### Instant

- `instant + duration`, `duration + instant`, `instant - duration` and `instant - instant -> Duration` are exact. `instant + instant` is a compile error. Instants compare by their position on the timeline, so `03:30+02:00` equals `01:30Z`.
- An instant holds any time from 0001-01-01 to 9999-12-31 UTC, to the nanosecond, the range RFC 3339 can write. Leaving it traps.
- `text.parse_instant()` reads RFC 3339 text with any offset into a `Result`. `instant.to_string()` prints RFC 3339 in UTC with a trailing `Z`, with no fraction for a whole second and otherwise 3, 6 or 9 digits. `instant.epoch_milliseconds()` returns an `Int`, rounded toward negative infinity as JavaScript's `Date` reads it, and `Instant.from_epoch_milliseconds(n)` converts the other way for data that arrives as epoch numbers.
- **Reading the clock stays a tool.** `std/time` gains `time_now() -> Instant uses time_wall`, which is traced and replayed exactly as `time_now_utc` is, to the nanosecond the clock gives. The parsing and formatting methods above are pure, so they replace the `time_parse_iso` and `time_format_iso` tools. Those tools stay, with their reference entries pointing to the methods, because removing them would break programs without giving anything back. `TimeInstant` gains no new fields, and `time_now_utc` keeps working.
- **Tools and prompts carry time as text.** A tool or prompt signature may use `Duration` and `Instant` from slice 2: the interpreter sends and reads them as ISO 8601 and RFC 3339 text through `corvid_time`, and a prompt's schema says `format: duration` or `format: date-time`, so a model never invents a format. Routes, stores, tables, connectors and `to_json`/`from_json` wait for the JSON slice, which publishes the same text forms in the contract.

### Time zones and local time

Amended on 2026-11-24 under the syntax simplicity rule, before slice 3 landed.

**Construction.** A string literal where a `TimeZone` is meant is a zone, as a number literal where a `Decimal` is meant is a decimal: `home: TimeZone = "Europe/Berlin"`, `at.in_zone("Asia/Tokyo")`. The compiler checks the name against the bundled IANA database, so `"Europe/Berln"` is a compile error that suggests `Europe/Berlin`. `text.parse_time_zone()` returns `Result<TimeZone, String>` for names that arrive at run time, such as from a user's profile. `"UTC"` is accepted. Fixed-offset zones (`+02:00`) are not, because an offset is not a zone and does not change for daylight saving, which is the bug a zone type prevents.

**Conversion.** `instant.in_zone(zone) -> ZonedTime` always succeeds, because every instant has exactly one local time in every zone. `ZonedTime` has accessor methods for `year()`, `month()`, `day()`, `hour()`, `minute()`, `second()`, `nanosecond()`, `weekday()` (1 = Monday, as in ISO 8601), `offset_seconds()`, `time_zone()` (`zone` is a keyword) and `instant()`, plus `to_string()`, which prints RFC 9557 text (`2026-03-29T03:30:00+02:00[Europe/Berlin]`), and `text.parse_zoned_time()` reads that text back, refusing an offset the zone does not use then. Zoned times compare with `==` (same instant and zone) but have no `<`, because two zoned times in different zones can be the same instant; compare their instants.

**From local time to an instant.** Moving from local time to an instant is where daylight saving causes bugs. A local time can occur twice, in the autumn fold, or never, in the spring gap. `text.parse_local_time(zone)` reads a local date and time (`2026-03-29T02:30`) and returns `Result<ZonedTime, String>`. A gap or a fold is an `Err` whose message names the zone, the transition and both possible offsets. A program cannot silently book a meeting at a time that does not exist. Explicit resolution methods, which pick the earlier or the later offset, are listed under [Deferred](#deferred) until an example needs them.

**Calendar arithmetic.** `zoned.add_days(n)`, `zoned.add_months(n)` and `zoned.add_years(n)` keep the local wall-clock time and move the calendar date. `zoned + duration` moves along the timeline. Across the spring change in Berlin, `add_days(1)` on 2026-03-28 at 09:00 gives 2026-03-29 at 09:00, which is 23 hours later, while `+ 24 hours` gives 10:00. This pair is the B06 acceptance test. Two rules apply where a calendar move can land badly:

- A month or year move that lands past the end of the month moves back to the last day (31 January plus one month is 28 or 29 February), as Python's `dateutil`, Java and Temporal do.
- A move that lands in a daylight-saving gap moves forward by the length of the gap, and a move that lands in a fold takes the earlier offset. This is the rule from RFC 5545 (iCalendar) and Temporal's `compatible` option, and it is the only result a calendar application can show without asking the user. It is the same in every case, so it is defined behavior and not a hidden policy choice, and the reference documents it with the Berlin examples.

`zoned.with_zone(other)` views the same instant in another zone.

### JSON, contracts and SDKs

The seven types get a JSON form through `corvid-public-json` (`text_scalar`), so HTTP routes, `T.from_json`, tools, prompts and OpenAPI agree; each is a string in the type's canonical text, the one its own crate prints and parses:

| Type | JSON | OpenAPI |
| --- | --- | --- |
| `Decimal` | A string, such as `"19.99"`. JSON numbers lose precision in JavaScript, which is why payment APIs send amounts as strings or integer minor units. | `type: string`, `format: decimal` |
| `Duration` | An ISO 8601 string, such as `"PT30S"` | `type: string`, `format: duration` |
| `Instant` | An RFC 3339 string in UTC | `type: string`, `format: date-time` |
| `TimeZone` | An IANA name | `type: string` |
| `ZonedTime` | An RFC 9557 string | `type: string` |
| `Currency` | An ISO 4217 code | `type: string`, `pattern: ^[A-Z]{3}$` |
| `Locale` | A BCP 47 tag | `type: string` |

Decoding a JSON number into a `Decimal` field is a `JsonError` that names the field and asks for a string. Accepting numbers exactly would need the raw number text, and a decoder that went through `f64` would reintroduce the error that `Decimal` exists to remove. **The TypeScript client and the Swift, Kotlin and Python SDKs hold each type as a string with its exact text** (amended when slice 5 landed). The draft mapped them to platform types, but those lose data in two of the four languages: Python's `datetime` and `timedelta` and Swift's `Date` drop nanoseconds, and Swift's `Decimal` drops trailing zeros, so `19.90` would come back as `19.9`. JavaScript has neither an exact decimal nor a nanosecond time. The exact text is the one mapping every client holds without loss, it matches what OpenAPI publishes, and a client converts with its platform parser where it wants arithmetic. A JSON number sent for a `Decimal` is refused in the generated Python too.

**Table columns are not part of the JSON form.** How a column is stored decides how `where` and `order_by` compare it: a decimal stored as text sorts wrong, and Postgres timestamps and intervals hold microseconds, not nanoseconds. That mapping is a B09 item owned by the main line, which reuses the canonical encodings named here (`Decimal::to_string`/`parse` or `coefficient`/`scale`, `Instant::nanoseconds_since_epoch`, `Duration::nanoseconds`, `TimeZone::name`, `ZonedTime::instant` with its zone). Until it lands, a table column of a B06 type is a compile error.

### Across the tiers

Answered under the project's three-tier rule. Slice 5 delivered the JSON forms every tier reads; the browser-side bindings below still need the frontend's form work.

- **What the backend knows that the frontend and AI can use.** A `Decimal` carries its exact digits and its scale, so `19.90` is two places and not a binary approximation. A `TimeZone` carries the zone's daylight-saving transitions, and a `ZonedTime` carries the instant, the zone and the offset in force. Each of these is one declaration in source, such as `price: Decimal`.
- **What the UI gets for free.** From slice 5, the same declaration types the frontend. A `Decimal` field becomes a price input that accepts exactly the digits a `Decimal` holds and reports errors in `parse_decimal`'s words. The browser runs the `corvid-decimal` rules compiled to WebAssembly, not a JavaScript copy, so a total shown in the cart is the total the server charges. A `ZonedTime` or `Instant` field displays in the viewer's zone and handles daylight-saving changes in the browser. Currency display (slice 4) uses the one formatter with an explicit locale. A separate stack needs a decimal library on each side and a convention for sending amounts as strings, and the two drift. Here the compiler checks both sides against one type, and changing a field from `Float` to `Decimal` points at every form, route and prompt that uses it.
- **What an assistant can safely do.** From slice 5, a prompt or tool typed with `Decimal` gives the model a JSON schema whose amounts are strings in the exact decimal form, and the reply is decoded with `parse_decimal`'s rules. An amount never passes through a float on its way to or from a model. From slice 2, a prompt or tool typed with `Instant` or `Duration` already does this for time: the model's schema says `format: date-time` or `format: duration`, and its reply is parsed by `corvid_time`, so a model never invents a date format; zoned times follow in slice 3 as RFC 9557 text that names the zone. Nothing becomes available to an assistant except through a tool, prompt or action the source declares. Effects, approval, replay and budgets apply to them as to any other value. Whether `@budget` should count cost as a `Decimal` instead of a `Float` is recorded for B06's JSON slice.

### Replay, effects and purity

Every method above is pure and deterministic, so replay never records them, and they are allowed in `@deterministic` code and in `fn`s. Only `time_now` reads the clock, and it is a tool. Zone rules come from the database compiled into the binary, so a trace replayed by a Corvid build with a newer database could see a different offset for a zone whose rules changed. The trace header therefore records the database version (trace schema v3). Replay never refuses a trace over it, since most traces touch no zone: each zone-dependent operation the replay runs warns once, naming the operation, the zone and both versions, when the databases differ or the recording's is unknown. This changes the trace schema, which every tier writes, so it is its own increment (3b) after the zones themselves.

### Native code

Both tiers call `corvid-decimal` and `corvid-time`. The native representation (`Duration` and `Instant` as 64-bit integers, `TimeZone` as an index into the bundled database, and `ZonedTime` as a pair) is decided in each type's native slice, with parity tests that compare results on both tiers. Until then, programs that use these types run on the interpreter, and native builds refuse them with a message naming the slice, as they refused regular expressions before B03's native slices.

Slice 6 decided `Decimal`. Native code keeps every value in one 8-byte slot, so a 128-bit value would need a heap object anyway. A native `Decimal` is therefore a Corvid String holding its exact canonical text, the text of its JSON form, which reuses string reference counting, list and record storage and printing. Every operator and method is one call into a runtime entry point that parses with `corvid-decimal`, computes, and returns canonical text, so both tiers share each rule and message. Structural equality compares decimals by value. Parsing on every operation costs time; the roadmap records measuring it and moving to a compact heap value if it dominates.

### Not an invention

These are commodity features. They follow the design principle's rule for commodity features: the best known semantics with the papercuts removed. The papercuts removed here are a compile error for mixing `Decimal` and `Float`, division that declares its rounding, no default locale, formatting that never rounds, zone names checked at compile time, no ambiguous `days()` duration, and errors for local times that fall in a daylight-saving gap or fold. They get reference documentation and tests, but no invention-catalog entry or tour topic.

## Syntax alternatives considered

Recorded under the project's syntax simplicity rule.

- **Constants.** `Decimal("19.99")` (Python's spelling) makes every price a call with a string inside it, and it is a second way to write a number the compiler already knows is meant as a decimal. A suffix (`19.99d`, as in C#) is shorter but still a second spelling, and it is a lexical rule shared with frontend code. Typing literals by context (Swift's `ExpressibleByFloatLiteral` shape, without Swift's detour through `Double`) won: the common case is the shortest, and a reader sees `price * 1.08`.
- **Division.** Six `divide_<mode>(b, places)` methods doubled the API and read backwards (`a.divide_half_even(b, 2)`). A default rounding for `/` would hide a money policy. `(a / b).round_<mode>(places)` keeps `/` where a reader expects it and puts the policy next to it, with one set of mode names for rounding and division.
- **Duration construction.** `30.seconds()` (methods on `Int`) reads as code, not as a time, and would remain a second spelling once a literal exists. `seconds(30)` is a function call for a constant. `Duration.parse("PT30S")` is a string. `30 seconds` won: it is how the sentence is said, it matches `within 30 seconds`, and multiplying it covers computed values without a second constructor.
- **Zones.** `TimeZone("Europe/Berlin")` is a call wrapped around a string the compiler can already check where a zone is meant, and a second spelling of a zone. Typing the literal by its slot, as decimals are, won. `text.parse_zoned_time(zone)` for local text was renamed `parse_local_time(zone)`, because the text holds a local time, not a zoned one; `parse_zoned_time()` reads RFC 9557 text that names its own zone.
- **Currency formatting.** `format_currency("EUR", "de-DE")` with literals and `format_currency_checked(currency, locale)` returning a `Result` for computed values was two methods for one job. Typing the arguments as `Currency` and `Locale`, checked by their slot like zones, leaves one total method.
- **Duration readings.** `total_seconds()` (Python) returns a fraction in Python, so the same name truncating would surprise; `in_seconds()` does not say it truncates. `whole_seconds()` says what it returns.
- **Epoch numbers.** `n.epoch_milliseconds_to_instant()` puts the unit in a long method name on `Int`. `Instant.from_epoch_milliseconds(n)` names the type it makes, as Temporal does.
- **The mode.** `round(2, half_even)` needs six names in scope in every program, and `Up` and `Down` would collide with user sum types. `round(2, "half_even")` is a string the reader must trust. The mode in the method name (`round_half_even(2)`) is plain words, needs no import and autocompletes as a family.

## Slices

1. **Decimal on the interpreter.** The `corvid-decimal` crate, the `Decimal` type, literals typed by context, `parse_decimal`, the conversions, the operators and the compile error for mixing with `Float`, rounded division, the rounding and other methods, and a compile error for `Decimal` at JSON boundaries until slice 5.
2. **Duration and Instant on the interpreter.** The `corvid-time` crate, the two types, the operators, parsing and formatting, epoch conversion, `time_now` in `std/time`, and time at tool and prompt boundaries as text.
2b. **The duration literal** (`30 seconds`) with the calendar-unit diagnostic, after agreement with frontend development.
3. **Time zones on the interpreter.** `TimeZone`, `ZonedTime`, zone literals checked by their slot, conversion, `parse_local_time` with its errors for gaps and folds, `parse_zoned_time`, calendar arithmetic, the Berlin daylight-saving acceptance test, and the scheduler moved onto `corvid-time`.
3b. **The tz database version in the trace header**, with replay reporting a mismatch.
4. **Currency formatting.** `Currency`, `Locale`, `format_currency` and `minor_digits` with CLDR data.
5. **JSON, contracts and SDKs** for all seven types, as exact text; table columns are a B09 item.
6. **Native Decimal**, with parity tests.
7. **Native time types**, with parity tests.

## Deferred

- **Integer literals beyond `Int`'s range in a `Decimal` slot.** The lexer reads an integer literal as an `Int`, so a whole number above 9,223,372,036,854,775,807 is written with a point (`100000000000000000000.0`). Lexing every integer literal as digits, as float literals now are, removes this limit.
- **Table columns.** Storing `Decimal`, `Instant`, `Duration` and `ZonedTime` in B09 tables touches the main line's table files. It is coordinated with the main line after slice 5, when each type has its JSON form.
- **A `Money` type that carries its currency**, so that adding USD to EUR is a compile error. This needs either currency as a type parameter or a runtime check, and it is decided after a backend example has used `Decimal` for money.
- **Explicit gap and fold resolution** (`parse_zoned_time_earlier` and `parse_zoned_time_later`), added when an example needs to accept such times instead of reporting them.
- **Plain dates and times without a zone** (Temporal's `PlainDate`), such as birthdays. Deferred until an example needs one. A `ZonedTime` in `UTC` covers dates in the meantime.
- **Byte and binary encodings of these types**, which wait on B04's byte representation.
