# Compiler questions for coding AIs (B22, launch track step 6)

**Status:** proposal, 2026-11-28. It adds a CLI command and an MCP server, no shared grammar, so it is built in its turn on the launch track without waiting for frontend review.
**Owner:** backend and AI (Claude Code, Windows session).
**Roadmap:** [launch track](../backend-ai-roadmap.md#launch-track-decided-by-the-owner-2026-11-28) step 6.

## The problem

A coding assistant working on a Python or TypeScript app learns the codebase by reading files and searching text. It guesses who calls a function, misses a caller in another layer, and finds out a change broke the frontend when a test fails, if one exists. The most common failure of AI-written changes is not bad code but code that is right locally and wrong somewhere the assistant never looked.

Corvid's compiler already knows the whole application across tiers: every screen, route, action, agent, prompt, table, effect, permission and cost, and which of them reach each other. The editor service answers "go to definition" and "find references". `corvid contract app --diff` already says what a change does to an app's published actions. A coding assistant cannot ask any of it.

## The design

```text
$ corvid mcp
```

`corvid mcp` runs a Model Context Protocol server over stdio for the project in the current directory. A coding assistant (Claude Code, Cursor, Copilot and others) adds it once, and then asks the compiler instead of guessing. Every tool is read-only, answers in typed JSON, and is computed by the same code the CLI and the editor use.

| Tool | Answers |
| --- | --- |
| `callers_of(name)` | Every place that calls an action, agent, prompt, tool or route, in every tier: screens, routes, agents, assistant exposure and tests, with file and line. |
| `impact_of_change()` | What the working tree's changes do to the app, against the last commit: actions whose inputs, results or permissions changed, every affected caller in each tier, and changes to the published contract that break a client. |
| `effects_of(name)` | An agent's or action's effect row, its approvals, and its worst-case cost under `@budget`. |
| `check()` | The project's diagnostics, with codes, spans and the compiler's hints. |
| `explain(code)` | The reference text of an error code. |
| `actions()` | Every action, with its inputs, result, permission, assistant exposure and conflict behavior. |

- **Read-only, always.** The server never writes a file, runs a program or calls a model. Changing code stays the assistant's job, done through its own tools where the person can see it.
- **One source.** `callers_of` uses the editor service's reference index; `impact_of_change` uses the contract diff; `effects_of` uses the checker's cost analysis. A second copy of any of them would drift, so none is written.
- **Nothing exposed by default** is a rule about the running app's actions and assistants. This server answers questions about source code the developer already gave the assistant, so it adds no new exposure. It reads only the project directory it was started in.

## Three tiers

- **Backend:** the compiler's knowledge becomes an API, built from the code paths that already produce it.
- **Frontend:** `callers_of` and `impact_of_change` include screens and forms, so an assistant changing an action sees the UI it breaks before it breaks it.
- **AI:** this is the coding AI half of the organizing principle: "a coding AI can ask the compiler what calls what and what a change breaks".

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Rely on the editor service (LSP) | Assistants do not speak LSP, and LSP has no "what does this change break" across tiers. MCP is what assistants call. |
| A JSON CLI command per question | Useful too, and each tool could also be `corvid query <tool>`; MCP is the one assistants discover on their own. |
| Let the server apply fixes | A tool that edits code behind the assistant's back hides changes from the person. Read-only keeps the person in control. |

## Slices

- [ ] CQ-1. `corvid mcp` over stdio with `check`, `explain` and `actions`, tested with a recorded MCP session.
- [ ] CQ-2. `callers_of` across tiers, from the editor service's index, with a test that finds a screen, a route, an agent and an assistant exposure for one action.
- [ ] CQ-3. `impact_of_change` from the contract diff, with a test that changes an action's input and lists every affected caller.
- [ ] CQ-4. `effects_of` with the worst-case cost.

## Non-scope

- Answering questions about a deployed app's live data: that is an app's own assistant, through its declared actions.
- Writing code.
