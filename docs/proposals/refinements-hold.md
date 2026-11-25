# Refinements hold for every value (B09)

**Status:** Refine-1 and Refine-2 shipped: the interpreter and native code check every refined value, with one message.
**Owner:** backend (Claude Code). It changes no grammar.
**Roadmap:** [B09](../backend-ai-roadmap.md), the item "refinements hold for every value", which the refined-`take` bound waits on.

## The problem

A field refinement says what values a field may hold:

```corvid
type Search:
    query: String where len_between(1, 200)
    limit: Int where between(1, 50)
```

Corvid checks it where values enter from outside: a form, a JSON body, `T.from_json`, a prompt's structured output (which is repaired until it holds). Code that builds a record is not checked:

```corvid
agent broad() -> List<Book>:
    search = Search("dune", 500)
    return Book.where(fn (b) -> b.title.contains(search.query)).take(search.limit)
```

`Search("dune", 500)` runs. So a `Search` value does not promise `limit <= 50`. Nothing that reads one can rely on its refinement: not `@budget`, not a query bound, not the application contract, which advertises `maximum: 50` to every client.

## The decision

**Every value of a refined field holds its refinement, however it was made.**

| Where a refined field gets its value | Check |
| --- | --- |
| A constructor or record literal with a constant: `Search("dune", 500)` | Compile error naming the field, the refinement and the value |
| A constructor, record literal or record update with a computed value: `Search(q, n)`, `Search { limit: n, ..s }` | Checked when the value is built |
| A field assignment: `search.limit = n` | Checked when it is assigned |
| A form, JSON body, `from_json`, prompt output | Checked as today, as a typed error or a repair |
| A row read from a table | Checked when the row is read |

A computed value that breaks the refinement **stops the run**, with the type, the field, the refinement and the value in the message:

```text
`Search.limit` must be between(1, 50), but this value is 500
```

That is how Corvid already treats a value that cannot exist: an overflowing `+` and an out-of-range index stop the run too. A value built in code comes from the program, so a bad one is a bug in the program, and stopping at the line that made it is the most useful thing to do. Values from outside, which a program cannot control, keep their typed errors: a form shows the message beside the field, `from_json` returns a `JsonError`, and a prompt is repaired.

A program that builds a refined value from a number it does not trust checks it first, with the bounds its type declares:

```corvid
agent limited(requested: Int) -> Search:
    if requested < 1 or requested > 50:
        return Search("dune", 50)
    return Search("dune", requested)
```

### What each tier gets

- **Backend.** A `Search` value always has `1 <= limit <= 50`, so `take(search.limit)` can be bounded by 50 for `@budget` (the refined-`take` item, built next), and a table never stores a value its column refinement forbids.
- **UI.** The contract's `minimum` and `maximum` describe every value the backend sends, not only the ones it receives.
- **Assistants.** A value an assistant receives holds the refinements its schema states, so an assistant can rely on the schema it reads.

### Composition with the guarantees

- **Replay.** The check is pure, so a replay stops at the same line with the same message.
- **Native code.** The checks run in native code too, with the same message, held by parity tests.
- **Budgets.** This is what makes a bound read from a refinement sound.

## Alternatives considered

| Form | Why not |
| --- | --- |
| A constructor of a refined type returns `Result<T, RefinementError>` | Every construction of a refined record becomes fallible, including the ones with literals the compiler can check. Programs would unwrap with `?` everywhere, and the error would surface far from the bug. |
| Check only at boundaries, as today | A refinement could not be trusted by anything that reads the value, which is the problem. |
| Prove each computed value in range at compile time | Needs flow-sensitive range analysis for arbitrary arithmetic; worth having for constants and simple guards later, but not a reason to leave computed values unchecked now. |
| Clamp an out-of-range value to the nearest bound | Silently changes the program's data, the kind of hidden default the rules forbid. |

## Slices

- [x] Refine-1. The interpreter: compile errors for constant values in constructors and record literals; run-time checks at constructors, record literals, record updates and field assignments; table rows checked when read.
- [x] Refine-2. Native code: the same checks and message, held by parity tests.
- [ ] Refine-3. The refined-`take` bound: `take(search.limit)` charges `@budget` the refinement's maximum.

## Non-scope

- Range proofs for computed values (the third alternative). A later slice may let the compiler skip a check it can prove, without changing what a program means.
- Refinements on other types (`Decimal` ranges, list lengths). The two forms the language has today, `between` and `len_between`, are covered.
