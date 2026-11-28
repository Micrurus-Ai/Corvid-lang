# `corvid graph`: the agent graph drawn from the compiled program (B47)

**Status:** proposal, 2026-11-28. A CLI command with no grammar change, so it is built without a review window.
**Owner:** backend (Claude Code, Mac session), from the launch track.
**Roadmap:** B47 in [backend-ai-roadmap](../backend-ai-roadmap.md); [Beyond LangChain](../langchain-parity.md#tools-and-agents).

## The problem

LangGraph Studio draws an agent's graph from a graph the developer builds by hand alongside the code, so the picture shows what the developer declared, not what the code does. In Corvid the control flow *is* the code, and the compiler already knows it: the cost analysis behind `@budget` walks every agent's steps, branches, loops, tool calls, prompt calls and nested agents to bound its worst-case cost. Nothing shows a developer, a reviewer or an assistant that structure, so "what does this agent do, what can it spend, and which tools does nothing call" means reading every file.

## The design

```console
$ corvid graph src/main.cor --agent refund_bot
refund_bot                                  worst case $0.0300
├─ tool lookup_order                        $0.0000
├─ approve IssueRefund                      a person decides
├─ if amount > 100
│  ├─ prompt escalate                       $0.0200
│  └─ (else) tool issue_refund              $0.0100  dangerous
└─ for item in order.items (at most 50)
   └─ tool notify                           $0.0002 × 50

Never reached: tool archive_order (src/main.cor:41)
```

- `corvid graph [FILE] [--agent NAME] [--format text|mermaid|json]`. Without `--agent`, every entry point is drawn: public agents, actions, route handlers and schedules.
- **One source.** The graph is the cost analyzer's tree, the one `@budget` checks, so its numbers are the budget check's numbers and it cannot drift from the code. The tree gains one node kind, an `approve`, which costs nothing and marks where a person decides.
- **Each node** shows its kind (tool, prompt, agent, approval, branch, loop), its name, its worst-case cost in every dimension the program declares, a loop's bound (`at most 50`, or `unbounded`, as the budget warning says), whether a tool is `dangerous`, and its source location.
- **Never reached.** Tools and prompts no entry point's tree reaches are listed with where they are declared. They are dead weight, or a missing call.
- **Formats.** `text` is the default above. `mermaid` writes a flowchart for a pull request or the docs. `json` is the tree with every field, for an assistant (B22's compiler over MCP), the dev console, and tests.

## Alternatives considered

- **A graph builder in the language** (LangGraph's `StateGraph`). A second description of the control flow that must agree with the first, which is exactly the drift this avoids.
- **A separate walk of the IR for the picture.** It would compute branches and loop bounds a second way, and could show a loop bound `@budget` does not trust. Reusing the cost tree keeps one answer.
- **Drawing from a trace.** A trace shows one run's path, not every path; `corvid trace lineage` already draws runs.

## Across the tiers

- **Backend:** a reviewer sees what a change does to an agent's paths and worst case, and which tools became unreachable.
- **Frontend:** the dev console can show an action's graph from the JSON, beside its form.
- **AI:** an assistant or a coding agent asks the compiler what an agent calls and what it can cost, as typed data, not by reading source (the organizing principle: the application and the AI can always see each other). Nothing is exposed that the program's own source does not already state.

## Slices

- [x] G1 (2026-11-28). The cost tree records approvals and each node's source span, and marks `dangerous` tools; `corvid graph` prints the text form for one agent or every entry point, with the never-reached list.
- [x] G2 (2026-11-28). `--format mermaid` and `--format json`: the JSON is `GraphDocument`, a public serde type in `corvid-driver` that B22 serializes too, with a stable id, location, kind, effects, numeric worst case and an approval's guarded tool per step (fields asked for by B22's owner). Documented in [the reference](../reference/agent-graph.md), with the invention's public proof.
- [ ] G3. The dev console and B22's compiler-over-MCP read the JSON (G3a and G3b shipped 2026-11-28: routes and live declarations as roots, an action's route, policy, assistant offer and forms, and table operations as steps; see below). In the order B22's `callers_of` and `impact_of_change` need them: (1) routes and live declarations as roots (method and path, the handler's steps under them), and an action's routes and forms; (2) an action's assistant exposure (the `assistants:` mode and description), so who can call it includes agents; (3) table operations as steps (table and operation); then schedules; and the graph spans a project's files.

## G3a: routes, live declarations, and who can call an action

B22's `callers_of` and `impact_of_change` must answer "what reaches this tool, and who can call that" without reading source. Routes and actions are where outside callers enter, so G3a adds them to the document. Both additions are new fields, so a `v1` reader that ignores unknown fields keeps working.

```json
"routes": [
  { "id": "shop:GET /orders/{id}", "server": "shop", "method": "GET", "path": "/orders/{id}",
    "live": null, "requires": { "authenticated": true, "roles": [], "permissions": ["orders.read"] },
    "location": { "line": 40, "column": 5 }, "effects": ["lookup"], "worst_case": { "cost": 0.0 },
    "steps": [ { "id": "shop:GET /orders/{id}/lookup_order", "kind": "tool" } ] }
],
"entries": [
  { "id": "move_card", "kind": "action",
    "action": { "method": "POST", "path": "/actions/move_card",
                "requires": { "authenticated": true, "roles": [], "permissions": ["board.edit"] },
                "assistants": { "mode": "allow", "description": "Move a card to another column." },
                "used_by": [ { "builder": "form_for", "in": "board_controls", "location": { "line": 33, "column": 12 } } ] } }
]
```

- **Routes are roots.** A route's handler is costed by the same analysis as an agent's body, so its steps and worst case are the budget analysis's. A `live` declaration is a route whose `live` names the row type it delivers. `requires` is `null` for a public route, so a reader never mistakes a missing policy for an empty one. The id is the server, method and path, unique in a program, and step ids under a route start with it.
- **An action says how it is called.** The route is built from `ACTIONS_PATH`, the constant `corvid serve` mounts, never a copy. `requires` is the action's policy. `assistants` is the mode and description, or `null` when no assistant is offered it; because nothing is exposed to an AI unless the source says so, `null` is the common case. `used_by` lists every `form_for` and `button_for` that names the action, with the agent or route containing it.
- **Rejected:** a separate list of callers per tool. It would be a second copy of the tree, and B22's `callers_of` can derive it from step ids.

Table operations as steps (G3b) and schedules (G3c) follow in that order.

## G3b: table operations as steps

An impact query that stops at the last tool never reaches persistence, so a changed column cannot be traced to the actions that write it. G3b makes each table operation a step: `{"kind": "table_operation", "name": "Card.insert", "table": "Card", "operation": "insert"}`.

- The cost tree gains a `Table` node, which costs nothing, so no `@budget` verdict changes. It is recognized by `table_call_target` and `table_query_call`, the functions the checker and the lowerer already use, never a second matcher.
- A write is `get`, `insert`, `save` or `delete`. A query chain (`Card.where(...).order_by(...).take(10)`) is one step whose operation is `query`, or the aggregate that ends it (`count`, `sum`, `min`, `max`, `count_by`, `sum_by`), because the chain runs as one statement.
- The step id names the table and operation (`board_app:live /boards/{board}/cards/Card.query`), so it survives an unrelated edit as other ids do.
- **Rejected:** one step per query stage. The stages are one statement to the database, and a reader asking "what touches `Card`" wants one answer per statement.

## Not in scope

- A run's actual path: `corvid trace lineage` draws that.
- Editing the graph: the code is the graph.
