# Syntax basics

## File structure

A Corvid file is a sequence of declarations. There is no `main()`
function — the entry point is whichever `agent` you run.

```corvid
effect ...        # named effect with dimensions
tool ...          # side-effecting function
prompt ...        # LLM-backed function
agent ...         # composable program entry
fn ...            # pure function
struct ...        # record type
import ...        # bring in another module
```

## Comments

```corvid
# single-line comment

#: doc comment for the next declaration; renders in --help and LSP hover
```

## Identifiers and types

Identifiers are snake_case. Type names are PascalCase. Effect names are
snake_case (they look like values, not types — they ARE values).

Built-in types: `Int`, `Float`, `Bool`, `String`, `List<T>`, `Map<K,V>`,
`Option<T>`, `Result<T,E>`, `Grounded<T>`.

## Declarations

### `fn` — pure function

```corvid
fn double(x: Int) -> Int:
    return x * 2
```

Pure functions have no effect row. They cannot call prompts, tools, or
agents.

### `prompt` — LLM-backed function

```corvid
prompt summarize(text: String) -> String uses llm_effect:
    "Summarize: " + text
```

The body is the prompt template. Variables interpolate. The return
type tells the compiler what to decode the model output as. Decoding
failure is a typed error, not a panic.

### `tool` — side-effecting function

```corvid
tool send_email(to: String, body: String) -> Unit uses email_effect:
    @host.email.send(to, body)
```

The `@host.x.y` syntax is the FFI call to the runtime's host
interface. The runtime resolves this to the configured connector at
deploy time.

### `agent` — composable program entry

```corvid
@budget($0.50)
agent main(input: String) -> String:
    ...
```

Agents compose prompts and tools. They have effect rows that the
compiler infers from their bodies (you can also write them
explicitly). Agents can call other agents.

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

```corvid
if condition:
    # ...
else:
    # ...

while predicate:
    # ...

for x in xs:
    # ...

match value:
    Some(x) -> x
    None -> 0
```

## Expressions

Strings concatenate with `+`. Numeric ops are conventional. Boolean ops
are `and`, `or`, `not`.

Method-call syntax (`x.method()`) works on any type. Postfix `?` on a
`Result<T,E>` propagates the error.

## Effect rows in signatures

```corvid
prompt foo() -> String uses llm_effect, retrieval_effect:
    ...
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
