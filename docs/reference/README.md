# Reference

Lookup material. Skim a page when you need a specific name, rule,
shape, or invariant. Read top-to-bottom rarely.

## Language

- [Grammar](grammar.md) — formal EBNF derived from
  `crates/corvid-syntax/src/parser/`.
- [Lexer rules](lexer-rules.md) — line continuation, brackets,
  triple-quoted strings.
- [Core semantics](core-semantics.md) — registry-derived spec.

## CLI

- [CLI reference](cli.md) — every `corvid` command, with examples.

## Compile-time guarantees

- [Compile-time guarantees](guarantees.md) — the registry of
  properties the compiler enforces.
- [Inventions](inventions.md) — full proof matrix per invention
  (shipped status, runnable command, test refs, spec links).
- [Inventions tour](inventions-tour.md) — short index with
  `corvid tour --topic <name>` pointers.

## Standard library

- [Standard library](stdlib/) — the `std/*` modules.
- [Stdlib connectors](stdlib/connectors/) — per-connector reference
  (Gmail, MS365, Slack, Calendar, Tasks, Files).

## Examples

- [Reference apps](reference-apps.md) — canonical example programs.

## Packaging

- [Package imports](package-imports.md) — package manager flow.
