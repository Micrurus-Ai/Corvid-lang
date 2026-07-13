# 46e — `parallel:` concurrent arms (design)

Status: DRAFT for review (pre-implementation, per the Phase 46
pre-phase agreement). Closes audit B8. Implements the parallel
composition the effect spec sketched in
`docs/internals/effect-spec/10-interactions.md §6` ("Tracked in
Phase 22" — never shipped).

## Problem

Corvid programs cannot express "fetch these three things at
once." The runtime is tokio and `ensemble` voting already proves
concurrent LLM dispatch works (`interp/prompt/voting.rs`); the
language is the missing layer. The audit calls this B8 — the
feature every agent-loop evaluator reaches for on day one.

## Syntax: named arms, no tuples

```corvid
parallel:
    weather = fetch_weather(city)
    news = fetch_news(city)
    brief = summarize(city)

return render(weather, news, brief)
```

A `parallel:` block contains two or more `name = call(...)` arms.
After the block, every arm's name is bound. This is the
Python-easiest form and needs NO tuple type, no destructuring
ceremony, no closure syntax. (`parallel` is a contextual keyword —
`Ident("parallel") ':'` at statement level — so no programs using
`parallel` as a name break.)

### v1 arm restriction: one effectful call per arm

Each arm's right-hand side must be a CALL to a tool, prompt, or
agent (arguments are arbitrary expressions, evaluated
SEQUENTIALLY before the block starts — so argument evaluation
cannot race). Arbitrary statement bodies per arm are post-v1:
they would require interpreter re-entrancy machinery that buys
little — the thing worth parallelizing IS the effectful call, and
an agent call arm can wrap arbitrary logic today.

The checker rejects non-call arms ("wrap the logic in an agent
and call it") and `parallel:` blocks with fewer than two arms.

## Semantics

- All arms START together; the block completes when ALL arms
  complete (join — no racing/select form in v1).
- **Error rule (deterministic):** join everything, then if any
  arm failed, the block fails with the FIRST failed arm in ARM
  ORDER (not completion order). No cancellation in v1 — arms run
  to completion; their effects happened and their trace events
  are recorded.
- Each arm executes on a FRESH sub-interpreter (same IR, same
  runtime, fresh environment) — agents already get a fresh env
  per call, so semantics are unchanged; the arm simply runs on
  another task.

## Effect composition (per the effect-spec table)

The checker composes arm rows with the PARALLEL operator:
`cost` Sum, `tokens` Sum, `latency_ms` Max, `trust` Max,
`reversible` AND, `data` Union, `confidence` Min. `@budget` cost
analysis sums the arms (sound: both are paid); latency analysis
takes the max (parallelism hides latency up to the slowest arm).

## Replay: record concurrent, normalize to arm order

The design that keeps the ENTIRE existing replay machinery
unchanged:

- Each arm's trace events are BUFFERED per arm while arms run
  concurrently.
- At join, buffers flush to the trace IN ARM ORDER — the recorded
  trace is indistinguishable from a sequential execution of arm
  0, then arm 1, then arm 2.
- On replay, arms execute SEQUENTIALLY in arm order (substitution
  is instant; concurrency buys nothing on replay), consuming
  events exactly as recorded. The sequential replay cursor works
  UNCHANGED — zero trace-schema changes, zero new matching rules.

A `parallel_join` marker event (one per block, recording arm
count) documents block boundaries for trace tooling; replay does
not depend on it.

## Budget accounting

Arm costs accumulate on their sub-interpreters and are charged to
the PARENT budget at join, in arm order, before the error rule
runs. A block that exceeds `@budget` therefore fails
deterministically at the join point. Mid-arm budget termination
inside an arm applies only to that arm's own stream/loop checks
(existing machinery), scoped by the same parent budget snapshot.

## Memory model (RC interaction)

Values crossing into arms are Arc-backed shared cells (the Phase
17 model) — memory-safe under concurrency by construction.
Semantics: arms that MUTATE shared cells are racy in wall-clock
order; field-level atomicity is guaranteed, ordering is not. The
book documents the rule: arms should treat shared inputs as
read-only; a program whose arms race mutations gets whichever
interleaving happened, and replay reproduces the RECORDED values
(not the race). This matches the reference-semantics story from
45b rather than fighting it.

## Non-scope (v1, recorded)

- Racing/select (`parallel race:`), timeouts, cancellation.
- Arbitrary statement bodies per arm (wrap in an agent).
- Streaming arms (`-> Stream<T>` calls in arms are rejected in
  v1 — join semantics for streams need their own design).
- Nested `parallel:` inside an arm's AGENT is fine (it's just a
  call); syntactically nested blocks in the same body are
  rejected in v1.

## Implementation map

1. Parser: contextual `parallel ':'` statement; arms
   `IDENT '=' call NEWLINE`; ≥2 arms.
2. AST `Stmt::Parallel { arms: Vec<ParallelArm> }`; resolver
   binds arm names after the block; checker types each arm call,
   rejects non-calls/stream calls, applies the parallel effect
   operator; cost analysis sums arms.
3. IR `IrStmt::Parallel { arms }`; interpreter: evaluate args
   sequentially, spawn per-arm sub-interpreters via JoinSet
   (voting.rs precedent), buffer traces per arm, join, flush in
   arm order, charge costs, bind results, apply the error rule.
4. Tracer: an arm-buffering handle (`Tracer::buffered()` →
   flushable queue) — additive, no schema change.
5. Compiled tiers: interpreter-only v1 with loud degradations
   (the 45i/45j/45n precedent).
6. Tests: determinism (arm-order error rule), effect composition
   (cost sums, latency maxes), replay round-trip (record
   concurrent → replay sequential, byte-identical results),
   budget-at-join. Book chapter section + grammar + tour topic +
   inventions row (this IS an invention: governed concurrency —
   parallel arms whose costs sum into `@budget`, whose traces
   replay deterministically).
