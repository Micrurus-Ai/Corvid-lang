# Guides

Task-focused how-to material. Each page answers a specific
"how do I X?" question. Skim by title; read what you need.

## Building applications

- [Backend](backend.md) — HTTP server, routes, middleware pipeline.
- [Persistence](persistence.md) — `std.db`, migrations, audit log,
  encrypted token storage.
- [Jobs and schedules](jobs.md) — durable runner, retries, idempotency,
  approval-wait, DST-aware cron.
- [Auth and approvals](auth.md) — JWT verification, OAuth flows, the
  approval product surface.
- [Observability](observability.md) — OTel spans, lineage graphs,
  redaction, runbooks.
- [Connectors](connectors.md) — Gmail, MS365, Slack, Calendar, Tasks,
  Files. Mock + replay + real modes.

## Deploying

- [WASM target](wasm.md) — browser, edge, wasmtime.
- [FFI: Python](ffi-python.md) — calling Python from Corvid.
- [FFI: C/Rust](ffi-c-rust.md) — exposing Corvid as a cdylib.
- [Editor and LSP](editor-and-lsp.md) — VS Code, Neovim, JetBrains.

## Operating

- [Debugging](debugging.md) — common errors and how to fix them.
- [Performance](performance.md) — what's fast, what's slow, when to
  drop into Rust via the FFI.

## See also

- [Operations](../operations/) — production checklist, receipts,
  runbooks.
- [Recipes](../recipes/) — small focused patterns (RAG,
  approval-gated, multi-step, …).
- [Reference](../reference/) — lookup material.
