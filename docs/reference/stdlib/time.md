# `std.time` — Time surface with deterministic replay

> **Status:** Slice 45m (closed 2026-07-11). Four tools; the clock
> reads are traced and substituted under replay, so time-dependent
> agents re-run deterministically.

## Quick reference

```corvid
import "./std/time" use time_now_utc, time_monotonic_ms, time_parse_iso, time_format_iso

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

## Durations

Durations are plain `Int` milliseconds. Ordinary arithmetic IS the
duration API — `deadline = now.epoch_ms + 30 * 1000` — and the
always-checked arithmetic rule covers overflow. There is no
separate Duration type to learn.

## Non-scope

No timezone database surface (UTC only; render local time in the
host application). No calendar arithmetic (`add_months`) — epoch
math plus `time_format_iso` covers the agent-workload cases; a
calendar surface would ship as a separate slice with explicit
DST/overflow semantics.
