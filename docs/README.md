# Corvid documentation

Corvid is a programming language where dangerous AI actions either
compile with a proof or do not compile at all. The compiler reads
`approve`, `Grounded<T>`, effect rows, and budget annotations the
way other compilers read types, and it refuses to emit a binary if
a tool call that should be gated by an approval is reachable
without one.

The docs follow the [Diataxis](https://diataxis.fr/) shape:
tutorials, how-to guides, reference, explanation. Use whichever
section matches what you are doing.

## [The Corvid Book](book/) — sequential learning

Read top to bottom to learn the language.

1. [Why Corvid](book/00-why-corvid.md)
2. [Install](book/01-install.md)
3. [Quickstart](book/02-quickstart.md)
4. [Tutorial: build a refund agent](book/03-tutorial-refund-agent.md)
5. [Syntax basics](book/04-syntax.md)
6. [Types](book/05-types.md)
7. [Modules and imports](book/06-modules.md)
8. [Effects](book/07-effects.md)
9. [Approve](book/08-approve.md)
10. [Grounded](book/09-grounded.md)
11. [Agents](book/10-agents.md)
12. [Prompts](book/11-prompts.md)
13. [Errors and Result](book/12-errors.md)
14. [Pattern matching](book/13-pattern-matching.md)
15. [Project layout](book/14-project-layout.md)
16. [Testing](book/15-testing.md)
17. [Building and targets](book/16-building.md)
18. [Replay](book/17-replay.md)

## [Guides](guides/) — task-focused how-to

- [Backend](guides/backend.md) — HTTP server, routes, middleware.
- [Persistence](guides/persistence.md) — `std.db`, migrations, audit log.
- [Jobs and schedules](guides/jobs.md) — durable runner, retries, DST cron.
- [Auth and approvals](guides/auth.md) — JWT, OAuth, approval product.
- [Observability](guides/observability.md) — OTel, lineage, redaction.
- [Connectors](guides/connectors.md) — Gmail, Slack, MS365, etc.
- [WASM target](guides/wasm.md) — browser, edge, wasmtime.
- [FFI: Python](guides/ffi-python.md) — calling Python from Corvid.
- [FFI: C/Rust](guides/ffi-c-rust.md) — exposing Corvid as a cdylib.
- [Editor and LSP](guides/editor-and-lsp.md) — VS Code, Neovim, JetBrains.
- [Debugging](guides/debugging.md) — common errors and how to fix them.
- [Performance](guides/performance.md) — what's fast, what's slow.

## [Recipes](recipes/) — small focused patterns

RAG, approval-gated tool, multi-step agent with checkpoints,
provider routing, local model fallback, audit log per decision.

## [Reference](reference/) — lookup material

- [Grammar](reference/grammar.md) — formal EBNF.
- [Lexer rules](reference/lexer-rules.md) — continuation, brackets, triple-quoted.
- [CLI reference](reference/cli.md) — every `corvid` command.
- [Compile-time guarantees](reference/guarantees.md) — the registry.
- [Inventions](reference/inventions.md) — full proof matrix.
- [Inventions tour](reference/inventions-tour.md) — short index.
- [Standard library](reference/stdlib/) — per-module reference.
- [Stdlib connectors](reference/stdlib/connectors/) — Gmail, Slack, etc.
- [Reference apps](reference/reference-apps.md) — canonical examples.
- [Core semantics](reference/core-semantics.md) — registry-derived spec.
- [Package imports](reference/package-imports.md) — package manager flow.

## [Migration](migration/) — coming from another language

- [From Python](migration/from-python.md) — for LangChain / DSPy refugees.

## [Operations](operations/) — running Corvid in production

- [**Developer production guide**](developer-production-guide.md) — the canonical "ship Corvid in production" walk-through
- [**Maintainer runbooks**](maintainer-runbooks.md) — release checklist, security advisory process, CI gates, benchmarks, claim review, rollback
- [Production checklist](operations/production-checklist.md)
- [Receipts and signed builds](operations/receipts-and-signing.md)
- [Observability conformance](operations/observability-conformance.md)
- [CI](operations/ci.md)

## [Security](security/) — model and policy

- [Security model](security/model.md) — TCB, threat model, what's defended.
- [Stability contract](security/stability-contract.md) — SemVer rules + v1.0 surface.

## [Internals](internals/) — spec and implementation

- [Effect spec](internals/effect-spec/) — the formal effect algebra.
- [Bundle format](internals/bundle-format.md)
- [WASM ABI](internals/wasm-abi.md)
- [Host compliance](internals/host-compliance.md)
- [Testing primitives](internals/testing-primitives.md)
- [Package manager scope](internals/package-manager-scope.md)

## [Help](help/)

- [FAQ](help/faq.md)
- [Glossary](help/glossary.md)

## [Meta](meta/) — project-level docs

- [Project conventions](meta/conventions.md)
- [**Release policy**](release-policy.md) — channels, SemVer scope, breaking-change rules, signoff
- [Upgrade migrations](meta/upgrade-migrations.md)
- [Beta program](meta/beta-program.md)
- [Launch claim audit](meta/launch-claim-audit.md)
- [Claim inventory](meta/claim-inventory.md)
- [Launch rehearsal](meta/launch-rehearsal.md)
- [v1.0 demo script](meta/v1.0-demo-script.md)
- [AI benchmarks](meta/ai-benchmarks.md)
- [Reference apps launch status](meta/reference-apps-launch-status.md)

## [Phases](phases/) — development records

Phase-by-phase dev records, audits, and refactor logs. These are
historical engineering documents, not user-facing material. Browse
`phases/phase-NN-*.md` for any phase mentioned in `ROADMAP.md`.
