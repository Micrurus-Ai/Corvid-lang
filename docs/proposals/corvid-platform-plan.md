# Corvid platform plan

Status: **adopted for backend and AI on 2026-11-23.** Items owned by frontend development are proposals until frontend development accepts them into its own queue.
Readers: the project owner, backend and AI development, frontend development.

This plan gathers the November 2026 brainstorming into one ordered set of work. The backend and AI items appear as tasks B21 to B42 in the [backend and AI queue](../backend-ai-roadmap.md#ordered-tasks). The frontend items are listed here for frontend development to accept, change or reject.

## The goal

Corvid is a general-purpose language for building an entire application — UI, backend and AI — in one language that a first-time reader can follow. Anything developers build on the backend and AI side with Python or TypeScript must be buildable in Corvid. The defining advantage is that a person and an AI agent use the application through the **same typed actions**.

## The principle that organizes the plan

**In Corvid, the application and the AI can always see each other.**

The compiler understands the whole application: every screen, action, type, permission, effect and cost, across UI, backend and AI. Other stacks show an AI a pile of files, an API description at best, or screenshots. Corvid can hand the AI that understanding directly. And because the AI acts only through the application's own actions, everything it does can be shown back to people.

**The AI sees the application.**
- An end-user assistant receives the typed state of the current screen, not pixels, and acts through the same actions the buttons use, under the same permissions.
- A coding AI asks the compiler questions instead of searching files: who calls this action, what breaks if this input changes, what this route costs, and how a recorded run went.
- An outside AI can use a deployed Corvid application through the actions its author chose to offer.

**People see the AI.**
- Users see what an agent did, in the application's own interface: which action, why, at what cost, and whether it can be undone. Approval prompts come from effect declarations.
- Developers see one timeline across UI events, backend calls, agent steps and model calls, and can rewind it.

**What the AI can see is always declared.** Which actions an assistant may discover, which fields it may read and whether outside AIs may connect are declared in source. Omitting the declaration is a compile error that names the decision, as the project's rule on consequential defaults requires. Nothing is exposed to an AI by default.

Every item below is judged by how it helps the application and the AI see each other, or how it lets a developer build more of the application in Corvid.

## How each brainstorming idea becomes stronger in Corvid

| Idea | In Corvid |
| --- | --- |
| A working app in 30 seconds | The first project already includes an assistant that operates it, and runs with no API key. |
| Live shared state | Agents take part in the live session: a person watches an agent move a card, then approves or undoes it. Every change goes through a typed action with its permission. |
| Time-travel debugging | One timeline across all three tiers. A coding AI reads the trace and proposes a fix, and replay shows the fix works. |
| A compiler that sees across tiers | It sends a page, or an agent, only the fields it uses, and proves an agent can reach only the actions it was given. |
| Self-healing model output | Repair checks model output against the application's own types, actions and rules. |
| Converting an existing app | A converted application becomes usable by agents without extra work, which is the reason to migrate. |
| One-command deployment | The deployed application carries its human interface and its declared agent interface together. |

## Track 1: the first hour

A developer decides in the first hour whether to stay. This track comes first, because later capabilities matter only to developers who stay.

| ID | Work | Owner | Depends on | Today | Done when |
| --- | --- | --- | --- | --- | --- |
| B21 | First run without an API key | Backend and AI | none | The quickstart's first model call needs a provider and a credential. | `corvid new` produces a project whose model calls run from recorded responses, offline and repeatably, with a one-line switch to a live model. `corvid model pull` adds an optional local model through the existing OpenAI-compatible path. A clean machine reaches a running AI step with no account. |
| B22 | Compiler questions for coding AIs | Backend and AI | none | Compiler knowledge is reachable through the CLI and the dev console, not through a query interface. | A coding assistant connected over MCP can ask who calls an action or route, what an input change affects across UI, backend and agent callers, what a route's effects and costs are, and what a recorded run did. Answers come from the same checker and contract the build uses. |
| B23 | First-hour friction audit | Backend and AI | none | Not done. | Install, create, run and reach the AI step on clean Windows, macOS and Linux machines, recording each stumble with its fix as a task. Repeat after B21 and the flagship demo. |
| B24 | The flagship task-board demo | Backend and AI, with frontend | B10, B11, F-P1 | `ui.assistant_action` links a control and an assistant to one handler named by a string. | A task board where a person drags a card, an assistant moves another through the same typed action under the same permission, and adding an input to the action makes the compiler list the affected UI, backend and agent code. Runs with B21 and ships as `corvid new board`. |
| — | README rewrite | Both | B24 for the final step | Rewritten on 2026-11-23 around this goal using only shipped features. | The flagship demo, its recording and a measured comparison with the same application in TypeScript and Python lead the README. |

## Track 2: the application and the AI see each other

| ID | Work | Owner | Depends on | Done when |
| --- | --- | --- | --- | --- |
| B10, B11 | Shared action declarations and assistant authority (existing) | Backend and AI, with F15 | B09 | As already specified in the queue. These are the base of every item in this track. |
| B25 | Screen state for assistants | Backend and AI, with F-P2 | B10 | An assistant receives the typed state of what the person is viewing, limited to declared fields, and can name and act on it. |
| B26 | Outside agent access | Backend and AI | B11 | A deployed application offers declared actions to outside assistants over MCP, extending the scoped remote adapter in the main queue. Omitting the exposure declaration is a compile error. |
| B27 | Visible agent activity | Backend and AI, with F-P3 | B11 | Every action records whether a person or an agent called it, with reason, provenance, cost and reversibility. Reversible actions offer undo. The application can show this history without custom code. |
| B28 | One timeline with rewind | Backend and AI, with F-P4 | A10 in the main queue | One trace covers UI events, backend calls, agent steps and model calls. A developer rewinds to a step, changes a prompt or code, and continues from recorded earlier steps without running them again. |
| B29 | Live shared state with declared scope | Backend and AI, with F08 | B14 | State shared across people, agents and the backend declares its scope (person, room or tenant) and who may read and write it. Changes go through typed actions. Omitting the scope is a compile error. |
| B30 | Only the fields that are used | Backend and AI | B09, B10 | When a page or an agent uses only some fields of a record, the compiler narrows what is sent, and the database query where Corvid issues it. Measured payload and query reductions are published. |
| B31 | Checked generation | Backend and AI | B12 | `with repair` extends to model-generated SQL and code, which run in a sandbox and are checked against the application's own types and rules before use. |
| B32 | Production traces on a laptop | Backend and AI | A10 in the main queue | A production failure exports as a redacted trace that replays on a developer's machine with external writes disabled. |
| B33 | Personal data as a checked type | Backend and AI | none | A type for personal data that cannot reach logs, traces or a model except through a declared redaction step, checked at compile time. |

## Track 3: breadth and performance

The existing tasks B02 to B09 and B12 to B20 continue in their order. These items join them.

| ID | Work | Depends on | Done when |
| --- | --- | --- | --- |
| B34 | Tensors with checked shapes | B02, B07 | A tensor type whose shape, including symbolic dimensions such as `Batch`, is checked at compile time, running through typed Python interoperability with PyTorch. A shape mismatch is a compile error with both shapes named. |
| B35 | Automatic differentiation | B34 | `grad` over tensor functions, verified against numerical gradients. |
| B36 | MLIR code generation | B34 | Tensor code compiles through MLIR for CPU, then CUDA, Metal and ROCm, with results matching the interoperability path. |
| B37 | Declared placement of model work | B36, B11 | A program declares where model inference may run (device, browser, cloud) and the fallback order. The runtime never chooses a placement the source did not allow. |
| B38 | Structured concurrency at scale | none | Lightweight tasks with structured cancellation serve 100,000 concurrent connections in a published benchmark against equivalent Go and Node.js services. |
| B39 | Ownership-based memory on every target | none | The native backend's reference-count ownership analysis applies on every target, with published reference-count and pause measurements. |
| B40 | Type inference design | none | A design discussion settles the inference scheme, Hindley–Milner or another, judged by what it infers and by the clarity of its error messages. |
| B41 | C and C++ headers | B02 | Import a C header with checked types and call it from native builds. C++ follows once C is complete. |
| B42 | Converting an existing application | B20 | `corvid absorb` converts a FastAPI or Next.js application with assistant help. The original's tests pass against the converted application before the conversion is reported as complete. |
| B43 | One-command deployment | none | `corvid ship` deploys to an existing provider in one command, asks the developer who may reach the application, and never publishes without confirmation. A Corvid-hosted service is a separate business decision. |

## Proposals for frontend development

Each needs frontend development's agreement before it enters the frontend queue.

| ID | Work | Pairs with |
| --- | --- | --- |
| F-P1 | Task-board interface for the flagship demo, with drag and keyboard movement | B24 |
| F-P2 | Declared screen state that an assistant may read | B25 |
| F-P3 | Agent presence and activity in the interface, with approve and undo | B27 |
| F-P4 | Timeline inspector for the one-timeline debugger | B28 |
| F-P5 | Browser playground that runs curated examples without installing, building on the [playground contract](../meta/playground-examples-contract.md) | B21 |
| F-P6 | Desktop and mobile targets for the same interface code | — |

## Order of work

1. Finish what remains of B01 that does not wait on frontend agreement, and run B23, the friction audit.
2. B21, then B22.
3. B09, B10 and B11, moved ahead of B02 to B08 because the flagship demo and all of Track 2 depend on them.
4. B24, the flagship demo, then the README's final step.
5. Track 2 in its listed order, interleaved with B02 to B08 and B12 to B20 by dependency.
6. Track 3 by dependency.

## Claims we make and claims we do not

Every claim in the README, the website or a launch post must have a runnable demonstration, a test and, for performance, a measurement on equivalent workloads.

We do not claim:
- that a program that compiles cannot fail at runtime. Networks fail, disks fill and some properties are undecidable. We list each class of failure Corvid rules out, with its proof.
- that checked model output is free of hallucination. Checking proves the output is well-formed and within the declared rules, not that it is true. `Grounded<T>` carries where a value came from.
- speed over C++, Rust, Go or any other language without a published measurement on the same workload.
- "100%" of anything.

## Decisions for the project owner

- Whether to run a Corvid-hosted deployment service for B43.
- Whether to collect first-hour usage measurements. Any collection must be opt-in and declared.
- Whether an optional local model download is offered during installation or only on request.
