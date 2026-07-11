# `std.random` — Randomness with deterministic replay

> **Status:** Slice 45m (closed 2026-07-11). Two tools; draws are
> traced and substituted under replay, so a program that drew 0.42
> draws 0.42 again on every re-run.

## Quick reference

```corvid
import "./std/random" use random_float, random_int

agent roll() -> Int:
    return random_int(1, 6)
```

## The design decision

Randomness is a **tool, never a builtin** — the same decision as
`std/time` and for the same reason. Tool calls are traced and
substituted under replay; the checker rejects tool calls inside
`@deterministic` bodies. A "deterministic" agent that secretly
rolls dice is a compile error, not a production incident.

## The two tools

### `random_float() -> Float uses rand_draw`

Uniform draw in `[0.0, 1.0)` from OS entropy (53 uniform bits —
the standard f64 recipe).

### `random_int(min: Int, max: Int) -> Int uses rand_draw`

Uniform integer in `[min, max]` — INCLUSIVE on both ends (Python's
`randint` contract: `random_int(1, 6)` is a die). Uses rejection
sampling, so there is no modulo bias. `min > max` is a runtime
error.

## Non-scope

No seeded/reproducible PRNG surface — reproducibility in Corvid
comes from replay, not from seed management. No distributions
(gaussian, exponential); compose them from `random_float` or ship
a follow-up slice if agent workloads demand them.
