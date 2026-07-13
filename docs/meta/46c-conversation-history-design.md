# 46c — First-class conversation history (design)

Status: DRAFT for review (pre-implementation, per the Phase 46
pre-phase agreement). Closes audit B7 second half.

## Problem

`std/ai.cor` declares `AiMessage { role, content }` and nothing
consumes it. Multi-turn programs — chat apps, agent loops — have no
way to pass prior turns into a prompt call. Every mainstream SDK
does this with a messages array; Corvid needs the same power with
less ceremony and with trace/replay/budget composition intact.

## Design

### Surface: history is a typed parameter, not new syntax

A prompt parameter typed `List<AiMessage>` IS the history surface:

```corvid
import "./std/ai" use AiMessage

prompt chat(history: List<AiMessage>, question: String) -> String uses llm_call:
    system: "You are a careful assistant."
    user: "{question}"

agent main() -> String:
    turns = [
        AiMessage("user", "What is Corvid?"),
        AiMessage("assistant", "An AI-native language."),
    ]
    return chat(turns, "Who makes it?")
```

Zero new syntax. The decision-principle reasoning: history is a
commodity concept — the invention budget goes to how it composes
with the moat (traces, replay, budgets), not to novel syntax. A
typed parameter is the Python-easy form, it works with every
existing prompt feature (routing, ensembles, sampling, streaming),
and the type system already carries `List<AiMessage>` everywhere.

### Recognition rule

The checker recognizes a param as history when its type is
`List<S>` where `S` is a struct named `AiMessage` whose fields are
exactly `role: String` and `content: String` (local or imported —
recognition is by name + shape, so the std/ai declaration and a
user's own compatible declaration both work).

- Multiple `List<AiMessage>` params: compile error ("one history
  parameter per prompt").
- A history param referenced as `{history}` in any template:
  compile error — history splices structurally; interpolating it
  as JSON is almost always a bug. The error says both things.
- Agents/tools/fns with `List<AiMessage>` params: unaffected —
  recognition applies only to prompt declarations.

### Splice position

Provider message order: all `system:` blocks first, then the
history messages in list order, then the prompt's non-system
blocks (the current turn). For a single-template prompt, history
precedes the template's user message. Roles inside history are
validated AT RUNTIME to be `user` / `assistant` / `system`
(anything else is a typed runtime error naming the index) —
system messages inside history are allowed and stay in place
(they follow the declaration's own system blocks).

### Canonical rendered string (46b's decision extends)

`rendered` remains the single source of truth: history renders
into the role-labeled concatenation in splice order. Traces, cache
fingerprints, token estimates, cites checks, and mock keying all
keep working with ZERO new trace schema. The history list also
rides `args` (it is an ordinary argument), so replay matching by
prompt + args is exact.

### Context-window policy

`model` declarations gain a `context_window: N` field (positive
integer, validated like the 46a sampling fields; lowered on
`IrModel`). At dispatch, with `budget = context_window −
(sampling.max_tokens or the completion estimate)`:

1. Estimate tokens of the full rendered request.
2. If over budget: drop history messages OLDEST-FIRST — whole
   messages only, never split; declaration system blocks and the
   current turn are never dropped.
3. If still over budget with all history dropped: typed runtime
   error (`context window exceeded`), not a silent provider 400.

Truncation is a pure function of (messages, estimates, budget) —
deterministic, so replay reproduces the same truncation from the
same trace. The trace records the truncated rendered string (the
request that was actually sent), which is exactly what replay
substitutes against. When no `context_window` is declared, no
truncation happens (the provider is the limit, as today).

### Explicit non-scope (v1)

- Tool-call turns in history (`ToolResultEnvelope`) — post-46f.
- `AiSession` stays an inert envelope; session persistence is the
  app's concern (or the durable queue's).
- Token-exact budgeting (the estimate heuristic stays; the
  `estimate_tokens` upgrade is a separate concern).
- Smarter eviction (summarization, importance weighting) — the
  oldest-first policy is the honest v1; anything smarter is an
  invention slice of its own.

## Implementation map

1. Checker: recognition rule + the two compile errors
   (`checker/prompt.rs`).
2. IR: `IrPrompt.history_param: Option<usize>` (param index);
   `IrModel.context_window`.
3. VM: splice in `render_messages`/`render_prompt` (history renders
   into both the structured messages and the canonical concat);
   truncation in the request build path (`cost.rs`), where the
   selected model's `IrModel` is already consulted for sampling.
4. Adapters: no changes — they already consume `messages`.
5. Tests: checker recognition + duplicate-param error +
   interpolation error; VM splice order test; truncation e2e
   (tiny declared window drops oldest turn deterministically);
   ch 11 history section + grammar note + drift evidence.
