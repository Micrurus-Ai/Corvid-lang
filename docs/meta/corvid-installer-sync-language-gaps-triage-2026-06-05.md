# corvid-installer LANGUAGE-GAPS.md triage (2026-06-05)

> Triage of the 8 language-side gaps the corvid-installer maintainer
> documented in their
> [`LANGUAGE-GAPS.md`](https://github.com/Micrurus-Ai/corvid-installer/blob/main/LANGUAGE-GAPS.md).
> Their report was tested against commits `259dd59` and `ae8e3dd`;
> this triage re-checks each disposition against Corvid-lang HEAD.

---

## TL;DR

**All 8 gaps are CLOSED at HEAD.** Five were closed in the phases
the maintainer already noted (20l + 20m). The other three — L-3,
L-4, L-7 — the maintainer's report marks as either "Open (roadmap)"
or "Deliberate design choice," but all three actually shipped in
**Phase 20n** on 2026-05-08, after their `259dd59`/`ae8e3dd` test
cutoff. Same staleness pattern as
[OPEN-GAP-PROMPTS.md](https://github.com/Micrurus-Ai/corvid-installer/blob/main/OPEN-GAP-PROMPTS.md):
the audit predates Phase 20n's lands.

There is **no language-side work outstanding** from this document.
The next-action ask is to update `LANGUAGE-GAPS.md` to mark
L-3 / L-4 / L-7 closed; same close-out shape as the L-3/L-4/L-7
items in OPEN-GAP-PROMPTS.md (you drive close-out or we send the
PR — no preference on our side).

---

## Per-gap disposition

| Gap | Maintainer claim | HEAD verdict | Evidence |
|-----|------------------|--------------|----------|
| L-1: `corvid check` ignores import errors | FIXED | **CONFIRMED-FIXED** | [`crates/corvid-cli/src/commands/misc.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-cli/src/commands/misc.rs) calls `compile_with_config_at_path`; regression test at [`crates/corvid-cli/tests/check_validates_imports.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-cli/tests/check_validates_imports.rs). Original fix at [`bfe6232`](https://github.com/Micrurus-Ai/Corvid-lang/commit/bfe6232). |
| L-2: Python codegen loses nested struct types | FIXED | **CONFIRMED-FIXED** | [`crates/corvid-codegen-py/src/lib.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-codegen-py/src/lib.rs) emits concrete struct names via PEP 563 forward refs; nested struct test asserts `inner: Inner`, NOT `inner: object`. |
| L-3: Native codegen rejects struct returns | Open (roadmap) | **CLOSED — maintainer audit stale** | Shipped under Phase 20n slice **20n-C** on 2026-05-08, across six commits ([`10107cc`](https://github.com/Micrurus-Ai/Corvid-lang/commit/10107cc), [`1361a61`](https://github.com/Micrurus-Ai/Corvid-lang/commit/1361a61), [`9d8e19d`](https://github.com/Micrurus-Ai/Corvid-lang/commit/9d8e19d), [`cfb131d`](https://github.com/Micrurus-Ai/Corvid-lang/commit/cfb131d), [`6f04db5`](https://github.com/Micrurus-Ai/Corvid-lang/commit/6f04db5), [`5e1b864`](https://github.com/Micrurus-Ai/Corvid-lang/commit/5e1b864)). Both prompt-bridge and entry-agent struct-return paths now lower natively (Int/Bool/Float/String field types). See [`ROADMAP.md:1409`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md#L1409). |
| L-4: WASM target rejects String parameters | Open (roadmap) | **CLOSED — maintainer audit stale** | Shipped under Phase 20n slice **20n-B** on 2026-05-08, across six commits ([`9e00719`](https://github.com/Micrurus-Ai/Corvid-lang/commit/9e00719), [`bf7d55f`](https://github.com/Micrurus-Ai/Corvid-lang/commit/bf7d55f), [`6bfc7ae`](https://github.com/Micrurus-Ai/Corvid-lang/commit/6bfc7ae), [`8da006e`](https://github.com/Micrurus-Ai/Corvid-lang/commit/8da006e), [`231c88c`](https://github.com/Micrurus-Ai/Corvid-lang/commit/231c88c), [`14ffb07`](https://github.com/Micrurus-Ai/Corvid-lang/commit/14ffb07)). Bare `(ptr, len)` UTF-8 ABI across codegen, JS loader, .d.ts emitter, manifest; multi-value `(result i32 i32)` returns; 200-iteration churn test. See [`ROADMAP.md:1408`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md#L1408). |
| L-5: Auto-dispatch has no fallback on native failure | FIXED | **CONFIRMED-FIXED** | Lazy interpreter-fallback in [`crates/corvid-driver/src/run.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-driver/src/run.rs) — emits `↻ running via interpreter:` notice when native staticlib is missing. |
| L-6: ANSI color leaks to non-TTY output | FIXED | **CONFIRMED-FIXED** | [`crates/corvid-driver/src/render.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-driver/src/render.rs) checks both `NO_COLOR` env var AND `std::io::IsTerminal`; strips ANSI when output is not a terminal. |
| L-7: No backslash line continuation | Deliberate design choice | **CLOSED — design choice reversed** | Original deferral was real (recorded in `learnings.md`), but Phase 20n slice **20n-A** explicitly reversed the design decision and shipped support on 2026-05-08 at [`eb4a962`](https://github.com/Micrurus-Ai/Corvid-lang/commit/eb4a962). Lexer at [`crates/corvid-syntax/src/lexer.rs:331-332`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-syntax/src/lexer.rs#L331-L332) consumes `\` + newline + leading whitespace as silent continuation outside strings AND inside `"..."` literals. Triple-quoted blocks unchanged. Diagnostic for `\` not at end-of-line. See [`docs/reference/lexer-rules.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/lexer-rules.md) "Continuation rules" paragraph. |
| L-8: `approve` naming rule undocumented | docs updated | **CONFIRMED-FIXED** | Compiler accepts BOTH PascalCase and snake_case forms matching the tool name (case-normalised match in [`crates/corvid-types/src/checker/case.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-types/src/checker/case.rs)); docs at [`docs/book/08-approve.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/book/08-approve.md) explain both forms with PascalCase recommended as convention. |

---

## What changed since `259dd59` / `ae8e3dd`

The shape of the staleness:

- **Phase 20l / 20m** — the closures your report already credits
  (L-1, L-2, L-5, L-6, L-8). No surprise here.
- **Phase 20n** — three additional slices the report doesn't
  mention because they hadn't shipped yet at your test cutoff:
  - **20n-A** (L-7): the deferral reasoning in `learnings.md`
    was real, but a design-override directive on 2026-05-08
    reversed it and the lexer now does silent
    backslash-continuation outside strings and inside `"..."`
    literals (triple-quoted blocks unchanged). Per the
    project's "design-override" pattern (recorded in
    [`memory/project_phase_20n_closed.md`](https://github.com/Micrurus-Ai/Corvid-lang/tree/main/memory)),
    when a deferral is reversed we document the directive
    explicitly so future audits don't mistake it for drift.
  - **20n-B** (L-4): bare `(ptr, len)` UTF-8 string ABI across
    codegen-wasm + JS loader + `.d.ts` emitter + manifest.
    Multi-value returns, content-keyed deduplicated string
    pool, 200-iteration churn test pinning page count.
  - **20n-C** (L-3): both prompt-bridge and entry-agent
    struct-return paths lifted; new `corvid-prompt-format`
    crate extracted to keep JSON Schema generation usable from
    codegen without dragging the interpreter in; generic JSON
    parse/build primitives in the runtime; per-struct
    decoders/encoders cached by `DefId`. Field-type coverage
    v1 is Int / Bool / Float / String (mirrors the four scalar
    prompt bridges).

The unifying root cause is the same one we hit with
[OPEN-GAP-PROMPTS.md](https://github.com/Micrurus-Ai/corvid-installer/blob/main/OPEN-GAP-PROMPTS.md):
no auto-sync of audit-document state from Corvid-lang. Under
Option A, the closing-loop for both files should be the same:
when a Corvid-lang slice closes a corvid-installer-tracked gap,
the closing commit message should call it out by gap ID, and
either you sweep both audit docs periodically or we send sweep
PRs against them. Same offer as before — no preference on which
direction drives close-out.

---

## What this does NOT cover

Three gaps the maintainer's report explicitly says are tracked
in `LIVE-TEST-GAPS.md` (the install/release-engineering
companion document) rather than here — those are out of scope
for this triage:

- Gap #1 (vendor_std path) — shipped at
  [`7b92e90`](https://github.com/Micrurus-Ai/Corvid-lang/commit/7b92e90).
- Gap #2 (`corvid check` not validating imports) — same root
  cause as L-1; shipped at
  [`bfe6232`](https://github.com/Micrurus-Ai/Corvid-lang/commit/bfe6232).
- Gap #3 (Windows code-signing) — open, filed as ROADMAP slice
  [33P7](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md).

---

## Next round

The remaining ask from the maintainer's reply is the
release-matrix reconciliation (FOLLOWUPS.md item). Status:
both drift items shipped today —
- [`eb12802`](https://github.com/Micrurus-Ai/Corvid-lang/commit/eb12802):
  `aarch64-unknown-linux-gnu` added to release matrix.
- [`e8b7344`](https://github.com/Micrurus-Ai/Corvid-lang/commit/e8b7344):
  `aarch64-pc-windows-msvc` added to release matrix.

The next release run from
[`.github/workflows/release.yml`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/.github/workflows/release.yml)
will produce six prebuilt archives instead of four, fully
matching what your install scripts ask for.

With that, every actionable item from your maintainer reply
is closed at HEAD. Re-run the LIVE-TEST Gap #1 reproducer when
you can and the auto-sync round-trip will close out under
Option A.
