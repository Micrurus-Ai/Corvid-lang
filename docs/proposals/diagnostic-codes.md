# Diagnostic codes declared by their errors

Status: proposed, 2026-11-26 (macOS session). Roadmap: the diagnostic-code item under test infrastructure in [the backend roadmap](../backend-ai-roadmap.md).

## The weakness

A diagnostic's code (`[E0101] error: …`) is found by matching words in its
message (`detect_error_code` in `corvid-driver/src/render.rs`). Twenty or so
phrases have codes. Every other error prints `E0000`, which covers most of the
135 checker error kinds and every newer one: durations, decimals, tables,
actions. Rewording a message can silently change or lose its code. Some
errors write a code into their message text instead (`E5204`), which the
matcher does not know. Codes are what people search and what tools filter
on, so a code that depends on wording cannot be relied on.

## Design

1. **Each error kind declares its code.** `TypeErrorKind::code()` is an
   exhaustive `match`, so adding a kind without a code fails to compile.
   Lexer, parser, resolver and effect errors do the same through their own
   kinds.
2. **Existing codes keep their meaning.** Every code a message mapped to
   before (`E0101`, `E0201`–`E0209`, `E0250`, `W0251`, `E0001`–`E0003`,
   `E0051`–`E0054`, `E0301`, `E0302`) stays on the kind that produced it. A
   code written into a message (`E5204`) moves into `code()`.
3. **New codes by area**, in ranges the reference lists: `E01xx` effects
   and approval, `E02xx` types, `E03xx` names, `E04xx` JSON and records,
   `E05xx` scalars (`Decimal`, time, currency, bytes), `E06xx` tables, `E07xx`
   actions and routes, `E08xx` connectors, `E09xx` streams and agents,
   `E52xx` identity and access, as AGENTS.md already uses.
4. **`Diagnostic` carries its code** from the source that made it. The
   renderer prints it and never infers one, and `detect_error_code` is
   removed.
5. **One test** refuses two kinds sharing a code, and **an error reference**
   (`docs/reference/errors.md`) lists every code with its message and hint,
   generated from the kinds and held by a drift test.

## Alternatives

- **Keep matching messages and add phrases.** This is what failed: it stays
  incomplete and couples codes to wording.
- **Number kinds by declaration order.** Codes would change whenever a kind
  is inserted.
- **Derive codes from kind names** (`E-DurationLiteralInvalid`). Stable,
  but long and unsearchable next to the existing `E0101` style.

## Three tiers

- **Backend and CLI:** stable codes in `corvid check` output and JSON
  diagnostics.
- **Frontend:** the editor integration and the browser build's error overlay
  show and link the same code.
- **Assistant:** a coding AI can look a code up in the reference, or filter a
  run's diagnostics by area, without parsing English.

## Slices

1. `TypeErrorKind::code()` with every existing code preserved, `Diagnostic`
   carrying it, and the duplicate-code test.
2. Lexer, parser, resolver and effect error codes, and the removal of
   `detect_error_code`.
3. The generated error reference and its drift test.

## Non-scope

- A `corvid explain <code>` command, until the reference exists to drive
  it.
