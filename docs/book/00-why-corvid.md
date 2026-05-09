# Why Corvid

## The problem Corvid solves

Every team shipping AI is rebuilding the same five things by hand:

1. A way to require human approval before dangerous tool calls.
2. A way to track which information came from a retrieval source vs. a
   model hallucination.
3. A way to budget token cost, latency, and tool calls per run.
4. A way to replay an agent run when something goes wrong.
5. A way to tell whether a model upgrade silently broke a workflow.

These are not language features in Python or TypeScript. They are
middleware. Middleware drifts. The library you wrote in February doesn't
match the library you wrote in May, the library someone else wrote in
April doesn't match either, and at incident-response time you discover
that the safety check you thought you had only ran in 80% of code paths.

## What Corvid does instead

Corvid puts these five things in the language. The compiler is the
load-bearing safety surface. Every dangerous tool call carries an effect
row in its type. Every approval is a static token. Every retrieval-backed
value is `Grounded<T>` instead of `T`. Every prompt has a budget. Every
run produces a deterministic trace.

If a code path has a refund call without an approval, the binary does not
compile. Not "produces a warning." Not "fails a runtime check that you
hope someone wrote." Does not compile.

## The invariant in one sentence

> A program whose AI can do dangerous things without an explicit
> compile-time proof of authorization, grounding, and budget — does not
> exist as a Corvid program.

This is the moat. The compiler enforces it; the language teaches it; the
replay tooling proves it after the fact.

## What you give up

- Dynamic typing on agent surfaces. You write effect rows, you don't get
  to skip them.
- Implicit "the LLM will figure it out" code paths. If the model is doing
  a dangerous thing, you write the approval explicitly.
- One-language-fits-all. Corvid is good at AI orchestration; for tight
  numeric inner loops you call out to Rust or C through the FFI.

## What you get back

- A test suite that exercises real safety properties, not just happy paths.
- A replay tool that reproduces production incidents in milliseconds.
- A model-upgrade workflow where `corvid eval --swap-model gpt-5` tells
  you what changed before the change ships.
- An audit log that shows your security team exactly what the AI is
  authorized to do.
- A budget surface where compile-time-known cost ceilings are real
  numbers, not vibes.

## How to evaluate it

Read **[Quickstart](/docs/quickstart)** and run the five-minute walkthrough.
Then read **[Tutorial](/docs/tutorial)** and build a refund agent. If by
the end of the tutorial the compile-time approval refusal feels like
overkill, Corvid is wrong for your use case. If it feels like the thing
you have been hand-writing for the last year, you are home.
