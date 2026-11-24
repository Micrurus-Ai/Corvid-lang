# Parallel backend sessions

Backend and AI work on Corvid runs in two Claude Code sessions at once:
one on Windows (the WSL Ubuntu machine) and one on macOS. Frontend syntax
belongs to Codex on a third machine. This page is how the sessions share
the repository without stepping on each other. Read it, `CLAUDE.md` and
[`docs/backend-ai-roadmap.md`](../backend-ai-roadmap.md) before starting.

## Who owns what

| Session | Owns |
| --- | --- |
| Windows (main line) | The roadmap's main sequence: B09 typed persistence, then B10 shared actions and B11, in order. Also reviews and merges worker branches built on that machine. |
| macOS | Tasks the roadmap marks as running in parallel with the main line, one at a time. They depend on nothing the main line is building. The Mac also runs the macOS checks no other machine can: native builds and shared libraries on macOS, and the Swift SDK tests on Apple's toolchain. |
| Codex | Frontend syntax and the frontend roadmap. Grammar shared with the frontend (B10's action grammar, field renaming in B01, raw string literals) is agreed with Codex before either backend session implements it. |

The first macOS task is **B06: decimal money, durations and time zones**.
It depends on nothing and adds new types and methods, not table, action
or query code. After B03 is on `main`, B04 (cryptography) is the natural
next one.

Avoid editing the main line's files unless a change requires it, and then
keep the edit small and say so in the commit body. These are currently:
`corvid-types/src/table_ops.rs`, `corvid-types/src/checker/tables.rs`,
`corvid-runtime/src/tables.rs` and `tables/`, `corvid-vm/src/interp/tables.rs`,
`corvid-driver/src/table_schema.rs` and the `corvid migrate` commands.
Adding a variant to a shared enum (`Type`, `IrExprKind`, `BuiltIn`) is
expected; add the arm at every exhaustive match in the same commit.

## Claiming and ordering

1. Before starting a task, add a line to the order note in
   `docs/backend-ai-roadmap.md` saying it runs in parallel on the macOS
   session, and push that change on its own. The rule against reordering
   the roadmap without an amendment applies to both sessions.
2. Work on one branch per task, named for the work it delivers, such as
   `backend/decimal-and-time`. Keep task IDs such as `B06` out of branch,
   file and test names; they belong in commit messages and the roadmap
   (see [professional naming](../../CONTRIBUTING.md#professional-naming)).
3. Each slice of the task ships with its docs: the proposal or reference
   page, the roadmap checklist, `dev-log.md` and `learnings.md`.
   `dev-log.md` and `learnings.md` collect entries from every session, so
   a rebase usually conflicts there. Keep both entries.

## Pushing

- `git pull --rebase` before you push. Never force-push `main`, and never
  rewrite a commit another session may have pulled.
- Push to `main` only when the gate below is green on your machine.
- Report each pushed slice to the user in a few lines: what shipped, what
  is left in the task, anything the other sessions must know (a changed
  shared enum, a behavior change).
- Ask the user before destructive git operations, publishing anything
  outside the repository, or decisions that belong to the owner
  (hosting, telemetry, bundled models, billed services).

## The gate

Run from the repository root:

```sh
cargo check --workspace --tests
cargo test -p <each crate you touched> --lib
cargo test -p <crate> --test <each integration test you added or touched>
cargo run -q -p corvid-cli -- verify --corpus tests/corpus
```

`verify` must report the same divergences as on `main` and no new ones;
today that is `tier_disagree.cor`, a deliberate fixture. Restore
`Cargo.lock` after a run unless the change adds a dependency on purpose.

A change to a shared enum or to code every tier uses (the checker, IR
lowering, the runtime, codegen) also runs, before pushing:

```sh
cargo test -p corvid-driver --no-fail-fast
cargo test -p corvid-abi --no-fail-fast
cargo test -p corvid-runtime --no-fail-fast
cargo test -p corvid-cli --no-fail-fast
cargo build --release -p corvid-test-tools && cargo test -p corvid-codegen-cl --test parity
```

Without `--no-fail-fast`, `cargo test` stops at the first failing test
binary and the rest of the package never runs.

## Setting up macOS

- Rust stable through `rustup`, and the Xcode command-line tools for the
  native linker and `swiftc`.
- Native-target tests link `libcorvid_runtime.a`. Build it once with
  `cargo build -p corvid-runtime` and `cargo build --release -p corvid-runtime`;
  tests find Cargo's output directory through `corvid-test-paths`, so a
  custom `CARGO_TARGET_DIR` works.
- The SDK round-trip tests compile the generated Kotlin and Swift. They
  find `swiftc` and `kotlinc` on `PATH` (`kotlinc` from
  `brew install kotlin`), or through `CORVID_SWIFTC` and `CORVID_KOTLINC`.
  Kotlin also needs `CORVID_KOTLIN_CLASSPATH`, the kotlinx-serialization
  core and json 1.7.3 jars separated by `:`. A test without its toolchain
  prints `skipping:` and passes, so check for that line before claiming
  SDK coverage.
- Report anything that behaves differently on macOS than on Linux as its
  own finding, with the command that shows it.
