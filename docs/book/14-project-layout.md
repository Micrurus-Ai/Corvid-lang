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
