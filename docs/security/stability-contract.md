# Stability contract

This document defines the launch stability contract for Corvid v1.0.

## What's stable at v1.0

- **Syntax** — every program that compiles at v1.0 will compile on
  v1.x for x ≥ 0.
- **Type system** — effect rows, `Grounded<T>`, `approve`, agent
  composition rules.
- **CLI surface** — every command in [CLI reference](../reference/cli.md)
  is stable through v1.x. New commands may be added; existing commands
  cannot break.
- **Stdlib core** — `std.io`, `std.json`, `std.http`, `std.db`,
  `std.jobs`, `std.auth`, `std.observability`, `std.connectors`. Per
  the contract, "stable" means signatures and effect rows; backing
  implementation may change.
- **Benchmark-claim semantics** — what the published archives mean,
  the ratio definitions, the model-latency separation rule.
- **Published bundle formats** — see
  [`internals/bundle-format.md`](../internals/bundle-format.md).

## SemVer rules

The following are part of the public compatibility promise:

- Core syntax accepted by the parser.
- Typechecker behavior for shipped language features.
- CLI command names and top-level subcommand structure.
- Public standard-library module names under `std/*`.
- Published benchmark archive formats under `benches/results/*/ratios.json`.
- Published bundle formats documented in
  [`internals/bundle-format.md`](../internals/bundle-format.md).

Compatibility rules:

- **Patch releases** may fix bugs, improve diagnostics, and tighten
  behavior only where the documented semantics already required the
  stricter result.
- **Minor releases** may add new syntax, stdlib APIs, CLI subcommands,
  and metadata fields in backward-compatible ways.
- **Major releases** may remove or rename user-facing language / CLI /
  stdlib surfaces.

## What's NOT stable

- Internal compiler structure (`corvid-types`, `corvid-codegen-cl`,
  etc. — these are workspace crates, not a stable library API).
- The bytecode format produced by `corvid build` (for v1, the binary
  is the artifact, not the bytecode).
- The trace format internal layout (use `corvid replay` and `corvid
  trace dag`; do not parse trace files directly).
- Internal trace event counts or wording outside documented schemas.
- Exact benchmark numbers.
- Unpublished archive layouts under temporary smoke or debug sessions.

## Launch-claim discipline

Any public claim about Corvid's safety, replay, grounding, packaging,
WASM, or benchmark behavior must point to:

1. A runnable command,
2. A checked-in example or archive, or
3. A test.

See [`meta/launch-claim-audit.md`](../meta/launch-claim-audit.md).

## Versioning

Corvid uses SemVer. v1.x is backwards-compatible with v1.0; v2.x
will land its own migration story.
