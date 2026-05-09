# Corvid syntax — surface rules

This document covers the lexer-and-parser-level shape of Corvid
source code. For type system rules, see
[`docs/internals/effect-spec/`](./effects-spec/).

## Continuation rules

Corvid uses **physical-line lexing** with the structural
`Indent`, `Dedent`, and `Newline` tokens you'd expect from a
Python-shaped grammar. A logical line ends at a physical newline
unless one of the following continuation rules applies.

### Bracket-grouped continuation

Newlines inside `(`, `[`, or any future bracket form are absorbed
silently. The most common form for long signatures and
expressions:

```corvid
agent compose(
    first: String,
    second: String,
    third: String,
) -> String:
    return first + second + third
```

This is the idiomatic way to span a logical line across multiple
physical lines in Corvid. Use it whenever bracket grouping is
already structurally appropriate.

### Backslash line continuation

A backslash `\` immediately followed by a newline (`\n` or
CRLF `\r\n`) is consumed silently. Any leading whitespace on the
next physical line is also consumed, joining the two physical
lines into one logical line. This works in two contexts:

**Outside any string**, e.g. between adjacent tokens or
expressions:

```corvid
agent main() -> Bool: \
    return true
```

is lexically equivalent to

```corvid
agent main() -> Bool: return true
```

No structural `Newline` or `Indent` is emitted at the join.

**Inside a `"..."` single-quoted string literal**, the backslash
plus newline plus leading whitespace is consumed, joining the two
physical lines into one logical string:

```corvid
return "first part \
        second part"
```

lexes as the string `"first part second part"` (note the leading
whitespace on the second line is consumed; if you need
preserved whitespace, write the spaces *before* the backslash on
line 1).

**Triple-quoted `"""..."""` strings are NOT rewritten.** Triple-
quoted strings already span lines naturally; the backslash-
newline-leading-whitespace rewriting does not apply inside
them. A triple-quoted string preserves exactly the bytes between
its delimiters, with the existing escape-sequence rules
(`\n`, `\t`, `\r`, `\\`, `\"`, `\0`) intact.

### Backslash not at end of line

A backslash followed by anything other than a newline is still
an error at top level (`E0003: unexpected character '\\'`).
Inside a string literal, the existing escape-sequence rules
apply — `\n` is the literal newline byte, `\t` is the literal
tab, etc.

### Choosing between continuation forms

For multi-line **signatures**, **expressions**, and **collection
literals**, prefer bracket-grouped continuation — the brackets
are already structurally appropriate.

For multi-line **string literals**, prefer triple-quoted
`"""..."""` blocks. They preserve every byte exactly and don't
interact with the indentation tracker.

Reach for `\` line continuation only when neither of the above
applies — typically when you want to break a long single-line
expression that doesn't already have brackets, or when you want
to stitch two adjacent string literals together across a line
boundary.
