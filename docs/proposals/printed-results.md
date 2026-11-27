# One printed form for a program's result

Status: building, 2026-11-26 (macOS session). Roadmap: the command-line printing item in [the backend roadmap](../backend-ai-roadmap.md).

## The weakness

`corvid run` prints what the entry agent returned, and the two tiers print
it differently, so the same program's output depends on which tier ran it:

| Result | Interpreter | Native |
| --- | --- | --- |
| `String` `hello` | `"hello"` | `hello` |
| a record with `@json(key: "order-id")` | `Refund(amount: 42, order_id: "o-1")` | `{"order_id":"o-1","amount":42}` |
| `List<Int>` | `[1, 2]` | refused, so it runs on the interpreter |

Native code prints a record with its own encoder (`struct_encode.rs`), which
takes scalar fields only and ignores renamed keys. It refuses lists, options
and results, so such a program never runs natively.

## Design

A printed result is what a person or a script reading standard output needs:

1. **A `String` prints as its text**, unquoted, as `print` would.
2. **Every other scalar prints its `to_string()` text**: `42`, `2.0`, `true`,
   a `Decimal`'s or a duration's canonical text. A `Bytes` value keeps
   `Bytes(<hex>)`, the one form both tiers print today.
3. **A structured result** (a record, variant, list, option, result or map)
   **prints its public JSON form, compact, by declared type**. That is the
   JSON a route returns for the same value, with keys sorted as `to_json()`
   writes them: renamed keys, `null` for `None`, and
   `{"err":"no","tag":"err"}` for a `Result`. A script can pipe it straight
   into `jq` or the generated client's decoder.

Both tiers print through the public JSON form: the interpreter with
`tool_value_json`, native code with the embedded plan (`lower_json_encode`).
The command line stops using `struct_encode.rs`'s scalar-only encoder; the
`extern "c"` library wrapper still uses it, and is filed separately because
its output is a host ABI. Lists, options, results
and maps cross the native command line whenever the type has a native public
form, so the boundary refuses only what has none.

## Alternatives

- **The interpreter's `Display` form everywhere**
  (`Refund(amount: 42, order_id: "o-1")`). It reads well, but no tool parses
  it, it is not the form any other surface uses, and a renamed key would not
  show.
- **JSON for every result, a `String` included** (`"hello"`). This is
  consistent, but most programs that print return text, and quoting it makes
  the common case worse. A quoted scalar is still one `to_json()` away.
- **Pretty-printed JSON.** That is longer and harder to process line by line.
  Pipe the output to `jq` to read it.

## Three tiers

- **Backend:** the same JSON the route serves.
- **Frontend and generated clients:** the same decoder reads a printed result
  as a response.
- **Assistant:** reading a run's output sees the declared shape and keys, not
  an interpreter-internal rendering.

## Non-scope

- The REPL keeps its `Display` rendering, which shows type names while
  exploring.
- A command-line argument of a record type stays refused. Arguments are text,
  and the program decodes one with `from_json`.
