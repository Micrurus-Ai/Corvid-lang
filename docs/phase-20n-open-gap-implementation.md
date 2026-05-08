# Phase 20n — Open-Gap Implementation

Closes the three open language gaps L-3, L-4, and L-7 surfaced by
the original external-reviewer report and re-confirmed by the
verification round in 20m. Phase 20l deferred L-7 on
language-identity grounds; 20m deferred L-3 and L-4 as feature
work for their owning phases. Phase 20n reverses both deferrals
under a 2026-05-08 directive from the language designer:
**implement all three end-to-end as a dedicated phase**.

## Design override — L-7 reversal

The 20l-F deferral note read:

> Rejecting Python-mimicry features when the language identity
> argument outweighs the ergonomic argument is itself a learning,
> not a TODO.

That deferral is **reversed for 20n-A**. The directive: implement
the feature end-to-end (Decision A from the open-gap prompt set).
Triple-quoted-string semantics stay unchanged; backslash line
continuation now works outside strings and inside `"..."` literals.

The reversal is recorded here, not silently absorbed, because:

1. CLAUDE.md "no aspirational vocabulary" applies in both
   directions — past learnings that get overridden need explicit
   reversal markers, otherwise future sessions can't tell the
   difference between drift and decision.
2. The 20l-F learnings entry stays in `learnings.md` as the
   *original* design rationale; the 20n closing learnings entry
   adds the *reversal* rationale alongside it. Both stand as
   record.

## What the verifier confirmed

| Gap | Verifier verdict | 20n action |
|---|---|---|
| L-3 native codegen struct returns | confirmed; broader scope than original report (entry-agent boundary too) | **20n-C** — implement at both sites |
| L-4 WASM String params/returns | confirmed verbatim | **20n-B** — implement bare `(ptr, len)` ABI |
| L-7 lexer line continuation | confirmed verbatim | **20n-A** — implement (override 20l-F deferral) |

## Sequencing rules

Per CLAUDE.md "When splitting" — unchanged from 20j/20k/20l/20m:

- One slice = one feature; one or more commits per slice.
- Validation gate between every commit:
  - `cargo check --workspace` (zero new errors)
  - `cargo test -p <crate-modified>` (lib + targeted tests green)
  - `cargo run -q -p corvid-cli -- verify --corpus tests/corpus`
    (capture exit with `> file 2>&1; echo exit=$?`; established
    Windows whoami baseline is exit 2)
- Push before starting the next slice.
- Pre-phase chat per slice; no autonomous chaining.
- **Step-0 audit required** for 20n-B and 20n-C before any code:
  these are feature slices, bigger than typical fix slices, and
  the 20l/20m discipline calls for reading-and-planning before
  drafting fixes.

## Slice 20n-A — L-7 lexer line continuation (Decision A)

### Goal

`\` followed immediately by a newline (and any leading whitespace
on the next line) is consumed silently:

- **Outside strings**: between adjacent tokens / expressions,
  acting as a silent continuation that suppresses the newline.
- **Inside `"..."` single-quoted strings**: the `\` + newline
  pair (plus leading whitespace) is consumed; the result is the
  same string as if it were written on one line.
- **Inside `"""..."""` triple-quoted blocks**: behavior unchanged.
  Triple-quoted strings already span lines naturally; no
  continuation rewriting applies.
- A `\` not at end-of-line (e.g., before a non-newline character)
  remains an `E0003` error, with the existing `consume_escape`
  handling for inside-string sequences (`\n`, `\t`, etc.) intact.

### Files to touch

- `crates/corvid-syntax/src/lexer.rs` — `lex_string` and the
  top-level dispatcher
- New tests covering the four cases (inside-string continuation,
  outside-string continuation, `\` followed by non-newline
  rejected, triple-quoted unchanged)
- `docs/syntax.md` (new) — "Continuation rules" paragraph
- `learnings.md` — entry capturing the design-reversal context

### Acceptance criteria

1. Both "first part " \\ "second part" reproductions from the
   gap report compile under `corvid check`.
2. New regression tests cover the four cases.
3. Existing tests remain green.
4. `docs/syntax.md` includes a "Continuation rules" paragraph
   describing the behavior.

### Estimated commits

1.

## Slice 20n-B — L-4 WASM String parameter and return support

### Step-0 audit (before code)

Before drafting the fix, read `crates/corvid-codegen-wasm/` end-to-
end and confirm:

- Whether `corvid-runtime-wasm` exists, and whether it already
  exports `corvid_alloc` / `corvid_free` or anything equivalent.
- How the existing Int/Float/Bool/Nothing parameters lower today.
  In particular: where the `i32` types are emitted, how the JS
  loader unwraps them, how the `.d.ts` emitter maps them.
- Whether the manifest emitter (`<file>.corvid-wasm.json`) carries
  per-parameter type descriptors today, or whether all parameters
  collapse to `i32`.

The audit produces a refined plan covering the bare `(ptr, len)`
UTF-8 ABI implementation across codegen + loader + types +
manifest. The refined plan goes back to the user for pre-phase
chat before any code.

### Goal (post-audit)

`String` parameters and return values cross the WASM boundary as
UTF-8 byte sequences with explicit length. From JS:

```js
import { shout } from './main.js';
await shout('hello'); // => 'hello'
```

…and `main.d.ts` types `shout` as `(msg: string) => Promise<string>`.

### Out of scope

- WASM Component Model adapters (use the bare ABI first).
- UTF-16 or DOM string interop.
- `Stream<String>` (separate streaming-channel feature).
- Multi-string return tuples.

## Slice 20n-C — L-3 native codegen struct returns

### Step-0 audit (before code)

Before drafting the fix, read:

- `crates/corvid-codegen-cl/src/lowering/prompt.rs` (the prompt-bridge
  rejection site) and the entry-agent lowering file (find via grep
  on the entry-boundary error string).
- `crates/corvid-runtime/` — specifically the JSON
  serializer/deserializer. The open-gap prompt explicitly flags the
  risk: "verify whether the existing encoder handles structs before
  designing the layer; if not, the slice grows to include it."
- `crates/corvid-runtime/` — the existing `Grounded<T>` heap-
  allocation pattern that L-3 should mirror.

The audit produces a refined plan covering: heap-allocation strategy,
JSON deserialization extension if needed, prompt-bridge return
plumbing, entry-agent serialization. The refined plan goes back to
the user for pre-phase chat before any code.

### Goal (post-audit)

Both reproductions compile under `--target=native` and produce a
working binary that calls the LLM via the prompt bridge, marshals
the JSON LLM response into a heap-allocated struct, and serializes
the struct (JSON to stdout) before exit.

### Out of scope

- Streaming struct returns (`Stream<Decision>`).
- Recovery from malformed LLM JSON beyond what `Grounded<T>`
  already does.
- Removing the `Grounded<T>` primitive-only restriction.
- Cross-FFI struct passing for tools.

## Phase-done checklist

- [ ] 20n-A L-7 line continuation — landed with regression tests
  and `docs/syntax.md` paragraph.
- [ ] 20n-B L-4 WASM String ABI — step-0 audit, refined plan,
  pre-phase chat, then implementation with multi-byte round-trip
  test.
- [ ] 20n-C L-3 native struct returns — step-0 audit, refined
  plan, pre-phase chat, then implementation at both codegen sites.
- [ ] Closing audit appended to this document with per-slice
  status, the design-reversal note for L-7, and any scope
  expansions discovered during step-0 audits.
- [ ] `learnings.md` updated per slice.
- [ ] ROADMAP.md Phase 20n entry checkboxes ticked, `✅ closed`
  marker added.
- [ ] Memory record
  `C:\Users\SBW\.claude\projects\c--Users-SBW-OneDrive---Axon-Group-Documents-GitHub-corvid\memory\project_phase_20n_closed.md`
  written with two patterns:
  (a) the design-override pattern — when a deferral is reversed,
      record the directive explicitly so future sessions don't
      mistake it for drift;
  (b) the step-0 audit pattern — substantive feature slices need
      a read-and-plan step before code; mid-implementation scope
      expansion is the failure mode this prevents.
  Add a one-liner to MEMORY.md.

## Sequencing reminder

Per CLAUDE.md "pre-phase chat mandatory" and "no autonomous
chaining": each slice gets its own pre-phase confirmation. 20n-A
ships first because it's the smallest and the design decision is
already authorised. 20n-B and 20n-C each need a step-0 audit
before implementation; the audit produces a refined plan that
goes back for pre-phase chat.

The recommended order: **A → B → C**. A is small and self-
contained. B is medium scope but bounded to the WASM crate +
loader. C is the largest because it spans codegen + runtime
serialization and may grow during step-0 audit.
