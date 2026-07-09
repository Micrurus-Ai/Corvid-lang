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

`public` applies to `type`, `tool`, `prompt`, `agent`, and store
declarations. **Effects cannot be `public`** — an effect is private
to its file today, so a module exposing a public tool declares the
tool's effect in the same file. (Effect export via `use` lands with
slice 45o.)

```corvid-fragment
# src/refund.cor
effect refund_effect:
    cost: $50.00
    trust: supervisor_required

public tool refund(amount: Float, id: String) -> String dangerous uses refund_effect
```

## Importing local modules

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
    file = io_read_text("note.txt")
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
import python "mylib" as ml
```

Brings a Python module's registered tools into scope through the
embedded PyO3 runtime (see the [Python FFI guide](../guides/ffi-python.md)).

## Re-export

> **Planned.** `public import … use …` re-export syntax does not
> parse today — a module cannot re-export another module's surface.
> Consumers import the defining module directly. (Effect re-export
> is slice 45o; declaration re-export is unscheduled and will be
> designed alongside it.)
