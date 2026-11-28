# The agent graph: `corvid graph`

`corvid graph` draws what each agent does, from the compiled program: every tool, connector operation, Python function, prompt, nested agent and `approve`, the paths of each branch, and each loop's bound. Every step shows its worst-case cost and where it is in the source. Dangerous tools are marked. Tools, connector operations and prompts that no entry point reaches are listed.

The graph is the tree `@budget` checks, not a second description of the program. So it cannot drift from the code, and its numbers are the ones the budget check compares.

```console
$ corvid graph refund.cor
agent refund_bot                                 worst case $0.030
├─ tool lookup_order                             free                     refund.cor:22
├─ approve IssueRefund                           a person decides         refund.cor:23
├─ one of these paths                            $0.020                   refund.cor:25
│  ├─ path 1
│  │  └─ prompt escalate                         $0.020                   refund.cor:26
│  └─ path 2
│     └─ tool issue_refund (dangerous)           $0.010                   refund.cor:28
└─ loop, at most 50 times                        $0.010                   refund.cor:29
   └─ tool notify                                $0.0002                  refund.cor:30

Never reached:
  tool archive_order (refund.cor:16)
```

The program is the `agent-graph` tour topic (`corvid tour --topic agent-graph`).

## The command

```sh
corvid graph [FILE] [--agent NAME] [--format text|mermaid|json]
```

- `FILE` defaults to the project's `src/main.cor`. A program that does not check prints its errors, and the command exits 1.
- Without `--agent`, every **entry point** is drawn: each action, each public agent, and each agent no other agent calls. Route handlers and scheduled agents are among those. `--agent NAME` draws one agent, and the never-reached list still covers every entry point, so naming one agent does not make the rest of the program look unused. An unknown name exits 1.
- A step's location is the line that calls it. A never-reached declaration's location is where it is declared.

## What a step costs

- A step's cost is its worst case in each dimension the program declares. `cost` is in US dollars. A cost under a tenth of a cent shows its digits (`$0.0002`).
- A branch costs its most expensive path. A loop with a known bound costs its body times the bound. That bound comes from a list literal, `range(start, end)`, a query's `take`, or a refined field or parameter.
- A loop nothing bounds, and a lambda whose body costs something, are drawn with their steps but have **no bound**. Their cost reads `no bound`, never a number, because the number would cover one pass, not the worst case. `@budget` refuses to pass such an agent for the same reason.
- An `approve` costs nothing and reads `a person decides`.
- A connector operation's retries and polls are a bounded loop around its call.

## Mermaid

`--format mermaid` writes a flowchart for a pull request or the docs. Steps follow one another in the order they run. A branch fans out to its paths, and the paths meet again at the next step. A loop's body runs from the loop node and returns to it on a dotted edge. Each kind of step has its own shape:

| Step | Shape |
| --- | --- |
| entry point | stadium |
| tool, connector operation, Python function | box |
| prompt | rounded box |
| nested agent | subroutine |
| `approve` | parallelogram |
| branch | diamond |
| loop | hexagon |

Dangerous tools are outlined in red. The never-reached declarations are a separate group.

## JSON

`--format json` writes a `GraphDocument` (`corvid_driver::agent_graph`). That one serde type is what the dev console and an assistant asking the compiler (B22) serialize, so every reader gets the same shape. Every document carries `"schema": "corvid.agent_graph.v1"`, and a change a reader must handle differently takes a new version.

```json
{
  "schema": "corvid.agent_graph.v1",
  "file": "refund.cor",
  "entries": [
    {
      "id": "refund_bot",
      "kind": "agent",
      "name": "refund_bot",
      "location": { "line": 21, "column": 1 },
      "effects": ["dangerous", "lookup", "message", "model_call", "payment"],
      "worst_case": { "cost": 0.03, "latency_ms": 0.0, "tokens": 0.0 },
      "steps": [
        { "id": "refund_bot/lookup_order", "kind": "tool", "name": "lookup_order", "location": { "line": 22, "column": 14 },
          "effects": ["lookup"], "dangerous": false, "worst_case": { "cost": 0.0, "latency_ms": 0.0, "tokens": 0.0 } },
        { "id": "refund_bot/approve.IssueRefund", "kind": "approval", "name": "IssueRefund", "location": { "line": 23, "column": 5 },
          "effects": [], "dangerous": true, "guards": "issue_refund", "worst_case": { "cost": 0.0, "latency_ms": 0.0, "tokens": 0.0 } }
      ]
    }
  ],
  "never_reached": [
    { "kind": "tool", "name": "archive_order", "location": { "line": 16, "column": 1 } }
  ]
}
```

An **entry** has `id`, `kind` (`agent` or `action`), `name`, `location`, `effects`, `worst_case` and `steps`.

Each **step** has these fields:

| Field | Meaning |
| --- | --- |
| `id` | The entry's name, then each enclosing step and this one, each named by what it calls (`approve.Label` for an approval; `branch`, `loop` or `lambda` otherwise), with `pathN` for a branch's paths: `refund_bot/branch/path2/issue_refund`. A repeat of the same name in the same block is `#2`, `#3`: `refund_bot/lookup_order#2`. Adding or removing an unrelated step changes no other step's id, so a coding assistant's saved reference keeps its meaning. |
| `kind` | `tool`, `connector_operation`, `python_function`, `prompt`, `agent`, `action`, `approval`, `branch`, `loop` or `lambda`. |
| `name` | The callee, for a call. The label, for an approval. Absent for a branch or loop. |
| `location` | 1-based `line` and `column` (in characters) of the call. |
| `effects` | The effects the callee declares with `uses`, plus `dangerous` for a dangerous tool. For an agent, branch or loop, every effect under it. Sorted. |
| `dangerous` | For a tool or connector operation, whether it is dangerous. For an approval, whether the tool it guards is. |
| `guards` | For an approval, the dangerous tool its label names (`IssueRefund` guards `issue_refund`, by the rule the checker uses). |
| `iterations` | For a loop, the most times it runs. Absent when nothing bounds it. |
| `worst_case` | Worst-case cost by dimension, as numbers (`cost` in US dollars), or `null` when a loop inside has no bound. |
| `steps` | The steps inside a nested agent, action or loop. |
| `paths` | A branch's paths, each a list of steps. An empty list is a path that does nothing. |

`never_reached` lists each tool, connector operation and prompt that no entry point's tree reaches, with its `kind`, `name` and declared `location`.

## Not in scope

- A run's actual path. `corvid trace lineage` draws that.
- Editing the graph. The code is the graph.
- Several files at once. The graph draws the agents of one file. A route, a schedule and a table operation are not yet steps of their own. The graph's G3 slice brings them in, with the dev console and B22 reading this document.
