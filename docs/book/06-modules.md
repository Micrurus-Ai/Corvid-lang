# Modules and imports

Every fenced code block marked `corvid` compiles through the real
driver in CI. Blocks marked `corvid-fragment` are multi-file examples
verified against the live compiler (the snippet guard can't stage
sibling files, so they carry the fragment tag).

## One file = one module

Every `.cor` file is a module. Public declarations are visible to
other modules via `import`; imports name the file by relative string
path (the `.cor` extension is implicit).

## Visibility

Three levels:

- (no modifier) — private. Only the same file can see it.
- `public` — visible to any module that imports this one.
- `public(package)` — visible to other modules in the same package
  but not to consumers outside the package.

`public` applies to `type`, `tool`, `prompt`, `agent`, `effect`,
`model`, and store declarations. A `public effect` (or `public
model`) is importable via `use` — the imported effect joins your
file's effect registry, so `uses json_egress_read` composes
exactly like a locally-declared effect. Every executing stdlib
module (`std/io`, `std/http`, `std/db`, `std/json`, `std/time`,
`std/random`) exports its effect rows this way.

```corvid-fragment
# src/refund.cor
effect refund_effect:
    cost: $50.00
    trust: supervisor_required

public tool refund(amount: Float, id: String) -> String dangerous uses refund_effect
```

## Reusing a named function

A module can export a pure function with `public fn`. Both CLI targets can call it by its qualified name or by a name introduced with `use`.

```corvid-fragment
# math.cor
public fn double(n: Int) -> Int:
    return n * 2
```

```corvid-fragment
# main.cor
import "./math" as math

agent main() -> Int:
    return math.double(4)
```

This returns `8`. A pure function can call another module's public function through either import form. Calling an imported agent, prompt or tool from a pure function is a type error. Both CLI targets can also pass `math.double` to a callback parameter of type `(Int) -> Int`. On both targets, `[1, 2].map(math.double)` produces a new list containing `[2, 4]`.

A callback can keep a local integer after the function that created it returns. For example, a library can export this function.

```corvid
public agent add_by(offset: Int) -> (Int) -> Int:
    return fn (value: Int) -> value + offset
```

A caller can write `add_three = math.add_by(3)` and then `add_three(4)` to get `7`. Native callbacks currently accept integer parameters and return an integer or boolean. A callback can capture integer and boolean locals by value. It can also keep a reference to a `List<Int>` and read its length or items. If the caller updates that list, later invocations see the updated items.

```corvid
public agent reader(values: List<Int>) -> (Int) -> Int:
    return fn (i: Int) -> values[i]

agent main() -> Int:
    values = [3]
    read = reader(values)
    values[0] = 10
    return read(0)
```

This returns `10`. The callback keeps the list alive after `reader` returns.


## Importing local modules

In the editor, hover over a call such as `math.double(4)` to see its
declared signature. Go-to-definition opens the public declaration in
the local module. These actions also work when you import a function
under a different name. Type `math.` to list the module's public members.
If the module is open, navigation and member completion use its current
editor contents, including unsaved changes.

Rename a public function at its declaration or at a qualified call to update its
callers throughout the local workspace, including closed Corvid files. For
`use double as twice`, renaming the export changes `double` in the import.
Renaming `twice` changes the alias and its uses within that file.

```corvid-fragment
# src/main.cor
import "./refund" as r

agent main() -> String:
    approve Refund(50.0, "cust_123")
    return r.refund(50.0, "cust_123")
```

`import "./refund" as r` brings in `src/refund.cor` and exposes its
public declarations under the `r.` prefix. Without `as`, the prefix
is the filename (`refund.`).

## Selective import (`use`)

The `use` clause lifts specific names into scope directly — no
braces, per-item aliases with `as`:

```corvid
import "./std/io" use io_read_text, io_write_text as write

agent main() -> Result<String, String>:
    file = io_read_text("note.txt")?
    return Ok(file.contents)
```

Only the named items are bound; the rest of the module is not in
scope. `use` lifts tools, prompts, agents, and types — not effects
(see Visibility above).

## Importing from packages

Package imports use `corvid://` URIs resolved through the package
manager; the version pin lives in `corvid.toml`:

```corvid-fragment
import "corvid://json-helpers@1.0.0" as jh
```

`corvid import-summary` shows the full transitive set. Remote HTTPS
imports are hash-pinned (`hash:sha256:<digest>`).

## Python-ecosystem imports

```corvid-fragment
import python "math" as numbers effects: pure:
    fn gcd(left: Int, right: Int) -> Int
```

A Python import declaration requires a list of host effects. Add a trailing colon and indent one bodyless `fn` signature per imported function. Corvid checks argument counts, argument types and the declared result type at a call such as `numbers.gcd(18, 12)`. The import's effect list applies to each declared function. If an effect declares a cost, each call counts toward the caller's budget, including calls made through another agent. A pure Corvid `fn` cannot call a Python function. With the Python-enabled CLI, `corvid run` selects the interpreter and returns the checked result. For example, an agent returning `numbers.gcd(18, 12)` returns `6`. A Python exception fails the call with the module, function and exception details. [Python FFI guide](../guides/ffi-python.md)

## Re-export

> **Planned.** `public import … use …` re-export syntax does not
> parse today — a module cannot re-export another module's surface.
> Consumers import the defining module directly. (Direct effect and
> model EXPORT shipped in slice 45o — this note is about
> RE-exporting a surface imported from somewhere else, which is
> unscheduled.)

### Rebuilding a project offline

Install a dependency with `corvid add package @example/math@1 --registry ./registry`.
Import the exact version URI recorded in `Corvid.lock`. Keep the lockfile and
the project's `target/cache/packages` directory when moving a build offline.
With `CORVID_OFFLINE=1`, `corvid check` and `corvid build` can read locked
package source from that cache. The compiler checks each digest before use.

### Using a Python library in a service

Declare a typed tool in Corvid and implement it in the project's `tools.py`. Both `def` and `async def` work with `@tool("name")`. See the running service example and packaging instructions in [Calling Python from a Corvid service](../guides/ffi-python.md).

When an imported function's declared trust level requires review, write `approve alias.member(args)` before the call. The interpreter checks the reviewed argument values and consumes the approval once. [Runnable example](../guides/ffi-python.md#direct-imports-in-an-interpreted-program)
