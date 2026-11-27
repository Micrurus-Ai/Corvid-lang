# `try ... else` in native code (recoverable failures TE-3)

**Status:** TE-3a to TE-3d built 2026-11-27. What remains is the widening on the roadmap (failing calls in `match` arms, guards and lambdas) and `timeout`. No grammar change: `try ... else` is already parsed, checked and run by the interpreter (TE-1, TE-2). This slice makes compiled code mean the same thing.
**Owner:** backend (Claude Code, Mac session).
**Roadmap:** `try ... else` TE-3 in [backend-ai-roadmap](../backend-ai-roadmap.md); parent proposal [recoverable failures](recoverable-failures.md).

## The problem

A native build refuses `try ... else` by name. The reason is not in the lowering of the form: compiled code has no way for a failure to come back. Every runtime entry point that meets a failure prints the message and calls `exit(1)`:

- `corvid_runtime_call`, which carries tool and connector calls, exits when the call fails, and when a tool's answer does not decode;
- prompts exit when the model call fails or its reply never fits after the repair attempts;
- table and database operations exit on a failed statement.

So a compiled program that should show "Search unavailable" stops instead, and the same program behaves differently on the two tiers.

## The design, in a realistic program

```corvid
table Shelf:
    key shelf: Int
    owner: String
    title: String

tool lookup_cover(isbn: String) -> String

agent cover_for(isbn: String) -> String:
    return lookup_cover(isbn)

agent shelf_titles(owner: String) -> List<String>:
    return [s.title for s in Shelf.where(fn (s) -> s.owner == owner)]

agent page(owner: String, isbn: String) -> String:
    titles = try shelf_titles(owner) else []
    cover = try cover_for(isbn) retry 2 else "no-cover.png"
    return cover + " " + titles.to_json()
```

The source is unchanged. What changes is that the native binary catches the outage two calls down (`lookup_cover` inside `cover_for`, the table read inside `shelf_titles`) exactly where the interpreter does, with the same rule.

## How an outage travels

1. **One rule decides what an outage is.** A runtime entry point that fails classifies its `RuntimeError` with `RuntimeError::is_outage`, the function the interpreter's `try` uses. A failure that is not an outage (a bug, a denied approval, a budget, a replay divergence) still stops the program with the same message as today. Nothing new can be caught.
2. **An outage is held, not printed.** On an outage the entry point stores the error in the runtime's pending-outage slot, and returns a status. It also returns zero bits for its result: NULL for a reference, which `corvid_release` already accepts, and 0 for a scalar.
3. **Only code that can fail checks.** Native lowering computes, over the program's call graph, which agents can reach a tool, a prompt, a table operation or a connector. An agent can fail if its body makes such a call, or calls an agent that can fail; the computation runs to a fixed point, so recursion is handled. After each call that can fail, the compiler emits one test of the pending slot. Pure code, and calls that cannot fail, get no test and run as fast as today. The IR carries no per-call effect summary, so this pass computes it, and a test holds it to the effect checker's view of the same program.
4. **Unwinding is an early return.** When the test finds an outage outside any `try` in the function, the function leaves the way `return` does: every owned local is released by the scope-exit drops the ownership pass already places, and it returns zero bits. Its caller checks in turn. There is no `longjmp` and no stack scanning, so the reference counts stay exact and the collector's trial deletion stays sound.
5. **`try` takes it.** Inside `try X else Y`, the test after `X` takes the pending error, clears the slot and evaluates `Y`. With `retry N`, an outage re-runs `X` up to `N` times, as the interpreter does, before the fallback.
6. **Nothing caught it.** If an outage reaches the generated `main`, it prints `corvid: <message>` and exits 1, which is the message and status a compiled program gives today. A program with no `try` behaves exactly as before.

### Recording and replay

The interpreter records a caught failure like the failure it replaces, so a replay takes the same fallback. The native entry points record through the same `Runtime` methods (tool calls already write their trace events there), and the held error carries its `outage` flag into `RuntimeError::Recorded` on replay. A replayed native run therefore takes the same branch as the recorded one. A parity test records on one tier and replays on the other.

## Alternatives considered

- **`setjmp`/`longjmp` to the nearest `try`.** It needs no per-call checks, but it skips the drops of every frame it jumps over. Those values leak, and because the collector treats a leaked count as an outside reference, it can never free them. Refused: exact reference counts are what make native memory correct.
- **Return a status from every compiled function.** That changes every native signature, the C ABI and every call site, for code that can never fail. The pending slot costs nothing where the call graph shows no outage is possible.
- **Catch only what appears directly inside the `try` expression.** Simpler, but `try helper() else ...`, the common case, would still stop the program, and the tiers would disagree. Refused as the design. It is also not offered as a temporary narrowing, because the full form is no larger to build.

## Across the tiers

- **Backend:** a compiled server or worker degrades instead of exiting, with the same rule the interpreter uses, so moving a program to native never changes which failures it survives.
- **Frontend:** a screen whose loader is compiled shows its fallback. The form's meaning does not depend on the tier the code runs on.
- **AI:** an assistant's tool call through a compiled action gets the declared fallback instead of an aborted run, and the trace records which failure was caught, so a person can see why the fallback appeared.

## Slices

- [x] TE-3a. The pending-outage slot and its status, in `corvid_runtime_call` (tools and connectors without `on failure`), classified by `RuntimeError::is_outage`. A failure that is not an outage keeps today's message and exit. Built 2026-11-27, in `ffi_bridge/outage.rs`. Holding is turned on only by a program with a `try` that catches natively, so every other program stops at the failing call exactly as before.
- [x] TE-3b. The compiler emits a test of the slot after each call that can fail, and unwinds through compiled agents by the early-return path. `try X else Y` and `try X retry N else Y` are lowered natively, and the native refusal is removed. Parity tests cover:
  - a tool outage two agent calls deep;
  - a fallback that itself calls a tool;
  - a non-outage that still stops the program;
  - no leak under the leak detector;
  - no change to a program without `try`.

  Built 2026-11-27. `outage_plan.rs` decides which calls can fail and which `try` forms native code catches exactly; native lowering and the tier picker both ask it. The expansion that turns each into ordinary `return`, `continue` and `break` is `dup_drop/outage.rs`, in the pass that already exposed `?` to ownership analysis. A test holds the expansion and the plan together.

  Narrowed: a `try` that can reach a failing call inside a `match` arm, a guard, a lambda, or a `parallel` or `approve` statement runs on the interpreter, refused by name, because the IR has no statement form there to unwind through. A call in those positions is checked by stopping the program, as native code always did.
- [x] TE-3c. Prompts report outages through the same slot. Built 2026-11-27. Table operations are not native yet (B09 P6); they will use the same slot when they are.
- [x] TE-3d. Record and replay, with a cross-tier replay test. Proved 2026-11-27 in `replay_native.rs`; it needed no code, because a replayed failure carries its `outage` flag into the pending slot.

## Not in scope

- `try ... timeout` in native code. That needs a cancellation point in the synchronous call path, and stays refused by name, as it is today.
- The fallback naming which failure happened (`else (failure) -> ...`). That waits on the failure value's type, as on the interpreter.
