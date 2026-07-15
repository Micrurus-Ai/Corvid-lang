# Slice 50i — Untrusted-content taint (injection defense) v1 design

Decided 2026-07-15, at slice time per the Phase 50 pre-phase
decision. The claim this slice earns: **prompt injection is a
compile error** — OWASP's #1 LLM risk answered structurally, not
with vibes and regex.

## The threat, precisely

Prompt injection is a confused-deputy attack: untrusted CONTENT
(a retrieved document, a user message, an untrusted MCP server's
output) embeds instructions; an LLM reading that content obeys
them; the obedient output then parameterizes an action the program
author never intended — a `dangerous` tool call with
attacker-chosen arguments. The defining property is that the
attack rides *data flow*, which is exactly what a type system can
see.

## The design: `Grounded<T>`'s machinery, inverted

Corvid already tracks a data property through call graphs:
`data: grounded` on an effect wraps returns in `Grounded<T>` at
every call site (`ground_if_effect_grounded`). Taint is the same
mechanism with inverted polarity and stricter flow rules:

1. **Sources.** An effect may declare `data: untrusted`. Any
   tool/prompt/agent whose effect row carries it returns
   `Tainted<T>` — declaration-driven, zero new syntax on the call.
   This is how retrieved documents, inbound user messages, and
   untrusted-MCP wrappers get marked: their *effects* say where the
   bytes come from.
2. **Contagion.**
   - `Tainted<T>` is **never** assignable to `T`. Unlike
     `Grounded<T>`'s legacy coercion, taint must not launder
     silently — that asymmetry is deliberate and load-bearing.
   - Binary operations with a tainted operand produce tainted
     results (`"prefix " + tainted_doc` is tainted).
   - **A prompt that consumes a tainted argument returns
     `Tainted<output>`.** This is the rule that models the actual
     attack: the LLM read attacker-controlled text, so its output
     is attacker-influenced. Prompts MAY consume tainted content —
     analyzing untrusted text is their job — but their outputs
     carry the mark.
3. **The sink rule.** A call that requires approval — a `dangerous`
   tool, or any effect at `supervisor_required`/`human_required`
   trust (the 47g derivation) — **refuses tainted arguments at
   compile time.** The diagnostic names the argument and the two
   honest ways out.
4. **The boundary.** `trusted(expr)` unwraps `Tainted<T>` to `T`.
   It is loud, greppable, and reviewable — the single place a human
   asserts "I have looked at how this value is constrained and I
   accept it." Typical shapes behind it: a `with judged` guard that
   scored the content, a refinement-constrained decode that forces
   the value into a known-safe shape, or an allowlist check. v2 can
   type those patterns; v1 makes the assertion explicit instead of
   impossible-to-see.

## What v1 deliberately does not do

- **No runtime representation.** Taint is compile-time only; at
  runtime a `Tainted<String>` IS a `String` (like erased generics).
  There is nothing to carry: the value's provenance story belongs
  to `Grounded<T>`; taint is a static permission, not data.
- **No classifier.** Content-based injection *detection* (is this
  text trying to jailbreak?) is a complementary runtime concern —
  the `with judged` guard already expresses it. The type system
  handles the flow property; the judge handles the content
  property.
- **Struct-field granularity.** v1 taints whole values. A struct
  decoded from tainted text is tainted as a unit; `trusted(...)`
  unwraps it as a unit.
- **Implicit sanitizer typing.** Recognizing "this judged guard
  makes the value trustworthy" without an explicit `trusted(...)`
  is v2; v1 keeps the human assertion visible.

## Surface summary

```corvid
effect web_content:
    data: untrusted

tool fetch_page(url: String) -> String uses web_content

effect send_money:
    trust: human_required
    reversible: false

tool pay(recipient: String, amount: Float) -> String dangerous uses send_money

agent assistant(url: String) -> String:
    page = fetch_page(url)              # page: Tainted<String>
    summary = summarize(page)           # prompt output: Tainted<String>
    # pay(summary, 100.0)               # COMPILE ERROR: tainted argument
    recipient = trusted(validate(summary))
    approve Pay(recipient, 100.0)
    return pay(recipient, 100.0)
```

## Enforcement inventory

- `Type::Tainted(T)` in the checker; `Tainted<T>` parses as a named
  generic (the `Grounded<T>` path).
- `taint_if_effect_untrusted` beside `ground_if_effect_grounded`
  for tool/prompt/agent returns.
- Prompt-argument contagion in the prompt call path.
- Binary-op contagion for tainted operands.
- Sink check inside the existing approve-requirement machinery
  (dangerous + trust-derived), new diagnostic
  `TaintedDangerousArgument` with the two-way help.
- `trusted(expr)` primary expression; lowering and the VM treat it
  as identity.
- Guarantee registry row `taint.untrusted_cannot_reach_dangerous`
  (Static) with positive + adversarial test refs.
