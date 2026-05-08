# Phase 20m — Verifier-Driven Corrections

Closes the two real corrections surfaced by re-testing Phase 20l's
fix set against the same external-reviewer methodology that produced
the original 8-gap report. The verifier scorecard confirmed five of
eight gaps verbatim, found two with wrong details (L-6 and L-8),
and one with right diagnosis but wrong root-cause framing (L-5).
L-6's actual fix already landed under 20l-C, so 20m only needs to
address L-5 and L-8.

The phase exists not just for the corrections themselves but to
institutionalise the verifier-correction pattern. Future
external-reviewer rounds (there will be at least one more before
33M opens) will follow the same shape: report → first-round fixes
→ verification round → corrections. Documenting the pattern makes
each round cheaper than the last.

## What the verifier confirmed

| Gap | Verdict | Action |
|---|---|---|
| L-1 corvid check imports | confirmed verbatim | 20l-A shipped, no further work |
| L-2 Python codegen object collapse | confirmed verbatim | 20l-B shipped, no further work |
| L-3 native struct prompt returns | confirmed; broader scope (entry-agent boundary) | filed against Phase 17/20 follow-up |
| L-4 WASM String params | confirmed verbatim | filed against Phase 23 follow-up |
| L-5 corvid run picks native | reframed | **20m-B** — auto-fallback when native fails |
| L-6 ANSI color leak / NO_COLOR | confirmed; original "NO_COLOR works" claim was retroactively wrong | 20l-C already covers both `is_terminal()` AND `NO_COLOR` honoring; no further work |
| L-7 backslash line continuation | confirmed verbatim | deferred under 20l-F (workarounds suffice; identity argument) |
| L-8 approve must be PascalCase | overly strict | **20m-A** — both PascalCase and snake_case accepted |

## My own meta-error

I shipped 20l-E with a logged learning that read: *"When verifying a
docs-only fix, double-check claimed grammar against the parser
before the spec ships."* Then I immediately violated it.

I checked `expected_approve_label: pascal_case(tool_name)` in
`crates/corvid-types/src/{approval_reachability.rs, checker/call.rs,
checker/import_call.rs}` and concluded "labels are PascalCase." The
field name `expected_approve_label` and its `pascal_case(tool_name)`
value are the **suggested form for the diagnostic hint** — what the
checker tells the user to write in `add 'approve PascalCase(...)'`
help text. They are NOT the **acceptance criterion** for an
approve label.

The acceptance criterion lives at the comparison site, not the
suggestion field. `crates/corvid-types/src/checker/call.rs:127`:

```rust
.any(|a| snake_case(&a.label) == tool_name && a.arity == args.len());
```

The user's label is normalised to snake_case and compared against
the tool name (which is already snake_case). Both `approve
SendEmail(...)` and `approve send_email(...)` accept because both
normalise to `send_email`.

The lesson worth carrying: **`expected_*` fields are diagnostic
suggestions, not acceptance criteria. To find acceptance, find
the comparison site.** Add it to `learnings.md` under the 20m
entry.

## Sequencing rules

Per CLAUDE.md "When splitting" — unchanged from 20j/20k/20l:

- One commit per fix.
- Validation gate between every commit:
  - `cargo check --workspace` (zero new errors)
  - `cargo test -p <crate-modified>` (lib + targeted tests green)
  - `cargo run -q -p corvid-cli -- verify --corpus tests/corpus`
    (capture exit with `> file 2>&1; echo exit=$?`; established
    Windows whoami baseline is exit 2)
- Push before starting the next slice.
- Pre-phase chat per slice; no autonomous chaining.
- Zero unrelated changes during a fix commit.
- Commit message: `<type>(<crate>): <imperative summary>` — body
  cites slice id (20m-A or 20m-B), reproduction, root cause, fix,
  validation commands run.

## Slices

### 20m-A — Correct `approve` naming docs (L-8 v2)

**Reproduction.**

```corvid
tool send_email(to: String) -> Nothing dangerous

agent notify(to: String) -> Nothing:
    approve send_email(to)            # accepted — checker normalises
    return send_email(to)
```

The `corvid check` of the program above passes. My current §6.1
of `docs/effects-spec/03-typing-rules.md` says "must be the
PascalCase form." That's wrong.

**Verified site.**
`crates/corvid-types/src/checker/call.rs:127` —
`snake_case(&a.label) == tool_name`. The `snake_case` helper at
`crates/corvid-types/src/approval_reachability.rs:353` lower-cases
each character and inserts an underscore before each uppercase
letter (except at index 0), so any reasonable casing of a
PascalCase or snake_case identifier round-trips.

**Fix.**

1. `docs/effects-spec/03-typing-rules.md` §6.1 — replace "must be
   the PascalCase form" with "may be either the PascalCase form or
   the snake_case form" and explain the comparison rule. Preserve
   the greppability story by phrasing it as "every authorised call
   site is greppable per-tool by its snake_case form (or its
   PascalCase form, since they're equivalent under the comparison
   rule)."
2. `crates/corvid-cli/src/tour.rs` — soften the `approve-gates`
   pitch: keep the example `approve IssueRefund(id)` but mention
   that the snake_case form is equivalent.
3. `docs/site/site.js` — mirror the tour pitch update.

**Acceptance.** A new user reading the typing-rules spec or the
tour learns both forms are accepted and the comparison rule
underpinning that. No claim contradicts the checker.

**Estimated commits:** 1.

### 20m-B — Auto-fall-back to interpreter on native link failure (L-5 v2)

**Reproduction.**

```corvid
agent compute(x: Int, y: Int) -> Int:
    return x * y + 5

agent main() -> Int:
    return compute(6, 7)
```

Today on a binary-install machine with no `corvid_runtime.lib` on
disk, `corvid run src/arith.cor` (auto target) fails with the
multi-line diagnostic from 20l-D. The diagnostic is informative,
but the user has to copy-paste either `corvid run --target=interpreter`
or `cargo build -p corvid-runtime --release` to recover. For the
common case where the program is interpreter-runnable, the
runtime should just retry transparently.

**Verified site.**
`crates/corvid-driver/src/run.rs:162–171` — the `RunTarget::Auto`
arm calls `run_via_native_tier` first when the eligibility scan
passes; if the native build fails (link error, missing staticlib,
codegen rejection) the error propagates straight to the user
without falling back. The Auto branch's `Err(reason)` arm only
fires when the *eligibility scan* says native is unavailable; it
doesn't fire when the native build itself fails downstream.

**Fix.**

Wrap the `run_via_native_tier(...)` call inside the Auto arm so
that a returned error matching the missing-staticlib signature
(e.g. an error string contains `corvid-runtime staticlib missing`
from `missing_staticlib_diagnostic`) emits the existing UX
prefix `↻ running via interpreter: native staticlib unavailable`
and proceeds with `run_via_interpreter_tier(path, &ir)`. The
20l-D diagnostic stays as the explicit-`--target=native` error
message for users who opted into native and need to recover by
hand.

Approximate diff at `crates/corvid-driver/src/run.rs:162`:

```rust
RunTarget::Auto => match &scan {
    Ok(()) => match run_via_native_tier(path, &source, &ir, tools_lib) {
        Ok(code) => Ok(code),
        Err(err) if is_missing_staticlib_error(&err) => {
            eprintln!("↻ running via interpreter: native staticlib unavailable");
            run_via_interpreter_tier(path, &ir)
        }
        Err(err) => Err(err),
    },
    // … existing arms unchanged
}
```

`is_missing_staticlib_error` is a small helper that string-matches
on `"corvid-runtime staticlib missing"` (the canonical phrase from
`missing_staticlib_diagnostic`). String-matching the diagnostic is
acceptable here because the phrase is stable, owned by the same
crate, and the unit test in 20l-D already pins the wording.

Other native build failures (linker errors unrelated to the
staticlib, codegen rejections) keep propagating as before — those
are real bugs the user should see.

**Regression test.** Add a new test next to
`run_with_target_auto_uses_native_for_pure_program` in
`crates/corvid-driver/src/tests.rs` that exercises the
staticlib-missing-but-falls-back path. The test sets
`CORVID_RUNTIME_STATICLIB_OVERRIDE` to a non-existent path so the
override branch fires, then asserts that `run_with_target` with
`RunTarget::Auto` returns `Ok(_)` (the interpreter ran the
program) rather than `Err(_)` (the native path bailed without
recovery). Pin the stderr capture to assert the `↻ running via
interpreter` prefix appears.

**Acceptance.**

- Pure-arithmetic programs Just Work via `corvid run` on
  binary-install machines without a staticlib, automatically.
- Explicit `corvid run --target=native` still surfaces the
  20l-D actionable diagnostic — no UX regression for users who
  opted into native.
- Existing `run_with_target_auto_uses_native_for_pure_program`
  test continues to pass under the existing whoami baseline.

**Estimated commits:** 1.

## Phase-done checklist

- [ ] 20m-A `approve` naming docs corrected — landed with the
  spec section update, tour blurb update, and site-mirror update.
- [ ] 20m-B native-link auto-fallback — landed with the regression
  test that exercises the staticlib-missing path.
- [ ] Closing audit appended to this document with per-slice status,
  the meta-lesson about `expected_*` diagnostic fields versus
  acceptance criteria, and the verifier-correction pattern
  documented for future external-reviewer rounds.
- [ ] `learnings.md` updated with the meta-lesson — *"`expected_*`
  fields are diagnostic suggestions, not acceptance criteria. To
  find acceptance, find the comparison site."* Plus a brief note
  on the verifier-correction pattern.
- [ ] ROADMAP.md Phase 20m entry checkboxes ticked, `✅ closed`
  marker added to the heading.
- [ ] Memory record
  `C:\Users\SBW\.claude\projects\c--Users-SBW-OneDrive---Axon-Group-Documents-GitHub-corvid\memory\project_phase_20m_closed.md`
  written with three patterns:
  (a) the verifier-correction pattern (institutionalise it; each
      round is cheaper than the last);
  (b) the `expected_*` confusion (verify the comparison site, not
      the suggestion field);
  (c) the auto-fallback UX preference (`↻ running via interpreter:
      …`) over actionable-error UX when the recovery path is
      mechanical and the user wasn't asking for native specifically.
  Add a one-liner to MEMORY.md.

## Sequencing reminder

Per CLAUDE.md "pre-phase chat mandatory" and "no autonomous
chaining": each slice gets its own pre-phase confirmation before
any code edits. Refactor commits land sequentially with push
between, never batched.

The recommended order: **A → B**. A is a pure docs fix; B is the
substantive code change that tightens the user's first-run UX on
binary-install machines.

## Out of scope for this phase

The L-6 verification surfaced a separate emitter:
`crates/corvid-repl/src/lib.rs` has 20+ hardcoded
`\x1b[1m...\x1b[0m` ANSI escape sequences with no `NO_COLOR` or
`is_terminal()` check. Same shape as the L-6 fix the renderer
already got, but in a different module the original verifier
didn't test (they used `corvid check / build / run`, not
`corvid repl`).

Filed for a future REPL-touching slice rather than rolled into
20m. The 20l/20m scope is verifier-confirmed gaps only — pulling
in adjacent emitters under the same phase risks the kind of
scope-creep the slice gate is designed to prevent.
