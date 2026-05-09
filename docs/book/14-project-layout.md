# Project layout

## What `corvid new` creates

```
my-app/
├── corvid.toml         # project manifest
├── src/
│   └── main.cor        # entry point (any agent in here can be run)
├── tests/
│   └── main_test.cor   # tests (run via `corvid test`)
├── .gitignore
└── README.md
```

`corvid run`, `corvid build`, `corvid check`, and `corvid test` all
operate on the project rooted at the directory containing `corvid.toml`.

## `corvid.toml` reference

```toml
[package]
name = "my-app"
version = "0.1.0"
description = "What this project does"
license = "MIT"

[dependencies]
"@stdlib/io" = "1"
"@stdlib/http" = "1"
"@scope/some-pkg" = "0.4"

[build]
target = "native"            # native | wasm | server | cdylib
optimize = "release"         # debug | release
sign = false                 # require ed25519 signature on the cdylib

[runtime]
default_provider = "openai"  # which LLM substrate adapter to use by default

[approvals]
operator = "ops@example.com"
require_for = ["refund_effect", "email_effect"]

[budgets]
default_per_run = "$0.10"

[replay]
storage = ".corvid/replay"   # where traces are persisted
retention_days = 30
```

Sections:

- `[package]` — project identity. `name` is required; the rest are
  metadata.
- `[dependencies]` — pinned package list. The package manager (Phase 25)
  resolves these; `corvid import-summary` audits what each one brings in.
- `[build]` — default target, optimization level, sign requirement.
  Override per-invocation with `corvid build --target=...`.
- `[runtime]` — runtime config, including the default model substrate.
- `[approvals]` — host-side approval policy. Effect names listed here
  require an operator approval at runtime in addition to the
  compile-time `approve` token.
- `[budgets]` — per-run cost ceiling. Agents without an explicit
  `@budget` annotation inherit this.
- `[replay]` — replay store config.

## Multi-file projects

Add additional `.cor` files under `src/`:

```
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
