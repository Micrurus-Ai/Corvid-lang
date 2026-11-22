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

### Start a team project and hand it over

Choose the app and its first end-to-end task before scaffolding. Start directly with the modules your contributors will own. Put the shared request and result types in one module, give model-driven work a typed tool interface, and put durable reads and writes behind the persistence module. For a full-stack app, connect the screen to the authenticated server handler and expose the chosen operation to the assistant with the same permission checks. Keep a deterministic model response for repeatable tests.

Give a teammate the project revision, toolchain version, documented setup and one shared-type change. From a clean checkout, have them trace the screen event to the model input and the durable write before running the app. After the change, have them use the diagnostics to find affected callers, run the application tests, rebuild the package and verify the saved value after restart. Record their predictions, the first failing command, any help they needed and the time until the packaged workflow passes.

During the full-stack trial, have the teammate diagnose a reported problem, upgrade the toolchain and deploy the change in the team's authorized environment. In that deployment, exercise the UI, authenticated operation, durable write, real model call and authorized assistant action together. Record the revision, toolchain and service versions, manual workarounds and any private setup instructions the teammate needed.

For the subsequent Python and TypeScript comparison, carry over the team's actual task, tool schemas, model settings, starting records and durability requirements. In each stack, interrupt the same operation, predict what survived, then verify it after restart. Use the same change and reported fault. Record prediction errors, elapsed change and debugging times, documentation lookups, interventions and each contributor's stack preference with their reason. If a blocker recurs, retain the failing task and repeat it with a fresh contributor after the fix. [Independent trial criteria](../frontend-roadmap.md#how-developers-should-experience-corvid)

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
