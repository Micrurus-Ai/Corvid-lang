# Corvid

**Build the whole application — interface, backend and AI — in one language, where people and AI agents use the same typed actions.**

Corvid is a general-purpose programming language. A small app takes a few lines, and a team can build a large AI application at full scale from the first day with the same grammar, types and tools. Anything you build on the backend and AI side with Python or TypeScript and their libraries is meant to be buildable in Corvid, through built-in features or typed interoperability.

> Corvid is pre-1.0 and under active development. This page shows only what runs today and says plainly what is still being built. The [platform plan](docs/proposals/corvid-platform-plan.md) describes what comes next.

## Why one language for the interface, backend and AI

Today one feature is split across a database, a backend in one language, a frontend in another and an AI layer added on top, each with its own types, validation and permissions, kept in step by hand. Corvid puts them in one program that one compiler understands:

- **One definition, everywhere.** A type or a table is declared once and used by the database, the backend, the interface and the AI. Rename a field and the compiler points at every place that uses it, instead of a user finding the break in production.
- **The API keeps itself in sync.** The backend publishes a typed contract, and the TypeScript client and the Swift, Kotlin and Python SDKs are generated from it, so a client cannot drift from the server.
- **People and AI use the same actions.** A button and an assistant can run the same handler. There is no second API to build and secure for the agent, and the author chooses what an assistant may call.
- **No glue code.** No hand-written client, no duplicated validation, and migrations planned from the table declarations.

Next, an action is declared once, with its input, permission, validation and conflict rule, and the compiler derives the button, the endpoint and the assistant tool from it. After that, assistants see the current screen's typed state instead of screenshots, and one timeline follows a request across the interface, backend, agent and model steps.

## The problems every team adding AI runs into

| The problem | What Corvid does |
| --- | --- |
| **Model output is untyped text.** A parser breaks the day the model phrases things differently. | A prompt returns a declared type. When a reply does not match, Corvid sends the model the exact error and asks again. |
| **A bad answer cannot be reproduced.** The same input gives a different result, so it cannot be debugged. | Model calls, tool calls and approvals are recorded, and `corvid replay` runs a trace again with the same results, without calling the model. |
| **The bill is the first sign of the cost.** | Declare a budget, and code that could exceed it does not compile. |
| **An agent can call anything.** One injected instruction in a page can make it act on your data. | An irreversible tool needs an `approve` step, and text marked untrusted cannot reach it. Both are checked before the program runs. |
| **A grounded answer looks like an invented one.** | Values from retrieval carry their sources as `Grounded<T>`, and the compiler refuses to present an unsourced value as grounded. |
| **Testing AI features is guesswork.** | Tests, evaluations, fixtures and mocks are part of the language, and a recorded run can become a test. |
| **One AI feature touches four codebases.** | Declare a table once and get typed rows, typed queries and planned migrations, on SQLite or Postgres. |

The sections below show each of these running.

## The application and the AI see each other

One compiler sees every screen, action, type, permission, effect and cost in the application. So Corvid can give an AI the application itself, instead of screenshots or a hand-written API, and show people everything the AI does.

```corvid
import "std:ui" as ui

server progress:
    route GET "/" -> html:
        record = ui.assistant_action("Read one page", "read_page", "record_page_read", "Add one page to this session's reading count.")
        clear = ui.action("Start over", "reset_pages")
        return ui.interactive_page("Reading progress", [ui.int_state("Pages read this session", 0, [record, clear])])
```

A person clicks **Read one page**. An assistant in the browser finds a tool named `record_page_read`, with its description, and calling it runs the same `read_page` handler. There is no separate agent API. The author decides what an assistant can find: **Start over** is offered to people only. [Full example](examples/reading_list/src/progress.cor) · [Browser views guide](docs/book/19-browser-views.md#package-and-restart-a-reading-list)

Today the handler is named by a string. Next, an action is declared once, with its input, permission, validation and conflict rule, and the compiler derives the button, the HTTP endpoint and the assistant tool from it. Changing the action's input will list every interface, backend and agent caller that must change.

## Try it

Install on macOS or Linux (Windows instructions are under [Install](#install)):

```sh
curl -fsSL https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.sh | sh
```

The installer also adds the Corvid extension to VS Code, Cursor, VSCodium and Windsurf when it finds them. Then create and run a project:

```sh
corvid new hello
cd hello
corvid run
```

Continue with the [five-minute quickstart](docs/book/02-quickstart.md), or run `corvid tour --list` to see each language feature as a runnable demo. The quickstart's model step currently needs a model provider; running the first project without an API key is the next task in the [platform plan](docs/proposals/corvid-platform-plan.md#track-1-the-first-hour).

## What the compiler checks for you

**Model output has a type.** A prompt returns a declared type. When a reply does not match, `with repair` sends the model the exact error and asks again, and the budget counts every attempt:

```corvid
type Triage:
    category: String
    urgent: Bool

effect llm_call:
    cost: $0.01

prompt classify(ticket: String) -> Triage uses llm_call:
    with repair 2
    "Classify this support ticket: {ticket}"

@budget($0.05)
agent triage(ticket: String) -> Triage:
    return classify(ticket)
```

Three attempts at $0.01 fit the $0.05 budget. Raise the cost to $0.02 and the program no longer compiles: `static worst-case cost exceeds the declared budget`.

**Actions with consequences need approval.** An agent cannot call an irreversible tool without an `approve` step on the line before it:

```corvid
type Receipt:
    id: String

effect transfer_money:
    cost: $0.50
    trust: human_required
    reversible: false

tool issue_refund(id: String) -> Receipt dangerous uses transfer_money

@budget($1.00)
@trust(human_required)
agent refund(id: String) -> Receipt:
    approve IssueRefund(id)
    return issue_refund(id)
```

Delete the `approve` line and the compiler stops with `this call needs prior approval` and suggests the line to add.

**Data from outside has a type.** `Order.from_json(text)` decodes JSON into a declared type, and an error names the field that did not match:

```corvid
type Order:
    id: Int
    note: Option<String>

agent order_id(text: String) -> Result<Int, String>:
    order = Order.from_json(text).map_err(fn (e) -> e.path + ": " + e.message)?
    return Ok(order.id)
```

HTTP routes, the published OpenAPI schema and the generated TypeScript client all use the same JSON mapping, so the backend and its callers cannot disagree about a record's shape.

**Every run can be replayed.** Model calls, tool calls and approvals are recorded, and `corvid replay` runs a recorded trace again with the same results, without calling the model or repeating side effects. Values that come from retrieval carry their sources as `Grounded<T>`, and the compiler refuses to present an unsourced value as grounded.

## What you can build today

- **Web applications** with server-rendered views, forms and browser controls. [Browser views](docs/book/19-browser-views.md)
- **HTTP services** with typed routes, JSON, uploads, sessions, roles, OAuth and OpenAPI. [Backend guide](docs/guides/backend.md)
- **Agents** with typed tools, approvals, budgets, step limits, evaluations and replay. [Prompts](docs/book/11-prompts.md)
- **Data work** with typed tables on SQLite and Postgres (typed rows and queries, revision-checked writes, atomic blocks and migrations planned from the declarations), HTTP, files and JSON. [Tables](docs/reference/tables.md) · [Talking to the outside world](docs/book/18-talking-to-the-outside-world.md)
- **Background work** with durable jobs, schedules, retries and restart recovery.

What is missing compared with Python and TypeScript is listed honestly in the [backend and AI audit](docs/backend-ai-roadmap.md#audit-of-the-current-implementation).

## Where Corvid is going

The [platform plan](docs/proposals/corvid-platform-plan.md) organizes the next work around one principle: the application and the AI can always see each other, within what the author declares.

- **The first hour:** a first project that runs with no API key, a flagship task-board demo, and a compiler that coding assistants can ask what calls what and what a change breaks.
- **The application and the AI see each other:** typed shared actions, assistants that see the current screen's typed state, agent activity people can review and undo, live shared state, and one debugging timeline across interface, backend, agent and model steps.
- **Breadth and performance:** typed Python and npm interoperability on every target, typed persistence, real-time connections, vector search, tensors with checked shapes, and published benchmarks on equivalent workloads.

## Invention catalog

Each entry has a runnable tour, a specification and tests. The [full catalog](docs/reference/invention-catalog.md) has every entry's example and scope, and the [proof matrix](docs/reference/inventions.md) lists the tests behind each one.

| Area | Invention | Try it |
| --- | --- | --- |
| Safety At Compile Time | [Approve Before Dangerous](./docs/reference/invention-catalog.md#approve-before-dangerous) | `corvid tour --topic approve-gates` |
| Safety At Compile Time | [Dimensional Effects](./docs/reference/invention-catalog.md#dimensional-effects) | `corvid tour --topic dimensional-effects` |
| Safety At Compile Time | [`Grounded<T>` Provenance](./docs/reference/invention-catalog.md#groundedt-provenance) | `corvid tour --topic grounded-values` |
| Safety At Compile Time | [Strict Citation Contracts](./docs/reference/invention-catalog.md#strict-citation-contracts) | `corvid tour --topic strict-citations` |
| Safety At Compile Time | [Provenance Propagation + `@grounded_pure`](./docs/reference/invention-catalog.md#provenance-propagation--grounded_pure) | `corvid tour --topic provenance-propagation` |
| Safety At Compile Time | [Compile-Time Budgets](./docs/reference/invention-catalog.md#compile-time-budgets) | `corvid tour --topic cost-budgets` |
| Safety At Compile Time | [Confidence Gates](./docs/reference/invention-catalog.md#confidence-gates) | `corvid tour --topic confidence-gates` |
| Safety At Compile Time | [Secrets That Never Reach a Trace, a Prompt or a Log](./docs/reference/invention-catalog.md#secrets-that-never-reach-a-trace-a-prompt-or-a-log) | `corvid tour --topic secrets` |
| AI-Native Ergonomics | [AI-Native Keywords](./docs/reference/invention-catalog.md#ai-native-keywords) | `corvid tour --topic language-keywords` |
| AI-Native Ergonomics | [Trace-Aware Evals](./docs/reference/invention-catalog.md#trace-aware-evals) | `corvid tour --topic eval-traces` |
| AI-Native Ergonomics | [Replay And Receipts](./docs/reference/invention-catalog.md#replay-and-receipts) | `corvid tour --topic replay-receipts` |
| AI-Native Ergonomics | [Effect-Audited Skills](./docs/reference/invention-catalog.md#effect-audited-skills) | — |
| AI-Native Ergonomics | [Typed MCP In One Command](./docs/reference/invention-catalog.md#typed-mcp-in-one-command) | — |
| AI-Native Ergonomics | [Prompt Injection Is a Compile Error](./docs/reference/invention-catalog.md#prompt-injection-is-a-compile-error) | `corvid tour --topic injection-taint` |
| AI-Native Ergonomics | [Replay-Safe Secret Access](./docs/reference/invention-catalog.md#replay-safe-secret-access) | `corvid tour --topic replay-safe-secrets` |
| AI-Native Ergonomics | [Provenance-Keyed Cache](./docs/reference/invention-catalog.md#provenance-keyed-cache) | `corvid tour --topic provenance-cache` |
| AI-Native Ergonomics | [Governed Cron](./docs/reference/invention-catalog.md#governed-cron) | — |
| AI-Native Ergonomics | [Replay Quarantine For Durable Jobs](./docs/reference/invention-catalog.md#replay-quarantine-for-durable-jobs) | `corvid tour --topic replay-quarantine` |
| Adaptive Routing | [Typed Model Routing](./docs/reference/invention-catalog.md#typed-model-routing) | `corvid tour --topic model-routing` |
| Adaptive Routing | [Progressive Refinement](./docs/reference/invention-catalog.md#progressive-refinement) | `corvid tour --topic progressive-routing` |
| Adaptive Routing | [Ensemble Voting](./docs/reference/invention-catalog.md#ensemble-voting) | `corvid tour --topic ensemble-voting` |
| Adaptive Routing | [Jurisdiction And Privacy Routing](./docs/reference/invention-catalog.md#jurisdiction-and-privacy-routing) | `corvid tour --topic privacy-routing` |
| Streaming | [Streaming Effects](./docs/reference/invention-catalog.md#streaming-effects) | `corvid tour --topic streaming-effects` |
| Streaming | [Progressive Structured Streams](./docs/reference/invention-catalog.md#progressive-structured-streams) | `corvid tour --topic partial-streams` |
| Streaming | [Typed Stream Resumption](./docs/reference/invention-catalog.md#typed-stream-resumption) | `corvid tour --topic stream-resume` |
| Streaming | [Declarative Fan-Out / Fan-In](./docs/reference/invention-catalog.md#declarative-fan-out--fan-in) | `corvid tour --topic stream-fanout` |
| Verification | [Proof-Carrying Dimension Registry](./docs/reference/invention-catalog.md#proof-carrying-dimension-registry) | `corvid tour --topic effect-registry` |
| Verification | [Adversarial Bypass Testing](./docs/reference/invention-catalog.md#adversarial-bypass-testing) | `corvid tour --topic adversarial-tests` |
| Verification | [Executing File-I/O Surface](./docs/reference/invention-catalog.md#executing-file-io-surface) | `corvid tour --topic file-io` |
| Verification | [Declared HTTP Client](./docs/reference/invention-catalog.md#declared-http-client) | `corvid tour --topic http-client` |
| Verification | [Executing SQLite Surface](./docs/reference/invention-catalog.md#executing-sqlite-surface) | `corvid tour --topic sqlite` |
| Verification | [Executing JSON Surface (Opaque + Typed)](./docs/reference/invention-catalog.md#executing-json-surface-opaque--typed) | `corvid tour --topic json` |
| Verification | [Governed Retrieval](./docs/reference/invention-catalog.md#governed-retrieval) | `corvid tour --topic governed-retrieval` |
| Verification | [MCP With Governance](./docs/reference/invention-catalog.md#mcp-with-governance) | — |
| Verification | [Governed Concurrency](./docs/reference/invention-catalog.md#governed-concurrency) | — |
| Verification | [Executing Time & Randomness Surface](./docs/reference/invention-catalog.md#executing-time--randomness-surface) | `corvid tour --topic deterministic-time` |
| Verification | [The Agent Graph, Drawn from the Code](./docs/reference/invention-catalog.md#the-agent-graph-drawn-from-the-code) | `corvid tour --topic agent-graph` |
| The Application Surface | [Define Once, Get Everything](./docs/reference/invention-catalog.md#define-once-get-everything) | `corvid tour --topic application-surface` |
| The Complete Application Runtime | [The Backend Proves Its Own Contract, Or Refuses To Start](./docs/reference/invention-catalog.md#the-backend-proves-its-own-contract-or-refuses-to-start) | `corvid tour --topic contract-closure` |
| The Complete Application Runtime | [Cancel Fast, But Never Past a Point of No Return](./docs/reference/invention-catalog.md#cancel-fast-but-never-past-a-point-of-no-return) | `corvid tour --topic parallel-cancellation` |
| The Complete Application Runtime | [OAuth Login That's Safe By Construction](./docs/reference/invention-catalog.md#oauth-login-thats-safe-by-construction) | `corvid tour --topic oauth-login` |
| The Complete Application Runtime | [Protocol-Typed Connectors](./docs/reference/invention-catalog.md#protocol-typed-connectors) | `corvid tour --topic connectors` |
| The Complete Application Runtime | [Verified Provider Protocols](./docs/reference/invention-catalog.md#verified-provider-protocols) | `corvid tour --topic verified-provider-protocols` |
| The Complete Application Runtime | [Typed Tables With Revision-Checked Writes](./docs/reference/invention-catalog.md#typed-tables-with-revision-checked-writes) | `corvid tour --topic tables` |
| The Complete Application Runtime | [One Action for People and Assistants](./docs/reference/invention-catalog.md#shared-actions) | `corvid tour --topic shared-actions` |

## Architecture

```text
source .cor
  -> lex / parse
  -> resolve names
  -> typecheck
  -> effect, budget, confidence, grounding, approval, routing checks
  -> typed IR
  -> interpreter, native Cranelift backend, Python backend, WASM backend
  -> traces, replay, receipts, bundles
```

The language is designed as one compiler pipeline with multiple execution tiers. Safety properties belong in the shared frontend and IR, not in one backend's runtime glue.

## Status

Corvid is pre-v1.0 and under active development. The compiler, interpreter, effect system, model substrate, streaming substrate, replay/bundle infrastructure, native backend, signed cdylib attestation, separate-binary ABI descriptor verifier, and claim explanation workflow are in the repository today. Some backend paths intentionally reject newer high-level features until parity work lands; signed builds fail closed when contract-like syntax is not mapped to a registered guarantee.

A 2026-04-29 internal audit of Phases 35-41 found four phase-done bullets in Phases 38-41 that were structurally absent (multi-worker job runner + crash-recovery / DST tests, real JWT verification + `corvid auth/approvals` CLI, OTel SDK conformance, connector real mode + `corvid connectors` CLI). The ROADMAP now carries audit-correction tracks (35-N, 38K-M, 39K-L, 40J-K, 41K-M) that close the gaps end-to-end. Slice checkmarks before those tracks land are honest only at the layer the slice named — composition with the surfaces above stays disabled.

Use the roadmap for source-of-truth status:

```bash
rg -n "^- \\[ \\]" ROADMAP.md
```

## Install

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.ps1 | iex
```

**macOS / Linux**:

```sh
curl -fsSL https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.sh | sh
```

The installer downloads a prebuilt `corvid` for your OS/arch into `~/.corvid/`, adds `~/.corvid/bin` to your PATH, and runs `corvid doctor`. If a prebuilt archive is not available for your platform, it falls back to a `cargo install` from source.

Override defaults with `CORVID_REPO`, `CORVID_VERSION` (e.g. `v0.1.0`), or `CORVID_HOME`.

### Or via your package manager (planned, post-v1.0)

> **Status:** the install scripts above (`install/install.{sh,ps1}`)
> are the **canonical install path** for Corvid v1.0. The package-
> manager manifests below are tracked as Phase 33P slices in
> [`ROADMAP.md`](./ROADMAP.md#L1924) and ship **post-v1.0** — none of
> them is the "installer," each is additive metadata that a
> language-installer manager's central repo consumes and routes back
> to the GitHub Release artifacts `release.yml` produces. They're
> filed but not built yet; running them today will report
> "formula/manifest not found."

| Manager | End-user command (post-v1.0) | Filed slice |
|---|---|---|
| Homebrew (macOS / Linux) | `brew install Micrurus-Ai/corvid/corvid` | `33P1-homebrew-tap` |
| Scoop (Windows) | `scoop bucket add corvid https://github.com/Micrurus-Ai/scoop-corvid && scoop install corvid` | `33P2-scoop-bucket` |
| winget (Windows) | `winget install Micrurus-Ai.Corvid` | `33P3-winget-manifest` |
| Chocolatey (Windows) | `choco install corvid` | `33P4-chocolatey-package` |
| AUR (Arch Linux) | `yay -S corvid-bin` (or equivalent AUR helper) | `33P5-aur-package` |
| APT / RPM (Debian / Fedora) | `apt install corvid` / `dnf install corvid` after one-time repo add | `33P6-apt-rpm-repo` |

If you want to **track or contribute** any of these,
[`ROADMAP.md`'s Phase 33P block](./ROADMAP.md#L1924) names each
filed slice with what it actually does. Manifest contributions are
welcome at any time — the GitHub Release artifacts they consume
(`release.yml`) are already shipping; the manifests are
post-v1.0 only because Path A defers public packaging-manager
listings until launch by deliberate design.

## Install From Source

```bash
cargo install --path crates/corvid-cli
corvid doctor
```

Python runtime pieces are only needed when using the Python backend. Native and interpreter work do not require a Python deployment target.

## Developer Commands

```bash
cargo check --workspace
cargo test --workspace
cargo run -q -p corvid-cli -- tour --list
cargo run -q -p corvid-cli -- check examples/offline_refund_bot/src/refund_bot.cor
```

If `cargo fmt --check` fails because `cargo-fmt` is not installed, install the Rust formatter for the active toolchain before treating formatting as validated.

## Documentation

- [docs/proposals/corvid-platform-plan.md](./docs/proposals/corvid-platform-plan.md): the platform plan and the principle that organizes it.
- [ROADMAP.md](./ROADMAP.md): build plan and shipped slices.
- [docs/backend-ai-roadmap.md](./docs/backend-ai-roadmap.md) and [docs/frontend-roadmap.md](./docs/frontend-roadmap.md): the ordered backend and AI queue and the browser frontend queue.
- [docs/reference/invention-catalog.md](./docs/reference/invention-catalog.md): every invention with its example, specification, tour and non-scope.
- [docs/reference/inventions.md](./docs/reference/inventions.md): invention proof matrix.
- [docs/internals/effect-spec/](./docs/internals/effect-spec/): AI-native effect system, grounding, budgets, confidence, streaming, model substrate, replay, and verification specs.
- [docs/reference/core-semantics.md](./docs/reference/core-semantics.md): generated guarantee registry with ids, classes, phases, and test references.
- [docs/security/model.md](./docs/security/model.md): signed artifact trust boundary, host acceptance workflow, and explicit non-goals.
- [docs/operations/ci.md](./docs/operations/ci.md): CI matrix, including optional Python FFI feature coverage.
- [docs/internals/bundle-format.md](./docs/internals/bundle-format.md): signed bundle and receipt format.
- [ARCHITECTURE.md](./ARCHITECTURE.md): compiler design and repo structure.
- [CONTRIBUTING.md](./CONTRIBUTING.md): project rules and contribution expectations.
- [docs/internals/effect-spec/bounty.md](./docs/internals/effect-spec/bounty.md): public submission process for effect-system bypasses and false positives. Accepted reports are credited to the reporter and added to [docs/internals/effect-spec/counterexamples/](./docs/internals/effect-spec/counterexamples/) as permanent regression fixtures.
- [docs/internals/package-manager-scope.md](./docs/internals/package-manager-scope.md): what the package manager does today vs what would require a hosted registry service.
  Corvid ships package format and local/self-hosted registry tooling; no Corvid-hosted package registry service runs yet.
- [dev-log.md](./dev-log.md): chronological build journal.
- [learnings.md](./learnings.md): durable engineering lessons.

## License

Corvid is released under the [MIT License](LICENSE).

Contributions are licensed under the same MIT terms (inbound = outbound)
unless the contributor marks otherwise.
