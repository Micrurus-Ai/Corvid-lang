# The Corvid Book

Sequential learning material. Read top to bottom on your first
pass; come back to specific chapters as reference later.

## Table of contents

1. [Why Corvid](00-why-corvid.md) — the moat in three minutes.
2. [Install](01-install.md) — get Corvid on your machine.
3. [Quickstart](02-quickstart.md) — first program in five minutes.
4. [Tutorial: build a refund agent](03-tutorial-refund-agent.md) —
   end-to-end walkthrough exercising every shipped invention.
5. [Syntax basics](04-syntax.md) — narrative intro to the language shape.
6. [Types](05-types.md) — primitives, generics, structs, sum types.
7. [Modules and imports](06-modules.md) — multi-file projects, visibility.
8. [Effects](07-effects.md) — the load-bearing primitive.
9. [Approve](08-approve.md) — compile-time approval tokens.
10. [Grounded](09-grounded.md) — provenance-carrying types.
11. [Agents](10-agents.md) — composable program entry points.
12. [Prompts](11-prompts.md) — LLM-backed functions.
13. [Errors and Result](12-errors.md) — recoverable vs unrecoverable failures.
14. [Pattern matching](13-pattern-matching.md) — exhaustive match, destructuring.
15. [Project layout](14-project-layout.md) — `corvid.toml` and conventions.
16. [Testing](15-testing.md) — `test`, `eval`, `fixture`, `mock`.
17. [Building and targets](16-building.md) — native, wasm, server, cdylib.
18. [Replay](17-replay.md) — deterministic re-execution.

## After the book

When you finish the book, the next stop depends on what you're
building:

- An HTTP backend → [guides/backend.md](../guides/backend.md).
- A persistence layer → [guides/persistence.md](../guides/persistence.md).
- A durable agent → [guides/jobs.md](../guides/jobs.md).
- An auth layer → [guides/auth.md](../guides/auth.md).
- Production deployment → [operations/production-checklist.md](../operations/production-checklist.md).
