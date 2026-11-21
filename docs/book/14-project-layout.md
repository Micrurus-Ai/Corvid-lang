# Project layout

## What `corvid new` creates

```text
my-app/
├── corvid.toml         # project manifest + [io]/[http] security boundaries
├── .gitignore
├── src/
│   ├── main.cor        # entry point (any agent in here can be run)
│   └── std/            # vendored stdlib modules (io, http, db, json, …)
└── tools.py            # optional Python host tools for the starter echo
```

`corvid run`, `corvid build`, `corvid check`, and `corvid test` all
operate on the project rooted at the directory containing `corvid.toml`
(discovered by walking upward from the source file, like Cargo).

## `corvid.toml` reference

The scaffold writes this (comments trimmed):

```toml
name = "my-app"
version = "0.1.0"

[llm]
# No default model is set. Pick one explicitly:
#   default_model = "claude-opus-4-6"

[io]
# File-I/O root for the executing io_read_text / io_write_text /
# io_list_dir tools. Path traversal and absolute-path escapes outside
# the root are refused. Override at run time with CORVID_IO_ROOT.
root = "."

[http]
# HTTP egress allowlist for the executing http_get / http_post_json
# tools. The SSRF block (RFC1918 / loopback / link-local) is ALWAYS ON
# and not configurable. Empty list = HTTP fails closed. Override with
# CORVID_HTTP_ALLOW=host1,host2.
allow = []
```

Additional parsed tables for advanced configuration:

- `[effect-system]` — user-declared effect dimensions and dimension
  policy (see the effect-registry reference).
- `[package-policy]` — package import policy for the package manager.
- `[run]` — run-time defaults for `corvid run`.

Package dependency pins are covered in the
[package-imports reference](../reference/package-imports.md).

## Multi-file projects

Add additional `.cor` files under `src/`:

```text
src/
├── main.cor
├── refund.cor
├── retrieval.cor
└── policy.cor
```

Each file is a module. Cross-file references go through `import`. See
**[Modules and imports](/docs/modules)**.

## Conventions

- Snake_case for filenames (`refund_logic.cor`, not `RefundLogic.cor`).
- One agent per file is a good default; multi-agent files are fine when
  the agents share state.
- `tests/` mirrors `src/` directory structure: `tests/refund_test.cor`
  exercises `src/refund.cor`.
- Examples that ship with libraries go under `examples/`.
- Benchmarks under `benches/`.

### Formatting a project

Run `corvid fmt` from the project root to format Corvid files under `src/` and `tests/`. To format one file, use `corvid fmt src/main.cor`. Use `corvid fmt --check` in CI. It lists files that need formatting and exits with status 1 when it finds differences. It returns 0 when the files already match.

After cloning a project, run `corvid check` to check the entry point and its imports, `corvid run` to execute it, and `corvid test tests/math.cor` to run tests in that file.

When you edit an imported module, run `corvid run --target native` again. Corvid checks direct and nested imports before reusing a compiled executable. You can leave the project cache in place between edits.
