# Shared actions (B10)

**Status:** building, under the propose-then-build grammar law (posted for frontend development; no objection within a working day). B10-1 shipped 2026-11-26.
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

**Every action says who may call it.** The `requires` clause is mandatory on an action: leaving it out is a parse error that lists the choices, and an action every caller may use says `requires anyone`, on its own. An operation reachable from outside never becomes open by silence ([no hidden defaults](../../AGENTS.md#no-hidden-defaults-for-consequential-policy)). The checker refuses a signed-in policy without an `identity` block, and a role or permission the identity block does not declare, because no caller could ever meet it.

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

### Amendment for frontend review (2026-11-26): the form binding

Building item 2 found three things the agreed text did not settle. This amendment replaces item 2 and the HTML half of item 3. Under the grammar law it is built one working day after it is posted unless frontend development objects.

**One form primitive, not two.** `form_for("POST /save", initial, "Save")` already renders a typed form from a declared route, checks the initial record at compile time, and is used by the reading list and the book. A second primitive, `ui.form(action)`, would give the same thing two spellings. So `form_for` takes an action by name in the same position:

```corvid
fn board_page(card: Card) -> String:
    move = View(form_for(move_card, MoveCard(card.id, Done, 0), "Move"))
    archive = View(button_for(archive_card, card.id, "Archive"))
    return ui.page("Board", [move, archive])
```

- `form_for(action, initial, label)`: the action must take one record of scalar fields (the rule `form_for` already applies to a route's record). The compiler resolves `move_card` and checks `initial` against its input type, so renaming a field of `MoveCard` is an error here. The form posts to `/actions/move_card`.
- `button_for(action, value, label)` replaces `ui.button(label, action, value)`: one submit button carrying a prepared input as hidden fields, for an action whose input is that value. The argument order matches `form_for`: what to call, with what, under which label.
- `ui.action(label, "handler")` and `ui.assistant_action` keep their string handlers for browser callbacks only; for server work, examples move to `form_for` and `button_for`, and the string forms are retired for server actions after the reading list moves (B10-5).

**CSRF for a plain form.** A plain HTML form cannot set `X-CSRF-Token`, so a signed-in action form would always get `403`. `form_for` and `button_for` add a hidden `corvid_csrf` field. The page script fills it from the `corvid_csrf` cookie on submit, and the server accepts the token from that field when the header is absent. The double-submit check is the same one; only where the copy travels changes. Without the page script the field stays empty and the post is refused, which fails closed.

**What a browser form gets back.** An action returns data, not a page. A form post (`application/x-www-form-urlencoded` with `Accept: text/html`) that succeeds answers `303 See Other` back to the posting page, taken from `Referer` and accepted only when it has the server's own origin; otherwise `303` to `/`. A refused input or a failure answers an HTML page naming the field or the error, with a link back. A queued approval answers a page with the approval id. With the page script, the form submits by `fetch` and shows the result in its `<output>` without leaving the page, the way `agent_post_form` receipts work today. JSON callers are unchanged.

What frontend development owns here: the page script's CSRF fill, the fetch submission and receipt, and the browser test. The backend owns `form_for` and `button_for` checking, the hidden fields, the form-field CSRF acceptance and the `303` and HTML answers.

## Slices

- [x] B10-1. The declaration: parse and check `action`, typed calls from Corvid code, the mandatory `requires` checked against the identity block, and `corvid contract app` listing actions with their input, result and caller policy. Built 2026-11-26. The policy is enforced where outside callers arrive, so enforcement and the `action:<name>` trace entry move to B10-2, which adds the first such caller.
- [x] B10-2a. The endpoint: `POST /actions/<name>` in `corvid serve` for JSON and forms, the policy enforced before decoding (with CSRF for cookie sessions), the input decoded through the action's types, the `action` trace event with the calling surface, and each action in OpenAPI. Built 2026-11-26. The input's shape is one rule in `corvid_types::action_input`, which the contract publishes as `input_type`: one parameter of a declared type is the whole input, anything else is an object with one field per parameter. Also: an action's signature may not hold a `Secret`, and a route may not take an action's path.
- [x] B10-2b-1. The caller, as a parameter: `action claim(card: Int, by: Caller) -> Card requires authenticated`. The server fills `by` from the verified session and leaves it out of the input; Corvid code passes one (`claim(path.card, actor)`), and the interpreter checks the action's `requires` against it, so a route that requires less cannot reach an action that requires more. Considered: binding `actor` implicitly in a signed-in action's body, as a route does. Rejected because an action is also called from code, and an implicit caller would have to flow from the calling code unseen; the parameter says in the signature that the action reads its caller, and the compiler sees every place one is passed. Built 2026-11-26.
- [x] B10-2b-2. `@approval(...)` and the other annotations on an action, so an `approve` inside one queues for the declared reviewer instead of being denied. Routes and actions share one approval rule. Built 2026-11-26.
- [x] B10-2b-3. Forms for actions with several parameters, one field per parameter by the query-field rule. Built 2026-11-26.
- [ ] B10-2b-5. The form binding, as amended above: `form_for(action, initial, label)` and `button_for(action, value, label)`, the hidden CSRF field and its server acceptance, and the `303` and HTML answers to a browser form. Posted for frontend review 2026-11-26; built after one working day unless frontend development objects.
- [x] B10-2b-4. Action methods in the generated TypeScript client, typed by the contract: the record or one field per parameter, and `T | ApprovalPending` for an action with `@approval`. Built 2026-11-26.
- [x] B10-3a. The `assistants: confirm "..."` line: parsed only as an action's first line, `allow` and `never` refused with what to write instead, an empty description refused, and the contract and OpenAPI carry the mode and description. Until an assistant can call the action, `corvid serve` refuses to start a program with the line (contract closure gap naming B10-3b), so no surface is advertised that nothing serves. Built 2026-11-26.
- [x] B10-3b. The assistant endpoint, as decided below: `POST /mcp` in `corvid serve` and the person's confirmations. Built 2026-11-26. Frontend development's half, WebMCP tools in the page from the same `tools/list` and the confirmation prompt, is on the frontend roadmap.

### B10-3b design: the assistant endpoint

**Whose authority.** In B10 an assistant acts only inside the person's own signed-in session: the in-page assistant (WebMCP) and any client the person hands their session to. `POST /mcp` authenticates exactly as `POST /actions/<name>` does (session cookie, CSRF for a cookie session), so an assistant can never do what the person could not. A delegated token, so an outside assistant acts for a person without their session, is B11's on-behalf-of authority and is not offered here.

**What it lists.** MCP `tools/list` returns, for the signed-in caller, the actions that have an `assistants:` line and whose `requires` the caller meets, each with its description and its input's JSON Schema (the OpenAPI request schema, from the same `input_type` rule). An action without the line, or one the caller may not call, is not listed, and `tools/call` naming it answers as an unknown tool, never as a refusal that confirms it exists.

**How `confirm` reaches the person.** `tools/call` never runs a `confirm` action directly. It validates the input through the action's types (a bad input is a tool error naming the field), then records a pending confirmation bound to the caller's actor and tenant, and answers with its id and a sentence naming what will happen. The person confirms in the page with `POST /__confirmations/<id>/confirm` (or declines with `/decline`), which requires the same actor's session and CSRF; only then does the action run, through the same path as `POST /actions/<name>`, and the result is returned. This is not the approval queue: an approval needs a different reviewer (separation of duties), while a confirmation is the person agreeing to what their own assistant asked. A pending confirmation is in memory and lasts 15 minutes, the fixed window shown to the person; it is not configurable in B10 because a shorter fixed window only removes risk, and making it longer is a decision B11 takes with on-behalf-of authority.

**The trace and the caller.** Each call records the `action` event with `surface: "assistant"`, the confirmation id and the actor; the confirmation records its own `assistant.confirmation` event when proposed, confirmed, declined or expired. An action's `Caller` parameter is the signed-in person, since the assistant acts with their authority.

**Non-scope.** Delegated tokens and `allow` (B11); a durable confirmation store that survives a restart (with B13); the in-page WebMCP registration and the confirmation prompt UI (frontend development, reading `tools/list`).
- [x] B10-4a. `corvid act <action> --field value ...` (or `--json`), reading flags by the request-text rule through the shared `action_calls` decoders, printing public JSON, traced as `surface: "cli"`. An action that requires a signed-in caller is refused by name until the command line can sign in (B11). Built 2026-11-26.
- [x] B10-4b. Workers: `jobs enqueue --task <action>` checks the payload against the action's input type at enqueue time, and `jobs run` runs the action through the same path, traced as `surface: "worker"`. A signed-in action needs the enqueuing person's authority carried with the job, which is B11's on-behalf-of authority. Built 2026-11-27, for actions that require no one: `jobs enqueue --source` checks the payload with `action_calls::decode_json_input`, and a worker checks it again and runs the action. A signed-in action is refused by name until B11-5. Found and closed on the way: the plain job executor would run an action's body as an agent from a positional payload, which could carry a forged `Caller`; it now refuses an action's body.
- [x] B10-5a. Cross-tier change report: `corvid contract app --diff BASE` reports each action change as breaking, additive or a note, with every outside surface it reaches, and exits 1 on a breaking change. A test renames an input and checks each reported caller. Built 2026-11-26.
- [ ] B10-5b. The project-board example (`move_card`) and the reading list's `apply_reviewed_titles` and `read_page` on actions, deleting the hand-written form script. Waits for the form binding (B10-2b-5).

## Non-scope

- `allow` exposure and on-behalf-of authority: B11.
- Idempotency keys for retried calls: a follow-up slice once B10-2 has the endpoint. The reading list keeps its proposal key until then.
- Durable, resumable actions (checkpoints, pause, leases): B13.
- Actions that run in the browser: `browser callback` already covers them.
