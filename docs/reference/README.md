# Reference

Lookup material. Skim a page when you need a specific name, rule,
shape, or invariant. Read top-to-bottom rarely.

## Language

- [Grammar](grammar.md) — formal EBNF derived from
  `crates/corvid-syntax/src/parser/`.
- [Lexer rules](lexer-rules.md) — line continuation, brackets,
  triple-quoted strings.
- [Core semantics](core-semantics.md) — registry-derived spec.
- [Functions](functions.md) — every builtin method and stdlib call.
- [Text and encoding](text-and-encoding.md) — base64, hex and URL
  encoding, and UUIDs.
- [Exact decimals](decimal.md) — the `Decimal` type for money and
  other exact quantities.

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
