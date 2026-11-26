# Shared actions (B10)

**Status:** proposal, awaiting frontend agreement (F15). No code until the grammar is agreed.
**Owner:** backend and AI grammar (Claude Code); the UI binding is shared with frontend development (Codex).
**Roadmap:** [B10](../backend-ai-roadmap.md) and, for assistant authority, B11.

## The problem

A person and an assistant should change an application through the same typed operation. Today Corvid assembles that operation by hand from five places:

| Piece | Where it lives today |
| --- | --- |
| The behavior | a `public agent` |
| The UI control | `ui.action("Read one page", "read_page")` or `ui.assistant_action(...)`, naming the agent **by string** |
| The HTTP surface | two routes per operation, one `-> html` and one `-> json`, each repeating `form T` and `requires permission(...)` |
| Assistant exposure | raw `toolname=` / `tooldescription=` HTML plus hand-written script for CSRF and `agentInvoked` (the reading list's `apply_reviewed_titles`) |
| Validation and conflicts | hand-written checks in the agent body and a hand-written result variant |

A renamed handler string compiles. A permission on the HTML route can differ from the permission on the JSON route. An assistant can be offered an operation that the form never was. B10 removes each of those by making the operation one declaration.

## The proposal

```corvid
type MoveCard:
    card: Int
    to: Column
    position: Int where between(0, 500)

action move_card(move: MoveCard) -> Result<Card, Conflict<Card>> requires permission("board.edit"):
    return match Card.get(move.card):
        Some(card) -> Card.save(Card { column: move.to, position: move.position, ..card })
        None -> Err(Deleted)
```

`action` is one new keyword. Everything else is grammar Corvid already has: an agent's signature and body, a record with field refinements, the table operations and their [conflicts](write-conflicts.md), and the route's `requires` clause. A save's conflict is already a typed value, `Changed(current)` or `Deleted`, so the action returns it as it is; an earlier draft needed its own `MoveRefused` sum type and a helper to translate the save's result into it. An application with refusals of its own, such as a full column, declares a sum type for them and carries the conflict inside one of its variants.

### What one declaration gives every tier

| Tier | Derived from the declaration | Replaces |
| --- | --- | --- |
| Backend code | `move_card(m)` is an ordinary typed call from agents, tests and workers. | — |
| HTTP | One endpoint, `POST /actions/move_card`. A form body or a JSON body decode into `MoveCard`; the response is the typed result as JSON, or the page's HTML when the request came from a form. | The `-> html` / `-> json` route pair |
| UI | `ui.form(move_card)` renders the inputs, with the refinements as browser validation, and `ui.button("Move", move_card, m)` submits a prepared value. The handler is a **name the compiler resolves**, never a string. | `ui.action(label, "handler")` strings, hand-written forms |
| Assistants | The action is a tool whose input schema is `MoveCard`'s JSON Schema and whose result is `Conflict<Card>` or `Card`, through WebMCP in the page and through an MCP endpoint served by `corvid serve`. Only actions that say so (see [Assistant exposure](#assistant-exposure)). | `toolname=` HTML and the reading list's hand-written script |
| CLI | `corvid act move_card --card 3 --to Done --position 0`, checked against the same type. | A separate `read_page_cli.cor` |
| Workers | `corvid jobs enqueue --task move_card --payload '{...}'` decodes into the same type. | — |
| Contract | `corvid contract app` lists each action with its input, result, permission and exposure, so a change shows up as a contract diff. | Agents listed without permission or exposure |

**Validation runs once, at the boundary, from the type.** Every caller that is not Corvid code (form, JSON, assistant, CLI, worker) reaches the body only after the input decodes into `MoveCard` and its refinements hold. A refused input is a typed `invalid_input` response that names the field; the body never sees it. The browser shows the same refinement before submitting.

**Permission is checked once, before decoding, for every caller.** `requires permission("board.edit")` applies to the endpoint, the assistant tool, the CLI and the worker alike. Under B11, an assistant acts with the authority of the person it acts for, never more.

**Conflicts are the result type.** The declared error type is the conflict behavior: every caller receives `Changed(current)` as a value. The UI can show the current card. An assistant can read it and decide again. No caller gets a bare HTTP 409.

### Assistant exposure

Nothing is exposed to an assistant unless the source says so ([no hidden defaults](../../AGENTS.md#no-hidden-defaults-for-consequential-policy)). An action opts in with one line, directly under its header:

```corvid
action move_card(move: MoveCard) -> Result<Card, Conflict<Card>> requires permission("board.edit"):
    assistants: confirm "Move a card to another column."
    atomic:
        ...
```

| Value | Meaning |
| --- | --- |
| `allow "description"` | An authorized assistant may call it directly. |
| `confirm "description"` | Each call waits for the person's confirmation, through the existing approval path. |
| *(line absent)* | People only. The action is not listed to any assistant, and a call that names it is refused as unknown. |

The description is required, because it is what an assistant reads to decide whether to call the tool. This line is **B11's** `allow | confirm | never` decision. B10 ships only `confirm` and absence: `allow` waits for B11's on-behalf-of authority, and `never` is the absent line, so there is one way to say it. Under the shipping contract, a value is offered only once it runs completely.

### Composition with the guarantees

- **Effects.** An action's effects are its body's effects. `@budget`, `@trust` and the other annotations go on the action as on an agent, and they bind every caller, assistant calls included.
- **Approval.** A `dangerous` tool inside an action still needs its `approve`. `assistants: confirm` is a separate gate, for who may start the action.
- **Replay.** Each call is traced as `action:move_card`, with the decoded input, the actor, which surface called it (`http`, `assistant`, `cli`, `worker`, `code`) and the result. Replay feeds the recorded input through the same body.
- **Provenance.** A result built from `Grounded<T>` keeps its provenance in the JSON an assistant receives.
- **Tables.** An action body is an ordinary agent body: `atomic:` blocks, revision-checked saves and `Result` conflicts work unchanged.

### Seeing a change across tiers

Changing `MoveCard` (renaming `to`, adding a required field) is a compile error at every Corvid caller: `move_card(...)` calls, `ui.form(move_card)`, `ui.button(..., move_card, value)`, and tests. For callers outside the program (a deployed assistant, a saved CLI script), `corvid contract app --diff BASE` reports the change as breaking for each surface the action is exposed on. That is the measurable gain: a separate stack finds a renamed form field when a request fails in production.

## Alternatives considered

| Form | Why not |
| --- | --- |
| An attribute on an existing agent: `@action public agent move_card(...)` | Shorter to adopt, but an action is not "an agent plus a flag": it has an endpoint, a permission and an exposure, and the attribute hides those behind a symbol. A reader should see "action" as the first word. |
| Every setting as a line in the body: `requires:`, `assistants:`, `description:` | Routes already put `requires` on the header line, so moving it into the body would make two ways to write a permission. The `assistants:` line is kept in the body because the header would grow past one readable line. It is recognized only as an action's first line, so `assistants` is reserved there and nowhere else; `assistants: confirm "..."` cannot be a typed binding, since a binding's type is not followed by a string. |
| Exposure as an annotation: `@assistants(confirm, "...")` | Corvid's annotations constrain *how* code runs (`@budget`, `@deterministic`). Exposure is *who* may call it, which sits with `requires`. The `assistants:` line keeps the description next to what it describes. |
| Several parameters instead of one input record: `action move_card(card: Int, to: Column)` | Allowed: the parameters form the input type. The board example uses a record so the same `MoveCard` value can be prepared, stored and passed around. Both forms derive the same schema. |
| Keep two routes, generate only the tool | Leaves the HTML and JSON permissions able to drift, which is the bug this removes. |

## What frontend development needs to agree (F15)

1. The `action` declaration and the `assistants:` line as written above.
2. `ui.form(action)` and `ui.button(label, action, value)` taking the action **by name**, replacing the string handler in `ui.action` and `ui.assistant_action` for server actions. The string forms stay until each example moves, then are retired. (Browser-only callbacks, `browser callback`, are unchanged: they run in the page and are not server actions.)
3. One endpoint per action, answering forms with HTML and JSON with JSON, and the page's generated script handling CSRF and WebMCP `agentInvoked` for every action form, replacing the reading list's hand-written script.
4. The typed refusal shape (`invalid_input` naming the field, the action's own error variants) that the UI renders.

## Slices

- [ ] B10-1. The declaration: parse and check `action`, typed calls from Corvid code, `requires` checked for every caller, the trace entry, and `corvid contract app` listing actions.
- [ ] B10-2. The endpoint and the UI binding: `POST /actions/<name>` for forms and JSON with boundary validation; `ui.form(action)` and `ui.button(label, action, value)`.
- [ ] B10-3. Assistant exposure: the `assistants:` line (`confirm` and absence), WebMCP tools in the page and an MCP endpoint from `corvid serve`, both from the declaration.
- [ ] B10-4. The CLI and workers: `corvid act` and `jobs enqueue --task <action>`, decoding through the same type.
- [ ] B10-5. Cross-tier change report: `corvid contract app --diff`, and the project-board example (`move_card`), with a test that renames an input and checks every reported caller. Port the reading list's `apply_reviewed_titles` and `read_page` onto actions, deleting the hand-written form script.

## Non-scope

- `allow` exposure and on-behalf-of authority: B11.
- Idempotency keys for retried calls: a follow-up slice once B10-2 has the endpoint. The reading list keeps its proposal key until then.
- Durable, resumable actions (checkpoints, pause, leases): B13.
- Actions that run in the browser: `browser callback` already covers them.
