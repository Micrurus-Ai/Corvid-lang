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

- [ ] G1. The cost tree records approvals and each node's source span, and marks `dangerous` tools; `corvid graph` prints the text form for one agent or every entry point, with the never-reached list.
- [ ] G2. `--format mermaid` and `--format json`, with the JSON shape documented and tested.
- [ ] G3. The dev console and B22's compiler-over-MCP read the JSON.

## Not in scope

- A run's actual path: `corvid trace lineage` draws that.
- Editing the graph: the code is the graph.
