# `std.time` — Time surface with deterministic replay

> **Status:** Slice 45m (closed 2026-07-11). Four tools; the clock
> reads are traced and substituted under replay, so time-dependent
> agents re-run deterministically.

## Quick reference

```corvid
import "std:time" use time_now_utc, time_monotonic_ms, time_parse_iso, time_format_iso

agent deadline(days: Int) -> String:
    now = time_now_utc()
    return time_format_iso(now.epoch_ms + days * 86400000)
```

## The design decision

The clock reads are **tools, not builtins**. That single decision
buys the whole replay story for free: every tool call is traced
(`ToolCall`/`ToolResult` events) and substituted from the recorded
trace in replay mode, so an agent that read `2026-07-11T08:30:00Z`
reads exactly that instant on every re-run. It also means the
checker's declaration-kind classifier rejects clock reads inside
`@deterministic` bodies with zero extra machinery — a
"deterministic" agent that secretly reads the clock is a compile
error.

## The four tools

### `time_now_utc() -> TimeInstant uses time_wall`

Wall-clock now. `TimeInstant` carries `epoch_ms: Int` (UTC epoch
milliseconds) and `iso: String` (the same instant pre-rendered as
RFC 3339), plus `effect_meta`. Nondeterministic; replay
substitutes the recorded instant.

### `time_monotonic_ms() -> Int uses time_monotonic`

Monotonic milliseconds since an unspecified origin (first read in
the process). Only DIFFERENCES between two reads are meaningful —
use it for elapsed-time measurement, never as a timestamp.

### `time_parse_iso(text: String) -> Result<Int, String> uses time_wall`

RFC 3339 / ISO-8601 parse to UTC epoch milliseconds. Accepts
offset forms (`+02:00`) and normalizes to UTC. Malformed input
flows through the `Result` — never a trap.

### `time_format_iso(epoch_ms: Int) -> String uses time_wall`

Renders epoch milliseconds as RFC 3339 UTC with a trailing `Z` and
millisecond precision.

## Durations and instants

B06 adds two exact types. `Instant` is a point on the UTC timeline,
to the nanosecond, from 0001-01-01 to 9999-12-31. `Duration` is an
exact length of elapsed time. Read the clock with `time_now()`:

```corvid
import "std:time" use time_now

agent wait_report(started: Instant) -> String:
    waited = time_now() - started
    return "waited " + waited.whole_seconds().to_string() + " s (" + waited.to_string() + ")"
```

`time_now() -> Instant uses time_wall` is a tool like
`time_now_utc`: it is traced, replay returns the recorded instant,
and `@deterministic` code cannot call it. Everything else below is
pure.

| Expression | Result |
| --- | --- |
| `instant - instant` | the `Duration` between them |
| `instant + duration`, `duration + instant`, `instant - duration` | an `Instant` |
| `duration + duration`, `duration - duration`, `-duration` | a `Duration` |
| `duration * n`, `n * duration`, `duration / n` with `n: Int` | a `Duration`; `/` truncates to the nanosecond |
| comparisons of two durations or two instants | `Bool` |

Instants compare on the timeline, so `2026-03-29T03:30:00+02:00`
equals `2026-03-29T01:30:00Z`. Adding a bare number to a duration
or an instant is a compile error, because a number has no unit, and
adding two instants is a compile error too. Arithmetic is exact and
traps if a result leaves the range.

| Method | Result |
| --- | --- |
| `text.parse_instant()` | `Result<Instant, String>` from RFC 3339 with any offset |
| `text.parse_duration()` | `Result<Duration, String>` from ISO 8601 such as `PT1H30M`; `P1D` is rejected because a day is not a fixed length |
| `instant.to_string()` | RFC 3339 in UTC, such as `2026-07-11T08:30:00.012Z` |
| `instant.epoch_milliseconds()` | `Int`, rounded toward negative infinity |
| `Instant.from_epoch_milliseconds(n)` | the `Instant` `n` milliseconds after 1970-01-01T00:00:00Z |
| `duration.to_string()` | ISO 8601, such as `PT1H30M` |
| `duration.whole_hours()`, `whole_minutes()`, `whole_seconds()`, `whole_milliseconds()`, `whole_microseconds()`, `whole_nanoseconds()` | `Int`, truncated toward zero |
| `duration.abs()` | the length without its sign |

A tool or prompt can take and return `Instant` and `Duration`. They
cross as RFC 3339 and ISO 8601 text, and a prompt's schema tells the
model which format to use. Routes, stores, tables, connectors and
`to_json` refuse them until B06's JSON slice publishes the same text
forms in the application contract. Native builds refuse them until
their native slice.

A duration literal, `30 seconds`, is proposed and waits for agreement
with frontend development. Until it lands, durations come from
subtracting instants or from `parse_duration`.

`time_parse_iso` and `time_format_iso` keep working; new code uses
`parse_instant` and `to_string`, which are pure and need no tool call.
Older code that keeps durations as `Int` milliseconds also keeps
working.

## Time zones

`TimeZone` is an IANA zone and `ZonedTime` is an instant seen in one.
A string literal where a `TimeZone` is meant is a zone, checked
against the bundled IANA database when the program compiles, so a
typo such as `"Europe/Berln"` is a compile error that suggests
`Europe/Berlin`:

```corvid
agent opening(at: Instant) -> String:
    local = at.in_zone("Europe/Berlin")
    return local.hour().to_string() + ":00 in " + local.time_zone().to_string()
```

A zone name that arrives at run time, such as from a user's profile,
goes through `text.parse_time_zone()`, which returns a `Result` whose
`Err` suggests the closest name. A fixed offset such as `+02:00` is
not a zone, because it has no daylight-saving rules.

| Call | Result |
| --- | --- |
| `instant.in_zone(zone)` | the `ZonedTime`; always succeeds |
| `zoned.year()`, `month()`, `day()`, `hour()`, `minute()`, `second()`, `nanosecond()` | the local field as an `Int` |
| `zoned.weekday()` | 1 for Monday through 7 for Sunday |
| `zoned.offset_seconds()` | the offset from UTC in force then |
| `zoned.time_zone()`, `zoned.instant()` | the zone and the instant |
| `zoned.with_zone(zone)` | the same instant in another zone |
| `zoned.add_days(n)`, `add_months(n)`, `add_years(n)` | the same local time on another calendar date |
| `zoned + duration`, `zoned - duration` | a move along the timeline |
| `zoned - zoned` | the `Duration` between their instants |
| `zoned.to_string()` | RFC 9557, such as `2026-03-29T09:00:00+02:00[Europe/Berlin]` |
| `text.parse_zoned_time()` | `Result<ZonedTime, String>` from RFC 9557 |
| `text.parse_local_time(zone)` | `Result<ZonedTime, String>` from a wall-clock time such as `2026-03-29T09:00` |

**Calendar days are not 24 hours.** In Berlin the clocks move forward
on 29 March 2026, so:

```corvid
agent next_morning(saturday: ZonedTime) -> ZonedTime:
    return saturday.add_days(1)
```

For `2026-03-28T09:00:00+01:00[Europe/Berlin]` this returns
`2026-03-29T09:00:00+02:00[Europe/Berlin]`, which is 23 hours later.
Adding a `Duration` of `PT24H` instead gives 10:00.

`add_months` moves a day past the end of the month back to its last
day, so one month after 31 January is 28 or 29 February. When a
calendar move lands on a local time the zone skips, it moves forward
by the length of the gap, and when it lands on one the zone shows
twice, it takes the earlier offset. This is the rule RFC 5545
(iCalendar) and Temporal use.

**Reading a local time never guesses.** `parse_local_time` returns an
`Err` for a time the zone skips (`2026-03-29T02:30` in Berlin) or
shows twice (`2026-10-25T02:30`), and the message names both offsets.
`parse_zoned_time` refuses text whose offset the zone does not use at
that time.

Zoned times compare with `==` (same instant and same zone). They have
no `<`, because two zoned times in different zones can be the same
instant; compare `a.instant() < b.instant()`.

Zone rules come from the IANA database compiled into Corvid
(`corvid_time::TZDB_VERSION`), the same one the queue scheduler uses,
so a schedule and a program agree on every daylight-saving change.

## Non-scope

Plain dates and times without a zone (such as birthdays), and explicit
earlier or later resolution of skipped and repeated local times, wait
for an example that needs them.
