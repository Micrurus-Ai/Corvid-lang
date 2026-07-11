# Syntax basics

Every fenced code block in this chapter marked `corvid` compiles
through the real driver in CI
(`crates/corvid-driver/tests/book_snippets_compile.rs`). Blocks
marked as **Planned** show syntax that is designed but not yet
implemented — each names the roadmap slice that ships it.

## File structure

A Corvid file is a sequence of declarations. There is no `main()`
function — the entry point is whichever `agent` you run.

```corvid-fragment
effect ...        # named effect with dimensions
tool ...          # side-effecting function (signature-only; see below)
prompt ...        # LLM-backed function
agent ...         # composable program entry
type ...          # record type
import ...        # bring in another module
```

> **Planned — `fn` pure-function declarations land in slice 45r.**
> Until then, an effect-free `agent` is the pure-function shape.

## Comments

```corvid-fragment
# single-line comment
```

> **Planned — `#:` doc comments (rendered in `--help` and LSP hover)
> land with slice 45q.** Today `#:` lexes as an ordinary comment.

## Identifiers and types

Identifiers are snake_case. Type names are PascalCase. Effect names are
snake_case (they look like values, not types — they ARE values).

Built-in types: `Int`, `Float`, `Bool`, `String`, `Nothing`,
`List<T>`, `Stream<T>`, `Option<T>`, `Result<T,E>`, `Grounded<T>`.

> **Planned — `Map<K,V>` lands in slice 45g.**

## Declarations

### `prompt` — LLM-backed function

```corvid
effect llm_effect:
    cost: $0.01
    latency: medium
    confidence: 0.9

prompt summarize(text: String) -> String uses llm_effect:
    "Summarize: {text}"
```

The body is a single prompt template string. Parameters interpolate
with `{param}`. The return type tells the compiler what to decode the
model output as. Decoding failure is a typed error, not a panic.

### `tool` — side-effecting function

```corvid
effect email_effect:
    cost: $0.001
    reversible: false

tool send_email(to: String, body: String) -> Nothing uses email_effect
```

Tools are signature-only declarations: the signature carries the type
contract and the effect row; the implementation is provided by the
host through the runtime's registered-tool dispatch. Three ways to
provide one:

1. **Executing stdlib tools** — `std/io`, `std/http`, `std/db`, and
   `std/json` tools are implemented inside the runtime itself (real
   Rust: reqwest, rusqlite, serde). No host code at all.
2. **Rust FFI** — a `#[tool]`-annotated Rust function compiled into a
   signed cdylib and loaded by the runtime.
3. **Python host tools** — a matching function in the project's
   `tools.py`, executed via the embedded Python runtime.

In every case the effect row is enforced by the compiler at the call
site and by the runtime at dispatch — the implementation cannot
escape the declared contract.

### `agent` — composable program entry

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

prompt condense(text: String) -> String uses llm_call:
    "Condense to one sentence: {text}"

@budget($0.50)
agent main(input: String) -> String:
    return condense(input)
```

Agents compose prompts and tools. They have effect rows that the
compiler infers from their bodies (you can also write them
explicitly). Agents can call other agents. Dimensional annotations
like `@budget($0.50)` bound the whole subtree of calls beneath the
agent.

### `effect` — named effect declaration

```corvid
effect refund_effect:
    cost: $50.00
    trust: supervisor_required
    reversible: false
    data: external_action
```

Effects are values you compose by name into prompts, tools, and agents
via `uses`. See **[Effects](/docs/effects)** for the full dimension
catalog.

## Control flow

Shipped today: `if`/`else`, `for … in` over lists, strings, and
streams, with `break` and `continue`, plus early `return`.

```corvid
agent count_positives(xs: List<Int>) -> Int:
    total = 0
    for x in xs:
        if x > 0:
            total = total + 1
    return total
```

`while` re-evaluates its condition before every iteration;
`break` exits the innermost loop, `continue` skips to its next
iteration, and both are compile errors outside a loop (compiled
in CI):

```corvid
agent drain(limit: Int) -> Int:
    n = 0
    total = 0
    while n < limit:
        n = n + 1
        if n % 2 == 0:
            continue
        if n > 7:
            break
        total = total + n
    return total
```

`match` gives multi-way branching over sums, `Option`, `Result`,
and literals — see [Pattern matching](./13-pattern-matching.md).

> **Planned — `elif` chaining lands in slice 45q.** Until then,
> chained conditions nest `if`/`else`.

## Expressions

Strings concatenate with `+` (both sides must be `String` — see the
conversion note in **[Types](/docs/types)**). Numeric ops are
conventional (`+ - * / %`, checked). Boolean ops are `and`, `or`,
`not`. Postfix `?` on a `Result<T,E>` or `Option<T>` propagates the
error/none to the caller.

Method-call syntax (`x.method()`) works on user-declared types via
`extend` blocks and on built-in types via the builtin-method table
(shipped in 45c with `s.length()`; assignment targets like
`x.field = v` and compound `+=` shipped in 45b).

> **Planned — the string/list method batches (`s.split(…)`,
> `xs.append(…)`) land with slices 45d/45f.**

## Effect rows in signatures

```corvid
effect llm_effect:
    cost: $0.01
    latency: medium
    confidence: 0.9

effect retrieval_effect:
    cost: $0.001
    latency: low

prompt answer(question: String) -> String uses llm_effect, retrieval_effect:
    "Answer using retrieved context: {question}"
```

Effect rows are sets, not tuples. Order doesn't matter. You can union
effects across calls; the compiler computes the closure.

## Types in depth

The full formal grammar (EBNF derived from the parser) is on the
**[Grammar](/docs/grammar)** page.
[`docs/reference/lexer-rules.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/lexer-rules.md)
documents the lexer's continuation rules (backslash, brackets, triple-
quoted strings).
The typing rules are in
[`docs/internals/effect-spec/03-typing-rules.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/internals/effect-spec/03-typing-rules.md).
