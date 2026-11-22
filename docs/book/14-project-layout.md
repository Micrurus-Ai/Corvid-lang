# Project layout

## What `corvid new` creates

```text
my-app/
  src/
    main.cor
  corvid.toml
  .gitignore
```

Start in `src/main.cor`. Add a sibling file when part of the app deserves its own name. Use `import "./cart" as cart` for your own module and `import "std:ui" as ui` for the bundled UI module. The optional `--with-python-tools` flag adds `tools.py` and `src/python_tools_example.cor`.

`corvid run`, `corvid build`, `corvid check`, and `corvid test` all
operate on the project rooted at the directory containing `corvid.toml`
(discovered by walking upward from the source file, like Cargo).

## `corvid.toml` reference

A new project starts with these settings:

```toml
name = "my-app"
version = "0.1.0"

[io]
root = "."

[http]
allow = []
```

Additional configuration tables include:

- `[llm]` sets the default model for model calls.
- `[effect-system]` — user-declared effect dimensions and dimension
  policy (see the effect-registry reference).
- `[package-policy]` — package import policy for the package manager.
- `[run]` — run-time defaults for `corvid run`.

The top-level `version` identifies your application. The installed compiler determines which language syntax the project accepts. Run `corvid --version` to see the compiler version, source revision and build date. When testing an upgrade, run `corvid upgrade check .` and your application tests with the candidate compiler.

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
