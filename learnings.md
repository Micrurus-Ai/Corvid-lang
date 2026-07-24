# Corvid — learnings

> A pragmatic guide to what you can write in Corvid today, how to run it, and where the edges are. Grows with every slice that adds a user-visible feature. Cross-references to the dev-log where decisions were made.

This is the "how to actually use Corvid" document. For the pitch, see [README.md](README.md). For the full feature roadmap, [FEATURES.md](FEATURES.md). For architecture, [ARCHITECTURE.md](ARCHITECTURE.md). For the build journal, [dev-log.md](dev-log.md).

---

## Quick start

```bash
# Build the compiler
cargo install --path crates/corvid-cli

# Write your first program
cat > hello.cor <<'EOF'
agent main() -> Int:
    return 42
EOF

# Compile to a native binary (one .exe, no runtime installer)
corvid build --target=native hello.cor
./target/bin/hello     # prints: 42
```

Corvid is Python-shaped on the surface, with AI-native primitives on top. You already know how to read most of it.

---

## File structure

```
my_project/
├── corvid.toml          # project config (optional — not needed for single files)
├── src/
│   └── main.cor         # convention: put sources under src/
└── target/
    ├── bin/             # native binaries (from --target=native)
    ├── py/              # generated Python (from --target=python)
    └── trace/           # JSONL traces from runtime calls
```

Single `.cor` files compile fine without any project structure. The `corvid build` command creates `target/` alongside wherever the source lives.

---

## Types & values

Corvid has five scalar-and-composite value types shipping in the native compiler today.

### `Int` — 64-bit signed integer

```corvid
agent n() -> Int:
    return 42
```

Arithmetic (`+`, `-`, `*`, `/`, `%`) **traps on overflow**. `i64::MAX + 1` does not silently wrap — the binary exits with a runtime error:

```
corvid: runtime error: integer overflow or division by zero
```

Division and modulo by zero trap the same way. The safety story behind this is in [dev-log Day 19](dev-log.md).

If you want wrapping arithmetic (e.g., hash mixing), a `@wrapping` annotation is on the long-range roadmap.

### `Bool` — true / false

```corvid
agent is_even(n: Int) -> Bool:
    return n % 2 == 0
```

Stored as a single byte internally (`I8`). No truthy/falsy coercion — `if 0:` is a type error, not "it's falsy so skip." Bool is its own type.

`and` / `or` **short-circuit** on both the interpreter and the native binary. The right side is not evaluated if the left determines the answer:

```corvid
# This returns true — the divide-by-zero is skipped.
agent f() -> Bool:
    return true or (1 / 0 == 0)
```

Short-circuit semantics landed in [dev-log Day 20](dev-log.md).

### `Float` — 64-bit IEEE 754

```corvid
agent total(price: Float, quantity: Int) -> Float:
    return price * quantity
```

Follows **IEEE 754 semantics** — no traps:
- `1.0 / 0.0` returns `+Inf`
- `0.0 / 0.0` returns `NaN`
- `NaN == NaN` is `false`; `NaN != NaN` is `true`

Mixed `Int + Float` promotes to `Float` (same widening rule as Python). Why IEEE and not trap-on-divide? Float's design intent is that `Inf`/`NaN` ARE the safety mechanism — they propagate upstream errors without aborting. Design rationale in [dev-log Day 22](dev-log.md).

Int vs Float policy:
- Int traps — integer overflow is never the desired behavior.
- Float follows IEEE — `Inf`/`NaN` carry meaning.

### `String` — immutable UTF-8

```corvid
agent greet(name: String) -> String:
    return "hello, " + name

agent matches(a: String, b: String) -> Bool:
    return a == b
```

Operators: `+` (concat), `==`, `!=`, `<`, `<=`, `>`, `>=` (bytewise lexicographic — matches Unicode codepoint order for the BMP).

String literals live in `.rodata` and are **immortal** — retain/release on them are no-ops. Concatenated strings are heap-allocated and automatically freed when their last reference goes away (refcount reaches zero). No manual memory management.

What's NOT supported yet:
- String length (`len(s)` / `s.len`) — needs a `len` builtin mechanism, planned future work
- Indexing (`s[0]`) — planned future work
- Iteration (`for c in s`) — needs iterator protocol, future slice
- Slicing / case-folding / search — stdlib work

String semantics landed in [dev-log Day 24](dev-log.md); the memory model behind them is [dev-log Day 23](dev-log.md).

### `Struct` — user-declared records

Declare with `type`, construct with `TypeName(args)`, access fields with `.`:

```corvid
type Order:
    id: String
    amount: Float

type Ticket:
    message: String
    refund: Order

agent is_expensive(o: Order) -> Bool:
    return o.amount > 100.0

agent main() -> Bool:
    t = Ticket("damaged", Order("ord_42", 49.99))
    return t.refund.amount > 10.0
```

Field access can nest arbitrarily (`t.refund.amount`). Structs are **immutable** — there's no `s.foo = x` assignment; build a new struct instead.

Memory: each struct is a single heap allocation laid out as `[header | field0 | field1 | ...]` with 8 bytes per field. Structs with refcounted fields (Strings, nested Structs) get an auto-generated destructor that releases each refcounted field before the struct itself is freed. No leaks possible at the language level — the leak detector verifies this on every parity test.

Struct semantics + constructor syntax landed in [dev-log Day 25](dev-log.md).

### `List<T>` — ordered collections

Declare with a bracketed literal; index with `[i]`; iterate with `for`:

```corvid
agent sum() -> Int:
    total = 0
    for x in [1, 2, 3, 4, 5]:
        total = total + x
    return total      # 15

agent third_item() -> Int:
    xs = [10, 20, 30]
    return xs[1]      # 20

agent matches(needle: String, haystack: List[String]) -> Bool:
    for s in haystack:
        if s == needle:
            return true
    return false
```

Lists are **immutable** — no `.push` / `.append` / `list[i] = v`. Build a new list instead.

Memory: each list is one heap allocation laid out as `[header | length | element_0 | element_1 | ...]` with 8 bytes per element. Lists with refcounted elements (Strings, Structs, nested Lists) use a shared runtime destructor that walks the length and releases each element — the destructor doesn't need to know the element type because at the runtime level every refcounted element is an I64 pointer. Nested cleanup cascades naturally through each element's own header chain.

**Bounds checking is enforced at runtime.** `xs[5]` on a 3-element list traps with the same error path as integer overflow — exits non-zero with a stderr message. No silent out-of-range reads.

List semantics landed in [dev-log Day 26](dev-log.md).

---

## Local bindings

Python-style bare assignment. **No `let` keyword.**

```corvid
agent calc() -> Int:
    x = 10
    y = 20
    return x + y
```

Reassignment reuses the same binding:

```corvid
agent run() -> Int:
    total = 0
    total = total + 5
    total = total * 2
    return total     # 10
```

Type of a binding is inferred from the initial value. You can't change a binding's type via reassignment (the type checker enforces this).

### Scoping

Bindings introduced inside `if` / `else` branches are **not visible after** the branch — they belong to that branch's scope:

```corvid
agent run(flag: Bool) -> Int:
    if flag:
        x = 1
    # x is not accessible here — return would error.
    return 0
```

If you want a binding visible after an `if`, declare it before:

```corvid
agent run(flag: Bool) -> Int:
    x = 0
    if flag:
        x = 1
    return x
```

Local binding semantics landed in [dev-log Day 21](dev-log.md).

---

## Control flow

### `if` / `else`

Statement-only (not an expression). Both branches can return or fall through:

```corvid
agent classify(n: Int) -> Int:
    if n > 0:
        return 1
    else:
        if n < 0:
            return -1
        else:
            return 0
```

### `pass`

No-op statement. Useful as a placeholder in an empty branch:

```corvid
agent noop_check(x: Int) -> Int:
    if x > 0:
        pass     # we know x is positive; nothing to do here
    return x
```

### `for` / `break` / `continue`

Iterate over a list with `for x in list:`. Escape with `break`; skip to the next iteration with `continue`:

```corvid
agent first_even(xs: List[Int]) -> Int:
    for x in xs:
        if x % 2 == 0:
            return x
        continue         # explicit — `continue` is also fine
    return 0             # no even number found

agent sum_until_negative(xs: List[Int]) -> Int:
    total = 0
    for x in xs:
        if x < 0:
            break
        total = total + x
    return total
```

`break` and `continue` respect the nearest enclosing loop. Loop variables (`x` above) are typed to the list's element type automatically — no need to write `x: Int`.

Loop control landed in [dev-log Day 26](dev-log.md).

---

## Agents

Every Corvid program is a collection of `agent` declarations. Agents have typed parameters and a typed return, and they can call each other:

```corvid
agent double(n: Int) -> Int:
    return n * 2

agent quadruple(n: Int) -> Int:
    return double(double(n))

agent main() -> Int:
    return quadruple(5)   # 20
```

Recursion works. Mutual recursion works. The `main` agent (when one exists) is the entry point for `corvid build --target=native`; if there's only one agent, it's the entry by default.

### Entry agent restrictions (native compile only)

`corvid build --target=native` currently requires the entry agent to:

- Take **no parameters** — argv decoding lands in slice 12i.
- Return `Int` or `Bool` — Float / String / Struct returns land in slice 12i (the C shim needs a print-format variant).

Non-entry agents have none of these restrictions. A program can compose any types internally; only the outermost entry is constrained.

Interpreter path (`corvid run`) has no such restrictions — use it when you need to drive real runtime calls (tools, prompts, approvals).

---

## Tools, prompts, approvals — AI-native primitives

Corvid's AI surface uses three keywords: `tool`, `prompt`, `approve`.

### `tool` — external operations

```corvid
tool get_order(id: String) -> Order
tool issue_refund(id: String, amount: Float) -> Receipt dangerous
```

Tools have typed parameters and returns, no body — they're implemented in the host (Python or Rust runtime). The `dangerous` keyword marks tools that can't run without a prior `approve`.

### `prompt` — typed LLM calls

```corvid
prompt decide_refund(ticket: Ticket, order: Order) -> Decision:
    """
    Decide whether this ticket deserves a refund.
    Consider the order amount, the user's complaint, and fairness.
    """
```

Prompts route through a registered LLM adapter (Anthropic, OpenAI) and return a typed struct. The model is instructed to emit structured output matching the return type's schema — no string parsing at the caller.

### `approve` — compile-time safety

```corvid
agent refund_bot(ticket: Ticket) -> Decision:
    order = get_order(ticket.order_id)
    decision = decide_refund(ticket, order)

    if decision.should_refund:
        approve IssueRefund(order.id, order.amount)   # <-- required
        issue_refund(order.id, order.amount)           # <-- dangerous tool call

    return decision
```

Remove the `approve` line and the program **will not compile**:

```
[E0101] error: dangerous tool `issue_refund` called without a prior `approve`
```

This is the killer feature. Enforced at compile time, not runtime. Works in both `corvid run` (interpreter) and `corvid build --target=python`.

For a served application, the route also declares the complete decision
policy. Corvid deliberately has no default reviewer, expiry, risk class,
data class, cost ceiling, or reversibility:

```corvid
identity users:
    provider google
    provisioning:
        first_login: invited
        tenant: from_invitation
    roles:
        finance_reviewer: "approvals.decide"

server payments:
    @approval(role: "finance_reviewer", risk: "financial_transfer", data: "financial", expires_ms: 600000, max_cost_usd: $2500.0, irreversible: true)
    route POST "/payments" body Payment -> json Receipt requires authenticated:
        return submit_payment(body)
```

All six `@approval` fields are mandatory. The route must be
authenticated, the named role must be declared, and some declared role
must grant `approvals.decide`. At runtime the requester and tenant come
from the verified session; the other six values come from this compiled
policy. A reviewer must hold the exact named role and the permission,
must be in the same tenant, and cannot approve their own request.

### Current compilation gap

`corvid build --target=native` doesn't yet wire tool / prompt / approve calls into compiled code — native tool dispatch is provided by a proc-macro `#[tool]` registry. For now, AI-shaped programs run via:

```bash
corvid run refund_bot.cor                     # interpreter, fully native runtime
corvid build --target=python refund_bot.cor   # generates .py you can run with the Python runtime
```

The interpreter path has the full AI runtime (Anthropic + OpenAI adapters, approval flow, tracing, `.env` loading, secret redaction). See the demo: `cargo run -p refund_bot_demo`.

---

## Compilation targets

### `--target=native` (default when slice 12j lands)

```bash
corvid build --target=native src/program.cor
# → target/bin/program[.exe]
```

One binary, no runtime installer, statically linked. Uses Cranelift for codegen; the system C toolchain for linking (`cl.exe` on Windows, `cc` elsewhere).

Supports: Int / Bool / Float / String / Struct + all operators + `if`-`else` + local bindings + agent-to-agent calls.

### `--target=python`

```bash
corvid build --target=python src/program.cor
# → target/py/program.py
```

Generates runnable Python. The `corvid-runtime` Python package provides `tool_call` / `approve_gate` / `llm_call`. Useful when you want to deploy into an existing Python environment or stack.

### `corvid run` (interpreter)

```bash
corvid run src/program.cor
```

Executes via the Rust tree-walking interpreter in `corvid-vm`. Full AI runtime available (tools, prompts, approvals, tracing). Use this for day-to-day development and for AI-shaped programs until the native AI runtime path is complete.

---

## Error model

### Compile-time errors

- **Type errors** (wrong operand types, missing fields, etc.) — reported with line + column + fix hint via ariadne rendering.
- **Effect errors** (`E0101` — dangerous tool called without approve) — the headline safety check.
- **Resolution errors** (undefined names, duplicate declarations).

Every error has a code (`E0001`–`E0302`) and a suggested fix. See [ARCHITECTURE.md §8](ARCHITECTURE.md#L336) for the design target.

### `pub extern "c"` boundary types (33Q8)

A `pub extern "c"` agent exported in a `corvid build --target=cdylib`
library can now take and return user-declared structs whose fields
are all `Int` / `Float` / `Bool` / `String`. The struct travels the C
ABI as a JSON-encoded `const char*` buffer; the generated `.h`
embeds the JSON schema as a block comment above the agent's C
signature so a C caller knows the exact shape without reading the
`.cor` source. Returned buffers are freed with `corvid_free_string(...)`.
Nested-struct / list / option fields still need follow-up FFI work
and are rejected at typecheck time with a hint pointing at
`docs/reference/exported-abi.md`.


### Compile-time warnings

Some declarations parse + typecheck cleanly but won't execute on v1.0 because a runner slice is still pending. `corvid check` surfaces these as yellow `warning:` blocks BEFORE the success line so they can't be missed:

- **`W0280` — `schedule` declarations preserve but don't fire.** `schedule "0 9 * * *" zone "..." -> agent(...)` lowers to IR cleanly so the post-v1.0 scheduler runner can wire it up later, but in v1.0 the cron is not yet attached to the runtime tick loop. The warning names the agent and cron so a reviewer doesn't ship a daily summary they think is running.

A clean check with warnings prints `ok: file.cor — no errors (N warning(s) above)` and exits 0 — warnings are signal, not failure.


### Runtime errors (native binaries)

A compiled program can fail at runtime for:
- **Integer overflow / division by zero** — prints to stderr, exits 1.

That's currently the full list. Approval denial, tool failures, and LLM failures only apply to the interpreter/Python paths until the native dispatch path is complete.

### Runtime errors (interpreter)

- Arithmetic (overflow, div-zero)
- Type mismatch (belt-and-braces for typechecker bypass)
- Index out of bounds
- Missing field on struct
- Tool dispatch failed / unknown tool
- Approval denied
- LLM adapter failed / no model configured

All carry a source span so the error points at the offending code.

---

## Memory model

**Refcounted heap with automatic cleanup.** You never write `free` and you never see a leak.

- Every non-scalar value (String, Struct, eventually List) lives behind a 16-byte header: `atomic refcount + reserved`.
- Static literals have refcount = `i64::MIN` (immortal) — retain/release on them are no-ops.
- Structs with refcounted fields (Strings etc.) get an auto-generated destructor that releases each refcounted field when the struct is freed.
- Refcount updates are atomic — single-threaded today, but future multi-agent work won't need a migration.

### Leak verification

Every parity test runs the compiled binary with `CORVID_DEBUG_ALLOC=1`:

```bash
CORVID_DEBUG_ALLOC=1 ./target/bin/program
# → program output on stdout
# → stderr: ALLOCS=3\nRELEASES=3
```

The test suite asserts `ALLOCS == RELEASES` on every fixture. Any codegen bug that drops a release would fail the test immediately with the exact delta. As of dev-log Day 25, all 66 parity fixtures pass the leak check.

### When it matters for you

For short-lived programs (agents that run once and exit), refcount overhead is invisible. For long-running services (future RAG servers and multi-agent coordinators), the leak-free guarantee means a Corvid service can run for days/weeks without memory growth. Memory-management design rationale: [dev-log Day 23](dev-log.md) (foundation) and [dev-log Day 24](dev-log.md) (ownership wiring).

---

## Gotchas

### No `let` keyword

Python-style bare assignment. `let x = 5` is a parse error.

```corvid
# Wrong
let x = 5

# Right
x = 5
```

### No `if` as expression

`if` is a statement, not an expression. `x = if cond: 1 else: 2` doesn't parse. Use either a separate `if`/`else` writing to a pre-declared variable, or call helper agents.

### No string interpolation (yet)

`f"hello {name}"` doesn't exist. Use `"hello " + name`. String templating inside `prompt` bodies works differently — see the refund_bot demo.

### No `len` / indexing on strings yet

`s.len` and `s[0]` aren't supported yet. Planned future work.

### `for c in string` not yet in native code

Compiles via the interpreter; raises `NotSupported` in the native compiler. The fix is either a shared iterator protocol or a String-specific lowering path — neither is on the immediate roadmap. Use `for x in list` when you're writing native-targeted code.

### Writing tools in Rust

Native tool dispatch ships with a typed C ABI. Users write tool implementations in a Rust crate, decorate them with `#[tool("name")]`, build the crate as a staticlib, and pass the resulting `.lib` / `.a` to `corvid run --with-tools-lib <path>` or `corvid build --target=native --with-tools-lib <path>`.

Example tool crate:

```toml
# Cargo.toml
[package]
name = "my_tools"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["staticlib"]

[dependencies]
corvid-runtime = { path = "../path/to/corvid/crates/corvid-runtime" }
corvid-macros  = { path = "../path/to/corvid/crates/corvid-macros" }
tokio          = { version = "1", features = ["full"] }
```

```rust
// src/lib.rs
use corvid_macros::tool;

#[tool("get_order")]
async fn get_order(id: String) -> String {
    // call your DB, an HTTP API, anything
    format!("order: {id}")
}

#[tool("issue_refund")]
async fn issue_refund(order_id: String, amount: f64) -> i64 {
    // returns the refund ID
    42
}
```

Build + run:

```bash
cd my_tools
cargo build --release    # produces target/release/libmy_tools.a (or .lib on Windows)

cd ../my_corvid_app
corvid run main.cor --with-tools-lib ../my_tools/target/release/libmy_tools.a
```

Currently supported tool signatures (scalars only; Struct/List support comes later):

| Corvid type | Rust type   |
|-------------|-------------|
| `Int`       | `i64`       |
| `Bool`      | `bool`      |
| `Float`     | `f64`       |
| `String`    | `String`    |

Tools must be `async fn`. Wrap a sync body in `async { ... }` if you don't need to await anything. The tool function name in `#[tool("...")]` matches the Corvid `tool` declaration's name.

Without `--with-tools-lib`, programs that call user tools fall back to the interpreter (auto) or error out (`--target=native`). The interpreter tier needs tool implementations registered separately via `Runtime::builder().tool(...)` in a runner binary — that pattern is unchanged.

### Methods on types (`extend T:` blocks)

Methods attach to user-declared types via `extend T:` blocks. Methods can be ANY declaration kind — agent, prompt, or tool — and all dispatch through the same dot-syntax. The receiver is an explicit first parameter (no `self` keyword); the typechecker and IR rewrite `value.method(args)` into a regular call with the receiver prepended.

```corvid
type Order:
    amount: Int
    tax: Int

extend Order:
    public agent total(o: Order) -> Int:
        return o.amount + o.tax

    public prompt summarize(o: Order) -> String:
        "Summarize this order: amount {o.amount}, tax {o.tax}"

    public tool fetch_status(o: Order) -> Status dangerous

    agent compute_internal(o: Order) -> Int:    # private (default)
        return o.amount / 10
```

Call sites:

```corvid
agent process(o: Order) -> Int:
    t = o.total()              # pure agent call
    pitch = o.summarize()      # LLM dispatch through the native prompt bridge
    s = o.fetch_status()       # tool dispatch through the native tool bridge
    return t
```

Visibility:
- Default is **private** — callable only from code in the same file.
- `public` makes the method callable from anywhere the type is visible.
- `public(package)` reserves package-scoped visibility for future package-manager work; syntactically accepted now so user code doesn't need re-annotation later.
- `public(effect: ...)` is the syntactic slot reserved for future effect-scoped visibility.

Method-name rules:
- Two methods with the same name on the same type → compile error.
- A method whose name collides with a field on the same type → compile error.
- Methods with the same name on different types coexist (`Order.total`, `Line.total`).
- Methods on built-in types (Int, String, List) defer to later work to avoid orphan-rule complexity.

### Performance — when native wins

The first native-runtime close shipped with published numbers (ARCHITECTURE.md §18). End-to-end wall-clock on three representative workloads:

| Workload | Interpreter | Native | Ratio |
|---|---|---|---|
| 500k Int arithmetic ops | 256 ms | 19 ms | 13.6× |
| 50k String concatenations | 48 ms | 18 ms | 2.7× |
| 100k struct alloc + field reads | 74 ms | 21 ms | 3.5× |

**Spawn-tax crossover:** on Windows, every `corvid run` in native mode pays ~11 ms of OS-level process-spawn cost. For programs whose interpreter run-time is under ~5 ms, that tax outweighs the codegen speedup and interpreter wins end-to-end. Above ~20 ms of interpreter compute, native wins decisively. In between, measure.

Auto dispatch (`corvid run` default) still picks native for tool-free programs because the compile cache makes re-runs near-instant and real agent workloads exceed the crossover. Override with `--target=interpreter` for tiny programs where the spawn tax matters.

Reproduce locally: `cargo bench -p corvid-codegen-cl --bench native_foundation_benchmarks`.

### Running Corvid code

`corvid run <file>` picks the right execution tier automatically:

- **Native AOT** when the program uses only native-able features (arithmetic, Bool, Float, String, struct, list, agent-to-agent calls). First run compiles and caches; subsequent runs of the same source skip codegen entirely (≈15× faster). Cache lives at `<project>/target/cache/native/<hash>[.exe]` and is swept by `cargo clean`.
- **Interpreter** when the program uses anything that needs the async runtime (tool calls, prompt calls, `approve`, `import python`). Auto-fallback announces itself with one stderr line naming the specific construct and the native feature gap:

```
↻ running via interpreter: program calls prompt `greet` — native prompt dispatch is not available on this path yet
```

Explicit overrides:

```bash
corvid run foo.cor                         # auto (default)
corvid run foo.cor --target=native         # require native; error if not possible
corvid run foo.cor --target=interpreter    # force interpreter, even when native works
```

Use `--target=native` when you want to catch a regression the moment a change introduces an un-native-able feature. Use `--target=interpreter` when you need traces, the mock LLM runtime, or tool handlers that the native tier can't load yet.

### Entry agent constraints

Native compile accepts **scalar** entry agents: parameters and return may each be `Int`, `Bool`, `Float`, or `String`. `Struct` and `List` at the entry boundary still raise `NotSupported` — they need a serialization slice before they can round-trip through argv / stdout meaningfully. Wrap a composite-taking agent in a thin `main` that parses a `String`:

```corvid
# Native-compile-friendly — argv[1] becomes `name`.
agent greet(name: String) -> String:
    return "hi " + name

# Multi-arg entry — argv[1..3] become a, b.
agent sum(a: Int, b: Int) -> Int:
    return a + b
```

Invoking the binary:

```bash
corvid build greet.cor --target=native
./target/bin/greet world            # prints: hi world
./target/bin/sum 10 32              # prints: 42
```

Format rules the codegen-emitted `main` uses:
- `Bool` on the command line is `true` / `false` (case-sensitive). Result printing matches.
- `Float` is decoded with libc `strtod` and printed with `%.17g` (round-trippable).
- `Int` is decoded with `strtoll`; overflow or non-numeric input exits non-zero with a slice-specific error (not the overflow handler).
- `String` is taken verbatim from argv (UTF-8 pass-through — shells handle quoting).
- Arity mismatch (wrong number of argv args) exits non-zero with a clear message before the agent runs.

### No multi-threading

Corvid is single-threaded today. Atomic refcount is cheap insurance for future multi-agent coordinators.

---

## What's on the near-term roadmap

Per [ROADMAP.md](ROADMAP.md):

- **Cycle collection on top of refcount** — backstops the refcount runtime against reference cycles using a stop-the-world mark-sweep collector triggered by allocation pressure.
- **Polish, benchmarks, and stability guarantees**
- **Compiled-code tool / prompt / approve support** — proc-macro `#[tool]` registry and native AI dispatch.
- **Effect-tagged `import python "..."`** — TypeScript `.d.ts` analog.
- **More LLM adapters** — Google + Ollama alongside Anthropic + OpenAI.
- **Typed `Result` + retry policies**
- **Streaming, cost budgets, uncertainty types, replay as a language primitive, and arithmetic annotations**
- **Multi-agent composition + durable execution**

Features earn their place through real pull, not speculation. Adding something to the roadmap requires a proposal in `dev-log.md` per the rules in [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Feature Log

Each user-visible feature lands with a dev-log entry explaining the design decisions. Cross-references:

| Feature | Dev-log |
|---|---|
| Int arithmetic + overflow trap | [Day 19](dev-log.md) |
| Bool, comparisons, `if`/`else`, short-circuit | [Day 20](dev-log.md) |
| Local bindings + reassignment + `pass` | [Day 21](dev-log.md) |
| Float + IEEE 754 semantics | [Day 22](dev-log.md) |
| Memory management foundation (refcount + leak detector) | [Day 23](dev-log.md) |
| String operations + ownership wiring | [Day 24](dev-log.md) |
| Struct + constructors + destructors | [Day 25](dev-log.md) |
| List + `for` + `break` / `continue` | [Day 26](dev-log.md) |
| Parameterised entry agents + Float/String entry returns | [Day 27](dev-log.md) |
| Native as the default tier for tool-free programs + compile cache | [Day 28](dev-log.md) |
| Native-runtime close-out benchmarks: native is 2.7×–13.6× faster end-to-end | [Day 29](dev-log.md) |
| Tokio + corvid runtime embedded in compiled binaries; narrow native tool dispatch | [Day 30](dev-log.md) |
| `#[tool]` proc-macro + typed C ABI dispatch + `--with-tools-lib` | [Day 31](dev-log.md) |
| Native prompt dispatch + 5 LLM provider adapters (Anthropic / OpenAI / OpenAI-compat / Ollama / Gemini) | [Day 32](dev-log.md) |
| Methods on types (`extend T:` blocks, mixed agent/prompt/tool, public visibility) | [Day 33](dev-log.md) |
| Typed heap headers + per-type typeinfo + non-atomic refcount | [Day 16](dev-log.md) |
| Cranelift safepoints + emitted stack-map table | [Day 25](dev-log.md) |
| Cycle collector — mark-sweep over the refcount heap | [Day 26](dev-log.md) |
| Replay-deterministic GC trigger log + shadow-count refcount verifier with PC blame | [Day 27](dev-log.md) |

---

## Typed Heap Headers (what it means for users)

**Nothing to change in your Corvid code.** This is infrastructure for the cycle collector and the effect-typed memory model. It's behavior-preserving end-to-end — all 105 codegen parity tests pass unchanged.

What changed under the hood:

- **Every refcounted allocation now carries a per-type metadata pointer** (`corvid_typeinfo`) in its 16-byte header. The collector (17d) and the dump/debug tooling (later) both dispatch through this block rather than hardcoding per-type knowledge in the runtime.
- **Refcount is no longer atomic.** Corvid is single-threaded, so the atomic ops were paying a per-retain/release cost (~10-50× vs non-atomic on x86) for a multi-threaded scenario that doesn't exist yet. Future multi-agent work will bring a proper multi-threaded RC design — biased RC or deferred RC, not blanket atomics.
- **`List<Int>`-style primitive lists no longer mis-trace.** The old design couldn't tell at trace time whether a list held pointers or integers; the new typeinfo's `elem_typeinfo = NULL` sentinel is explicit. Compiled programs with `List<Int>` now carry a typeinfo that says "don't chase these slots."
- **Refcount bit-packing.** Top bits of the refcount word are reserved for the cycle collector's mark/color state (17d, 17h). Retain/release preserve those bits under an externally-set mark — pinned by a new runtime test.

What becomes possible next:

- The effect-typed memory model: most allocations bump-allocate in a per-scope arena driven by static escape analysis; the compiler elides RC ops entirely on provably-unique values (Perceus-style); in-place reuse converts functional-style updates into bump-free mutations.
- The cycle collector dispatches through each object's typeinfo during the mark phase. No per-type switch in the collector.
- `Weak<T>` slots into the typeinfo's reserved `weak_fn` field.

## Cycle Collector (what it means for users)

**Nothing to change in your Corvid code.** This closes the memory-foundation correctness promise: refcount handles the acyclic case in the fast path; a stop-the-world mark-sweep collector reclaims unreachable cycles.

What changed under the hood:

- **Hidden tracking-node prefix** before every refcounted allocation. The user-visible 16-byte header (refcount + typeinfo) is unchanged; the runtime now allocates a 24-byte prefix in front of it that links every live block into a global doubly-linked list. Static-literal codegen is untouched — the prefix is invisible to anything that reads through the public `corvid_alloc_typed` interface.
- **Mark phase walks the RBP chain.** Cranelift's `preserve_frame_pointers` flag is now on, so every Corvid-compiled frame has a standard `[rbp+0]=prev_rbp, rbp+8=return_pc` layout. The collector chases that chain, looks up each return PC in `corvid_stack_maps` (emitted by 17c), and marks every refcounted pointer at the recorded SP-relative offsets.
- **Two-pass sweep.** Pass 1 traces every unmarked block's children with a decrement-only marker so refcount bookkeeping stays consistent for any reachable children that an unreachable block referenced. Pass 2 frees the unmarked blocks and clears mark bits on survivors. The split avoids `destroy_fn` recursion during collection.
- **Allocation-pressure trigger.** `corvid_alloc_typed` fires the collector every `CORVID_GC_TRIGGER` allocations (default 10_000, set via env var; `0` disables auto-GC). Tests use `corvid_gc_from_roots` for deterministic, stack-walk-free invocation.

How to interact with it:

- `CORVID_GC_TRIGGER=N` — fire automatic GC every N allocations. Set to `0` to disable.
- `CORVID_DEBUG_ALLOC=1` — print alloc/release counters at exit (existing knob, still works).

## Refcount Verifier + GC Trigger Log (what it means for users)

This is Corvid-specific infrastructure that turns the cycle collector into **a runtime checker for the ownership optimizer (17b)**. Every GC cycle, the verifier traverses the reachable graph, computes the expected refcount of each block from its incoming edges, and diffs against the actual refcount. Drift means a miscompile.

How to use it:

- `CORVID_GC_VERIFY=warn` — verifier runs each GC cycle, prints a drift report to stderr if anything diverges, execution continues. Recommended for CI.
- `CORVID_GC_VERIFY=abort` — same, but `abort()` on any drift. Recommended for fuzzing / bug-hunting.
- `CORVID_GC_VERIFY=off` (default) — verifier skipped. Zero cost on the fast path.

Drift reports include:

```
CORVID_GC_VERIFY: refcount drift
  block:           0x... typeinfo=<name>
  expected_rc:     <count from reachability>
  actual_rc:       <count from refcount word>
  diagnosis:       under-count (missing retain; UAF risk) | over-count (missing release; leak)
  last_retain_pc:  <PC of most recent corvid_retain on this block>
  last_release_pc: <PC of most recent corvid_release, or 0 if never released>
```

The blame PCs are stamped by `corvid_retain` / `corvid_release` via compiler return-address intrinsics — they cost a single store on the already-dirty cache line, no observable overhead in the fast path.

The slice also lays the foundation for **replay-deterministic GC**:

- Every GC cycle appends a record to a trigger log: `(alloc_count, safepoint_count, cycle_index)`.
- A new `corvid_safepoint_count` global plus `corvid_safepoint_notify()` C entry are exposed for codegen / latency-aware triggers (17b-7) to drive collection at compiler-invariant points.
- Replay infrastructure can read the log via `corvid_gc_trigger_log_length` / `corvid_gc_trigger_log_at` accessors and replay GC at the same logical points across runs, even if the optimizer changes allocation patterns.

What this gets Corvid:

1. The ownership optimizer (17b) is runtime-verified on every program you run with `VERIFY=1`. No other refcount language ships this — they don't have the typed-graph traversal infrastructure to do it cheaply.
2. Refcount miscompilations carry source-locating blame instead of presenting as silent corruption later.
3. GC trigger points are explicit data the runtime exposes, not a hidden side-effect of allocation pressure — which is what makes replay-time reproduction possible.

## REPL And Replay (how to use it)

Corvid now has an interactive REPL:

```bash
corvid repl
```

The REPL keeps successful declarations and locals across turns. That means you can declare a type, construct a value later, and inspect fields after that in separate inputs.

Example:

```text
>>> type Point:
...     x: Int
...     y: Int
...
>>> p = Point(1, 2)
>>> p.x
2
```

### Multi-line input

If the first line of a turn ends with `:`, the REPL enters multi-line mode and keeps reading with the `... ` prompt until you submit a blank line.

Use this for:

- `type` declarations
- `extend T:` blocks
- multi-line `if` / `for`
- multi-line `try ... on error retry ...` expressions or statements

### What persists across turns

Successful turns commit.
Failed turns roll back.

That applies to:

- declarations
- top-level locals
- the top-level type environment

So a parse/type/runtime error in turn `N` does not poison the session state from turns `1..N-1`.

### Result / Option / `?` / retry in the REPL

The `Result` / `Option` / retry surfaces work directly in `corvid repl`:

```text
>>> Ok(Some("hi"))
Ok(Some("hi"))
```

```text
>>> try flaky_call() on error retry 3 times backoff linear 250
...
```

The REPL prints expression results with type-aware rendering for:

- `Result`
- `Option`
- `Struct`
- `List`
- `String`

Recursive composite values are guarded:

- repeated structural revisits print as `<cycle>`
- overly deep recursion prints as `<...>`

### History and shell behavior

- `Ctrl-D` exits cleanly
- `Ctrl-C` cancels the current in-flight turn
- history persists across sessions

History file location:

- Unix: `$XDG_DATA_HOME/corvid/history`
- Unix fallback: `~/.local/share/corvid/history`
- Windows: `%APPDATA%\\corvid\\history`

### Replay stepping

The REPL can load an existing JSONL runtime trace and step through it:

```text
>>> :replay target/trace/run-1713199999999.jsonl
loaded replay `target/trace/run-1713199999999.jsonl` [run run-1713199999999]: 5 step(s), 70 ms, final status: OK
```

Replay commands:

- `:step` or `:s` advances one recorded step
- bare `Enter` in replay mode also advances one step
- `:step N` advances `N` steps
- `:run` plays to the end
- `:show` reprints the current step
- `:where` shows the current position
- `:quit` or `:q` leaves replay mode and returns to the normal REPL

Replay output shows the recorded data, not reconstructed guesses:

- run start inputs
- tool call args and recorded results
- LLM prompt name, model, rendered prompt text, args, and recorded result
- approval request args and recorded decision
- final run result or error

If a trace is incomplete, replay reports `TRUNCATED` and still shows the recorded prefix. If the file is malformed or not a valid Corvid trace, the REPL prints a clear error and stays in normal mode.

## Weak References

Corvid now has first-class weak references with effect-typed invalidation.

### Basic syntax

`Weak<T>` means "a weak reference to `T`, with runtime checks only."

```corvid
agent cache(name: String) -> Weak<String>:
    return Weak::new(name)
```

`Weak<T, {effects}>` is the powerful form. The effect row says which effects may invalidate the checker’s proof that the weak is still fresh.

```corvid
agent cache(name: String) -> Weak<String, {tool_call, llm}>:
    return Weak::new(name)
```

Supported weak-effect names today:

- `tool_call`
- `llm`
- `approve`

### Construction and upgrade

```corvid
agent load(name: String) -> Option<String>:
    w = Weak::new(name)
    return Weak::upgrade(w)
```

`Weak::new(...)` refreshes the weak at the current effect frontier.

`Weak::upgrade(...)` returns `Option<T>`:

- `Some(value)` if the strong target is still alive
- `None` if the target has been cleared

### What the checker proves

The checker tracks whether an invalidating effect may have happened since the weak was last refreshed.

This is accepted:

```corvid
agent make(name: String) -> Weak<String, {tool_call}>:
    return Weak::new(name)

agent load(name: String) -> Option<String>:
    w = make(name)
    return Weak::upgrade(w)
```

This is rejected:

```corvid
tool fetch_name(id: String) -> String

agent make(name: String) -> Weak<String, {tool_call}>:
    return Weak::new(name)

agent load(name: String) -> Option<String>:
    w = make(name)
    fetch_name(name)
    return Weak::upgrade(w)
```

Why: `tool_call` is in the weak’s effect row, and there was no intervening refresh before the upgrade.

### Refresh rules

- `Weak::new(strong)` refreshes at the current effect frontier.
- successful `Weak::upgrade(w)` refreshes `w` at the current effect frontier.
- at control-flow merges, a weak is considered refreshed only if **all** incoming paths refreshed it.

This merge is therefore rejected:

```corvid
tool fetch_name(id: String) -> String

agent make(name: String) -> Weak<String, {tool_call}>:
    return Weak::new(name)

agent load(flag: Bool, name: String) -> Option<String>:
    w = make(name)
    if flag:
        Weak::upgrade(w)
    else:
        keep = name
    fetch_name(name)
    return Weak::upgrade(w)
```

One path refreshed `w`, one path did not, so after the merge the checker keeps the weaker fact.

### Runtime guarantees

On the native runtime:

- weak slots clear when the strong target’s refcount reaches zero
- weak slots also clear during GC sweep of unreachable cycles
- clearing happens before destroy-time re-entrancy can observe a stale pointer

The direct runtime weak tests live in:

- `crates/corvid-runtime/tests/weak.rs`

Current native parity coverage proves the live-upgrade path. A stronger source-level overwrite/drop parity case is still under audit in the native codegen / ownership interaction.

## VM Heap Handles

VM Heap Handles moved the interpreter's cycle-capable values onto VM-owned retain/release metadata in preparation for Bacon-Rajan cycle collection.

What changed:

- `Struct`, `List`, and boxed `Result` / `OptionSome` payloads no longer rely only on raw `Arc` clone/drop semantics for their Corvid-level lifetime.
- the interpreter now owns explicit retain/release accounting for those graph nodes
- native and VM heaps are still completely separate implementations; they only need to agree behaviourally

Important boundary:

- `String` stays a leaf `Arc<str>` in 17h.1
- if a future string-like value ever gains an outgoing refcounted edge
  (for example a rope node or a parent-backed string view), it must move
  onto a VM heap handle and participate in Bacon-Rajan like any other
  graph node

Why that boundary is honest:

- strings are heap values, but not cycle-forming graph nodes
- Bacon-Rajan needs ownership over the graph edges, and those live in struct/list/boxed payloads, not in leaf strings

Practical implication:

- this commit is the prerequisite plumbing for VM cycle collection, not the collector itself
- Bacon-Rajan lands on top of these VM-owned graph handles in 17h.2

## VM Cycle Collection

VM Cycle Collection adds Bacon-Rajan trial deletion to the interpreter tier.

What is collected:

- VM-owned graph nodes: `Struct`, `List`, `ResultOk`, `ResultErr`, and `OptionSome`

What is not collected by Bacon-Rajan:

- leaf `String` values, because they still have no outgoing refcounted edges

Trigger model:

- the VM buffers possible cycle roots when a graph node's strong count drops but does not hit zero
- collection runs explicitly via `corvid_vm::collect_cycles()`
- auto-collection uses the roots-buffer threshold from `CORVID_VM_GC_TRIGGER`
- `CORVID_VM_GC_TRIGGER=0` disables the auto trigger

Parity model:

- native and VM heaps are still separate implementations
- parity is asserted by tests, not by sharing allocator/runtime code
- current cycle parity is synthetic heap parity, not source-level parity, because Corvid source still cannot mutate fields to construct a cycle directly

## Memory Foundation Retrospective

This is where Corvid stopped being "a language with some RC plumbing" and became a language with a measurable memory story.

What users get now that they did not have before:

- typed native heap objects with per-type metadata
- native cycle collection for unreachable refcount cycles
- interpreter-tier cycle collection, so REPL and replay do not quietly diverge from native semantics
- weak references with checker-enforced effect invalidation rules
- replay-deterministic GC trigger logging
- runtime ownership verification with blame PCs

### What the measurements currently support

These numbers are the current pre-`.6d-2` baseline from `cargo bench -p corvid-runtime --bench memory_runtime ...`. The final foundation lock reruns the same harness after the unified ownership-pass cleanup lands.

| Claim | Supporting number |
|---|---|
| Native fixed-size allocation is now a real competitive strength | `tight_box_alloc`: about `30.6 ns/alloc` hot, `37.9 ns/alloc` after deterministic cold-cache preload |
| Native cycle collection scales cleanly with heap size | mark-sweep stays around `13–17 ns/node` across the current pooled runtime path |
| Runtime ownership verification is no longer prohibitively expensive | verifier `warn/off`: about `1.22x` on `tight_box_alloc`, `1.26x` on `string_heavy_concat`; list-heavy is noise in the current run |
| Future ownership optimizations still have a real baseline to beat | isolated RC ops: about `4.85–5.30 ns` for retain/release and `3.82 ns` for a retain-release pair in the current harness |

### Why this matters for Corvid's positioning

Corvid's moat is not "faster than Rust at everything." The stronger claim is narrower and better:

- replay-deterministic execution
- low audit cost
- ownership verification tied to the runtime's real heap graph

This is the first point where that claim has a measured baseline instead of architecture prose.

### What still depends on the optimization wave

The optimization wave should move the numbers from "credible" to "competitive":

- the unified ownership pass
- pair elimination
- drop specialization
- effect-row-directed RC
- latency-aware RC across tool / LLM boundaries

Those slices matter because they attack the measured overhead directly, not abstractly.

### Pair elimination, first cut

Slice `17b-1c` adds the first explicit retain/release pair-elimination pass to native codegen.

What it does today:

- looks only at same-block `Dup` / `Drop` pairs inserted by the ownership pass
- removes the pair when one safe internal use sits between them and nothing else touches the local
- refuses to pair across branches, loops, agent/tool/prompt/unknown calls, or weak-reference creation

Why the scope is narrow on purpose:

- it is sound without needing to reopen the active CFG/dataflow files
- it documents the safepoint argument explicitly, so GC behavior stays auditable
- it gives Corvid a real ARC-style optimization stage without pretending the broad SSA version already exists

Important measurement note:

- the current `baseline_rc_counts` workloads do not yet expose a same-block removable pair
- so this slice lands with a benchmark-shaped proof fixture and a no-op result on the current published baselines
- the honest next step is to rerun after `.6d-2b` and extend the RC-count suite with a workload that actually exercises pair pressure

### Effect-typed scope reduction

Slice `17e` adds the first effect-aware ownership optimization pass in native codegen.

What it does today:

- looks at `Drop` placement after the unified ownership pass and pair elimination
- classifies statements as either effect-free or effect barriers using a codegen-local sidecar keyed by `IrPath`
- moves `Drop` earlier only when the path between the defining `Let` and the existing `Drop` stays within one straight-line block and crosses no effect barrier

What counts as effect-free in the current slice:

- literal-producing expressions
- local reads
- unary arithmetic
- binary arithmetic

What counts as a barrier:

- all calls
- `approve`
- `if` / `for`
- `return`
- `break` / `continue`
- `Dup` / `Drop`

Why the slice matters:

- it is the first ownership optimization that uses Corvid's effect-awareness rather than only plain liveness
- it shrinks RC-alive windows without pretending the full interprocedural effect-row story already exists

Important honesty note:

- the first post-17e benchmark rerun showed a full-sheet slowdown, including primitive-only paths
- that is treated as environment noise until proven otherwise and is not folded into the published foundation numbers
- 17e ships on correctness first; measurement delta is held until a clean rerun

### Latency-aware RC at prompt boundaries

Slice `17b-7` narrowed a broad intuition into a precise optimization target.

The original hypothesis was "AI boundaries are expensive; optimize tool/LLM boundaries." The implementation work showed that this was too coarse:

- borrowed-local tool args were already close to flat after the unified ownership pass became default-on
- the remaining boundary RC traffic lives in prompt / LLM interpolation, specifically when a borrowed local `String` is threaded through prompt rendering

The shipped pass therefore does one thing on purpose:

- pin borrowed local `String` bindings across prompt lowering so the concat path does not mistakenly release the binding's structural `+1`

What it does **not** do:

- no runtime deferred-RC ledger
- no verifier bookkeeping change
- no attempt to optimize prompt-internal owned temps
- no claim that tool-only workflows materially move from this slice

That is a useful lesson from the memory-foundation work: the moat claim has to follow the measured hotspot, not the broader story we hoped would be true. For Corvid, the differentiated boundary optimization is prompt / LLM lowering, not generic tool dispatch.

### Comparative benchmark runners

The memory-foundation close does not stop at internal microbenchmarks.

Corvid now has a shared AI-workflow fixture set plus three benchmark runner surfaces:

- native Corvid under `benches/corvid/`
- stdlib Python under `benches/python/`
- Node/TypeScript under `benches/typescript/`

The key rule is fixed across all three:

- orchestration overhead equals measured wall time minus fixture-declared external wait

That matters because it keeps the claim honest. Corvid is trying to beat orchestration stacks assembled from libraries, not the network. The benchmark suite therefore measures what the runtimes contribute around prompt, tool, approval, retry, and trace boundaries rather than celebrating whichever runner happened to sleep less.

### Clean-run gate discipline

The first close-out rerun for `memory_runtime` was intentionally archived instead of published.

Why:

- the machine produced runs that disagreed across the full sheet, including one run that passed the primitive-control sentinel while still diverging materially elsewhere
- that is exactly the kind of result that looks tempting in a slide deck and is poisonous in a reproducible benchmark story

So the rule for the close-out is now explicit:

- preserve noisy runs as artifacts
- document the rejection reason
- do not promote them into the published results table until the session clears the quiet-host gate

### Same-session ratio publication

The published close-out numbers use a stricter rule than the earlier absolute microbenchmarks:

- run Corvid, Python, and TypeScript back-to-back in one interleaved session
- subtract fixture-declared external wait from wall time on every trial
- publish ratios and confidence intervals, not absolute milliseconds

That choice matters because this host was not quiet enough to support honest absolute timing claims. The published archive under `benches/results/2026-04-16-ratio-session/` therefore says one precise thing:

- Corvid is slower than both Python and TypeScript on the current comparative runners, and every reported confidence interval stays above `1.0`

That is not flattering, but it is the right close-out claim. The value of the slice is methodological:

- the cross-language benchmark surface is now real
- the subtraction rule is fixed
- the ratio archive is reproducible
- future optimization work has a defensible comparative baseline instead of an aspirational claim

### Internal timing and benchmark-path reductions

The first honest comparative sessions still left one asymmetry in place:

- Python and TypeScript reported in-process trial elapsed time
- Corvid was still paying parent-runner stdin/stdout transport around each
  measured trial

That mismatch is now removed.

The current runner discipline for Corvid native is:

- keep the persistent native process
- measure `wall_ms` inside the launched native benchmark process from trial
  start to trial completion
- subtract actual measured external wait

The measured Corvid path also now avoids benchmark-only overhead that was not
part of the workload itself:

- disabled tracing skips event construction entirely
- trace writes are buffered instead of flushed on every event
- fixture tools use direct typed wrappers
- mock prompt calls avoid decoding and concatenating strings that the mock path
  never consumes

Current same-session result:

- Corvid is faster than the current Python and TypeScript benchmark runners on
  the four shipped workflow fixtures

That statement is intentionally narrow:

- it is a ratio-only result on a noisy host
- it applies to the shipped fixture workloads, not every possible orchestration
  workload
- it is the correct developer and marketing claim only because the earlier
  harness artifacts are now explicitly archived and superseded, not erased

### Compile-time constant prompt rendering

Native prompt lowering now folds a prompt call down to one immortal string
literal when every interpolated argument is a compile-time string / int / bool
literal.

What that means in practice:

- no runtime stringify for those arguments
- no runtime concat chain for the rendered prompt
- the native binary calls the prompt bridge with one pre-rendered literal
  instead of rebuilding the same text every trial

Why it matters:

- several shipped benchmark workflows contain constant prompt calls
- after the internal-timing alignment, those prompt rebuilds were one of the
  clearest remaining avoidable costs
- the new same-session session improves again on both Python and TypeScript,
  especially on the more prompt-heavy workflows

### Professional naming in source

Source code now uses behavioral names rather than roadmap numbering.

What changed:

- benchmark targets use names like `memory_runtime` and `native_foundation_benchmarks`
- inline comments and public API docs describe the behavior directly
- roadmap / slice terminology stays in planning and retrospective documents, not in compiler or runtime source

Why it matters:

- code should read on its own merits
- public API docs should describe the feature, not the project-management history behind it
- roadmap identifiers still exist where they are useful: `ROADMAP.md`, `dev-log.md`, `learnings.md`, and the close-out / deferral docs

### Residual native hot-path profiling

Once startup cost, wait-accounting bias, and benchmark-path overhead were out
of the way, the remaining native orchestration cost turned out to be much
smaller than the earlier investigation suggested.

What the residual profiling slice found:

- the remaining benchmark-path orchestration bucket is already sub-millisecond
  on all four shipped workflows
- bridge / string-conversion work is the largest named remaining component
- prompt rendering, mock dispatch, and release-path time are all small in
  absolute terms
- the unattributed remainder is still a large share of the now-tiny total, but
  only a few hundredths of a millisecond in absolute terms

Why it matters:

- this is the point where micro-optimization stops being the obvious next move
- if we chase another benchmark-only win, the bridge path is the only sensible
  near-term target
- otherwise the correct engineering decision is to move on, because the
  residual cost is no longer large enough to dominate the shipped workflow
  fixtures

### Scalar prompt bridge fast path

The residual profile correctly identified the bridge / string-conversion path
as the last named benchmark bucket worth attacking on the shipped fixtures.

What changed:

- scalar prompt returns under the shipped env-mock path now bypass the generic
  prompt bridge and parse directly from a borrowed queued reply
- profiling-off runs cache the profiling guard state instead of checking the
  environment on every hot-path call

Why it matters:

- this is a real measured-path reduction, not another harness rewrite
- it improves all four shipped workflow scenarios again after the
  constant-prompt pass
- once the residual bucket is already tiny, the winning optimization is often
  removing a whole layer of generic machinery rather than shaving a few
  instructions inside it

### Immortal fixture-string reuse

Once the scalar prompt bridge was out of the way, the remaining fixture-path
overhead lived in ownership churn on canned prompt and tool replies.

What changed:

- repeated env-mock prompt replies are now interned to immortal
  `CorvidString` values
- benchmark tool replies use the same immortal-string path
- the shipped workflow fixtures therefore stop paying per-use release/free work
  on repeated canned replies

Why it matters:

- this is the kind of micro-optimization that is only worth doing once the
  benchmark path is already very small
- it confirms that the remaining hot-path work was in bridge ownership, not in
  prompt rendering itself
- it strengthens the fixture-scoped benchmark claim again without changing the
  measurement methodology

### RC/GC tuning assessment

Once the benchmark-path orchestration cost became small, the remaining question
was whether refcounting or the native cycle collector would become the next
obvious bottleneck under heavier allocation pressure. The answer is "not yet."
The stress matrix stays linear through `100000` releases per trial, the
ownership pass still suppresses retains to `0`, the default GC cadence remains
reasonable on the immediate-release shape, and the native cycle collector
handles `10000` mutual-reference pairs without a pathological spike. That
means RC/GC tuning is not the next roadmap lever; the evidence says to move on
to codegen quality / hot-loop analysis instead of spending another slice on
collector micro-tuning.

### Codegen quality / hot-loop assessment

The right time to do machine-code investigation is when the workload actually
contains a hot loop. The shipped benchmark fixtures do not: they are short
prompt/tool orchestration sequences. The native build is already using
optimized settings (`opt_level = "speed"`, release `opt-level = 3`, thin LTO),
and representative disassembly of the shipped binaries shows dense bridge/helper
call sequences rather than a compute-heavy loop body. That means codegen
quality is not the next benchmark lever for the current workflow sheet.
Machine-code tuning should be revisited when Corvid adds compute-heavy
benchmarks, not treated as the default next step just because other obvious
bottlenecks have already been removed.

### Native nullable `Option<T>` subset

The first honest native step for the Result/Option/retry family was not the
whole feature set. It was the subset the backend already represented cleanly:
nullable-pointer `Option<T>` where `T` is already a refcounted native payload.

What changed:

- the driver's native-ability scan now accepts `Option<String>` / similar
  nullable-pointer payloads
- wide tagged-union shapes like `Option<Int>` still reject cleanly
- parity coverage now proves helper agents can return `Option<String>` and
  wrapper agents can compare the result against `None`

Why it matters:

- this moves real native capability forward without pretending `Result`, `?`,
  or retry are already done
- it confirms the right strategy for the broader feature wave: land the
  genuinely supported subset first, then widen from proven machinery
- the slice also flushed out a real runtime-link contract bug
  (`corvid_bench_tool_wait_ns` missing from the FFI bridge), which is exactly
  why capability work needs end-to-end parity coverage and not just scan tests

### Native nullable `Option<T>` `?` propagation

Once native nullable `Option<T>` existed as `pointer-or-null`, the next sound
step was not more constructors. It was control flow: make postfix `?` work on
that exact representation and no more.

What changed:

- native codegen now treats `Option<T>?` as a null check plus early return when
  the enclosing function also returns a native nullable `Option<_>`
- the early-return path uses the same live-local cleanup walk as explicit
  `return`, so the new control-flow form stays ownership-correct
- native-ability accepts that subset and parity tests prove both `Some` and
  `None` propagation through helper agents

Why it matters:

- it turns native nullable `Option<T>` into a real internal control-flow type
  instead of a value-only curiosity
- it proves the broader feature wave should keep following the same pattern:
  widen from an already-proven runtime representation rather than trying to
  land `Result`, `?`, and retry as one opaque monolith
- it preserves the no-shortcuts rule: the slice still refuses `Result<T, E>`
  and retry until their layouts and control flow exist for real

### Native one-word `Result<T, E>` subset

The honest way to add native `Result<T, E>` was not "declare the whole feature
done." It was to pick a representation the backend can actually own today and
land that end to end. Corvid now lowers one-word `Result<T, E>` shapes as
typed heap wrappers with a fixed `[tag | payload-slot]` layout, plus emitted
destructor/trace/typeinfo metadata so RC and cycle collection see them as real
heap objects rather than a codegen special case. The first test pass exposed
the load-bearing integration point: the unified ownership analysis still
classified `Result<T, E>` as non-refcounted, so result locals leaked even
though the wrapper codegen was otherwise correct. Fixing the analysis, not
papering over the leak in codegen, was the right move. The resulting native
subset is credible: construction works, same-shape `?` propagation works, and
the feature participates in the existing ownership/runtime model instead of
bypassing it.

### Native `Result<A, E>?` to `Result<B, E>`

The next real step after same-shape `Result<T, E>?` was not a bigger wrapper
layout. It was the standard propagation rule users actually expect:
`Result<A, E>?` inside a function returning `Result<B, E>`. Corvid now does
that by rebuilding only the `Err` wrapper on the early-return path. That is
the important design point: the payload representation was already good enough;
the missing piece was a principled ownership-preserving conversion path between
two concrete result wrappers with the same error type. The widening slice
confirmed the same lesson again: once the representation is sound, the hard
part is preserving ownership and cleanup invariants during control flow, not
inventing more layout machinery.

### Native `try ... retry` for `Result<T, E>`

The honest first native retry slice was not "retry anything that can fail."
Compiled Corvid cannot catch process-level traps the way the interpreter can
catch `InterpError`, so the sound AOT subset is retry over the recoverable
native `Result<T, E>` path. Corvid now lowers that subset as explicit native
control flow: evaluate the body, branch on the result tag, release failed
wrappers before the next attempt, compute a deterministic linear/exponential
delay from the source backoff policy, sleep, and re-enter the body. That is
the right shape for future widening because it keeps retry as compiled control
flow rather than hiding it in one opaque runtime helper. The slice also
reconfirmed the testing rule: compile acceptance was not enough. The feature
needed queued mock replies to prove the native tier actually performed multiple
attempts and returned the final `Err` value without silently propagating or
leaking between attempts.

### Native wide scalar `Option<T>`

The honest next widening step after native nullable `Option<String>` was not
to pretend every `Option<T>` is already cheap and native. Corvid now supports
wide scalar `Option<Int>`, `Option<Bool>`, and `Option<Float>` by giving
`Some(...)` a tiny typed heap wrapper while keeping `None` as the zero pointer.
That matters because it preserves the same ownership and collector story as the
rest of the native runtime: the value is a real heap object with typeinfo when
it needs storage, not a codegen-only special case. The slice also exposed a
real generic bug that had been latent before: non-string binary ops were not
releasing refcounted operands after comparison/arithmetic. Wide `Option<T>`
surfaced that immediately through `value != None`, and fixing the generic
lowering path was the right move. The lesson is the same as the earlier native
`Result` work: widening support safely depends less on inventing clever
representations and more on making every new representation participate in the
existing ownership model without exceptions.

### Compositional native tagged unions

The next honest question after landing native `Result<T, E>`, wide scalar
`Option<T>`, and native retry was whether those pieces actually compose or only
work as isolated leaf features. Corvid now has explicit coverage proving that
`Result<Option<Int>, String>` works natively through construction, postfix `?`,
and deterministic retry without any new runtime machinery. That is an important
signal: the current one-word tagged-union representation is not just "barely
enough for the demo cases." It is compositional inside the subset it claims to
support. The lesson is strategic as much as technical: once representation,
typeinfo, ownership, and cleanup invariants are sound, widening support should
first look for shapes that naturally compose out of those primitives before
adding new special-case encodings.

### Wider native `Option<T>?` propagation

The next useful native widening was not a new runtime object shape at all. It
was removing an artificial restriction in `Option<T>?` propagation. Corvid now
lets `Option<T>?` early-return into any native `Option<U>` envelope, because
the `None` path does not care what the eventual `Some(...)` payload type is.
That matters because it keeps the widening semantic rather than representational:
the runtime already knew how to represent these options, and the control-flow
rule was simply narrower than the underlying model required. The same slice also
confirmed that retry composes one step further than the earlier minimal proof:
retrying a native `Result<A, E>` and then using `?` into `Result<B, E>` works
without new runtime machinery. That is the pattern to keep following. Widen the
native subset first where the representation already composes cleanly, then add
new encodings only when a real semantic need remains.

### Native option envelopes and retry composition

The next confirmation after widening `Option<T>?` was whether that broader rule
still held up when mixed with native retry and widened `Result` propagation. It
does. A retried native `Result<String, String>` can now flow through `?` into
`Result<Bool, String>` without new runtime machinery, and `Option<T>?` can
early-return `None` into any native `Option<U>` envelope because the control
flow only needs the envelope's `None` representation. That is the deeper rule:
when a branch of the semantics is payload-agnostic, Corvid should not keep a
same-shape restriction just because it was easier to implement first. The right
pattern is to prove the broader semantic rule once the representation and
ownership model are already strong enough to carry it.

### Structured native `Result` payloads can already ride the current subset

The next honest widening question was whether native `Result<T, E>` really only
handled leaf payloads, or whether the current subset already carried structured
payloads and simply lacked proof. The answer is the latter. Corvid now has
explicit native coverage for `Result<Boxed, String>` and
`Result<List<Int>, String>`, including postfix `?`, without any new runtime or
codegen machinery. The only thing blocking the list case was a frontend bug:
`List` had dropped out of the resolver's built-in generic heads, so
`Result<List<Int>, String>` died before native lowering ever ran. Fixing that
was the right move because it exposed the real property of the system: the
existing one-word native `Result<T, E>` subset already composes with structured
heap-backed payloads that participate in the ownership model. The lesson is to
prefer semantic proof over premature representation work. Before inventing a
new layout, first ask whether the current representation already supports the
broader case and simply lacks tests or a clean frontend path.

### Nested native `Result` payloads should be proven before new layouts are invented

The next meaningful widening after `Result<Struct, String>` and
`Result<List<Int>, String>` was not another leaf payload. It was whether native
`Result<T, E>` still behaved coherently when one side was itself another native
`Result`. Corvid now has explicit proof for nested ok-payloads
(`Result<Result<Int, String>, String>`) and nested error payloads
(`Result<Int, Result<String, Bool>>`), including widened postfix `?` where the
enclosing function changes the ok type but preserves the nested error shape. No
runtime change was needed. That matters because it says the current wrapper,
typeinfo, and ownership model are not just good enough for isolated examples;
they compose one level deeper without new machinery. The lesson is strategic:
before inventing a broader native tagged-union layout, first prove how far the
existing one already goes under realistic composition.

### Build unblocks should complete an unfinished front-end path, not paper over it

This same slice also hit a front-end problem unrelated to native lowering: the
lexer and AST already knew about `effect` declarations, `uses` clauses,
`@constraint(...)`, and cost literals, but the parser still had missing and
duplicate method paths for them. The right fix was to finish that parser path
coherently, not to stub around it just enough for one test to compile. Once the
parser, keyword tests, and declaration recovery all agreed on the same syntax,
the native work could continue without dragging a half-wired front-end branch
forward. The lesson is simple: when an unblock reveals a subsystem that is only
partially switched over, complete that subsystem to one internally consistent
state instead of layering more local exceptions on top.

### Retry should follow Corvid's actual failure carriers, not a narrower implementation subset

Once native `Result<T, E>`, native `Option<T>`, and postfix `?` were all
shipped, retry remaining `Result`-only was too narrow. In the language model
Corvid already exposed, both `Err(...)` and `None` are first-class "did not
produce a usable value" branches. Corvid now makes that explicit: the
typechecker accepts `try ... on error retry ...` only on `Result<T, E>` and
`Option<T>`, the interpreter retries on `Err(...)` and `None`, and native AOT
does the same for the shipped `Option<T>` subset. The lesson is that widening
Phase 18 correctly is often about aligning semantics across the language,
interpreter, and native tier, not just adding one more native representation
case. If a construct is already a language-level failure carrier, retry policy
should treat it coherently across both tiers.

### Prod traces are a regression suite the moment the harness can dispatch them

Corvid now treats every recorded `.jsonl` trace under a directory as a
regression test: `corvid test --from-traces <dir> --from-traces-source <file>`
loads + schema-validates each trace, applies the coverage filters
(`--only-dangerous`, `--only-prompt`, `--only-tool`, `--since`, `--replay-model`,
`--flake-detect`), and dispatches each surviving trace through the regression
harness. Exit code 0 means every trace still behaves the way production
behaved; exit code 1 flags drift. `--promote` now closes the loop: on a TTY it
prompts per divergence and atomically rewrites the golden trace when accepted;
in non-interactive pipelines it fails closed with a one-time warning. The
lesson is that a CLI that only previews the plan is half a feature. Phase 21's
invention is that *production traffic is the test suite*, and that only becomes
real when the CLI actually runs the traces against the current binary, prints a
per-trace verdict, and — when behavior genuinely changed for the better — lets
the operator promote the current run to the new golden instead of having them
re-record by hand.

### The flagship PR-review tool is itself a Corvid program

Corvid now ships `corvid trace-diff <base-sha> <head-sha> <path>`, a
git-integrated behavior-diff tool whose reviewer agent — the piece that
walks two ABI descriptors and emits a markdown PR behavior receipt — is
written in Corvid, not Rust. The `.cor` source lives at
`crates/corvid-cli/src/trace_diff/reviewer.cor`, is embedded into the
CLI binary via `include_str!`, and compiles + runs through the
interpreter on every invocation. The reviewer is `@deterministic`
(byte-identical receipts across reruns), declares its own
`AgentSummary` and `Descriptor` types that the Rust CLI coerces into
via `json_to_value`, and owns the diff logic itself — Rust is
plumbing (git, compile, descriptor extraction) but the "what changed,
and how do we render it" logic is the Corvid agent. That matters
because Corvid's thesis is that AI-native governance is a first-class
programming domain with compile-time guarantees; shipping the flagship
governance tool in the host language would have softened the thesis
the same way Python shipping its linter in bash would. Writing it in
Corvid forced one honest early finding about language scope: there's
no `Float→String` primitive today, which is why slice 1 omits cost
deltas from the receipt (the reviewer will surface cost changes once
the language grows one). Dogfooding surfaces language gaps by
construction, which is exactly the v1.0 polish loop we want.

### ABI-descriptor-as-behavior-surface gives PR review a principled scope

The receipt compares exactly the `pub extern "c"` exported surface and
its transitive closure — the same scope 22-B's `emit_abi` uses for the
ABI descriptor. That is not an arbitrary cut. The exported surface is
what a host actually consumes; it is also what users audit in a PR
because it is the contract that leaves the compilation unit. Private
helpers change often and don't change the host's view; comparing them
would produce noisy receipts that cry wolf. Keeping the scope aligned
with the ABI descriptor means a Corvid program's reviewable surface is
exactly the surface a host relies on, which is the principled answer
to "what should PR-level behavior-diff show." The lesson: when
inventing a review tool, anchor its scope to a pre-existing, defensible
boundary the rest of the system already respects — not a new boundary
invented for the tool.

### Jest-snapshot promotion needs a fresh-run driver helper, not a replay-with-different-flags

A `--promote` implementation that only adjusted replay's substitution knobs
would have been a shortcut. Promotion is semantically the opposite of replay:
replay *substitutes* recorded responses to verify the current code still
produces them; promote *ignores* recorded responses so a fresh run under the
current code + real adapters can overwrite the golden. The right shape is a
sibling driver helper (`run_fresh_from_source_async`) that extracts the trace's
agent + args, compiles the current source, builds a runtime with
`.trace_to(emit_dir)` and *no* replay configuration, runs the agent, and
returns the emitted trace path. The harness then atomically swaps the old
golden for the new one when the operator accepts the divergence. The lesson
generalises: when two behaviours look like knobs on the same pipeline but have
inverse semantics, shipping them as sibling helpers beats shipping them as
modes of one helper. The codepaths read cleanly, the tests cover each property
in isolation, and a future reader doesn't hunt through a single helper to
work out which branch handles which story.

### A sync CLI wrapping an async runner needs two driver helpers, not a nested block_on

The regression harness raises async runner requests (one per trace, each async
because replay dispatches through mock + real LLM adapters). The CLI is
fundamentally sync — `anyhow::Result<u8>` — so it wants to call
`tokio::Runtime::block_on` once at the top. But the runner closure inside the
harness is itself async and dispatches into the replay orchestrator, which was
originally a sync function that did its own `block_on` internally. Nesting a
`block_on` inside another `block_on` panics. The fix that stays honest is to
split the driver helper into a sync wrapper and an async variant
(`run_replay_from_source_with_builder` + `_async`), push all runtime
construction up to the CLI boundary, and let every level below the CLI stay
async. The lesson is that "just call `block_on` again" is a shortcut with a
runtime-panic price tag; if a crate offers a sync helper that other callers
rely on, the answer is to expose the async variant alongside it, not to thread
runtimes through function bodies.

### Visibility-before-imports: ship the rule first, then the mechanism

`lang-pub-toplevel` extended `public` / `public(package)` to
top-level `type` / `tool` / `prompt` / `agent` declarations —
private-by-default, backward-compatible with every existing
single-file program. The rule shipped **before** the mechanism
that makes it load-bearing (`lang-cor-imports-basic`).

Why that order: when imports land, every existing `.cor` file
needs to decide which of its declarations are importable. If we
shipped imports first, the entire ecosystem would be implicitly
public until each file was migrated — exactly the default-public
regret Python has lived with for 30 years. Shipping the rule
first means every file defaults to the right answer (private);
library authors opt in to `public` intentionally when they want
something importable.

The lesson generalises: when adding a language feature whose
semantics depend on a classifier (public/private, safe/unsafe,
pure/effectful), ship the classifier first with the conservative
default, then ship the mechanism that makes the classifier
load-bearing. Users migrate into intentional choices instead of
migrating out of an accidental anti-pattern.

### Honest names are load-bearing before algebraic composition

`21-inv-H-5-schema-fix` was a five-line pre-slice to `-stacked`:
rename `agent.approval.tier_weakened:` to `...tier_changed:` (same
for `reversibility_weakened:` → `...reversibility_changed:`),
bump `RECEIPT_SCHEMA_VERSION` 1 → 2. The delta emitter had always
fired on *any* transition — strengthenings were shipping under a
key called "weakened." The policy layer parsed the `from->to`
suffix so the gate behaved correctly, but the key name lied about
what it represented.

That lie was fine in isolation — no consumer of a single-PR
receipt was harmed by a misleading key. It becomes load-bearing
the moment you try to *compose* receipts. A stack receipt reasons
about the net algebra of N per-commit deltas; the first question
the composer asks is "does `tier_weakened:A→B` cancel with
`tier_weakened:B→A`?" The honest answer is yes (they're a
round-trip). But with the old names, the composer would be saying
"two weakenings cancel," which reads as nonsense and bakes the
naming bug permanently into the algebra's explanation of itself.

Fix the names *before* piling composition semantics on top. The
hardest rule — no shortcuts — includes naming shortcuts taken in
earlier slices, even when the shipped behavior is correct. The
trigger to look was the design question "do strengthenings emit a
delta?" asked in the middle of pre-phase chat for `-stacked`.
Without that audit, `-stacked` would have inherited the lie.

Schema bumps are load-bearing signal, not ceremony. v2 tells bots
pattern-matching on `agent.approval.tier_weakened:` prefixes:
*audit your matchers.* Without the bump, consumers would silently
stop matching anything after the rename. With it, they get a
clear "schema changed" signal they can pin on.

### CI-specific formats are renderers over the canonical Receipt

`21-inv-H-5-gitlab` added `--format=gitlab` in a single commit —
~165 lines of renderer + 6 integration tests — because the
canonical `Receipt` struct H-5 chose was already the source of
truth. Every CI-specific format is "translate deltas + the
verdict into the shape this CI expects"; no pipeline rewiring.
GitLab's MR widget pulls findings from CodeClimate-compatible
JSON (`artifacts.reports.codequality`), so the renderer emits
exactly that shape and disappears into GitLab's native surface.

Three small choices carry the weight:

- **Severity maps one-to-one from the default policy's verdict.**
  Regressions → `major`, everything else → `info`. We picked
  `major` (not `blocker`) so the MR widget surfaces findings
  without overriding the MR's own merge gate — the policy's
  non-zero exit code is what actually blocks merges; severity is
  for signal, not enforcement. The five-level CodeClimate scale
  leaves room to split regressions further in a future slice.
- **Fingerprint = hex-SHA256 of the canonical delta key.** This
  is the GitLab-specific answer to the same "byte-stable across
  runs" principle H-5-json chose with deterministic ordering.
  GitLab dedupes MR-widget issues by fingerprint; if fingerprints
  drift between re-runs, reviewers see phantom "new" findings on
  every push. The fingerprint_stays_stable_across_runs test is
  the regression guard that would catch a drift at commit time.
- **`GITLAB_CI=true` auto-selects the renderer.** `--format=auto`
  now has two CI-platform branches (GitHub Actions →
  `github-check`, GitLab CI → `gitlab`) before falling through to
  the pipe/tty default. Users drop `corvid trace-diff ...` into
  a GitLab job without touching `--format`; the CLI reads
  environment and does the right thing.

The same pattern extends trivially to any future CI surface
(CircleCI, Buildkite annotations, Azure Pipelines): add a
renderer, add an env-var branch in `detect_from_environment`,
done. The canonical Receipt absorbs the new integration as
another translation target rather than a new pipeline.

### in-toto integration: the attestation ecosystem is a free composition

`21-inv-H-5-in-toto` shipped in one commit because H-5-signed
already chose DSSE. Wrapping the Receipt in an in-toto Statement
v1 and swapping the DSSE envelope's `payloadType` to
`application/vnd.in-toto+json` is all it took — cosign,
slsa-verifier, and the rest of the in-toto ecosystem now consume
Corvid receipts natively with zero adapter code.

The lesson is the same one H-5-signed named in reverse:
*choosing the ecosystem-standard format at the base layer pays
compounding interest at every higher layer.* We didn't have to
invent in-toto compatibility — we declared the subject, the
predicateType, and the Statement wrapper, and the format
committee's prior work did the rest.

Design choices worth naming:

- **Subject = reviewed artifact, not the receipt itself.** The
  attestation is *about* the head source file (`sha256` in the
  subject's digest field). Self-attesting would have been
  redundant with H-5-signed's content-hash addressing, and
  would have confused consumers expecting the subject to point
  at the reviewed-thing.
- **PredicateType is Corvid-specific, not SLSA Provenance.**
  URI `https://corvid-lang.org/attestation/receipt/v1`. SLSA
  Provenance describes build inputs/outputs; Corvid receipts
  describe algebraic-effect deltas. Borrowing the wrong
  predicate type would have confused SLSA consumers.
- **Unsigned in-toto output is allowed.** Forcing `--sign` would
  have excluded pipelines that sign externally with cosign's
  KMS-backed signers or keyless OIDC. The unsigned-then-sign
  flow is a legitimate use case we support by not getting in
  the way.
- **`receipt verify` accepts both payloadTypes transparently.**
  The allow-list grew from one entry to two. Callers don't
  branch on payloadType; they just verify and get the bytes.
  Interpretation (Corvid receipt vs in-toto Statement) happens
  at the consumer layer where it belongs.

### Signed receipts move governance from informational to defensible

`21-inv-H-5-signed` turned the trace-diff receipt from "a text
output the CLI prints" into a signed DSSE envelope that external
tools can cryptographically verify. That's the step that moves
Corvid receipts from "a nice audit log" to "a defensible
artifact" — the kind of thing a regulator or security auditor
can check on their own machine without trusting the developer
who produced it.

Design choices worth naming:

- **DSSE over a hand-rolled format.** The Dead Simple Signing
  Envelope is the format used by Sigstore, in-toto, cosign, and
  every modern supply-chain tool. Adopting it means Corvid
  receipts plug into those ecosystems for free — and the
  follow-up `21-inv-H-5-in-toto` slice becomes just "wrap the
  DSSE envelope in an in-toto Statement." Building our own
  envelope would have been cheaper today and more expensive
  forever.
- **PAE over raw payload.** DSSE's Pre-Authentication Encoding
  adds explicit length prefixes so the signature binds both the
  payload AND its type. Ignoring PAE is a known class of
  signature-transplantation bug where an attacker crafts a
  different envelope that the signature still validates against.
  The DSSE spec exists because the non-PAE version was
  foot-guns; we followed it.
- **ed25519 over RSA / ECDSA.** Smaller keys, smaller signatures,
  deterministic (no per-signature RNG needed), single obvious
  security level. Less to go wrong at every layer from key
  generation to verification. Sigstore's keyless flow also
  defaults to ed25519, so staying there keeps the upgrade path
  cheap.
- **Hash-addressed cache with prefix lookup.** Receipts are
  shell-referenceable by their SHA-256 prefix (min 8 chars),
  the same way Git refers to commits. Operators who stared at
  commit hashes all day don't have to learn a new addressing
  scheme.
- **Key-source precedence: `--sign=<path>` > `CORVID_SIGNING_KEY`
  env var.** Explicit flag wins over implicit env — matches how
  every serious CLI handles auth material. Env var support is
  free and useful for CI; file path is the local-dev ergonomic.

The pattern that generalises: *when a domain has a standard
cryptographic format, adopt it whole rather than reinventing.*
The format committee already considered the attacks; your
"simpler" version will rediscover them the hard way.

### Governance receipts are the audit layer, not just a reporter

`21-inv-H-5` started as "add three output format modes" and was
reframed mid-chat to "the trace-diff receipt becomes the AI-safety
audit artifact of Corvid programs." That reframe changed every
design decision that followed:

- Receipt is a canonical structured object, not a concatenated
  string. Each format (`markdown`, `github-check`, `json`) is a
  view over the same struct. Adding a format is adding a
  renderer, never touching the pipeline.
- JSON output is schema-versioned from day one (`schema_version:
  1`). Bots pin against the version; breaking changes get v2
  while v1 consumers keep working. Schema evolution is a
  first-class commitment.
- Regression policy is its own concern, separable from the
  renderer. Shipping a baked-in conservative default today;
  promotable to a user-replaceable `.cor` program in the
  follow-up slice (`21-inv-H-5-custom-policy`). Governance-as-
  code for the gate itself.
- Exit code is policy output, not a flag. `--gate=on|off` was
  tempting and would have worked; rejected because the gate's
  WHY lives in the policy. Exit 0 on `verdict.ok`, exit 1
  otherwise. Custom policies replace the verdict; they don't
  ask the CLI for permission to fail.

The pattern generalises: whenever a language ships a structured
governance concept (effect algebra, approval contracts,
provenance), the corresponding receipt should be a first-class
audit artifact — structured, versioned, policy-gated, and
eventually signed — not a pretty string. The receipt is how the
compile-time guarantee becomes a durable record the rest of the
world can inspect.

The five follow-up slices filed (`-custom-policy`, `-signed`,
`-in-toto`, `-stacked`, `-watch`, `-gitlab`) each extend this
audit-layer thesis in a different direction. They land
independently because the receipt is structured — no one
follow-up has to know about the others.

### The CTO reframe: scope as leverage, not as a list

When planning `21-inv-H-5` I drafted a conservative chat with
three questions and five implementation decisions. The user asked
me to answer the questions "in the way that makes Corvid powerful
and limitless." That reframe moved H-5 from an incremental
feature to a category-defining one, and it taught an instruction
worth honouring for future planning:

**Default to ambition in design; default to discipline in scope.**
The canonical receipt + policy-as-code is the ambitious design.
The first slice ships the canonical receipt, three renderers,
and a baked-in policy with exit-code gating. Everything else —
`--policy=<path>`, signed receipts, in-toto attestations, stacked
PRs, watch mode, GitLab renderer — is explicitly filed as a
follow-up. The design vision is limitless; the shipping vehicle
is disciplined.

The failure mode to avoid is the opposite: conservative design
("just add format flags") + generous scope ("land all three
modes + signing + in-toto in one slice"). That's the worst of
both — no leverage AND a brittle ship. Ambition in design gives
follow-ups their meaning for free; discipline in scope makes the
current slice shippable.

### Non-deterministic generators, deterministic receipts: the wrapping-layer pattern

`21-inv-H-4` wanted an LLM-generated prose paragraph at the top of
the trace-diff receipt, but the receipt overall still had to be
byte-deterministic where it mattered (CI, `--format=json`). The
conflict is real: prompts produce different strings each run.

The resolution is a wrapping-layer pattern with three tiers:

1. A deterministic orchestrator (`review_pr`) renders the whole
   receipt from structured inputs. It is `@deterministic` —
   identical inputs always produce byte-identical output.
2. A narrow non-deterministic surface (`summarise_diff` prompt)
   produces exactly one piece of the structure: the narrative
   paragraph.
3. A deterministic pre-filter between them (`validate_narrative`)
   enforces strict all-or-nothing rules on the LLM's output; on
   rejection it substitutes a deterministic sentinel.

The critical property: the non-determinism is fenced inside tier 2
and never leaks into tier 1 or tier 3. `review_pr` renders the
narrative OR the boilerplate fallback, and both are deterministic
given their inputs. The caller can *opt into* the non-deterministic
path (`--narrative=on/auto`) or out of it (`--narrative=off`);
opting out makes the whole receipt byte-deterministic again with no
special casing in `review_pr`.

The lesson generalises beyond this slice: any language that wants to
mix LLM surfaces into deterministic artefacts needs a pattern like
this. A pre-phase chat almost settled on `Grounded<ReceiptNarrative>`
for H-4; what actually shipped is ungrounded plus strict
post-validation, because the language couldn't mint a grounded value
from a plain value today. The deferred follow-up is to re-wrap once
22-F lands the provenance-handle path across FFI. The pattern
doesn't change — only the type-level annotation gets sharper.

### Grounding across FFI is a runtime attestation, not an extended type wall

Thinking about how `Grounded<T>` crosses the FFI boundary (for the
post-22-F H-4 follow-up) forced a choice that applies to any
system mixing language-level effect walls with foreign hosts.

Inside Corvid, `Grounded<T>` is an effect wall: you cannot extract
`T` without staying in a grounded context. Across the FFI the host
has `T` the moment it reads the return value. Two options:

1. Grounding is informational at the boundary. Host receives
   `(payload, handle)`; handle queries sources + confidence. The
   Corvid-side guarantee survives as an *inspectable attestation*
   but not as a compile-time wall.
2. Grounding is enforced at the boundary. Host never receives `T`
   directly; only operates on opaque handles through FFI
   primitives.

Option 2 sounds purer but is wrong in practice. It forces hosts to
re-express their entire call graph through grounded-handle
primitives — impractical for real C/Python/Rust hosts — and gives
*false* security because a determined host casts the handle to a
pointer and reads the bytes. Pretending the type wall extends into
foreign code when it can't is worse than admitting it doesn't.

Option 1 is honest: the compile-time wall stays a compile-time
guarantee *inside* Corvid; at the boundary it transforms into a
runtime attestation the host can inspect when it cares. The
attestation surface (source names, confidence, handle lifetime)
lets sophisticated hosts act on the evidence. Hosts that don't care
just use the payload and release the handle.

The deep lesson: when a language-level effect guarantee meets a
foreign runtime, the honest move is to let the guarantee decompose
into runtime evidence rather than try to extend the wall. The same
reasoning will apply to `@dangerous`, `@deterministic`, approval
contracts — each has a compile-time teeth and a runtime receipt, and
each receipt is what crosses the boundary.

What actually shipped in `22-F` follows directly from that choice:

- the host gets `(payload, handle)`, not an opaque grounded-only value
- Level 1 exposes `List<String>` source names plus a confidence query,
  which covers the common host questions without freezing the richer
  internal provenance shape too early
- `0` is the null grounded handle and `release(0)` is a no-op, matching
  normal C conventions
- handle lifetime lives in a slotmap-backed attestation store with a
  generation counter, so stale or double-released handles fail cleanly
  instead of degrading into silent misuse

Just as important was what did *not* ship: host-side grounding minting.
Returning a grounded value from Corvid is **earned grounding** - the
runtime can point at the retrieval/prompt/tool path that produced it.
Letting the host construct a grounded handle is **asserted grounding**,
which needs a separate audit trail (`host_asserted`, provenance
ownership, review semantics). Splitting those into separate slices
keeps the trust model honest instead of blurring two very different
claims behind one ABI.

### Citation validation is what makes grounding meaningful, not decoration

`21-inv-H-4`'s citation rule from the pre-phase chat: all-or-nothing.
Every `delta_key` an LLM cites must be in the allow-list we computed
from the structural diff; a non-empty body with an empty citations
list is rejected; duplicate keys are rejected. Any violation drops
the entire narrative and falls back to boilerplate.

The alternative — partial acceptance — was considered and rejected.
Partial acceptance lets the narrative keep the phrases whose
citations validated and drop the phrases whose citations didn't. But
once the LLM's output has been surgically edited, the sentences that
remain may no longer flow, may reference changes out of order, or
(worst) may preserve a phrase whose cited `delta_key` was valid but
whose *semantic* claim was about a different change entirely. The
citation validates the key; it can't validate the prose. All-or-
nothing keeps the validation's meaning crisp: *either this whole
paragraph is honestly cited, or we don't trust any of it.*

The lesson for any system that mixes LLM text with structured
grounding: the grounding check is only meaningful when it's load-
bearing. A check you only sometimes act on isn't a check, it's
decoration.

### Positional struct constructors, not struct-literal braces

`21-inv-H-3` tried to build a sentinel `ApprovalLabelSummary` inside
the Corvid reviewer with `ApprovalLabelSummary { label: "", ... }` —
which parses as a call followed by a stray `{` and fails with
`unexpected token LBrace`. Corvid user-defined types are constructed
positionally: `ApprovalLabelSummary("", "", "")`. The language didn't
borrow Rust's named-init syntax, and the lesson is worth remembering
in future `.cor` authoring: *struct literal braces are not a Corvid
construct, positional calls are the one shape*. Our existing
`examples/structs.cor` demonstrates this and is the canonical
reference.

### Reachability, not visibility, decides what's in the ABI

`21-inv-H-3`'s integration test needed a non-`pub extern "c"` helper
agent (`explain`) to show a grounded-return transition, because
`Grounded<T>` can't cross the C ABI. The first fixture attempt put
the helper in the source but didn't call it from the exported
`refund_bot` — and the helper silently disappeared from the ABI.
`crates/corvid-abi/src/emit.rs` restricts `abi.agents` to the
*transitive closure of `pub extern "c"` agents* via
`collect_exported_agent_closure`. So for receipt tests (and for the
real-world PR-review workflow), the rule is: if a helper's contract
changes but nothing reachable from an exported agent calls it, the
receipt won't see the change. That is the correct behaviour (dead
code shouldn't pollute the receipt) but worth stating explicitly so
future integration fixtures don't fall into the same trap.

### The Corvid reviewer keeps ownership of structure even when the language lacks Int→String

`21-inv-H-2` wanted to render a "Counterfactual Replay Impact" section
with sentence-shaped summary counts — "Replayed 10 trace(s) against base
and head: 7 passed on both, 2 newly diverged under head, 1 newly passing
(base bug fixes), 0 diverged on both, 0 errored." Corvid doesn't yet have
an `Int→String` primitive. The temptation was to format the whole section
in Rust and pass the fully-rendered block to the reviewer as a `String`,
which would have collapsed the section into a Rust deliverable with the
`.cor` file only responsible for deciding "include or omit." The honest
split keeps the numeric formatting in Rust (where the primitive lives
today) but keeps *structure* ownership in the reviewer: the reviewer
chooses whether the section renders, where it sits in the receipt, what
narrative lines surround the pre-formatted summary, the heading for the
newly-divergent path list, and how the list itself is rendered. The
lesson is durable: when a language gap forces some work out of the
dogfooded layer, push only the narrowest possible piece out and keep
structure ownership where the thesis wants it. A future language slice
that adds `Int.to_string()` will make the reviewer fully self-sufficient
without a receipt layout change.

### Path-list caps in governance receipts protect the reader without losing data

The counterfactual impact report caps the newly-divergent trace path
list at twenty entries, appending an "... (and N more)" row so the
reader always knows the cap fired. The full list will be available in
the 21-inv-H-5 JSON output mode for bots that want it. The lesson
isn't the specific number — it's that governance output intended for
human review needs an explicit cap with a visible truncation marker,
not silent omission. A PR reviewer staring at a list of 600 broken
traces needs a "run the CLI locally for the full list" signal; a list
of 600 names without that signal either drowns the reviewer or gets
scrolled past.

### The spec is a runnable program, not a document

Phase 21's documentation slice follows the pattern already established for
the effect system: every numbered `.md` file in `docs/internals/effect-spec/` is a
program under disguise. Code blocks tagged `# expect: compile` are extracted
by `corvid test spec` and re-compiled against the current toolchain on every
build; a broken example fails CI. Writing [section 14](docs/internals/effect-spec/14-replay.md)
forced an honest audit of which Phase-21 surface is actually demonstrable
*today* (the `replay` language primitive with only the constructs the parser
accepts — no `.is_some()`, no `Int.to_string()`, no list `.push()`) vs. which
parts I was tempted to illustrate with constructs the language doesn't have
yet. Writing the spec as a runnable artefact is also what lets the v1.0
launch demo at `docs/meta/v1.0-demo-script.md` be a script of copy-pasteable
commands rather than a slide deck — every claim resolves to a command whose
output proves it. The lesson is durable across phases: specification work
that ships alongside a "does the compiler still agree with this?" harness
stays honest on its own; specification work that ships as prose drifts away
from the shipped compiler within weeks. For a language whose thesis is
"compile-time guarantees," the spec has to compile.

### Nullable-pointer options are only safe until they stop preserving information

The cheap native encoding for `Option<T>` is a good one when the payload has a
non-null native representation: `Some(payload)` is the payload pointer/value and
`None` is zero. But that encoding is not universally sound. As soon as the
payload is itself an option-shaped value, bare nullability collapses semantics:
outer `None` and `Some(None)` both become zero. Corvid now widens the native
representation at exactly that boundary by allocating a tiny typed wrapper for
nested option payloads while keeping direct nullable-pointer options on the fast
path. The lesson is architectural: representation widening should happen where
the current encoding stops being semantically injective, not just where it is
convenient to add one more case.

### Restricted filter DSLs keep effect queries honest in a way general expression languages do not

`22-D-effect-filter` could have shipped as "evaluate a tiny expression language
over the embedded descriptor" and been superficially more flexible on day one.
That would have been the wrong trade. The host-side question is narrow:
"which capabilities definitely satisfy these effect constraints?" A JSON AST
with only `all`, `any`, `not`, and leaf predicates makes that question explicit
and keeps every failure mode denotationally crisp: unknown dimension is
`UNKNOWN_DIMENSION`, invalid operator for a dimension is `OP_MISMATCH`, malformed
syntax is `BAD_JSON`. A free-form expression language would blur those into a
parser/runtime soup, make host bindings harder to generate, and invite
stringly-typed shortcuts at the FFI boundary. The broader lesson is that when
the domain is a constrained algebra, the honest API is a constrained algebraic
surface too, not a miniature programming language.

### Missing effect fields are a third truth value, not false and not true

The subtle design choice in `22-D-effect-filter` was how to treat agents that do
not declare the field a host queried. Returning true would silently widen
safety-sensitive queries like `trust_tier <= autonomous`; returning false would
make `not { dangerous == true }` look like a sound "definitely safe" query even
though the descriptor never asserted that fact. Corvid's filter now treats
missing fields as a third truth value: the predicate is unevaluable for that
agent, so the agent is omitted from the result set. This keeps narrowing
semantics monotonic and forces hosts that care about the omitted population to
ask a second, explicit question. The durable lesson is that optional metadata in
a safety-facing query surface should usually model "unknown" honestly rather
than collapsing it into either branch of a boolean.

### Effect bounds crossing into runtime should become attestations plus host policy, not runtime-owned walls

`21-inv-H-5`, `22-F`, and `22-G` all hit the same architectural seam from
different directions:

- `H-5` turned the regression gate into a structured receipt plus policy output
  instead of a hard-coded CLI branch.
- `22-F` let `Grounded<T>` cross FFI as `(payload, handle)` so the host can
  inspect provenance evidence without pretending the Corvid type wall survives
  inside foreign code.
- `22-G` applies the same pattern to cost and latency: the runtime records a
  per-call observation handle, exposes realized cost / latency / token counts,
  and reports whether the declared bound was exceeded, but it does **not**
  unilaterally kill the call.

The common lesson is the durable one: when a compile-time effect dimension
meets runtime or FFI, the honest shape is usually **attestation + host policy**.
The language keeps its compile-time guarantee where it is real; the runtime
turns observed reality into structured evidence; the host decides what to do
with that evidence. Trying to extend the compile-time wall wholesale into a
foreign host or a runtime policy engine either centralizes too much policy in
the runtime or offers false comfort when the host can always step around it.

`22-G` makes that concrete:

- one top-level `corvid_call_agent` returns one observation handle
- the handle owns realized `cost_usd`, `latency_ms`, `tokens_in`,
  `tokens_out`, and `exceeded_bound`
- if no cost bound was declared, `exceeded_bound` is simply `false`
- hosts that care about "no bound declared" can already learn that from the
  embedded descriptor and write their own policy on top

That pattern is now the default for future effect-facing Phase 22 / 23 slices:
compile-time algebra inside Corvid, runtime attestations at the boundary,
policy authored by the host unless a later slice makes a very explicit case for
runtime enforcement.

### Computation can be distributed as a replay capsule, not just as code

`22-H-replay-across-ffi` closes the loop on the earlier Phase 21 and 22
guarantees. Corvid already had three strong pieces in isolation:

- a compiled cdylib carrying its full ABI and effect surface
- deterministic replay over recorded traces
- structured receipts and policy outputs about what changed

The important move in `22-H` was to make those one artifact instead of three
related features. A Corvid execution can now be packaged as a **replay
capsule**: library, embedded descriptor, trace, and manifest bound together by
content hashes plus schema/version metadata. That turns an execution into a
portable unit for debugging, audits, regressions, and cross-host reproduction.

Two implementation choices are the durable lessons.

First, host-originated events belong in the same trace stream as runtime
events. `host_event` is not a sidecar file and not a separate schema. The host
submits the event through the ABI, but the cdylib remains the single writer.
That keeps replay, viewers, and tooling working over one timeline instead of
teaching every consumer how to merge multiple partial histories.

Second, determinism at the boundary has to be stated honestly. Corvid now seeds
its own run identity and clock reads from deterministic metadata when replaying
through the FFI, but it does **not** claim control over opaque SDK jitter or
adapter-internal scheduling it does not own. The strong claim is therefore:
seed-deterministic for the Corvid-controlled surface. That is still enough to
make capsules durable and cross-host portable without quietly overstating the
guarantee.

This also extends the same architectural pattern that showed up in `H-5`,
`22-F`, and `22-G`: runtime reality crosses the boundary as structured
evidence, and the host decides what to do with it. `22-H` applies that to whole
executions. The capsule is not just a convenience archive; it is the boundary
artifact hosts can inspect, replay, diff, sign later, and build policy around.

### Corvid does not flatten semantics at the FFI boundary; it projects them into the host as typed constructs

`22-I-host-bindings` could have stopped at a familiar "generate wrappers from a
descriptor" story. That would have produced callable Rust and Python surfaces,
but it would also have thrown away the reason Corvid's ABI descriptor exists in
the first place: the descriptor carries semantic layers, not just transport
types.

The slice becomes interesting only once those layers survive the boundary:

- effect algebra surfaces as host-visible constants and typed catalog queries
- `@dangerous` becomes an `Approver` requirement at the call site instead of a
  runtime convention the host might forget
- `Grounded<T>` becomes a payload wrapper plus provenance access with automatic
  cleanup, not "raw value plus maybe some extra helper functions"
- `Observation` becomes a first-class returned object with RAII/context-managed
  lifetime, so cost and latency evidence are part of the host API rather than a
  leak-prone side channel

The architectural hinge is descriptor-hash drift detection. Generated bindings
embed the descriptor hash they were projected from and compare it against the
loaded cdylib's own `corvid_abi_descriptor_hash` bytes at load time. That keeps
"bindings and library drifted apart" out of the realm of mysterious runtime
misbehaviour and makes it a designed failure mode instead. The host gets a
typed `DescriptorDrift` error before it can make the wrong call against the
wrong binary.

Two broader lessons fall out of that.

First, the source of truth has to stay semantic. The bindings generator reads
the `22-B` descriptor, not the C header. A header can tell you argument and
return shapes; it cannot tell you trust tier, approval contract, replay
metadata, reversibility, grounding, or future effect dimensions without
re-encoding all of that in a second place. Once the descriptor is the only
semantic source of truth, generated bindings stay projections instead of
becoming a second specification.

Second, "idiomatic bindings" is not the same thing as "thin bindings." The thin
approach would have exposed stringly filter JSON, manual handle release, and a
generic load error. The more honest idiomatic approach is slightly thicker: a
typed builder that still lowers to the runtime's JSON DSL, RAII/context-managed
wrappers over existing handle APIs, and a dedicated error variant for drift. In
other words, preserve semantics in the host surface without inventing a second
runtime or a parallel policy engine.

That pattern will matter again for future boundary-facing slices. Once a
language has real semantic layers, the boundary should preserve those layers as
typed host constructs wherever possible, and fall back to structured evidence
when compile-time projection is impossible. Flattening is the easy move. The
interesting move is to keep the language's meaning intact on the far side of
the FFI.

### Corvid makes FFI ownership compile-time-guaranteed instead of host-side convention

`22-J-ownership-check` closes the biggest semantic gap left after `22-I`. The
bindings already gave hosts RAII/context-managed wrappers for grounded values
and observation handles, but the ownership contract behind those wrappers was
still partly implicit: descriptor conventions, destructor naming patterns, and
generator-side knowledge of which handle families needed cleanup.

The important move in `22-J` is not "add ownership annotations." The important
move is to make ownership a structured semantic dimension of the FFI surface.
Extern signatures now carry ownership information through three layers at once:

- the checker infers or validates the contract at compile time and refuses
  ambiguous or unsound extern signatures
- the ABI descriptor carries ownership as typed JSON, including destructor kind
  and symbol when a host must release or drop something
- the Rust and Python generators read that ownership contract directly and emit
  the correct host-side lifetime or cleanup shape from the descriptor instead
  of from naming conventions

That changes the category. Most systems languages stop tracking ownership at
their FFI boundary. Rust's borrow checker is strong inside Rust and then hands
off to `extern "C"` with no semantic guarantee about who frees what on the far
side. Corvid keeps the ownership algebra intact across the boundary: a borrowed
string parameter becomes a borrowed host view, an owned grounded result becomes
a wrapper that knows which release symbol to call, and loosening ownership in a
public extern surface shows up as a receipt delta that policy can flag.

The destructor symbol is the load-bearing detail. Without it, a supposedly
typed ownership system still falls back to convention: guess `corvid_<type>_drop`,
hope the naming pattern survives, and special-case each new handle family in the
generator forever. Once the descriptor carries both the ownership mode and the
actual destructor contract, the generator no longer has to know that grounded
values or observations are special. They become ordinary ownership-projected
descriptor entries.

That is the real lesson from the slice: if a language claims semantic richness
at its FFI boundary, it cannot stop at "safe if documented carefully." The
compiler has to refuse unsound contracts, the descriptor has to encode lifetime
and destruction semantics structurally, and the host bindings have to derive
their cleanup behavior from that structure. Anything weaker is still
convention, just with better comments.

## 22-H-windows-record

Windows caught a real FFI bug that Linux had been letting us get away with.
The generic `corvid_call_agent` path was reusing exported `pub extern "C"`
wrappers as if their ABI were only the user-visible parameters plus the return
value. They were not. Those wrappers also append a hidden observation-handle
out-pointer. Linux happened not to explode when generic dispatch skipped that
pointer; Windows faulted immediately in the direct-observation finish path.

The lesson is narrower and more useful than "Windows is fragile." Shared FFI
dispatch code has to call the real exported ABI, not a simplified mental model
of it. If the wrapper owns observation bookkeeping, every generic call site has
to supply the same hidden out-parameter that a hand-written host would. The
fact that one platform survives undefined behavior is not evidence that the ABI
is sound.

The permanent fix is the guard, not just the patch. This slice added focused
Windows record and replay tests in `corvid-runtime/tests/trace_record.rs` and a
Windows CI job that runs them on every push. That changes the system: the next
time a host-dispatch path drops a hidden ABI parameter or returns a silently
wrong observation handle, the failure shows up in CI instead of months later in
a demo slice.

## 22-K-bundle-public-spec

The bundle story only became launch-grade once the demo stopped being private
scaffolding and became a public spec artifact. A happy-path bundle by itself is
easy to fake. A happy bundle plus typed failing siblings, lineage traversal,
counterfactual query, rebuild verification, and offline audit forces the
implementation to state exactly what it believes a trustworthy Corvid artifact
is.

That changed the quality bar in a useful way. The public surface could not stop
at `verify`; it had to answer six different questions with one coherent model:
is the artifact intact, does it rebuild, what changed from its predecessor,
what approval/provenance semantics are inside, which counterfactual delta
explains the difference, and does the predecessor chain still verify? Once the
examples were committed, any drift between those answers became visible
immediately.

The real lesson is that credibility comes from adversarial siblings, not from
README prose. The neighboring broken bundles matter as much as the happy path,
because they prove the failure boundaries are typed and reproducible: hash
tamper, receipt-signature tamper, rebuild drift, lineage fraud, and unsupported
counterfactual asks all fail for distinct reasons. That is what turns the
bundle format into a one-time public proposal moment rather than a screenshot
of an internal demo.

## 20b-strict-prompt-citations

`cites ctx strictly` is only meaningful if the compiler and runtime agree on
what "ctx" means. The compiler side now rejects strict citation clauses unless
the cited prompt parameter is explicitly `Grounded<T>`, so the annotation cannot
silently attach to an ordinary string with no provenance.

The runtime lesson was sharper: provenance wrappers are not citation text.
Citation verification must inspect the grounded payload, not the JSON envelope
that carries provenance metadata. The same boundary rule applies to tools and
prompts that return `Grounded<T>`: external JSON supplies the inner `T`; Corvid
adds the provenance wrapper after the trusted retrieval or grounded transform
has been established.

Native parity forced the same rule into codegen. `Grounded<T>` is an evidence
type, not a different runtime payload shape on the hot path. Codegen should
lower it as the inner scalar for interpolation, trace payloads, and prompt
bridge calls, then attach or verify provenance at explicit runtime boundaries.
That keeps the native tier behaviorally identical to the interpreter without
inventing a second citation checker in Cranelift.

## 20b-explicit-provenance-discard

Use `.unwrap_discarding_sources()` when a program intentionally drops
`Grounded<T>` evidence and continues with the inner `T`:

```corvid
effect retrieval:
    data: grounded

tool fetch_doc(id: String) -> Grounded<String> uses retrieval

agent export_text(id: String) -> String:
    doc = fetch_doc(id)
    return doc.unwrap_discarding_sources()
```

The method takes no arguments and only exists on `Grounded<T>`. It is not a
runtime conversion in native code; `Grounded<T>` is represented as the inner
payload on the hot path, so the explicit IR node records intent for the
compiler while preserving the efficient ABI shape.

This is a visibility feature as much as a convenience feature. Corvid still
keeps legacy `Grounded<T>`-to-`T` assignability for compatibility today, but
new code should prefer the method so provenance erasure is visible in source,
IR, reviews, and future policy tooling.

## 20d-wrapping-arithmetic

Overflow policy is part of the language contract, not a backend accident.
Default Corvid integer arithmetic should trap because silent wraparound is the
wrong default for safety-oriented agent code. The opt-out is explicit:

```corvid
@wrapping
agent mix(x: Int) -> Int:
    return x * 6364136223846793005 + 1
```

The useful implementation pattern is to preserve intent in IR. Lowering marked
agents into `WrappingBinOp` / `WrappingUnOp` nodes makes interpreter, Python,
native codegen, ABI walkers, and optimization passes handle the policy
deliberately instead of rediscovering it from agent metadata later.

`@wrapping` is deliberately narrow. It applies to integer add/sub/mul and unary
negation; division and modulo by zero still trap. That keeps hash-mixing and
low-level arithmetic possible without turning the annotation into a blanket
"unsafe arithmetic" mode.

## 20e-confidence-gated-trust

`autonomous_if_confident(T)` only matters if it is a runtime boundary, not just
a pretty trust value in the static effect algebra. The checker can treat the
gate's `above` tier as autonomous for compile-time composition, but the
interpreter must still inspect the actual confidence flowing through the call.

The right behavior is conditional approval activation:

```corvid
effect gated_refund:
    trust: autonomous_if_confident(0.90)

tool issue_refund(decision: String) -> Receipt uses gated_refund
```

If the input confidence is `0.95`, the tool runs autonomously. If it is `0.70`,
the interpreter calls the normal approval gate before dispatching the tool.
That preserves the safety model without requiring programmers to write two
manual branches for every uncertainty boundary.

Prompt confidence has to travel with the value. Measuring a prompt result's
confidence but returning a plain `String` loses the exact metadata downstream
confidence gates need. Wrapping low-confidence prompt outputs as `Grounded<T>`
keeps the runtime value carrying its statistical provenance while preserving
ordinary source-level ergonomics.

## 20e-calibrated-prompts

Use `calibrated` inside a prompt declaration when prompt confidence should be
audited against ground truth supplied by an eval runner or adapter:

```corvid
effect confident_model:
    confidence: 0.90

prompt classify(input: String) -> String uses confident_model:
    calibrated
    "Classify {input}."
```

The modifier does not invent correctness labels. It records samples only when
the runtime receives a real correctness observation with the LLM response. That
keeps calibration honest: production calls without labels keep running normally,
while evals and harnesses can accumulate model-level reliability statistics.

The runtime tracks calibration by `(prompt, model)` and reports sample count,
accuracy, mean confidence, confidence/accuracy drift, and a miscalibration flag.
The current flagging rule is intentionally simple: after at least three labeled
samples, drift above `0.25` is considered miscalibrated. Future eval tooling can
render these stats directly instead of treating self-reported confidence as
truth.

## 20e-repl-confidence-step-through

Use `:stepon` or `:stepinto` in the REPL to inspect confidence as the agent
runs. Boundary steps now show input confidence before tool, prompt, and agent
calls, and result confidence after prompt/tool/agent results.

Confidence gates are visible as approval boundaries:

```text
approval required: ConfidenceGate:issue_refund
  confidence gate: actual 0.700 / threshold 0.900 (triggered)
```

That matters because `autonomous_if_confident(T)` is dynamic. A program can be
statically valid and still require human approval on a specific execution if
the values flowing into an irreversible action are less confident than the
threshold. The REPL now shows that threshold comparison directly instead of
hiding it behind a generic approval prompt.

`:trace` includes the same metadata, so the confidence story is visible both
during live step-through and after the run when reviewing the last execution.

## 20f-stream-grounded-provenance

`Stream<Grounded<T>>` preserves provenance per element. Each yielded item is an
ordinary `Grounded<T>` value, so consumers can inspect the source chain of the
specific chunk they received:

```corvid
effect retrieval:
    data: grounded

tool fetch_a() -> Grounded<String> uses retrieval
tool fetch_b() -> Grounded<String> uses retrieval

agent docs() -> Stream<Grounded<String>>:
    yield fetch_a()
    yield fetch_b()
```

The stream itself also exposes an aggregate provenance union. That union grows
as elements are consumed, not when a producer buffers ahead. In REPL
step-through this means a stream local starts with no sources, then shows
`retrieval:fetch_a`, then `retrieval:fetch_a, retrieval:fetch_b` as those
elements are actually delivered.

This keeps streaming provenance honest: the aggregate describes observed
stream content, while each element keeps its precise source chain.

## 20f-mid-stream-model-escalation

Use `with escalate_to <model>` with a streaming prompt confidence floor when a
low-confidence stream should continue on a stronger model instead of failing:

```corvid
model expert:
    capability: expert

prompt draft(ctx: String) -> Stream<String>:
    with min_confidence 0.80
    with escalate_to expert
    "Draft {ctx}"
```

If the initial stream result is below `0.80`, Corvid records a
`StreamUpgrade` trace event and issues a continuation call to `expert` with the
partial output included as context. The stream consumer receives the upgraded
result through the same stream value; the trace shows where the model boundary
changed.

The escalation target is checked like other model-routing features. An
undefined target is a resolver error, and a target that resolves to a non-model
declaration is a type error.

## 20f-progressive-structured-partial-streams

`Stream<Partial<T>>` lets a prompt expose structured output before the full
object is finished:

```corvid
type Plan:
    title: String
    body: String

prompt plan(topic: String) -> Stream<Partial<Plan>>:
    "Plan {topic}"

agent first_title(topic: String) -> Option<String>:
    for snapshot in plan(topic):
        return snapshot.title
    return None
```

For a `Partial<Plan>`, `snapshot.title` has type `Option<String>`. It is `Some`
when the model has completed that field and `None` while the field is still
streaming. The VM schema asks adapters for explicit field states:
`{ "tag": "complete", "value": ... }` or `{ "tag": "streaming" }`.

This is intentionally interpreter-first. Native CL lowering rejects `Partial<T>`
for now, because shipping a real native version needs a dedicated tagged
field-state layout rather than a generic object shortcut.

## 20f-stream-resumption-tokens

`ResumeToken<T>` is the typed checkpoint for interrupted prompt streams:

```corvid
prompt draft(topic: String) -> Stream<String>:
    "Draft {topic}"

agent capture(topic: String) -> ResumeToken<String>:
    stream = draft(topic)
    for chunk in stream:
        break
    return resume_token(stream)

agent continue_it(token: ResumeToken<String>) -> Stream<String>:
    return resume(draft, token)
```

The typechecker requires `resume_token` to receive `Stream<T>` and requires
`resume(prompt, token)` to pair a `ResumeToken<T>` with a prompt returning
`Stream<T>`. The runtime token stores the prompt name, original arguments,
delivered chunks, and an optional provider session handle.

The current implementation is honest about its boundary: provider-native
continuation handles are represented but not fabricated. Until adapters expose
real session state, resume reopens the prompt locally with delivered elements
included as continuation context.

## 20f-stream-fanout-fanin

Corvid stream partitioning is now expressed in the language rather than as
library glue:

```corvid
type Event:
    kind: String
    body: String

agent fanout() -> Stream<Event>:
    groups = source().split_by("kind")
    return merge(groups).ordered_by("fair_round_robin")
```

`split_by` returns `List<Stream<Event>>`, with group order based on the first
time each key appears. `merge(...).ordered_by(...)` supports `fifo`, `sorted`,
and `fair_round_robin`.

The first version deliberately uses string-literal struct fields as key
extractors. That is less general than lambdas, but it is typechecked today and
does not invent an unowned function-value model. Real function extractors belong
with first-class functions.

## 20f-backpressure-propagation

Backpressure is now part of Corvid's language semantics, not just a runtime
queue setting:

```corvid
effect live_feed:
    latency: streaming(backpressure: pulls_from(producer_rate))

prompt watch(topic: String) -> Stream<String> uses live_feed:
    with backpressure pulls_from(producer_rate)
    "Watch {topic}"
```

`pulls_from(name)` is stricter than any bounded buffer and is source-sensitive
when used as a constraint. The VM implements it as a capacity-1 bounded channel,
which forces producers to wait for downstream consumption instead of filling an
unbounded queue.

Fan-in composes upstream policies: any unbounded input makes the merged stream
unbounded; matching pull sources stay pull-based; mixed pull and bounded
sources degrade to the bounded policy because buffering exists somewhere in the
path.

## 21-inv-H-grounded-receipt-narratives

The PR behavior receipt's optional prose summary is now grounded before the
deterministic reviewer can render it:

```corvid
agent review_pr(
    base: Descriptor,
    head: Descriptor,
    impact: TraceImpact,
    narrative_grounded: Grounded<ReceiptNarrative>,
) -> String:
    narrative = narrative_grounded.unwrap_discarding_sources()
    ...
```

Rust still validates that every LLM citation references a real compiler-derived
delta key. Only after validation does the host mint `Grounded<ReceiptNarrative>`
with one provenance entry per cited delta. This preserves the separation of
responsibility: the prompt may write prose, Rust proves the citations are real,
and Corvid's reviewer refuses to consume the narrative as plain ungrounded data.

The CLI executes this embedded reviewer on an explicit worker-thread stack.
That is product behavior, not a test hack: Windows release binaries can have a
smaller main-thread stack than Rust test threads, and complex receipts should
not depend on that platform default.

## 21-inv-H-custom-policy

Trace-diff policy is now governance-as-code in Corvid:

```corvid
@deterministic
agent apply_policy(receipt: PolicyReceipt) -> Verdict:
    ...
```

The key design choice is typed policy facts. Rust converts each canonical
delta key into a `PolicyDelta` with `category`, `operation`, `subject`,
`direction`, `safety_class`, `from_value`, and `to_value`. Policy authors
therefore reason over safety metadata directly instead of parsing strings.

The baked `default_policy.cor` preserves the old conservative gate, but
`--policy=<path>` lets a project replace the gate with its own Corvid program.
The receipt stays archival: custom policies change the verdict, not the
underlying delta record.

`List<T> + List<T>` is now supported so deterministic policy code can build
verdict flag lists naturally.

## 21-inv-H-stacked-aggregate-policy

Stacked PR receipts now gate on history, not just normal form. This matters
because algebraic cancellation can hide a transient safety regression:

```text
commit 1: agent.dangerous_gained:refund_bot
commit 2: agent.dangerous_lost:refund_bot
normal_form: []
history: [dangerous_gained, dangerous_lost]
verdict: failed by default policy
```

That is the right behavior for governance. The final diff may be clean, but the
reviewer still needs to see that the stack carried a dangerous state at an
intermediate waypoint.

The same Corvid policy engine now evaluates both single-commit receipts and
stack receipts. Stack mode passes history-derived `PolicyDelta` facts into
`apply_policy`, and custom `--policy` files can override the verdict without
mutating the archived stack history.

## 21-inv-H-watch-mode

`--format=watch` turns trace-diff into a local development loop:

```text
corvid trace-diff <base-sha> <base-sha> agent.cor --format=watch
```

The first SHA is the stable base. The watched file on disk becomes the live head
on each render. That keeps the mental model simple: CI emits durable receipts;
watch mode gives immediate safety feedback while editing.

The important design constraint is that watch mode reuses the real receipt
pipeline instead of inventing a looser preview path. It still compiles both
versions, computes the same semantic delta, applies the same Corvid policy
engine, and shows the policy verdict. Custom `--policy` files therefore behave
the same locally and in automation.

Watch mode intentionally does not sign receipts or compose stacked reviews.
Those are artifact concerns. The local loop is optimized for speed and clarity;
durable governance still belongs to JSON / in-toto / signed receipt modes.

## 20g-preserved-semantics-rewrite-reports

`corvid test rewrites` is now the public entry point for preserved-semantics
fuzzing. It runs the rewrite coverage matrix over the clean corpus and names
the semantic law attached to each rewrite.

The important part is the failure shape. A drift is not reported as "test
failed"; it names the exact rewrite rule and algebraic law that broke, includes
the first changed source line, shows the original and rewritten effect
profiles, and includes a shrunk reproducer when line deletion can minimize the
case.

Coverage gaps remain honest. Rows with no exercised corpus program are shown as
unexercised, but they are not treated as profile drift. The command distinguishes
"we need a better corpus" from "the checker is unsound under this rewrite."

## 20g-rule-to-test-cross-links

The effect-system spec now has a rule-to-test map. Each row starts from a
language rule family, points at the production module that implements it, then
points at the property/regression tests and corpus gate that keep it honest.

This matters because "inventive" language features need auditability. A reader
should not have to trust a prose claim that `Grounded<T>` or approve-before-
dangerous is enforced; the spec links directly to the checker, VM/runtime test,
and differential-verification gate that prove it.

`corvid test rewrites` is also part of CI now, so preserved-semantics fuzzing is
not an optional local ritual. If an AST rewrite causes profile drift, CI fails
with the rewrite rule, semantic law, first changed line, and shrunk reproducer.

## 20g-counterexample-metadata

The counterexample museum now has explicit metadata in each seed fixture:
counterexample name, bug exposed, fix/proof mechanism, and credit.

This is intentionally small but important. A counterexample without provenance
is just a test file; a counterexample with bug/fix/credit metadata becomes an
auditable safety record. The seed corpus is credited to the Corvid core team
until the public bounty process can attach reporter names to future fixtures.

## 20h-roadmap-reconciliation

The typed model substrate was more complete than the checklist showed. The
spec already has a shipped-section with commit trail, so the roadmap now marks
the real shipped surface complete: `model` declarations, model references,
capability `requires:`, route guards, progressive escalation, rollout,
majority ensembles, adversarial prompt pipelines, jurisdiction/compliance/
privacy dimensions, routing reports, and BYOM adapters.

The important discipline is precision. Some original design bullets changed
shape before shipping: classifier prompts became arbitrary Bool route guards,
`try ... else` became `progressive:`, and generator/validator critic syntax
became a typed three-stage adversarial prompt contract. Those are not
shortcuts; they are stricter language designs. Items that truly did not ship
remain open.

## 20h-cacheable-prompts

Prompt caching is language semantics, not an adapter convenience. The prompt
declares `cacheable: true`, IR carries that bit, and runtime computes a stable
fingerprint from the full semantic call boundary.

Replay determinism is the hard part. A cache hit cannot silently skip trace
events, because then replay would depend on local cache state. Corvid records
cache hits as metadata while still emitting the normal LLM call/result pair,
so replay sees the same behavioral trace and metadata consumers can still
distinguish live calls from cache hits.

## 20h-model-version-pinning

Model names are not stable enough for replay safety. A provider can change the
weights or behavior behind the same name, so Corvid traces now carry an
optional `model_version` and replay treats version drift as semantic drift.

The important product rule is backwards compatibility without silent
weakening. Old traces deserialize with `model_version: null`; new pinned traces
fail fast when the runtime catalog points the same name at a different
version. Reports also use `model@version` labels so operators can see which
revision produced which cost and latency behavior.

## 20h-output-format-routing

Structured output is a model capability, not prompt prose. A prompt requiring
`output_format: strict_json` should not silently route to a model that only
declares markdown-style output, even if that model is cheaper or the default.

The language now carries this constraint through the full stack: source model
catalogs, compile-time validation of named routes, runtime catalog selection,
and trace evidence. That matters because AI-native programs need to prove
compatibility at the boundary where free-form generation becomes typed program
data.

## 20h-weighted-ensemble-routing

Majority vote is a useful baseline, but AI-native routing should learn from
observed behavior. `weighted_by accuracy_history` turns calibration history
into a first-class dispatch signal: a historically reliable minority model can
beat an unreliable raw majority.

Disagreement escalation is the safety valve. When ensemble members disagree,
Corvid can route the same prompt to a declared stronger model instead of
pretending a split vote is decisive. The important design point is that both
the weighting and fallback are visible in syntax, typechecked as model
references, and recorded in traces.

## 20h-eval-swap-model

Retrospective model migration should be trace-based before it is source-eval
based. Production traces already contain the prompts, tools, approvals, costs,
and recorded outcomes needed to answer "what changes if this model changes?"
without re-running unrelated workflow steps.

The implementation discipline matters: `corvid eval --swap-model` is not a
fake eval runner. It delegates to deterministic replay for single traces and to
prod-as-test-suite replay for trace directories, then reports semantic drift
against the candidate model. The full source-level eval runner remains Phase
27, but model migration is useful now.

## 20h-cost-frontier

Cost-quality frontiers are only meaningful when both axes are real. Corvid
already records model-selection cost estimates, but quality must come from eval
evidence rather than routing frequency or confidence folklore.

The command therefore treats missing quality as data absence, not as zero or
average quality. `corvid cost-frontier` computes Pareto status only for models
with explicit eval-quality host events and leaves the rest unscored. That keeps
the operator tooling honest while still making model selection a visible
design-space exploration problem.

## lang-cor-imports-use

Selective imports need a distinct semantic identity. Treating `use Name` as a
local declaration would erase the module boundary; treating it as a plain import
alias would break unqualified calls. Corvid now uses `DeclKind::ImportedUse` so
the compiler knows a lifted name is unqualified at the source level but still
owned by another module.

That distinction matters for future AI-native imports. Effect-typed imports,
hash-pinned imports, and semantic summaries all need to know where a lifted
name came from. The language convenience therefore preserves provenance instead
of hiding it behind wildcard-style namespace merging.

## lang-cor-imports-requires

Imports should carry behavioral contracts, not just names. A policy library
that was deterministic yesterday should not silently become prompt-backed
tomorrow while callers continue compiling as if nothing changed.

Corvid now lets the importing file state the boundary requirement directly:
`import "./policy" requires @deterministic as p` and
`requires @budget(...)` are checked while the import graph is compiled. The
important design choice is that this is not a separate trust system; it reuses
the existing agent attributes and dimensional effect algebra, so module
boundaries participate in the same safety model as local calls.

## lang-cor-imports-semantic-summaries

A module boundary is only useful if developers can inspect what crosses it.
Corvid imports now carry a semantic summary with the public effects, approval
requirements, groundedness, budget cost, and replayability facts that matter to
AI-native code review.

The compiler and CLI read the same summary object. That avoids a common
tooling failure mode where enforcement says one thing but reports show another.
It also sets up signed and remote imports: the thing to hash, sign, diff, and
audit is not just bytes, but the exported semantic contract those bytes imply.

## lang-cor-imports-signed

Supply-chain safety starts before package registries. A local policy path can
drift just as dangerously as a registry package, so Corvid imports now let the
source file pin the imported bytes with `hash:sha256:<digest>`.

The important invariant is fail-closed ordering. The driver verifies the exact
file bytes before lexing, parsing, resolving, or typechecking the imported
module. If the digest changed, the module never enters the compiler's alias map.
That keeps hash pins as a real language trust boundary rather than a comment the
tooling happens to check later.

## lang-cor-imports-remote

Remote imports are only safe if identity is content-addressed. Corvid therefore
rejects `import "https://..." as p` unless the import also carries a SHA-256
pin. The URL says where to fetch; the hash says what program is trusted.

The implementation keeps remote modules in the same semantic pipeline as local
modules instead of inventing a second "package" path. Remote bytes are fetched,
verified, parsed, resolved, summarized, typechecked, and lowered through the
same import machinery. The only special part is module identity: remote files
use deterministic synthetic keys because they do not have filesystem paths.

## lang-cor-imports-versioned-lock

Package imports need two identities, not one. The source identity is semantic:
`corvid://@scope/name/v1.2` is what a developer means. The execution identity is
content-addressed: URL plus SHA-256 is what the compiler can safely trust.

Corvid now keeps those identities separate. Source imports stay stable and
human-readable, while `Corvid.lock` supplies the reviewed URL and digest. This
prevents both shortcut failure modes: source files do not become a pile of raw
hashes, and package imports do not float on mutable registry state.

## lang-cor-imports-versioned-registry

A package manager for an AI-native language cannot be only a downloader. It has
to resolve code and the behavioral contract that code exports. `corvid add`
therefore computes and stores the package semantic summary while writing the
lockfile.

The useful invention is policy-at-install time. Teams can reject packages whose
public exports require approval, violate their own effect constraints, or miss
determinism/replayability requirements before the dependency enters the project.
That makes package resolution part of Corvid's safety model instead of an
external supply-chain step.

## lang-cor-imports-versioned-signed-publish

Signing package bytes is not enough for Corvid. The thing downstream users trust
is bytes plus the AI-safety contract those bytes export. The package signature
therefore covers both the source digest and the computed semantic summary.

That makes signed publish a compiler-facing workflow rather than a registry
decoration. If a package source, URL, version, or exported effect/provenance
surface changes without the publisher re-signing it, `corvid add` rejects it
before the dependency enters `Corvid.lock`.

## proof-carrying custom dimensions

Custom dimensions now have two verification layers. Every dimension still runs
through Corvid's archetype law-check harness, and any dimension that declares a
machine-checkable proof also replays that proof through the relevant assistant:
`.lean` via Lean, `.v` via Coq.

This matters because domain teams can extend Corvid's effect system without
asking the compiler team to hard-code their dimension. The compiler accepts the
extension only if the algebra is executable: property tests pass, and declared
formal proofs actually replay on the local toolchain.

## native shadow replay daemon

Shadow replay is only credible if the daemon can exercise the same tier that
served production traffic. Corvid now makes that tier an explicit daemon
contract: interpreter traces replay under the interpreter executor, and native
traces replay under the native executor selected with `execution_tier = "native"`.

The important invariant is no cross-tier pretending. Native parity is not an
adapter that "mostly" compares native output to interpreter output; it runs the
compiled binary, records a native shadow trace, and rejects traces whose writer
does not match the selected executor. That gives the daemon real deployment
coverage without weakening replay determinism.

## wasm scalar foundation

The browser target has to start from an honest ABI boundary. Corvid can now emit
valid WASM for scalar, runtime-free agents, with JS and TypeScript companions,
but it refuses prompts, tools, and approvals until those capabilities are real
host imports.

That refusal is part of the feature. A WASM target that silently erases approval
or replay semantics would make Corvid less safe than the glue libraries it is
meant to replace. The foundation proves deployment mechanics first and leaves
AI-native host capabilities as the next explicit slice.

## wasm host capability imports

Prompt, tool, and approval calls in WASM are now imports, not erased runtime
magic. The browser host has to provide `prompt.*`, `tool.*`, and
`approve.*` functions, and the generated TypeScript file names the expected
surface.

That design keeps Corvid general-purpose while preserving its AI-native
contracts. A scalar pricing function can compile to standalone WASM; an agent
that calls an LLM compiles to WASM plus a visible host capability requirement.
The next hard part is making those host calls write the same replay traces as
native and interpreter runs.

## wasm trace recording

The generated WASM loader now treats host calls as traceable events. That is the
difference between "Corvid can run in a browser" and "Corvid's replay contract
survives in a browser." Prompt, tool, approval, and run-boundary events use the
same schema names as the interpreter and native tiers.

The remaining gap is execution harnessing: the browser can record compatible
events, but `corvid replay` does not yet drive a WASM module through a
Wasmtime/Wasmer host. That distinction matters because trace shape compatibility
is necessary but not sufficient for deterministic replay.

## wasm browser demo

A browser demo only proves deployment if it imports the generated loader and the
generated WASM module. `examples/wasm_browser_demo` keeps that invariant: the
source is Corvid, the artifacts come from `corvid build --target=wasm`, and the
page supplies the typed host object that the generated `.d.ts` describes.

The honest browser boundary is currently scalar AI-native host capabilities:
prompt, approval, dangerous tool, and trace recording. Strings, structs,
provenance handles, and streaming callbacks are still compiler/runtime work, not
demo-only shortcuts.

## wasm wasmtime parity

WASM parity needs a real runtime in the loop. The Wasmtime harness catches
problems that byte validation cannot: export signatures, host import names,
integer results, branch behavior, and dangerous-action approval flow all have to
work after instantiation.

The harness should follow the WASM ABI boundary, not overclaim beyond it. Today
that means interpreter parity for scalar arithmetic/branching/agent calls and
typed host execution for scalar prompt/approval/tool imports. Full native-corpus
coverage waits for the remaining WASM ABI work.

## lsp diagnostics

The LSP should not reimplement the compiler. Live diagnostics now reuse
`corvid-driver`, which means editor errors match CLI errors for syntax,
resolution, type, approval, effect, provenance, and budget violations.

LSP position mapping has to count UTF-16 code units, not bytes or Unicode scalar
values. Getting this right in a dedicated `position.rs` module prevents hover,
completion, and rename from each inventing their own slightly wrong range math.

## lsp server

The LSP transport should be boring and isolated. `transport.rs` only reads and
writes Content-Length framed JSON-RPC; it does not know how Corvid compiles.
That makes it safe to add hover, completion, and navigation without touching
stdin/stdout framing.

Full-document sync is the correct first server mode. Incremental sync is an
optimization; using full sync keeps live diagnostics correct while the language
surface is still expanding quickly.

## lsp hover

Hover is where Corvid's AI-native semantics become visible while writing code.
It should be compiler-backed for the same reason diagnostics are: effect rows,
approval boundaries, model routes, calibration, grounding, and inferred types
are compiler facts, not editor heuristics.

The initial hover implementation deliberately separates source facts from
protocol transport. `hover.rs` owns semantic summaries; `server.rs` only
serializes the hover response.

## lsp completion

Completion should be context-aware without becoming magical. Approval labels,
effect names, and model names are semantic completions tied to Corvid's
AI-native safety model, while ordinary declarations and keywords keep the
language usable for general programming.

The completion engine should tolerate partial source. Editors ask for
completion while code is incomplete, so `completion.rs` uses the parser's best
available file and narrows by local text context instead of requiring a clean
typecheck.

## lsp navigation

Navigation and rename must use compiler identity, not spelling. A Corvid file
can contain a tool `id` and a parameter `id`; renaming the parameter must not
touch the tool. The resolver already knows this through `DefId` and `LocalId`,
so the LSP should build on that side table.

Single-file navigation is the correct first layer. It gives users definition,
references, rename, and workspace symbols for open documents now, while leaving
cross-file package indexing as a separate package-manager/workspace problem.

## vscode client

The reference editor client should be thin but real. VS Code should not
reimplement diagnostics, hover, completion, or navigation; it should start the
same `corvid-lsp` binary every editor can use and add editor-specific polish:
language registration, highlighting, snippets, restart, and logs.

Server discovery matters for contributors. Supporting explicit setting,
environment variable, repository-local debug/release binaries, and PATH lets the
same extension work in development, installed-tool, and packaged workflows
without hardcoding one layout.

## package manifest lifecycle

Package management has to keep the semantic manifest and immutable lockfile in
sync. `corvid.toml [dependencies]` records the human intent and version
requirement; `Corvid.lock` records the concrete bytes, digest, signature, and
semantic summary. Treating either file as optional creates drift.

Update must reuse add's validation path. A package refresh is still a supply
chain event, so it must re-run source hash verification, signature verification,
semantic-summary extraction, and project policy checks before changing the
lockfile.

## package registry contract

The package registry should be dumb infrastructure. If a registry can be static
`index.toml` plus immutable `.cor` artifacts, the hard security logic stays in
the Corvid client: hash verification, signature verification, semantic-summary
reconstruction, and policy gates.

CDN cache headers are part of the contract. Versioned artifact URLs should be
immutable; if a registry cannot serve `Cache-Control: ... immutable`, users
cannot tell whether a URL is content-stable without relying on trust in the
server.

## package metadata pages

Package pages should be compiler output. A Corvid package is valuable because
of the behavioral contract it exports: effects, approval requirements,
grounding, replayability, determinism, and costs. If those are copied into a
README by hand, they drift. Rendering them from the semantic summary keeps the
registry honest by construction.

Signature provenance is different from source semantics. A local source file can
prove its exported contract, but it cannot prove who published it unless the
registry or publish path supplies a signature. The metadata command therefore
accepts signature provenance explicitly instead of inventing it.

## package conflict verification

Package compatibility is not only semver. In Corvid, the effect contract is part
of dependency compatibility. A package locked yesterday can become invalid today
if the project tightens `[package-policy]` to require replayability,
determinism, signatures, or no approval-required exports.

The right place to enforce that is lockfile verification. `corvid add` prevents
bad new packages from entering the graph; `corvid package verify-lock` proves
the graph remains valid after policy edits, merge conflicts, manual lockfile
changes, or dependency updates.

## test declarations

Testing should reuse the same behavioral assertion language as evals. Corvid
already has trace-aware assertions for process properties; splitting tests and
evals into two assertion models would make one weaker than the other.

The distinction should be runner semantics, not compiler semantics. `test`
declarations are deterministic developer checks; `eval` declarations add
statistical LLM behavior and model-quality reporting. Both can share the same
AST/IR assertion shape.

## test runner

Test execution belongs in the VM, not the CLI. A language-level test can call
agents, prompts, tools, and approvals, so the runner must use the same
interpreter semantics as normal program execution. The driver should compile
and render reports; the CLI should only route arguments and set the process
exit code.

Unsupported AI-native assertions must fail loudly. If `assert called tool` is
preserved by the compiler but the current runner cannot yet inspect traces, a
passing result would be a false safety signal. Reporting an unsupported failure
is less convenient, but it protects the guarantee.

## test mocks and fixtures

Mocks and fixtures should be language declarations, not runner-side text
rewrites. Once they lower into IR, the same resolver, typechecker, LSP,
differential rewrite, and VM paths can validate them.

Mocking must not erase safety boundaries. A mocked dangerous tool still has the
target tool's approval requirement because the interpreter checks the normal
tool gate before substituting the mock body. This keeps tests convenient without
creating a second, weaker execution model.

## test snapshots

Snapshot testing should capture typed runtime values, not rendered source text.
If `assert_snapshot` evaluates through the VM and serializes through the same
value-to-JSON path used elsewhere, snapshots become a stable contract over
program behavior instead of a brittle runner convention.

First-run creation is useful, but silent rewrite is dangerous. Missing
snapshots can be created in normal mode because there is no prior contract to
compare against. Existing mismatches require explicit update mode
(`--update-snapshots` or `CORVID_UPDATE_SNAPSHOTS=1`) so CI failures cannot
accidentally bless behavioral drift.

## trace fixture tests

Process assertions need evidence. `assert called tool` and
`assert approved Label` should not pass because the compiler preserved the
syntax; they should pass only when a trace shows the process happened. Binding
tests to JSONL fixtures gives deterministic CI checks over real production
behavior without re-running LLM calls.

Trace-fixture paths belong in the language declaration, while path resolution
belongs in the driver. The VM should evaluate already-lowered tests and inspect
schema-validated events; it should not guess where the user's source file lives.

## adversarial bypass testing

Corvid should use AI against itself, but the safety gate cannot depend on live
API calls. The stable core is a deterministic taxonomy plus compiler
classifier: generate complete `.cor` bypass attempts, run each through the full
frontend, and treat any clean compile as an escaped safety bug.

The first shipped taxonomy covers approval, trust, budget, provenance,
reversibility, and confidence. Live LLM generation can expand the corpus later,
but it must feed the same classifier rather than becoming a separate testing
path with weaker rules.

## executable spec site

The public spec site should be generated from the same source of truth as CI.
If the website uses hand-written examples, it will drift from the compiler. If
it uses `extract_spec_examples`, every "Run in REPL" block is also a block that
`corvid test spec` can verify.

Static output is enough for this slice. The site ships as plain HTML, CSS, and
JavaScript, with snippets copied into the local REPL. A future browser
playground can replace the copy button, but it should not replace the verified
spec extraction pipeline.

## effect-system bounty process

A bounty without a regression rule is just feedback. Corvid's process requires
accepted bypasses to become permanent `.cor` fixtures, with reporter credit,
the invariant they broke, and a test path that fails before the fix and passes
after it.

The issue template matters because most bypass reports are incomplete on the
first try. Requiring a full program, command, actual result, expected result,
and invariant category makes reports actionable and keeps the corpus from
turning into prose-only bug reports.

## signed dimension artifacts

Custom dimensions are type-system extensions, not ordinary packages. Installing
one must verify authorship and semantics before it enters `corvid.toml`.

The artifact contract is: one declaration, semver version, Ed25519 signature,
optional formal proof, and regression programs. The registry can host these
files later, but trust belongs in the local verifier. A hosted registry should
never be required for the compiler to know whether a dimension artifact is
valid.

## effect dimension registry

A dimension registry should be a distribution convenience, not a trust anchor.
The useful security boundary is local verification: index hash first, artifact
signature second, then law/proof/regression validation before installation.

The registry index should stay boring on purpose. A small TOML table with name,
version, immutable URL, SHA-256, and optional proof URL/hash is enough to support
HTTP/CDN hosting, private local registries, and CI fixtures without creating a
second package manager inside the effect system.

## runnable invention tour

An invention catalog should be executable. If the README says a feature exists
but no command can load a compiling example, the catalog will drift into
marketing. `corvid tour` keeps the catalog honest by making every demo source
compile through the normal driver pipeline.

The useful shape is metadata plus code: category, pitch, spec link, roadmap
anchor, test reference, explicit non-scope, and a REPL-loaded snippet. That gives
developers a fast entry point and gives maintainers a concrete checklist for
future inventions.

## README as proof index

The README should not be a looser version of the roadmap. For an AI-native
language, it has to behave like a proof index: every invention claim points to a
spec, a runnable demo, a roadmap slice, a test, and a non-scope.

This is the difference between bold positioning and shortcut marketing. Strong
claims are acceptable when the reader can immediately follow them to code and a
validation path.

## landing page claim discipline

A landing page can be visually bold without becoming less honest. The safe
pattern is to make the playground command the hero interaction: every invention
card should show how to run the example locally, not just describe it.

Comparative claims are a separate artifact. If a site says "faster" or "safer",
the same card needs a command that reproduces the comparison. Until that
command exists, the stronger product move is to omit the claim.

## proof matrix as marketing infrastructure

The standalone inventions page should end with a proof matrix because that is
what lets bold technical positioning survive scrutiny. Readers can scan the
idea first, then verify status, runnable command, test, spec, and non-scope
without leaving the page.

This structure is stronger than a traditional feature grid. It treats
non-scope as first-class, which prevents the project from accidentally turning
future inventions into present-tense claims.

## invention shipping contract

The invention catalog only stays true if it becomes a contribution rule. The
right default is not "remember to update docs later"; it is "the invention is
not shipped until README, tour, proof matrix, spec, tests, and non-scope all
land together."

This turns marketing discipline into engineering discipline. Future launch
claims become a byproduct of shipped proof instead of a separate cleanup pass.

## Contributing / feedback

See [CONTRIBUTING.md](CONTRIBUTING.md). The rules of the road are: design chat before code, per-scope commits at every boundary, dev-log entry for every session, no shortcuts. The `learnings.md` file you're reading gets updated when each user-visible feature ships.

## Phase 20j file-responsibility closeout

Responsibility regrowth mostly followed three repeatable paths: large actor
impls collected new domains, CLI command files absorbed sibling subcommands,
and lowering/codegen roots accreted helper passes. The durable fix was not a
line-count target by itself; it was naming the domain boundary and moving each
domain into a sibling module with the same public facade.

Multi-impl splits across sibling modules are now a proven pattern for runtime
actors. They keep call sites stable while moving methods by responsibility.
When siblings need helper access, `pub(super)` on the helper or field is enough;
larger re-export surfaces were not needed.

The validation baseline matters more than command tails. On Windows, `corvid
verify --corpus tests/corpus` currently exits `2` with the known `whoami`
linker error; capture the real exit code with file redirection and compare the
signature, because piping to `tail` hides the failing command's exit status.

`rustfmt` on a `#[path]` root module can follow and rewrite sibling modules.
For scoped refactor commits, format touched leaf files directly and restore any
incidental sibling formatting before committing.

## Phase 20k strict responsibility closeout

The stricter "exactly one responsibility" pass mostly found concept pairings
that the looser 20j rubric intentionally allowed: records plus an actor
facade, a facade plus cross-domain tests, and command dispatch plus embedded
subcommand-specific logic. The durable pattern is to keep the facade in the
root and move records, tests, and per-subcommand logic to named siblings.

Cross-domain test relocation works best when the original test harness stays
as a shared parent module until the final extraction. For test files with
embedded source programs, preserve the moved block text exactly; stripping
indentation from Rust code also strips indentation inside the embedded Corvid
program and changes parser behavior.

Public API preservation is cleaner with narrow `pub(super)` helpers than with
record re-exports. When a root only needs one field from a moved private
record, expose a domain helper such as `read_summary_cost_usd` instead of
making the whole record visible again.

Line count stayed useful as a smoke test but not as the final rule. Several
large files remain valid under the strict rubric because they are one command
enum, one renderer, one declaration function, one type implementation, or a
pure integration-test module. The audit has to name the responsibility, not
just count lines.

## regression corpus naming

An internal seed corpus and a public submission process are different claims.
Call the checked-in fixtures a seed or internal regression corpus until
accepted external reports actually land in that corpus. The bounty page and
issue template can be documented as the intake path, but source, specs, and
roadmap copy should not imply the corpus has already been fed by external
submissions.

## Support replay denial fixtures

Plain replay preserves approval-denied runs as nonzero command exits even when
the trace matches exactly. Negative approval fixtures should be committed and
tested as expected-denial cases in CLI or CI wrappers, while successful replay
fixtures remain the ones used for straight `corvid replay <trace>` commands.

## RAG replay timestamps

Grounded retrieval values carry provenance timestamps minted when a tool result
is wrapped, not just when the source trace was recorded. Plain replay for RAG
must preserve prompt, value, source identity, and result checks while
normalizing only retrieval-source `timestamp_ms` fields in grounded LLM args
and rendered context. Otherwise deterministic fixtures fail for the wrong
reason even when mock, replay, and real typed surfaces agree.

## hosted registry claims

Package format, signed-publish tooling, and a resolver that can read local or
self-hosted indexes are not the same as operating a hosted registry. Avoid
default service URLs unless the service actually exists, and keep the
`OutOfScope` guarantee id aligned with docs and generated semantics so the
honesty boundary is machine-visible.

## stdlib adversarial coverage

For repo-native Corvid stdlib modules, many safety boundaries are exported
helper shape plus metadata fields rather than Rust runtime branches. Negative
import/call tests are useful adversarial coverage: they prove there is no
public unsafe helper with the bypass shape, while source-shape assertions pin
redaction, provenance, effect-key, and replay-key metadata in the module that
owns the surface.

## optional feature CI

Feature-gated runtime paths need their own named CI job, not a comment in the
default workspace tests. Keep the job id behavioral (`python-features`), pin
the external runtime version, and document exactly which optional contracts the
job covers so future maintainers know why it is separate.

## wasm browser storage

IndexedDB is asynchronous, while the current Corvid WASM ABI is synchronous and
scalar. Keep durable browser state in the generated ES host layer for now: emit
a typed async store helper, have the demo call it around the WASM run, and make
the browser CI reload the page to prove persistence instead of pretending the
WASM module can synchronously block on IndexedDB.

## platform parity gates

When a platform-parity slice needs Windows coverage but the native linker has a
known environmental baseline, use a parity harness that is genuinely
cross-platform and avoids that baseline. For the current repo, the
WASM/Wasmtime harness is the right installer/doctor companion because it runs
the same generated module path on Linux, macOS, and Windows.

## benchmark closeouts

Before assuming a large benchmark slice still needs corpus work, count the
committed cases and rerun the deterministic drift gates. The 33N scaffold had
already grown to the full 50 compile-time cases and 3 governance apps; the real
remaining work was closing stale README/ROADMAP language after runner output
matched committed results byte-for-byte.

## reference demo wrappers

A one-command demo may expose missing CLI convention support before it exposes
demo logic. For `examples/refund_bot`, `corvid build` and `corvid run` needed
project-default source resolution, and `corvid replay <trace>` needed to use
the already-shipped plain replay runtime path plus `SchemaHeader.source_path`.

Keep the demo fixture boring and deterministic: a seed JSON file, a source eval
that asserts the visible contract, and a replay trace with only schema, seed,
run-started, and run-completed events were enough to prove the approval-gated
moat without inventing a mock provider surface that this app does not need.

For LLM-backed demos, wire every CLI execution surface that can call prompts to
the same provider runtime. `local_model_demo` exposed that `corvid run` already
had env mock plus provider adapters, while `corvid test` and `corvid eval` still
used bare runtimes. Once those shared the same mock surface, the Corvid tests,
eval harness, and one-command run all exercised the same typed program without
requiring a live Ollama process.

Plain replay fixtures need to match the no-env replay runtime, not just the
recording environment. The captured local-model trace included
`model="ollama:llama3.2"` because `CORVID_MODEL` was set during recording, but
plain replay substituted responses with no default model configured. Normalizing
that field to `null` kept replay deterministic while the model identity stayed
covered by tests and real-provider docs.

Provider-routing replay keeps the selected model name but not the catalog-only
model version on the substitution events. `provider_routing_demo` showed that
plain replay compiles the source with the default replay runtime, which does not
load the demo's `corvid.toml` model catalog. The route still selects
`openai_fast`, `ollama_local`, or `anthropic_deep`, but `model_version` must be
`null` on `llm_call` and `llm_result` for byte-strict substitution to match.

## code review demo replay

Structured prompt responses replay cleanly as JSON objects, but plain replay
still matches the runtime's model metadata exactly. `code_review_agent` showed
that a trace recorded with the env mock can include a default model name while
plain replay reruns without that model configured. Normalize `model` and
`model_version` on substitution events to the no-env replay runtime, and keep
provider identity covered by tests and real-provider docs instead of forcing it
into the replay fixture.

Eval assertions may invoke the same tool/prompt path more than once. If an eval
has two assertions that each call `main`, provide two queued mock tool results
and two queued mock LLM replies; a single mock response can pass `run` and
still exhaust during `eval`.

## reference demo pack closeout

The reusable shape across the six demos is not a new demo framework; it is the
plain project layout plus env-backed typed mocks, eval queues, and committed
replay substitution traces. `demo_project_defaults` is the useful local guard
because it catches the same "works as a source file but not as a project" drift
that CI would otherwise surface later.

When closing a demo pack, verify the workflow itself names every demo's build,
run, test, eval, and replay loop. A demo can have complete local artifacts and
still miss the closure gate if CI only runs the Rust-side smoke test.

## refund bot hardening

`mock` is a reserved Corvid keyword, so replay-invariant tests should avoid it
as a local binding name even when the provider mode is conceptually called
mock. Use names like `mock_result`, `replay_result`, and `real_result` to keep
the invariant readable without tripping the parser.

Do not overclaim semantic approval scope until the checker has field-level or
amount-ceiling predicates. A refund that receives some prior approval still
compiles even if the business meaning of the amount changed, so the hardening
test should cover the compiler-enforced boundary and the security model should
name semantic ceilings as a non-goal.

## local model hardening

Plain replay for LLM traces must run without provider env that changes model
metadata. `CORVID_MODEL=ollama:llama3.2` is correct for mock build, run, test,
and eval, but it makes plain replay observe a model name where the committed
trace expects `null`. Clear provider env before replaying deterministic
fixtures.

For Corvid boolean assertions, do not rely on Python-style multiline `and`
continuations. The parser treats the newline after `and` as the end of input.
Use named intermediate booleans and a single-line final return when an
invariant has several field comparisons.

Local LLM hardening should avoid claiming semantic prompt-injection prevention
unless there is a concrete checker or runtime policy for it. The enforceable
boundary here is `replay.deterministic_pure_path`: prompt-dependent flows
cannot be mislabeled deterministic, while model quality and instruction
following stay explicit non-goals.

## provider routing hardening

Provider routing can keep provider identity in the app-facing type without
depending on live provider metadata during replay. The useful invariant is that
`policy`, `selected_provider`, `selected_model`, `question`, and `answer` match
across mock, replay, and real surfaces; token counts, latency, and invoice data
belong in trace host events.

Provider-swap safety needs a direct invariant over each named route. The replay
invariant now checks standard/OpenAI, private/Ollama, and deep/Anthropic in one
test, which is stronger than relying on the one-command `main` path that only
exercises the standard route.

For multi-provider demos, mirror replay fixtures under `seed/traces` and wire
both locations into CI. The seed tree is what the hardening docs and runbook
point operators at, while the original demo-pack traces preserve the 33K
one-command demo contract.

## RAG QA hardening

For adversarial grounded-return fixtures, the registered guarantee emitted for
unsourced `Grounded<T>` construction is `grounded.provenance_required`. Do not
guess at a more general provenance id in harness assertions; make the
security-model threat table match the guarantee the checker actually returns.

The useful mock/replay/real invariant for RAG is over the app-facing
`RagAnswer`: `question`, `answer`, `source`, and `grounded`. Embedding vectors,
token counts, latency, and cost are provider metadata, so keep them in
seed/trace metadata instead of forcing them into the typed answer surface.

RAG hardening should avoid claiming source truth or model-level
prompt-injection immunity. The enforceable boundary is that ungrounded text
cannot be returned as a grounded answer; document KB authoring quality and live
provider behavior as operator responsibilities.

## support escalation hardening

Negative replay fixtures should stay first-class in hardening docs and CI. The
support escalation approval-denied trace is correct only when it exits nonzero;
CI now checks both the original trace and the mirrored seed trace as
expected-failure cases.

Tenant crossing can be represented in this demo only through the existing
`customer_id` seed data. The enforceable guarantee is still the approval
boundary before `issue_refund`; broader tenant authorization needs a separate
auth policy and should be documented as a non-goal until such a checker exists.

Eval assertions may call the default path more than once. For support
escalation, the eval harness needs queued `lookup_order` and
`escalate_to_human` mock responses, while build, run, and normal tests can use
the simpler single-response mock shape.

## code review agent hardening

The credential scan can match documentation that prints the literal scan
pattern. Use a self-excluding regex in docs, such as `[s]k-|g[h]o_|w[h]sec_`,
so the command stays useful without making the app fail its own scan.

The code review app's enforceable hardening boundary is the GitHub write
approval gate. Prompt injection, token-leak, and supply-chain source cases can
all be represented as unapproved write attempts today; semantic review quality
needs eval coverage and operator documentation rather than a stronger compiler
claim.

Mirrored seed traces need their `source_path` adjusted relative to
`seed/traces/`. For code review replay, `../../src/main.cor` keeps the
hardening trace runnable without duplicating the program.

## 42H reference app hardening

The reusable mock pattern across the hardening pass is not a new connector
trait. Each app exposes one small typed result surface in Corvid and lets mock,
replay, and real modes converge on that shape. That kept replay invariant tests
simple and avoided app-specific connector forks.

Most app threat models collapsed to one of three enforceable classes: dangerous
write requires approval, typed provenance must not be fabricated, and replay
fixtures must stay deterministic and redacted. Anything beyond those classes
belongs in evals, provider docs, or operator runbooks unless the compiler has a
registered guarantee for it.

## Phase 20l first-impression gap repair

Responsibility-rubric clean (20j/20k) and first-impression clean are different
gates. The 20j/20k passes can leave the workspace structurally sound while a
stranger running `corvid check && corvid build && corvid run` on a non-trivial
sample app still hits behavioural gaps the test fixtures didn't surface. Add a
periodic external-reviewer test before any user-visible release; the cost is
small, the leading-indicator value is large.

Three recurring "first-impression failure" shapes worth scanning every release:
path-anchored API used in some `cmd_*` paths but not others (the L-1 shape —
`corvid check` was calling the path-less driver entry despite holding a
`Path`); codegen TODO comments shipping as `object`-shaped type-fidelity
regressions ("safe approximation until..." in `python_type_hint_of` collapsed
nested struct, list, and option fields to `object` and defeated mypy/pyright);
and renderer surfaces that didn't auto-detect terminal/no-color/verbosity
context (the L-6 shape — ariadne emitted ANSI escapes into PowerShell conhost
because nothing checked `is_terminal()`).

Don't trust the surface phrasing of an external gap report. The reporter framed
20l-D as "auto-dispatch picks native when interpreter would suffice"; the
actual bug was an unactionable error message on the staticlib-missing path.
The dispatch logic was intentional and tested. Verify the diagnostic site,
the existing message, and the user-visible recovery path before drafting the
fix — sometimes the report tells you "this hurts" but not "where the wound
is."

`from __future__ import annotations` in the generated Python module means
unquoted forward references resolve at typecheck time. Prefer the unquoted
form (`inner: Inner`) over string-quoted (`inner: "Inner"`) for readability.
PEP 604 union syntax (`T | None`) likewise works on Python 3.10+ where the
`__future__` import is present. Keep the codegen aligned with the Python
version floor stated in the runtime preamble.

ariadne 0.4 only mostly respects `Config::with_color(false)` —
`ReportKind::Custom`'s embedded color and a residual `\x1b[39m` reset still
slip through. Post-render ANSI-stripping is the smallest behaviour-preserving
fix; switching to a built-in `ReportKind` would change the diagnostic kind
text. Document the workaround in the helper so future ariadne upgrades can
remove it once the upstream behaviour tightens.

When verifying a docs-only fix, double-check claimed grammar against the
parser before the spec ships. The first 20l-E draft documented a `dangerous as
Bar` opt-in syntax for custom approve labels — verified against
`expected_approve_label: pascal_case(tool_name)` in
`crates/corvid-types/src/{approval_reachability.rs, checker/call.rs,
checker/import_call.rs}`, no override path exists, removed before commit.
Aspirational docs are how language-identity drift starts.

`\` line continuation outside strings stays deferred. Corvid is positioned as
AI-native, not Python-shaped; the existing workarounds (`+` concatenation,
triple-quoted strings, paren-grouped continuation) are clean and stay
documented. Rejecting Python-mimicry features when the language identity
argument outweighs the ergonomic argument is itself a learning, not a TODO.

## Phase 20m verifier-driven corrections

`expected_*` fields are diagnostic suggestions, not acceptance criteria. To
find what the checker actually accepts, find the comparison site, not the
suggestion field. The L-8 docs error in 20l-E happened because I checked
`expected_approve_label: pascal_case(tool_name)` in three call sites and
concluded "labels are PascalCase." That field is the help-hint suggestion
the checker prints when nothing matches; the acceptance check is at
`crates/corvid-types/src/checker/call.rs:127` —
`snake_case(&a.label) == tool_name`, which accepts any reasonable casing
that round-trips through `snake_case`. I had logged this exact discipline
in 20l-E's learnings entry and immediately violated it. The lesson now has
its own line in this file so the next docs-only fix doesn't repeat the
mistake.

Verifier-correction pattern. After a first-round fix phase like 20l, the
right next step is a verification round: same external reviewer (or a
fresh one) re-tests against the post-fix HEAD and produces a scorecard
confirming verbatim entries, flagging wrong details, re-framing wrong
root-cause framings. The corrections phase that follows (20m here)
addresses only verifier-confirmed corrections; adjacent gaps surfaced
during verification (the REPL hardcoded ANSI escapes 20m-B verification
turned up) are filed as separate follow-ups rather than rolled in. Two or
three rounds usually suffice before the gap report converges on "no
actionable corrections." The pattern is documented in
`docs/phases/phase-20m-verifier-corrections.md` and
`memory/project_phase_20m_closed.md` so the next external-reviewer round
slots in without re-deriving the workflow.

UX preference: silent auto-fallback over actionable error when the
recovery path is mechanical and the user wasn't asking for the failed
mode specifically. 20l-D made the missing-staticlib diagnostic readable;
20m-B observed that for `corvid run` (auto target) on a binary-install
machine, the user wasn't asking for native — they were asking for "run
this program." Silent fall-back via the existing `↻ running via
interpreter:` UX prefix is the right move. Explicit `--target=native`
keeps the actionable diagnostic; opting in earns the explicit error. Rule
of thumb: when the dispatcher has multiple tiers and the user picked
"auto," failures in any one tier should fall through to the next tier
that can actually run the program before bailing.

Narrow string-matching of stable diagnostic phrases is acceptable as a
control-flow primitive when the phrase is workspace-owned and pinned by a
unit test. `is_missing_staticlib_error` matches on the canonical
`"corvid-runtime staticlib missing"` and
`"CORVID_RUNTIME_STATICLIB_OVERRIDE points at non-existent path"`
because both are pinned by the link.rs unit tests; if either upstream
wording changes, the link.rs test fails before this matcher silently
mismatches. The inverse rule: never string-match on third-party error
messages that you don't own.

## Phase 20n open-gap implementation

### Slice 20n-A — backslash line continuation (L-7)

Prior-decision reversals get explicit reversal markers, not silent
absorption. The 20l-F learnings entry deferred `\` line continuation on
language-identity grounds — "Rejecting Python-mimicry features when the
language identity argument outweighs the ergonomic argument is itself a
learning, not a TODO." The 2026-05-08 directive overrode that with
"implement the feature end-to-end." The 20l-F entry stays as record of
the original rationale; this 20n-A entry stands alongside it as the
explicit reversal record. Both are preserved so future sessions can see
*decision* (the override is documented) versus *drift* (someone forgot
the prior decision and reimplemented). When a deferral is reversed, the
phase doc and the learnings file both carry the reversal note before
any code lands.

The lexer-level continuation rewrite happens at two sites — top-level
between tokens and inside `"..."` strings — but uses one shared helper.
The peek-and-consume logic is identical: `\` immediately followed by
`\n` (or CRLF `\r\n`) plus any leading whitespace on the next physical
line. Triple-quoted strings already span lines naturally, so they are
intentionally not rewritten — special-casing them in the helper would
have been a "fix" for a non-problem. Preserving `had_content_on_line`
across the join is what keeps `Indent` / `Dedent` emission sane; without
it, the join would have looked like an outdent to the indent tracker.

### Slice 20n-B — WASM String ABI (L-4)

Real allocator over bump allocator, even at v1. The 1000-iteration
churn integration test in `crates/corvid-codegen-wasm/tests/allocator.rs`
pins the design choice: page count must stay at 1 across alloc/free
cycles. A bump allocator could not have passed that test. Hand-rolling
the free-list in `wasm_encoder` Instructions kept the WASM module self-
contained — the alternative (linking a pre-built C allocator) would
have introduced a build-system dependency that obscured what the module
actually contains. The two-pass coalescing (forward sweep merges
block-after-self; backward sweep merges block-before-self) is the
minimum that lets repeated alloc/free of equal-sized blocks reuse the
same memory.

Multi-value WASM returns over sentinels or out-pointers. The `String`
return shape is `(result i32 i32)` rather than a length-prefixed in-
memory sentinel or an out-pointer parameter. Multi-value is in
WebAssembly's stage-4 spec, supported by every modern engine, and
exposed by `wasmtime`'s `TypedFunc<(i32, i32), (i32, i32)>` and JS's
`WebAssembly.Function` destructuring without feature flags. Sentinel
designs would have aliased the input span against the return span,
breaking the ownership story. Out-pointer designs would have required
every caller to pre-allocate the result buffer at a size they couldn't
predict for runtime-computed strings. Multi-value sidestepped both
problems and matched the bare-ABI ergonomics goal.

`Br` label discipline in nested WASM blocks is unforgiving. The
allocator's pass-2 backward-coalesce inside an `If` inside a `Loop`
inside a `Block` has three branchable scopes: label 0 = the `If`,
label 1 = the `Loop`, label 2 = the outer `Block`. The first cut used
`Br(2)` to "skip self," intending to continue the sweep — but `Br(2)`
exits the outer Block entirely and terminates the walk. Caught by the
`forward_coalesce_lets_a_larger_alloc_fit_into_two_freed_blocks`
integration test, where alloc(28) returned the bump address (52)
instead of reusing the 36-byte coalesced block. Fixed by switching to
`Br(1)`, which jumps back to the Loop start. The defensive comment
explaining the label nesting now stays in `allocator.rs` — the next
person who edits the inner branch shouldn't have to re-derive the
nesting from scratch.

Uniform manifest `kind` discriminator on every parameter and return,
not just the new type. The temptation when adding a new ABI shape
("string") is to add a `kind: "string"` field only to the String
entries and leave the existing scalar entries human-readable. Resisted:
every entry now carries `kind` — `"i64"` for Int, `"f64"` for Float,
`"i32"` for Bool, `"void"` for Nothing, `"string"` for String.
Downstream tooling (registry consumers, alternative loaders, analysis
tools) switches on `kind` for ABI shape rather than parsing the
human-readable `ty` field. The `ty` field stays for human readers; the
`kind` field is the parseable discriminator. This is the pattern to
reach for whenever a manifest gains a new shape — make the
discriminator uniform across the existing shapes too, even if the
existing shapes were unambiguous before.

JS frees inputs only; agent returns may alias. The v1 ownership
convention sidesteps the "did the agent return an input span, a
literal, or a fresh allocation?" question by **not requiring the host
to know**. The host allocates inputs via `corvid_alloc`, decodes the
return into a host-owned string copy *before* freeing inputs, and
frees only the inputs it allocated. Agent returns are not freed by
the host. The generated JS loader's `finally` block guarantees the
input free runs even when the agent throws mid-call. Distinguishing
the three return cases would have required a third return field (an
ownership tag) and is deferred to a later slice if it ever becomes
necessary. For now, the cost of one extra TextDecoder copy is cheaper
than the cost of an ownership-tag-carrying ABI.

Compile-time string-literal pool deduplication via content-keyed
interning. `StringPool` is a `HashMap<String, u32>` keyed by the
literal's UTF-8 bytes; identical literals across multiple agents
collapse to a single pool entry. The pool is emitted as a single
active `DataSection` segment at memory offset `HEAP_BASE = 8` (right
after the 8-byte null-pointer sentinel), and the runtime heap starts
immediately past the pool at `8 + pool_size`. Literal addresses and
runtime allocations never alias because of the offset; the literal
space and the heap space are physically disjoint by construction.
Deduplication isn't an optimisation — it's the property that makes
"two agents both return `\"hello\"`" a single pool entry, which keeps
the bundle small even when many agents share boilerplate strings.

Per-slice learnings discipline: 20n-A's learnings landed at the 20n-B
closer rather than at the 20n-A close. That's a one-slice slip on the
"doc-and-feature land together" rule. The fix is procedural: every
slice closer commit must touch `learnings.md` even if the entry is
short. The 20n-A entry above was added retroactively here; the
prevention is to put a learnings-touch line in the per-slice closer
checklist, not to skip the entry.

### Slice 20n-C — native struct prompt + entry returns (L-3)

Step-0 audits correct phase-doc framing before they correct anything
else. The 20n-C phase doc said "mirror the `Grounded<T>` heap-
allocation pattern." The audit found `Grounded<T>` is a handle-store
for attestation metadata, not a heap-allocation pattern for the
value itself — the value crosses scalar and the handle indexes a
process-global slotmap. The actual analog was
`lower_struct_constructor`'s `corvid_alloc_typed(size, &typeinfo)`
call with 8-byte field slots. Recording the framing correction
(rather than silently doing the right thing) keeps the audit
discipline visible: "the phase doc was wrong, here's the corrected
framing, here's why" lets future sessions recognise the same
analogy mistake elsewhere.

Codegen-emits-per-type beats typed-bridge multiplication. The
runtime gains exactly one new bridge (`corvid_prompt_call_struct`)
plus generic JSON primitives. Codegen emits one decoder per
`Type::Struct(def_id)` (cached by `DefId`), and the bridge calls
into it via a C function-pointer callback. Adding a new prompt-
return type later (List, Optional, Result, Stream) means a new
codegen emitter and re-uses the existing one runtime API surface —
no combinatorial explosion. The opposite path (a typed bridge per
return-type-shape) would have produced
`corvid_prompt_call_list_int`, `corvid_prompt_call_list_string`,
`corvid_prompt_call_optional_int`, ad infinitum. Generalises to:
when the runtime's API surface threatens to grow per-type, push the
type-specific work into codegen and keep the runtime's surface
uniform via a callback or descriptor parameter.

Rename storage maps when their semantics outgrow their original
caller. `string_attestations` was the original name because the
first caller was the CorvidString descriptor pointer. The
underlying map (`HashMap<usize, Arc<GroundedAttestation>>`) was
always heap-pointer-keyed and worked for any refcounted heap object.
Adding a parallel `struct_attestations` map for `Grounded<Struct>`
would have introduced duplicate state for no capability gain. The
no-shortcut answer was to rename to `pointer_attestations` (4
helpers + 2 call sites + 3 tests touched) so the storage's actual
semantics became visible in the code itself. Future heap shapes
(lists, future allocations) reuse the same path without further
rename churn. Generalises to: when extending a single-purpose
storage's semantics to cover a second purpose, rename the storage
to reflect both purposes rather than introducing a parallel one.

Schema generation belongs in its own crate, not the interpreter.
The JSON Schema generator (`schema_for(&Type, &types_by_id) ->
Value`) lived in `corvid-vm/src/schema.rs` because the interpreter
happened to need it first — a historical accident, not a layering
decision. When the native code generator needed the same logic in
20n-C, "import from corvid-vm" would have pulled the entire
interpreter into the codegen-cl dep tree to reach a 200-line schema
function. The fix was to extract `corvid-prompt-format` as a
dedicated crate with deps on `corvid-types`, `corvid-ir`,
`corvid-resolve`, `serde_json` — exactly the language-level types
the schema needs and nothing else. Bonus: future LLM-provider
schema dialects (Gemini's structured-output, future Anthropic
revisions) have a discoverable home. Generalises to: when a
language-level concern lives in a crate it doesn't belong in
because of historical accident, the right move is to extract a
shared crate with the minimal dep set, not to import the
historical-host crate everywhere.

Field order = source order, preserved end-to-end. The CTO design
decision for struct-to-JSON encoding was "fields in `IrType.fields`
declaration order matches what the interpreter does in
`value_to_json`." The naive implementation
(`serde_json::Map<String, Value>` + `serde_json::to_string`) would
have alphabetised because Map is `BTreeMap`-backed. Flipping the
workspace-wide `preserve_order` serde_json feature would have
changed every existing JSON path. The right answer was a builder
that uses `Vec<(String, Value)>` internally + a hand-rolled
`serialize_object_in_insertion_order` helper that delegates each
value's serialisation to `serde_json::to_string` while writing the
outer object structure directly. No new deps, no workspace-wide
feature flag flip, source-order preserved everywhere it matters.
Generalises to: when a default behaviour conflicts with a contract
you need (here: insertion-order field iteration), prefer a local
override over a global feature flag, especially when the global
flag has cross-cutting effects.

Refcount discipline at FFI boundaries: retain-before-consume when
a callee takes +1 ABI from a borrowed source. The struct encoder
loads each `String` field as a borrowed descriptor pointer (the
struct still owns it). The runtime's
`corvid_json_object_set_str` consumes the string's +1 refcount via
its `read_corvid_string` move; if the encoder didn't retain first,
the struct's destructor would later release a stale pointer. Every
boundary like this — borrowed-load + consuming-callee — needs an
explicit `corvid_retain` between the load and the call. The
inverse pattern (the prompt bridge wrapping the LLM response in a
fresh `string_from_rust` and releasing after the decoder returns)
keeps the bridge owner of the +1 even though the decoder borrows.
The general rule: at every C-ABI handoff involving a refcounted
value, name explicitly which side owns the +1 and emit the
retain/release accordingly — uncommented "borrow" calls inside
mixed +1 / +0 ABI surfaces are how use-after-free regressions
land.

Panics through `extern "C"` abort, they don't unwind. All five
`corvid_prompt_call_*` bridges use `extern "C"` (not
`extern "C-unwind"`) for stable codegen ABI compatibility. A Rust
panic crossing such a boundary aborts the process rather than
unwinding. `std::panic::catch_unwind` cannot catch it. Tests that
need to validate panic-on-failure behaviour through a C-ABI bridge
have to drive the bridge end-to-end (compile a binary, run it
under a misconfigured mock, observe the abort + stderr) rather
than asserting via `catch_unwind` in a Rust unit test. The 20n-C
struct-bridge integration test made this scope-call explicitly
(decoder-always-fails was dropped from the unit-test plan because
of this constraint, with the rationale documented in the test
file's comment block).

### Phase 20n cross-slice patterns

Three patterns survive their slice contexts and apply to future
language-gap closure work:

**Design-reversal recording.** When a deferral is reversed, the
phase doc, the learnings file, and the memory record all carry
the reversal note before any code lands. The original-decision
entry stays as the original-rationale record; the new entry
stands alongside it as the explicit reversal record. Both are
preserved so future sessions can see decision (the override is
documented) versus drift (someone forgot the prior decision and
reimplemented). The 20l-F → 20n-A reversal on backslash line
continuation is the worked example.

**Step-0 audit before substantive feature slices.** Substantive
feature slices (not bug fixes) need a read-and-plan pass before
any code. The audit's job isn't just to confirm scope — it
sometimes corrects the phase doc's framing. 20n-B's audit shaped
the multi-value WASM ABI choice; 20n-C's audit corrected the
"mirror Grounded<T>" framing into "mirror lower_struct_constructor."
A refined plan goes back for pre-phase chat before code starts.
The mid-implementation scope expansion is the failure mode this
prevents.

**Codegen emits per-type, runtime stays type-agnostic.** When a
new language-level type shape (struct return, list return,
optional return) needs runtime support, the right division of
labour is: codegen emits one function per concrete type instance
(cached by `DefId` or equivalent), runtime exposes generic
primitives the codegen-emitted code uses. Runtime's API surface
stays at one bridge per concept, not one bridge per type-shape
combination. The pattern shipped in 20n-B (per-struct WASM
manifest entries with uniform `kind` discriminator) and in 20n-C
(per-struct decoders/encoders + JSON primitives).

**Rename storage maps when their semantics outgrow their original
caller.** A storage primitive named after its first caller often
turns out to serve a broader set of callers later. Renaming
upfront when the second caller arrives — instead of introducing a
parallel storage path — produces one canonical storage and one
canonical pair of helpers that future shapes can extend without
further churn. 20n-C's `string_attestations` →
`pointer_attestations` is the worked example.

**Multi-value / callback parameter over typed-bridge
multiplication.** When a C-ABI surface needs to carry information
that varies per type (the schema for a struct prompt return, the
decoder logic for a struct, the field count for an array return),
the right shape is a single function with a multi-value return or
a callback parameter. Combinatorial typed-bridge families
(`prompt_call_list_int`, `prompt_call_list_string`,
`prompt_call_optional_int`, ...) explode quadratically; one bridge
+ a callback or descriptor stays linear. 20n-B's multi-value WASM
returns and 20n-C's function-pointer decoder callback both fit
this pattern.

## Phase 35 closeout — defensible core

Registry as single source of truth scales when every consumer
derives, not duplicates. Phase 35-A's `GUARANTEE_REGISTRY` is
read by `corvid contract list`, `docs/reference/core-semantics.md`
generation, the bilateral verifier, `corvid claim --explain`,
and `corvid build --sign`. A drift-gate test
(`rendered_markdown_matches_committed_doc`) catches divergence
between the rendered spec and the committed doc; CI runs it on
every push. When N artifacts must agree, generate them all from
one in-code source of truth and gate divergence at CI rather than
relying on human discipline.

Registry honesty pinned in three orthogonal directions: forward
(`with_guarantee` debug_assert verifies tagged ids resolve in
the registry), inverse-broad (every Static/RuntimeChecked id
appears as a literal in non-test workspace source), inverse-
narrow (every typecheck-shaped Static id goes through the
tagged constructor). Each sentinel catches a different drift
mode. Phase 20m's "verify the comparison site, not the
suggestion field" rule generalises: every comparison-site
property gets its own sentinel; one isn't enough.

Honest classification beats optimistic tagging. Phase 35V-T1-B
found four typecheck-shaped Static rows whose enforcement
mechanism didn't fire a separately-tagged diagnostic. The
options were: discriminate in code (real engineering work), or
honestly downgrade to OutOfScope with explicit
`out_of_scope_reason`. The shortcut would be inventing fake
"subsumed_by" relationships that paper over the lack of separate
diagnostics. The phase chose discrimination where the
typechecker had the information (added an `approvals_seen_in_agent`
body-wide audit log) and honest downgrade where the unified
analyzer fundamentally fires one diagnostic for both
perspectives. Downgrading is not a shortcut; claiming Static
when only the parent enforces is.

Aspirational launch wording surfaces at verification, not at
implementation. Phase 35V-T1-H found ROADMAP, README, and
docs/security/model.md all carried "bilateral verifier" / "two
implementations" / "TCB shrinkage" claims that the implementation
doesn't deliver. The shipped verifier IS useful (post-link
descriptor tampering, build-cache drift) but not at the level
the wording promised. The corrective work was to tighten the
wording, not to invent the missing implementation. Launch
surfaces (ROADMAP slice descriptions, README claim boundaries,
security model doc) are checkable against shipped behavior; a
verification round that doesn't audit them misses a load-bearing
class of drift.

Cross-component coupling discovered at verification time. Phase
35V-T1-B downgraded three registry rows. Phase 35V-T1-J found
that `validate_signed_claim_coverage` in the driver still
required those ids in every signed claim set — without the
validator alignment, signed builds for any source touching those
surfaces would have rejected at sign time. Existing tests
(`signed_claim_coverage_*`) tripped after the downgrade,
catching the coupling. A registry change has cross-component
consequences that a phase-level audit catches but a slice-level
review does not.

Pre-existing baselines that survive multiple phases need
periodic re-evaluation. The whoami `secur32.lib` linker baseline
filed by Phase 20n was treated as "expected exit=2" through
20n, 20n-A/B/C, and the early Track 1 slices of Phase 35V. T1-H
found the fix was a one-line addition to two MSVC linker
invocation sites, AND it unblocked the bilateral-verifier tests
that 35-H ships as evidence. Lesson: when a "filed for later"
baseline is small enough to fix now, fix it now — especially when
it's blocking adjacent verification work. The cost of carrying
the baseline (every commit message qualifies "exit=2 is the
baseline") was higher than the cost of the fix.

The verifier-correction pattern from Phase 20m scales to a
launch-gate surface. Phase 35V applied it to 14 launch-gate
slices + 12 audit-correction slices + 4 closer slices (~30
verifications total). Outputs: 8 commits of corrective work in
Track 1, zero corrective work in Track 2 (all 12 audit-
correction tracks were honestly shipped), 4 closer commits in
Track 3. The pattern's value scales with the breadth of what's
claimed; per-slice verification cost stays roughly constant.

## Phase 35V closeout — running a launch-gate audit as its own phase

Audit rounds need their own phase, not a parallel review. Phase
35V was sized as a full phase (~30 slice-equivalents) with its
own ROADMAP entry, its own pre-phase chat checklist, and its own
slice-by-slice closer ceremony. Treating verification as
"inline polish on Phase 35" would have either been skipped
under deadline pressure or absorbed silently by Phase 35's slice
list. Sized as Phase 35V, the work has line-of-sight commits,
explicit corrective-track scope, and a closing audit that future
external reviewers can read. The phase-as-audit shape is what
makes the verifier-correction pattern carry weight at launch
scale; the audit's authority comes from its independence from
the work it audits.

Three-track decomposition keeps verification scope honest. Track
1 verifies the slices of the phase being audited (Phase 35).
Track 2 verifies prior audit corrections still match shipped
behavior (the 36/38/39/41 audit-correction completeness check).
Track 3 closes phases via the formal closer ceremony. Mixing
these tracks would have led to ambiguous "what does this slice
verify?" commits. Separating them gives each commit one job and
allows Track 2 to chain quietly through clean signals (zero
corrective commits in Phase 35V's case) while Track 1 pre-phase-
chats every drift discovery.

Bulk-tick at slice closure, not phase closure. Phase 35V used a
Python regex to bulk-tick all 30 slice checkboxes once each
slice's commit landed and was pushed. The discipline: each slice
gets its own commit FIRST (with the dev-log-style commit message
labelling clean-vs-drift outcome), THEN the ROADMAP checkbox
ticks as part of the next commit. The bulk-tick at the closer
itself is for the phase-done criteria boxes, not the slice
boxes — those should already be ticked. Mixing slice ticks into
the phase closer commit hides which slices actually shipped from
git history.

Closing audits are a deliverable, not paperwork. The closing
audit appended to `docs/phases/phase-35V-pre-launch-audit.md` (per-
slice outcome table, drift modes surfaced, verification
methodology) is the artifact the next external-reviewer round
reads first. A phase that closes without a closing audit
forces the next reviewer to reconstruct outcomes from commit
messages — which is fine until commits get squashed, branches
get pruned, or memory records turn out to summarise rather than
reproduce. The audit's weight comes from being inside the repo,
versioned with the code, and consumable by humans who weren't in
the conversation.

Memory records for audit phases capture *methodology*, not just
*outcomes*. The Phase 35 and Phase 36 memory records summarise
what shipped. The Phase 35V memory record additionally captures
the verifier-correction methodology, the three-track structure,
the orthogonal-sentinel discipline, and the launch-wording-audit
class of drift. Memory records that document HOW Phase 35V was
run let future audit rounds adopt the pattern without re-
deriving it from commit archaeology.

## Phase 33J prep — docs reorganization (Diataxis tree)

Docs reorganization is its own slice. The first attempt at the
website docs put 44 unrelated topics into one 4,660-line file
called `docs/website-content.md`. That violated the project's
file-responsibility rule (a "grab bag" with 44 internal sections
that share no state) and coupled the source content to its
intended consumer (the website) rather than to its content. The
fix was to split into per-topic files under a Diataxis-style
tree (`book/`, `guides/`, `recipes/`, `reference/`, `migration/`,
`operations/`, `security/`, `internals/`, `help/`, `meta/`,
`phases/`) and move existing user-facing docs into matching
locations.

The lesson generalises: **organise docs by content, not by
consumer**. A docs file under `docs/` should be readable by
anyone reading the repo directly, not coupled to the website
build pipeline. The website build reads multiple files and
composes them into a site — that's the website's job. The
source files stay one-topic-per-file regardless.

The Diataxis four-track structure (tutorials / how-to / reference
/ explanation) is the same shape Rust, Go, TypeScript, Gleam,
and Elixir all converge on. Researching peer languages first
surfaced this as a settled convention rather than a design
decision. Pre-phase chat with research before structure is the
no-shortcut move.

The reorg is a code-touching slice, not just a docs move. Phase
35V's pattern-5 ("cross-component coupling discovered at
verification time") applied here too: 67 source files
(including `include_str!` paths in `corvid-guarantees/src/render.rs`
and `PathBuf::from(...)` calls in `corvid-cli/src/commands/test.rs`)
embedded literal `docs/...` paths that broke when the underlying
files moved. A bulk find/replace plus a `cargo run -q -p
corvid-cli -- contract regen-doc` to refresh the registry-
derived spec doc closed the coupling. Future structural moves
in `docs/` should expect to touch source code and the registry-
derived doc together; treat the move + replace + regen as
one slice, not three.

`include_str!` failures are silent under `cargo build` — they
only surface under `cargo test`. When moving a doc file
referenced by `include_str!`, run `cargo test --no-run` to
catch the path break before assuming a clean build.

A drift-gate test that hardcodes a path is a coupling worth
calling out at the test site. Phase 33J6 (grammar drift gate)
should follow the same pattern: assert the rendered grammar
matches the parser, and embed the parser-tests path explicitly
so a future move surfaces immediately.

## Phase 33J7-prereq — `corvid-browser` crate (probe-first slice)

The 33J7-prereq slice (a WASM-compatible typechecker entry
point for the playground) was filed with a 2-week best-case, 6+
week hard-case estimate. The dep-audit probe finished it in a
single session.

The pattern that scales: **before agreeing to a multi-week
slice's scope, run the cheap probe first**. For 33J7-prereq the
probe was 60 seconds of `cargo tree -p <each-typechecker-crate>`
piped through grep for tokio/rayon/libloading/tempfile/etc. The
result reframed the slice: all four typechecker crates
(corvid-ast, corvid-syntax, corvid-resolve, corvid-types,
corvid-guarantees) already compiled to wasm32-unknown-unknown
with zero refactoring needed. The 6-week hard-case scenario was
ruled out before code started.

`corvid-driver` was the only blocking dep — it pulls tokio +
hyper through the codegen-py and replay surfaces. But the
typecheck pipeline is just 4 calls (`lex` → `parse_file` →
`resolve` → `typecheck_with_config`, visible at
`corvid-driver/src/pipeline/compile.rs:44-69`). Lifting those
lines into a new crate with `wasm-bindgen` glue was a one-day
slice, not a multi-week refactor.

Three patterns to carry forward:

**1. Probe before scope.** When the slice estimate has a wide
spread (best ↔ hard differ by 3-6×), the cheapest reducer is
the audit, not more planning. 60 seconds of `cargo tree` ruled
out the hard case Phase 33J7-prereq feared.

**2. Lift, don't refactor.** The typecheck pipeline already
existed in `corvid-driver`. The new crate didn't refactor the
typechecker — it copied the 4-line pipeline shape and added
glue. Phase 35V's "design-reversal recording" pattern from
20n-B applies in reverse here: when a brief over-estimates the
work, document the reduction.

**3. Flat wire schemas with `version` field.** The
`CheckResult` ships a `version: "v1"` field at the root.
Additive changes (new optional fields) don't bump it; older
renderers safely ignore unknown fields. Non-additive changes
(renamed / removed fields) bump it. Same pattern protobuf and
LSP both use. Cheap insurance against schema-change
coordination cost — write it in v1 so v2 doesn't break the
website renderer silently.

The slice also added the load-bearing
`dangerous_call_without_approve_refuses` integration test
asserting that the compile-refusal moat demo surfaces
`approval.dangerous_call_requires_token` through the wire
format. If a future change ever silently breaks that pipeline,
the playground's marquee demo would refuse to refuse. The test
pins that property and the CI enforces it.

## Phase 33J7a — multi-file typecheck (path-shape lesson)

The 33J7a slice (`check_project` for the playground) was
straightforward — an additive function on `corvid-browser` that
walks an in-memory file map instead of `std::fs`. ~280 lines of
new code in `multi_file.rs`, 8 new tests, no refactor. The
slice closed in one session.

One surprise worth recording: **PathBuf is the wrong abstraction
for web-context paths.** `PathBuf::from("./policy")` followed by
`PathBuf::join("src")` produces `src/./policy` on Linux and
`src\./policy` on Windows — neither matches the user's
`"src/policy.cor"` key. The corvid-resolve crate's existing
`resolve_import_path` helper uses `PathBuf::join` and inherits
this quirk; we couldn't reuse it. The fix was to use string
operations directly:

```rust
fn normalize_web_path(raw: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for seg in raw.split(|c| c == '/' || c == '\\') {
        match seg { "" | "." => continue, ".." => { stack.pop(); }
                    other => stack.push(other), }
    }
    stack.join("/")
}
```

Web-context path discipline:
- Always use `/` as the separator regardless of host OS.
- Drop `.` and `""` segments silently.
- Resolve `..` segments greedily.
- Treat `.cor` extension as implicit (user can write `./policy`
  or `./policy.cor`, both resolve to `src/policy.cor`).

The native `corvid` CLI uses filesystem paths and `PathBuf` is
fine there. The browser playground uses URL-shaped paths and
needs the string-based normalization. The split happens at the
in-memory file map boundary. Future runtime-split work (33J7b)
should expect to repeat this pattern at every boundary between
host-native code and browser-native code.

A second small lesson: **only Type / Store / Tool / Prompt /
Agent declarations export across files.** Plain `fn`
declarations are file-local — `collect_public_exports` in
`corvid-resolve` skips them. The test corpus initially used
`public fn allowed()` for the imported function and the
typechecker correctly refused to resolve it as a member. This
is a pre-existing Corvid invariant; document it in
`docs/book/06-modules.md` if it isn't already, and surface it
in the playground's first-time-user tips when they hit a
"unknown member" diagnostic on an imported `fn`.

## Phase 33J3 — handoff briefs as deliverables

Slice 33J3 (docs site build) closed via a handoff brief at
`docs/meta/website-docs-handoff.md` — a 509-line self-contained
spec that another developer (or coding agent) could execute
without prior Corvid knowledge. The deliverable shipped live at
<https://corvid-lang.org/docs> rendering the full Diataxis tree:
all 11 sections, 18 book chapters, Corvid syntax highlighting,
resolved cross-links.

The lesson generalises beyond docs: **when a slice's executor is
not on this conversation, the brief is the load-bearing
artifact**. A good brief is structured like a one-shot prompt:
- Both repo URLs up front.
- One-paragraph context for someone who doesn't know the domain.
- A complete directory tree of the inputs.
- Concrete deliverables with acceptance criteria.
- Framework recommendation with reasoning + "if you prefer X,
  here's the shape to keep."
- Three sourcing options (submodule / build-fetch / monorepo)
  with pros and cons.
- Cross-link, syntax-highlight, and theme requirements explicit
  enough to verify without back-and-forth.
- Constraints framed as no-shortcut rules ("do not edit upstream
  source", "do not edit auto-generated docs", "exclude phases/
  from rendered output").
- 9 numbered "where to start" steps.
- Open questions to ask before starting.
- Out-of-scope list (so adjacent slices don't expand scope).

Briefs that don't include all of these get clarification rounds
that defeat the point of a brief.

Two confirmations the brief made load-bearing:

1. **Sourcing strategy as the executor's choice, not the brief's
   choice.** The brief listed three options with pros/cons and
   said "pick one and document the choice." This lets the
   executor optimise for their tooling without our pre-empting.
2. **Acceptance criteria as binary checks.** Each criterion is a
   pass/fail observation a third party can verify by visiting
   the URL ("Visiting `/docs` shows the docs landing page",
   "Mobile breakpoints are usable at <768px", "Search returns
   results for 'approve'"). No subjective "looks good" criteria.

The brief also surfaced one drift mode worth pinning: when a
slice deliverable is verified live by URL, the verification step
is `WebFetch <url>` + structural-assertion. Future "verify a
live deliverable" slices should follow the same shape — name the
URL, list the structural assertions, log the WebFetch results.

## Phase 33J7b — direct-deps audit catches what transitive-deps misses

The 33J7b pre-phase chat included Q3: is `corvid-vm` a pure
port or its own split? The earlier probe (`cargo tree -p
corvid-vm | grep tokio`) showed tokio in its tree but
attributed it to the transitive `corvid-runtime` dep. The
working hypothesis was "split corvid-runtime, port the VM."

Reading `crates/corvid-vm/Cargo.toml` directly surfaced what
the tree summary hid: `tokio` is a **direct** dep in the VM's
own manifest, alongside `async-recursion` and `async-trait`.
The VM has its own async surface area for prompt/tool
dispatch; the runtime is one source of tokio but not the only
source. The "pure port" plan would have wedged on tokio after
the runtime split landed clean.

**Lesson:** when auditing a crate for wasm compatibility,
the `cargo tree` view tells you what compiles in; the
`Cargo.toml` view tells you what the crate *owns* directly.
Both audits are needed before scoping a split. The direct-deps
audit caught a ~1-week estimate revision (port → split) at
zero cost in pre-phase chat. Catching it in code would have
cost the whole port attempt.

Generalises: any "is X already wasm-clean?" question gets two
answers — the transitive tree's answer (am I pulled into a
build) and the direct manifest's answer (do I own this surface
to refactor). Both matter; check both.

## Phase 33J7b decisions recorded (chat closed 2026-05-12)

All six boundary questions resolved. The decision record is at
`docs/meta/runtime-split-design.md` under "Decisions". Six
load-bearing calls plus five risk mitigations.

The six decisions (D1-D6 in the design doc) for future audit
reference:

1. **Receipts** — core emits canonical bytes; host signs.
   Browser can verify but never sign.
2. **Replay** — state machine in core; persistence behind
   `ReplaySource` / `RecorderSink` traits.
3. **VM** — split (not port); `corvid-vm-core` synchronous
   IR-walker yielding `HostRequest`, `corvid-vm-host` native
   async wrapper.
4. **Stdlib impls** — single `corvid-runtime-host` crate with
   per-module feature flags; per-module crate extraction is a
   follow-up if any module crosses the file-responsibility
   threshold.
5. **Connector modes** — mock + replay in core, real in host.
   Phase 41L drift test splits into core-only (mock ≡ replay)
   + host integration (real ≡ replay).
6. **Public re-exports** — host re-exports core so
   `corvid_runtime::Foo` keeps working for native users.

Pattern worth pinning: **the chat-closing entry on a pre-phase
design doc is its own deliverable**. Future audit rounds (a
hypothetical Phase 35V-equivalent for Phase 33) read the
chat-closing decisions and verify shipped behavior against
them. A pre-phase chat doc that closes without a dated, signed
decision record fails to do its job — the rationale survives,
but "what we actually agreed to" gets reconstructed from
commit archaeology. Phase 35V's pattern-3 ("memory records
capture methodology, not just outcomes") applies one layer up:
pre-phase chat docs need to capture decisions, not just
agendas.

## Provenance Propagation closed (2026-05-16)

Eleven slices over four sessions; the shipped capability is the
contagion law (`Grounded<T>` flows through ordinary operators +
call sites), the runtime alignment (`maybe_ground_*_result`
delivers `Value::Grounded` wherever the type promises it), the
IR-visible discard (`UnwrapGrounded` at every legacy
`Grounded<T> -> T` coercion site), and `@grounded_pure` — the
compile-time moat that refuses any laundering inside an agent
body.

Three patterns kept surfacing across the phase and are worth
recording for the next moat-shaped feature:

1. **Design X reversal at slice 2.** The doc opened with a wrong
   premise — that the typechecker already saw `Grounded<T>` for
   `data: grounded` returns and just needed contagion glue. The
   slice-2 recon falsified it before any code: the typechecker was
   *grounded-blind* for effect-induced grounding. The fix was not
   "teach the interpreter to tolerate `Value::Grounded`" (that
   would leave the moat impossible); the fix was Design X — make
   `data: grounded` a type-system property so the typechecker
   sees what the runtime sees. The slice plan reshaped 11→12
   slices and the lesson generalised: *a design doc's first
   recon pass is the cheap moment to falsify the load-bearing
   premise*. Mid-implementation discovery costs the whole plan.

2. **Each downstream consumer of a type-level promise needs to
   be re-audited.** Design X (slice 2b) promoted return types at
   the typechecker. Slices 7b and 10 each found a downstream
   consumer that wasn't re-audited:

   - 7b: `maybe_ground_tool_result` only wrapped tools with
     literal effect name `"retrieval"`, not the general
     `data: grounded` row. Surfaced because the new IR
     `UnwrapGrounded` forced the runtime to encounter a value as
     `Grounded` for the first time.
   - 10: `expr_is_grounded`'s `Prompt` arm walked args only,
     never the prompt's own effect row. Surfaced because the new
     idiomatic fixture (return `Grounded<String>` end-to-end)
     forced the reachability analysis to recognise a no-args
     `data: grounded` prompt as a provenance source.

   The pattern: *when a type-level promise lands, list every
   downstream consumer (runtime grounding, reachability analysis,
   IR lowering, codegen, ABI) and re-audit each one against the
   new promise*. Each gap surfaces only when a later slice forces
   the value through that consumer.

3. **Sub-splitting load-bearing slices early.** Slices 2, 3, 5,
   and 7 all sub-split into a/b/c/etc after the recon revealed
   the work was bigger than the design doc estimated. The
   sub-splits paid off in two ways: each commit was small enough
   to roll back, and the dev-log + design doc could record the
   exact boundary where the work expanded (e.g., slice 7's
   sub-split into 7a typechecker recorder + 7b IR insertion +
   runtime alignment, where the runtime alignment was a
   pre-existing gap surfaced by 7b that the design doc had not
   anticipated). The rubric: *if a slice's recon finds two
   independent shapes of work, sub-split before the first
   commit, and update the design doc with the sub-split + the
   reason*.

The shipped moat is at `crates/corvid-types/src/checker/decl_grounded_pure.rs`. The slice-by-slice design doc with sub-split rationale is at `docs/meta/grounded-propagation-design.md`. The invention catalog entry is in `README.md` under "Provenance Propagation + `@grounded_pure`" and `docs/reference/inventions.md`.

## Path A locked — silent build to v1.0 (2026-05-17)

The launch chat opened with "I want us to launch" and closed with
"silent build, ship v1.0 when the full backend track is complete."
The decision is not the headline. The lesson is the framing move
that made the decision tractable.

**Frame shift that mattered.** The pre-chat ROADMAP entangled two
different launches under one v1.0 label: the defensible-core
launch (the language + the moat, gated by Phase 35) and the
production-backend launch (the same plus persistence + jobs +
auth + observability + connectors + deploy, gated by Phases
37-43). Both were sitting under one "v1.0" stamp with one big
estimate. Conflating them forced an answer of either "ship now
under-delivered" or "wait 3-4 years for the whole thing." Neither
is the actual decision.

Splitting them — naming the defensible-core surface as already
shipped and the production-backend surface as the actual launch
gate — unlocked the real call: pick the audience first, then the
scope falls out. AI engineers building products need the second
launch, not the first. The chat closed in three exchanges once
the frame was right.

**Path A vs Path B.** With the audience pinned, the next real
decision was operational, not strategic: build silently and ship
all at once (Path A) versus run a developer preview during the
build to shape the backend with real feedback (Path B). The
recommendation was Path B for one structural reason — the
original ROADMAP itself put a 20-developer beta ahead of the v1.0
cut as a gate, and Path A drops it. The user took Path A anyway,
which is a defensible call: silent build is lower PR overhead and
lets the launch land as a fully-formed surprise. Worth recording
that the call was made knowing the trade-off, not by default.

**Pattern worth pinning for the next launch-shape decision.**

1. *List the launches the ROADMAP secretly contains.* If a single
   version label is gated by multiple independent surfaces, the
   first move is to separate them and name them.
2. *Pick the audience before the scope.* Different audiences want
   different launches; conflating them is what makes the
   estimate balloon.
3. *Make the operational choice (silent vs preview) consciously,
   not by default.* Both are defensible; recording which one was
   chosen and why is the audit trail future planning needs.
4. *The pitch sentence is the anchor.* Every scope decision in
   the launch track hinges on it. Lock or iterate before the next
   phase opens, not after code is written against an implicit
   pitch.

## Audit-before-implement before any phase-tagged "next phase" (2026-05-17)

After Path A landed and the chat moved to "now implement Phase
37," the spot-check found Phase 37 was already shipped. Counting
slices across Phases 37-43 then found that Phases 38-42 were also
slice-complete; the open work is verification-shaped, not
implementation-shaped. The "~13-18 months remaining" estimate
landed in commit f42b508 the same morning had to be corrected to
"~3-5 months" within hours.

**Lesson — audit the actual state of an upcoming phase BEFORE
opening its pre-phase chat.** A phase-by-phase reading of the
ROADMAP's top-of-phase scope bullets is misleading because scope
bullets are not always re-ticked when slices close. The slice
checklist is the source of truth. Counting `[x]` vs `[ ]` on
slice lines (the lines that match `[0-9]+[A-Z]`) gives the
honest state in seconds.

**Generalises:** any time the next-step plan is "open phase X,"
the pre-step is "audit phase X's slice list against the
acceptance criteria — and audit the immediate downstream phases
too, in case their slice work is also further along than the
top-of-phase narrative suggests." The 30-minute audit cost would
have caught the 13-18-vs-3-5 month estimate gap before the
commit landed, not after.

**Why the gap existed at all.** Phase-done checklist items (claim
coverage rows + guarantee registry entries + named adversarial
tests + AI helpers + benchmark files) look superficially like
slice work but are actually verification work in the Phase 35V
Track 2 pattern. The slice closes when the runtime/typechecker/
CLI surface ships; the phase-done items get ticked later, often
by a verification round. Confusing the two over-estimates by an
order of magnitude.

**Pattern to lock for the next planning move:** before any
"open phase X" decision, run the slice-count script (`awk` over
the ROADMAP looking for `[x]` and `[ ]` patterns under each
phase heading) and report what's actually open. Treat top-of-
phase scope bullets and phase-done checklists as advisory, not
authoritative. The slice checklist is the source of truth.

## Cross-phase verification round CLOSED — P38-P42 (2026-05-17 → 2026-05-18)

Twelve commits across five phases. The verification round applied
the Phase 35V Track 2 pattern to Phases 38-42's phase-done
checklists and surfaced 36 launch-readiness/post-v1.0 filings
that the original phase-done ticks would have missed.

Headline metrics:

  - 5 audit reports landed (`docs/phases/phase-{38,39,40,41,42}-
    audit-2026-05-17.md`).
  - 5 phase-done sentinels landed (P38/P39/P40/P41 registry-id
    presence sentinels + P42 app-directory shape sentinel).
  - All OutOfScope reasons across P38-P42 tightened to name the
    specific filing slice that promotes them.
  - 33 launch-readiness filings + 3 post-v1.0 filings.
  - 1 permanent docs-as-code drift gate
    (`crates/corvid-cli/tests/docs_drift_gate.rs`) that extracts
    every fenced ```corvid``` block from `docs/guides/*.md` and
    routes each through `corvid check`; all 7 originally-exempt
    guides rewritten in 43W-1..43W-7 and EXEMPT_GUIDES is now
    EMPTY — every Corvid block in every guide is enforced.

Four systemic patterns worth pinning for the next moat-shaped
verification round:

1. **OutOfScope-promotion drift.** The 35-N slice (2026-04-29)
   added placeholder OutOfScope rows naming "the audit-correction
   slice that promotes." Downstream slices (38K, 39L, 40-various,
   41L/M) shipped their stated runtime/CLI surfaces but
   consistently did not own the registry-row promotion as part of
   "phase done." This was the most consistent drift across P38,
   P39, and P41 (P40 had only 1 OutOfScope row to begin with,
   which is why it was the cleanest phase). Future fix: when a
   phase ships an OutOfScope row that names a future slice for
   promotion, that future slice's phase-done checklist must
   include "promote OR tighten reason on row X." Without this,
   the row sits OutOfScope under stale "Slice N promotes"
   wording forever.

2. **AI-helpers-absent.** AI helpers (`corvid jobs explain`,
   `corvid approvals explain/policy-suggest`, `corvid connectors
   mock-fixture-gen/check --live narrator/fail-sim`) are the most
   consistently-deferred deliverable across P38/P39/P41. P40 was
   the exception (`corvid observe explain` + `corvid eval promote`
   both shipped in-phase). Pattern: when AI helpers ship in-phase,
   the launch-readiness tail shrinks; when they're deferred, it
   grows. Future fix: every phase that names AI helpers in its
   phase-done checklist should ship them as part of the phase OR
   explicitly file them as launch-readiness at the same time as
   the runtime they explain.

3. **Audit-recon-flips-estimates.** Three slices in this round had
   their estimates flip 4-10× after recon under the slice:
   35V2-P38-C (replay-quarantine: 2 hr test → multi-day cross-
   layer wiring), 35V2-P38-E (jobs.md docs rewrite: 2 hr → ~4 hr
   + 7 launch-readiness guide-rewrite filings + a permanent drift-
   gate sentinel), 35V2-P39-E (2 named-threat tests landable now:
   recon found both need runtime concepts that don't exist yet).
   Future fix: include "cross-layer wiring recon" in every audit-
   correction slice's first 30 minutes; don't commit to an
   estimate before that recon completes.

4. **Reference-apps-need-maturity-gate.** Phase 42's slice
   checklist closed the SHAPE (5 reference app directories with
   full subdir trees); the per-app maturity bars (≥10 tables,
   ≥5 migrations, ≥5 approvals, ≥3 cron, ≥10 evals, ≥5
   adversarial tests, ≥1500-line runbook, deploy manifests,
   benchmark file, CLAIM.md, AI helpers per app) are
   substantially un-met outside the marquee Personal Executive
   Agent. This isn't drift the verification round failed to
   catch — it's drift the verification round is uniquely
   positioned to surface because each per-app maturity gap is
   binary and easy to measure. Future fix: reference-app phases
   need a "per-app maturity gate" slice as part of the phase, not
   after — a maturity-validation slice that programmatically
   checks each app against the bar and fails the phase-done
   check until the bars are met.

The round shipped 12 commits and was structurally complete in
~6 hours of focused execution after the original ROADMAP
correction (commit `649676c`) revised the remaining effort
estimate from ~13-18 months to ~3-5 months. The verification
round itself fit inside that ~3-5 month window cleanly; the 33
launch-readiness filings represent the substance of the
remaining ~3-5 months.

Next: Phase 43 (Packaging, deployment, release, market readiness)
opens. Pre-phase chat mandatory before any code lands.

## 35V2-P40-C-LR-review-queue-ranking-cli closed (2026-05-19)

`corvid review-queue list --records <path>` shipped as the user-
visible surface that promotes `review_queue.cost_of_being_wrong_
ranking` from OutOfScope → RuntimeChecked. The subcommand reads a
JSONL stream of `ReviewQueueRecord` (captured by the host
backend; `-` reads from stdin), optionally filters by `--status
pending|approved|rejected|escalated`, and ranks by
`--rank=cost-of-being-wrong` so the highest-cost pending review
surfaces first. Renders as a fixed-width table by default;
`--json` emits the (optionally ranked) list as pretty JSON.

Five unit tests in `crates/corvid-cli/src/review_queue_cmd.rs`:

  - `rank_cost_of_being_wrong_sorts_highest_first` (positive)
  - `rank_unknown_policy_refused` (adversarial)
  - `jsonl_round_trip_preserves_records`
  - `parse_jsonl_skips_blank_lines`
  - `parse_status_filter_accepts_each_known_value`

Design note: the review-queue runtime is in-memory; persistence
is not a v1.0 commitment because the backend pattern is "operator
ships review-queue records into their own audit store + pipes
them through the CLI for triage." Filing a persistence layer
inside the runtime would create a second source of truth for a
domain operators already own. The JSONL-input shape ducks the
question and proves the ranking contract without forcing a
runtime-side persistence decision.

Phase 40 registry: 7 of 7 ids RuntimeChecked, 0 OutOfScope. The
launch-readiness tail loses one filing.

## 35V2-P39-L-LR-batch-data-class-equivalence closed (2026-05-19)

`corvid approvals batch` now enforces a data-class equivalence
rule at the runtime layer: a batch whose supplied ids span more
than one `data_class` is refused outright unless the operator
pins it with `--require-data-class <CLASS>`. Pinning surfaces
mismatched ids as per-id failures (the existing isolation
behaviour); spanning without a pin returns `Err` *before* any
approval is resolved, so the failure is total.

This closes the `batch-approval-drift-across-data-classes`
threat — the 7th of the 10 named adversarial threats for Phase
39's auth surface. The threat shape: an operator with the right
role lists a batch of pending approvals where some are
`financial` and some are `pii`; reviewer attention is allocated
to "financial reviews" but `pii` records resolve under the same
role check. Surface refusal forces the operator to be explicit
about which data class they are reviewing in this batch.

New registry row: `approval.batch_refuses_cross_data_class_drift`
ships RuntimeChecked from day 1 (Auth / Runtime phase). The
existing `approval.batch_equivalence_typed` row stays OutOfScope
but its reason is tightened to point at the new runtime row + at
the post-v1.0 `batch_with: ...` source-level sugar that
type-promotes it. Two test refs:

  - positive: `approvals_batch_require_data_class_pins_to_one_class`
  - adversarial: `approvals_batch_refuses_cross_data_class_drift_without_pin`

Design note: the inverse-coverage sentinel
`every_enforced_guarantee_id_is_wired_to_workspace_source`
caught the new row missing its in-binary anchor on the first
test run — exactly the drift mode the sentinel was designed for.
The fix was adding `pub const
GUARANTEE_ID_BATCH_REFUSES_CROSS_DATA_CLASS_DRIFT` next to the
enforcement site, the standard pattern. This is the second time
this round (the first was during P40 close-out) that the
sentinel surfaced a real anchor gap during a routine row add —
the cost of being explicit about enforcement-site / registry-row
coupling is paying for itself.

Phase 39 adversarial threat coverage: 7/10 (was 6/10). Three
threats still file-tied to their feature slice (session-fixation,
CSRF-bypass, scope-escalation) and ship with their respective
launch-readiness slices.

## 35V2-P39-G-LR-approvals-explain-helper closed (2026-05-19)

`corvid approvals explain <id> --tenant <T>` ships the first
half of the Phase 39 AI-helper pair. Renders a typed reviewer
summary: lifecycle classification + a one-line headline + the
typed reviewer facts (role, risk, cost ceiling, data class,
irreversibility, expiry) + every audit-event transition + a
short suggested-next-steps list. The output's `sources` array
carries the audit-event ids the explanation consulted —
Grounded<T> at the JSON layer: every claim back-references a
queue row.

Deterministic by construction. The "AI helper" framing names the
*role* the output plays for a reviewer (assistive
summarisation), not the call pattern. No LLM round trip, no
prompt template — typed classifier over the typed approval
record + typed audit-event trail. Same pattern as the Phase 40
`corvid observe explain` helper.

New registry row: `approval.explain_sources_grounded` ships
RuntimeChecked from day 1 (Auth / Runtime phase) with positive
+ adversarial test refs. The cross-tenant refusal is the
adversarial test — a request whose `--tenant` flag points at the
wrong tenant returns Err with "different tenant", not a silent
leak. This catches the operator-misconfiguration failure mode.

Four test refs:
  - approvals_explain_pending_carries_grounded_sources (positive,
    grounded-sources contract)
  - approvals_explain_after_resolution_records_approver (positive,
    post-resolution headline + sources still grounded)
  - approvals_explain_unknown_id_refuses (adversarial, clear
    error on unknown id)
  - approvals_explain_cross_tenant_refused (adversarial, explicit
    cross-tenant refusal)

Phase 39 launch-readiness tail loses one filing. The companion
generative helper `corvid approvals policy-suggest <tool>`
remains filed at `35V2-P39-H-LR-approvals-policy-suggest-helper`
— that one *does* need a generative model + prompt + grounded
sources for the proposed policy clause, so it's a bigger slice
than the assistive helper.

## 35V2-P42-D-LR-app-maturity-CodeMaintenance closed (2026-05-28) — Code Maintenance Agent reaches the bar; ALL FIVE reference apps now at Phase 42 maturity

Six-commit per-app maturity track for the Code Maintenance Agent, the
fifth and final app. Fourteen rows ✅ close; the same 5 cross-cutting +
2 post-v1.0-syntax rows defer. Closing audit:
[`docs/phases/phase-42-codemaintenance-maturity-2026-05-28.md`](docs/phases/phase-42-codemaintenance-maturity-2026-05-28.md).

With this track closed, **all five reference apps (PEA, PKA, Finance,
Customer Support, Code Maintenance) sit at the Phase 42 per-app maturity
bar.** Each ships auth, 3 cron jobs, 5 developer-authored approval
contracts with a domain-appropriate role/reversibility gradient, 11
eval cases, 3 promoted fixtures, ≥5 adversarial threats, a ≥1500-line
runbook, 3 deploy-manifest categories, and 5 typed permissions.

Code Maintenance reconfirmed every prior lesson and reused them
proactively rather than re-learning: the runbook hit ≥1500 with
coverage (pipeline walkthrough, CI-signal lifecycle, worked
failing-CI-to-merge example, glossary), the same-slice mock/test
discipline moved `write_plan.json` + two assertions when approvals went
2→5, the manifests used real env-var names, and — the CustomerSupport
`Grounded*` lesson applied up front — the CI-triage eval type was named
`CiTriageShape` from the start so it never tripped E0209.

**Cross-track lesson: the five-app per-app maturity programme converged
on one repeatable shape.** Every app went from a no-imports demo to the
bar through the identical 6-slice arc (foundations → 5 approval surfaces
+ gates → 11 evals + 3 fixtures → ≥1500 runbook → deploy + typed
permissions → audit/docs/ROADMAP). What differed per app was *only* the
domain posture expressed structurally — PEA's external-calendar share,
PKA's grounding + cross-tenant isolation, Finance's non-advice, Support's
policy-grounded replies, Code's CI-aware triage + writes-require-approval.
The invariant across all five: the domain constraint is enforced by the
*shape of the surface* (what tools exist, what effects the cron jobs
carry, what the eval asserts), not by a disclaimer — so the compiler and
the trace log can prove it. The five `D-LR-app-maturity-*` tracks are
done; the remaining Phase 42 tail is the cross-cutting
`35V2-P42-E/F/G/H-LR` + `33M-beta-feedback` slices that touch all apps
at once.

## 35V2-P42-D-LR-app-maturity-CustomerSupport closed (2026-05-28) — Customer Support Agent reaches the Phase 42 maturity bar

Six-commit per-app maturity track for the Support agent, the fourth
through the bar (after PEA, PKA, Finance). Fourteen rows ✅ close; the
same 5 cross-cutting + 2 post-v1.0-syntax rows defer. Closing audit:
[`docs/phases/phase-42-customersupport-maturity-2026-05-28.md`](docs/phases/phase-42-customersupport-maturity-2026-05-28.md).

Support reconfirmed the established lessons (≥1500 runbook met with
coverage not padding; per-app surface changes update `reference_apps.rs`
in the same slice; manifests use real `corvid deploy` env-var names;
approval flows are developer-authored with a deliberate gradient). One
new compiler lesson came out of the eval work.

**Lesson: a type name beginning with `Grounded` trips the E0209
grounded-return checker.** The support eval needed a shape to assert
"the draft reply is policy-grounded." The first name, `GroundedReplyShape`,
made `corvid eval` fail with `E0209 ungrounded return` — the checker
reads a return type whose name starts with `Grounded` as the
`Grounded<T>` builtin and demands a proven grounded source the plain
struct does not have. Renaming the type to `ReplyGroundingShape` (the
grounding word not leading) compiled clean. The takeaway: `Grounded` is
effectively a reserved prefix for the grounding system; do not name
ordinary structs `Grounded*`. (PKA's equivalent type was
`AnswerProvenanceShape` and Finance's was a `NonAdvicePosture` — neither
used the prefix, so neither hit this; Support hit it because "grounded"
is the natural word for its posture.)

Also reconfirmed the same-slice mock/test discipline at a new surface:
when D-CS-2 grew approvals 2 → 5, four things moved in lockstep in the
one commit — the source `support_eval_dashboard` value, the
`mocks/approvals_sla.json` fixture, the `reference_apps` approval-count
assertion, and (because 0005 was added) the migration-count assertion
(4 → 5). Missing any one would have left the suite red.

## 35V2-P42-D-LR-app-maturity-Finance closed (2026-05-28) — Finance Operations Agent reaches the Phase 42 maturity bar

Six-commit per-app maturity track for the Finance agent, the third
through the bar after PEA and PKA. Fourteen rows ✅ close; the same 5
cross-cutting + 2 post-v1.0-syntax rows defer. Closing audit:
[`docs/phases/phase-42-finance-maturity-2026-05-28.md`](docs/phases/phase-42-finance-maturity-2026-05-28.md).

Finance reconfirmed the three lessons the PKA track recorded (the
≥1500 runbook bar is met with coverage not padding; per-app surface
changes must update `reference_apps.rs` assertions in the same slice —
here the migration count 2→4→5 and the finance_% table count 7→11; and
manifests/runbooks must use the real `corvid deploy` env-var names).
Two new lessons came out of Finance specifically.

**Lesson 1: a reference app's approval flow is the developer's design
surface, not a fixed menu.** The user's directive when asked which 5
approval surfaces Finance should ship was "give the developer the power
to decide how he wants it to flow." The right reading was not "pick any
5" but "make the flow visibly the developer's choice." So the five
Finance contracts deliberately differ: `SubmitPaymentIntent` /
`ExportFinancialReport` / `ScheduleRecurringPayment` are Admin +
irreversible (money or data leaves); `CancelSubscription` /
`DisputeTransaction` are Reviewer + reversible. The role and
irreversibility gradient is authored in source — Corvid enforces the
`approve <Label>` boundary but never decides the gradient. The eval
encodes this as `case_irreversibility_matches_developer_intent` and
`case_role_gradient_matches_blast_radius` rather than PKA's blanket
"all irreversible" assertion. The takeaway for the remaining apps: when
a reference app demonstrates approvals, vary the contracts to show the
developer's control, don't clone one shape five times.

**Lesson 2: a regulated-domain posture is enforced structurally, not by
disclaimer.** Finance's non-advice posture is not a sentence in a doc —
it is the absence of any advisory tool plus the fact that the three
cron jobs (`nightly_balance_sync`, `weekly_anomaly_scan`,
`daily_subscription_renewal_check`) carry only read/observe effects and
cannot reach a `dangerous` write tool. The scheduler can wake the agent
up but can never authorize a payment, because every money movement
requires a human `approve` the compiler enforces. The eval's case 11
(`non_advice_posture_preserved`) and the runbook's incident B
(non-advice drift) keep this in the regression + ops surface. The
lesson: when an app has a domain constraint (non-advice, HIPAA,
no-autonomous-action), express it as the shape of the surface — what
tools exist, what effects the jobs carry — so the compiler and the
trace log can prove it, rather than relying on a policy that could be
edited away.

## 35V2-P42-D-LR-app-maturity-PKA closed (2026-05-28) — Personal Knowledge Agent reaches the Phase 42 maturity bar

Six-commit per-app maturity track for the PKA reference app, the
second app through the bar after PEA. Fourteen rows ✅ close; the same
5 cross-cutting + 2 post-v1.0-syntax rows defer as PEA. Closing audit:
[`docs/phases/phase-42-pka-maturity-2026-05-28.md`](docs/phases/phase-42-pka-maturity-2026-05-28.md).

The track was reshaped after the positioning call that **Corvid is the
general language for AI, not a docs/RAG niche.** PKA's original shape
was a private/local-only ingest+search demo that had no external-write
surfaces and so trivially "passed" the approval bar by having nothing
to approve. The reshape gave PKA five real external-write surfaces —
share-to-chat, share-via-email, publish-authoritative-answer,
export-tenant-corpus, cross-tenant-index-share — each `dangerous` and
behind a typed approval. The lesson for the remaining apps: a reference
app that dodges the approval bar by having no dangerous surface is not
meeting the bar, it is avoiding it. A real team agent writes outward.

Three lessons worth recording beyond the three from the PEA track
(import resolution, approve-label = tool-name-in-CamelCase, multi-line
boolean decomposition — all reconfirmed here).

**Lesson 1: the ≥1500-line runbook bar is a threshold, not a
heuristic — but the answer is coverage, not padding.** D-PKA-4's first
runbook pass landed at 1243 lines and felt "done" — every bar-required
section was present. It was still 257 lines under the explicit bar.
The honest close was not to inflate prose but to add the operationally
real coverage the first pass had left thin: tenant lifecycle
(onboarding/offboarding/isolation), the provenance-audit citation-chain
algorithm with a break→remediation table, three more incident runbooks
(embedding-model roll, cross-tenant leak, index corruption), capacity
planning thresholds, and per-approval decision trees. All of it is
content a real PKA operator needs. The line bar is a proxy for "did you
actually cover the operational surface" — meet it by covering more, not
by writing longer.

**Lesson 2: per-app surface changes must update `reference_apps.rs` in
the same slice.** The reference-app test suite carries per-app count
assertions: `execute_sql_dir(.., 3)` asserts the migration count, and
`assert!(stdout.contains("values: 5/5 passed"))` asserts the eval case
count. D-PKA-1 changed the migration count 3 → 5 and D-PKA-3 changed
the eval count 5 → 11, but neither updated the assert — so `cargo test
-p corvid-cli --test reference_apps` went red and stayed red across
three commits. D-PKA-5 caught and fixed both. The rule: when a slice
changes an app's table/migration/eval/approval count, grep
`reference_apps.rs` for that app and update the assertions in the same
commit, or CI breaks silently until someone runs the suite.

**Lesson 3: deploy manifests and runbooks must use the env-var names
`corvid deploy` actually emits.** The canonical names live in
`crates/corvid-cli/src/deploy_cmd.rs` (`CORVID_APP_ENV`,
`CORVID_CONNECTOR_MODE`, `CORVID_DATABASE_URL`, `CORVID_TRACE_DIR`,
`CORVID_REQUIRE_APPROVALS`) plus the auth-surface secrets PEA uses
(`CORVID_CONNECTOR_TOKEN_KEY`, `CORVID_API_KEY_PEPPER`,
`CORVID_SESSION_SIGNING_KEY`, `CORVID_CSRF_SECRET`, `CORVID_METRICS_LISTEN`).
The connector modes are `mock | replay | real | record` (from
`corvid-connector-runtime/src/test_kit.rs`), not "live". D-PKA-4's first
runbook draft invented `CORVID_STORAGE_MODE`, `CORVID_DB_URL`,
`CORVID_MASTER_ENCRYPTION_KEY`, `CORVID_METRICS_BIND`, and a "live"
mode — none of which the runtime reads. D-PKA-5 reconciled the runbook
and all the new manifests to the real contract. When authoring app ops
docs, copy the env-var names from the deploy scaffold, do not invent
plausible-sounding ones.

## 35V2-P42-D-LR-app-maturity-PEA closed (2026-05-27) — Personal Executive Agent reaches the Phase 42 maturity bar

Five-commit per-app maturity track for the PEA reference app. The
bar at `ROADMAP.md` Phase 42 phase-done checklist (17 rows that
apply per-app) now resolves to 12 ✅ closed, 3 deferred to
cross-cutting launch-readiness slices (`35V2-P42-E/F/G/H-LR` +
Phase 33M), and 2 deferred to post-v1.0 source-syntax sugar
(`35V2-P39-I`).

The closing audit lives at
[`docs/phases/phase-42-pea-maturity-2026-05-27.md`](docs/phases/phase-42-pea-maturity-2026-05-27.md).
Three cross-cutting engineering lessons worth recording came out
of this track.

**Lesson 1: `corvid check`'s import resolution is purely relative
to the importing file.** No workspace-stdlib root rule exists. Three
backend reference apps (PEA, audit_log, state_app) imported `"./std/X"`
that resolved to a non-existent `src/std/X.cor` — every app's
`corvid check` exited with 100+ errors. The fix is mechanical
(`"../../../../std/X"` for backend apps at `examples/backend/<name>/src/`)
but the deeper question is whether the compiler should support a
workspace-stdlib root rule. Filed as post-v1.0; for now the relative
path is the convention.

**Lesson 2: The `approve` label is the tool name in CamelCase.**
The compiler enforces snake_case → CamelCase match between a
dangerous tool's name and the `approve <Label>(...)` label. This
means contract names like "ExternalCalendarShare" drive the tool
name (`external_calendar_share`), not the other way around. The
first attempt of D-PEA-3 used the more intuitive ordering (intent →
contract → tool) and the compiler caught it; the fix was renaming
the tool so the label matched naturally. The lesson: when designing
a new approval contract for a Corvid app, pick the label first and
let the tool name fall out, not vice versa.

**Lesson 3: Multi-line boolean chains don't parse; decompose into
named intermediates.** Corvid agent bodies don't support the
`return\n    a\n    and b\n    and c` pattern that other indentation
-sensitive languages allow. Either everything goes on one line, or
the chain decomposes into named bindings:

```corvid
agent case_every_approval_irreversible_and_short_expiry() -> Bool:
    a = send_follow_up_email_approval()
    # ... four more bindings ...
    all_irreversible = a.irreversible and b.irreversible and c.irreversible and d.irreversible and e.irreversible
    a_expires = a.expires_in_hours <= 24
    # ... four more ...
    return all_irreversible and a_expires and b_expires and c_expires and d_expires and e_expires
```

The named-intermediate pattern is what the 11 PEA eval cases use.
It also makes the agent body more debuggable: each intermediate
can be inspected separately when an assertion fails.

The full per-app maturity work for the remaining four reference
apps (PKA, Finance, CustomerSupport, CodeMaintenance) will each
need their own D-LR track. PKA is next per ROADMAP order.

## 35V2-P38-C-replay-quarantine track closed (2026-05-27) — replay-mode runtime refuses to leak any side effect

`@replayable` durable jobs in Corvid have a runtime promise: no
real LLM call, HTTP request, application store write, or
filesystem write leaves the process during a Substitute-mode
replay. Recorded calls substitute from the trace; unrecorded ones
fail closed with a typed `RuntimeError::QuarantineViolation`
naming the surface (`"llm"`, `"http"`, `"store"`, `"io"`).
Differential mode (live LLM comparison) is the explicit opt-out.

```sh
# Record once
corvid jobs run --source app.cor --state queue.db --workers 1 --max-runtime-ms 0

# Replay reproduces the run from target/trace/jobs/<job_id>.jsonl
# with every side-effect surface quarantined
corvid jobs replay --source app.cor --job <job_id>
```

The registry row `jobs.replayable_side_effects` is now
`RuntimeChecked` with 4 positive + 4 adversarial test refs into
`crates/corvid-runtime/tests/replay_quarantine_corpus.rs`. Every
`@replayable` agent in a signed cdylib carries the guarantee in
its descriptor — the claim-coverage walker requires it alongside
the existing `replay.deterministic_pure_path`.

The cross-cutting engineering lesson worth recording: the audit
that filed `35V2-P38-C-deferred` estimated "~2-4 days when it
lands" based on the assumption that the agent-layer replay
infrastructure could be reused trivially for jobs. Recon under the
pre-phase chat (slice C-1's first deliverable, before any
implementation) found the assumption wrong — the queue runtime
and Phase 21 replay are separate layers with no `replay_job`
wiring, no job trace emission, no quarantine wrappers on the
side-effect manager types. Honest scope turned out to be six
sub-slices spanning ~3 weeks across runtime + driver + CLI
surfaces.

The pattern is general: when an audit names a missing test for a
guarantee that depends on a cross-layer integration, the test is
usually the smallest part of the work. Recon-before-tick is the
standing rule because audit estimates degrade fastest exactly
when the integration depth is the question. The deferral could be
overridden honestly (not silently ratified) only because the
pre-phase chat caught the gap before code started.

The closing design finding was equally instructive: the
queue-internal vs application-tool distinction the design doc
raised as an "open question for C-5" turned out to be enforced by
Rust's type system already — the durable queue uses
`rusqlite::Connection`, the trace writer uses
`JsonlTraceWriter`, and neither routes through `StoreManager` or
`IoRuntime`. The manager-level quarantine cannot block them
because they don't pass through. No `QuarantineContext` token
was needed; ownership boundaries already do the work.

See:
- Design brief: `docs/phases/phase-38-replay-quarantine.md`
- Cross-surface corpus: `crates/corvid-runtime/tests/replay_quarantine_corpus.rs`
- Tour topic: `corvid tour --topic replay-quarantine`
- README catalog: "Replay Quarantine For Durable Jobs"
- Inventions row: `docs/reference/inventions.md` "Replay Quarantine For Durable Jobs"
- Audit doc + override addendum: `docs/phases/phase-38-audit-2026-05-17.md`

## 35V2-P38-C-5 closed (2026-05-27) — replay quarantine extends to HTTP, store writes, file writes

C-4 quarantined the LLM registry; C-5 closes the loop for the three
other side-effect surfaces a `@replayable` agent can reach:

- **HTTP** (`HttpClient::send`) — every call refused during replay,
  including `GET`. HTTP semantics make read-vs-write distinctions
  unreliable (a `GET` can mutate server state via headers, cookies,
  or just by reaching a billing endpoint), so the quarantine is
  blanket: any send is a violation.
- **Store writes** (`StoreManager::put` / `put_record` /
  `put_record_if_revision` / `delete` / `delete_with_policy`) —
  refused with `surface: "store"`. Reads (`get`, `get_record`)
  pass through; they don't escape the process. The durable job
  queue uses raw `rusqlite` (not `StoreManager`), so the queue's
  own checkpoint writes during replay-mode internal bookkeeping
  are unaffected.
- **File writes** (`IoRuntime::write_text` /
  `write_text_with_effect`) — refused with `surface: "io"`. Reads,
  directory listing, and line streaming pass through. The runtime's
  JSONL trace writer uses `JsonlTraceWriter` directly, so trace
  emission during recording or schema-header re-emit during
  `Runtime::with_tracer` is unaffected.

Differential and Mutation replay modes
(`source.uses_live_llm() == true`) keep all four surfaces live
because their behaviour intentionally compares recorded output
against fresh side-effecting calls.

Error vocabulary that surfaces during a misbehaving replay:

```rust
RuntimeError::QuarantineViolation { surface: "llm",   detail: "..." }
RuntimeError::QuarantineViolation { surface: "http",  detail: "..." }
RuntimeError::QuarantineViolation { surface: "store", detail: "..." }
RuntimeError::QuarantineViolation { surface: "io",    detail: "..." }
```

`detail` names the call-site context — adapter + model + prompt
for LLM, method + URL for HTTP, kind + store + key + op for store,
path + op for IO. Operators reading the error message see exactly
which side-effect tried to escape.

A subtle design finding worth recording: the queue-internal vs
application-tool distinction the design doc flagged as an "open
question for C-5" (with a hypothetical `QuarantineContext` token
as the fallback) turned out to be enforced by Rust's type system
already. The durable queue uses `rusqlite::Connection` directly;
the runtime's trace writer uses `JsonlTraceWriter` directly.
Neither routes through `StoreManager` or `IoRuntime`, so the
manager-level quarantine cannot block them. No new token type was
needed.

Sub-slice of `35V2-P38-C-replay-quarantine`. Next and final: C-6
ships the cross-surface integration corpus, promotes
`jobs.replayable_side_effects` to `RuntimeChecked`, adds the
`corvid tour --topic replay-quarantine` topic + `docs/reference/inventions.md`
row, and walks `validate_signed_claim_coverage` for `@replayable`
jobs so a signed cdylib cannot ship the agent without the
guarantee in its descriptor.

## 35V2-P38-C-4 closed (2026-05-27) — replay-mode LLM registry refuses live calls

A runtime built in Substitute-mode replay (the default for `corvid
jobs replay` and `corvid replay`) wraps every registered LLM adapter
in a `QuarantinedLlmAdapter` at build time. The wrap delegates
`name()` and `handles(model)` to the inner adapter so the registry's
dispatch order is unchanged; only the `.call(&req)` path refuses,
returning a typed `RuntimeError::QuarantineViolation { surface:
"llm", detail }` whose detail names the adapter, model, and prompt.

This is defense-in-depth on top of the existing replay
infrastructure. The normal interpreter dispatch path
(`Runtime::call_llm_ref → ReplaySource::replay_llm_call`) already
intercepts every recorded `LlmCall` and substitutes the recorded
`LlmResult`; a recorded-event mismatch surfaces as
`ReplayDivergence`. The C-4 wrap closes the registry-layer hole for
any future caller that grabs an adapter directly from
`LlmRegistry::call(&req)` without going through `call_llm_ref`. The
two error variants are distinct so test corpora can tell them apart:

- `ReplayDivergence` — the substitution path caught an unrecorded or
  mismatched call.
- `QuarantineViolation { surface: "llm", .. }` — the adapter wrap
  caught a direct registry call that bypassed substitution.

Differential mode (the live-LLM-against-recorded-baseline comparison
that `corvid replay --swap-model` exercises) and Mutation mode (the
counterfactual step replacement) both skip the wrap: their behaviour
intentionally reaches the registry, and `source.uses_live_llm()`
returns true so `RuntimeBuilder::build` does NOT call
`quarantine_all` for those modes.

Behaviour the operator observes:

```sh
# Original record (writes target/trace/jobs/<job_id>.jsonl)
corvid jobs run --source app.cor --state queue.db --workers 1 --max-runtime-ms 0

# Replay — every LLM call comes from the trace, none touch the network
corvid jobs replay --source app.cor --job <job_id>

# Differential replay — LLM swap intentionally hits the new model
corvid replay <trace.jsonl> --source app.cor --swap-model claude-haiku
```

The quarantine wrap is invisible in the success path (substitution
intercepts first). It surfaces when an operator misroutes an adapter,
adds a future code path that bypasses the runtime's LLM dispatch, or
otherwise reaches the registry directly during a Substitute-mode
replay.

Sub-slice of `35V2-P38-C-replay-quarantine`. Next: C-5 applies the
same pattern to `HttpClient`, `StoreManager`, and `IoRuntime` so
HTTP requests, DB writes, and file IO during replay also fail closed
with typed `QuarantineViolation` errors.

## 35V2-P38-C-3 closed (2026-05-27) — `corvid jobs replay` replays a recorded job

Once a `@replayable` durable job has run (and recorded its JSONL
trace via C-2), `corvid jobs replay --source <path>.cor --job <job_id>`
reproduces the execution from that trace. The CLI compiles the source,
resolves the trace at `<trace_dir>/<job_id>.jsonl` (default
`target/trace/jobs/`, override with `--trace-dir`), and runs the
agent through the existing Phase 21 replay machinery in
`ReplayMode::Plain` — byte-identical reproduction, LLM dispatch
substitutes recorded responses, no live provider calls happen.

```sh
# Original run records the trace.
corvid jobs enqueue --state queue.db --task daily_brief --payload '["alice"]' \
  --max-retries 1 --budget-usd 0.10 --effect-summary brief --replay-key rk:alice
corvid jobs run --source app.cor --state queue.db --workers 1 \
  --lease-ttl-ms 5000 --max-runtime-ms 0

# Find the job id, then replay it.
corvid jobs replay --source app.cor --job <job_id>
```

Optional flags:

- `--state <queue.db>` — sanity-check the job exists in the queue
  before attempting replay; turns a "job id typo" into a clearer error
  than the file-not-found path. Omit to replay a trace file directly.
- `--trace-dir <dir>` — override the lookup directory; matches
  `DefaultJobRuntimeExecutor::with_trace_dir(...)` at record time.

Error vocabulary:

- Missing trace file → helpful diagnostic naming the path and listing
  the three most common causes: the original agent was not
  `@replayable` (so no trace was emitted by C-2), the trace dir was
  wiped between record and replay, or the job id is wrong.
- Trace exists but agent name is absent from the compiled source →
  the existing Phase 21 diagnostic ("trace's recorded agent `X` is not
  present in compiled source") fires; possible causes are the source
  has been renamed, the trace was recorded against a different file,
  or the agent was removed.
- Recorded args don't match the agent's current signature → typed
  diagnostic naming the offending parameter.

C-3 is the routing layer between the durable queue's `job_id` and the
existing Phase 21 replay surface. Quarantine wrappers around LLM /
HTTP / Store / IO so a replayed job genuinely cannot leak side effects
are filed for C-4 / C-5. C-3's correctness boundary is "the replay
entry compiles and round-trips a deterministic return value"; the
"no real side effect escapes" boundary is C-4 / C-5's promise.

Sub-slice of `35V2-P38-C-replay-quarantine`. Next: C-4 installs a
`QuarantinedLlmAdapter` that turns unrecorded LLM calls into typed
`QuarantineViolation` errors instead of network calls.

## 35V2-P38-C-2 closed (2026-05-26) — `@replayable` durable jobs persist JSONL traces

When a `@replayable` durable job runs through `corvid jobs run --source`,
the worker emits a per-job JSONL trace alongside the durable queue's
own checkpoint rows. Path is deterministic: `target/trace/jobs/<job_id>.jsonl`
by default, configurable per executor via
`DefaultJobRuntimeExecutor::with_trace_dir(...)`.

Trace contents reuse the existing `corvid-trace-schema` events the
interpreter already emits — no new event types are invented for jobs.
Every line is a `TraceEvent` JSON object; the executor adds nothing on
top. For an `@replayable noop() -> String: return "ok"` you get a
schema header → initial `SeedRead` for `rollout_default_seed` →
`RunStarted{agent: "noop"}` → `RunCompleted{ok: true, result: "ok"}`,
plus any interleaved `ToolCall` / `LlmCall` / `ApprovalDecision` /
`SeedRead` / `ClockRead` events the body produces.

The emission gate is the source-level attribute, lowered through IR:
`@replayable` (and `@deterministic`, which implies replayable) becomes
`IrAgent.is_replayable = true` during lowering, mirroring the
`wrapping_arithmetic` precedent. Non-`@replayable` agents skip trace
emission entirely — their queue checkpoints are still durable, but no
JSONL file is written for them.

`QueueJob.replay_key` is unchanged — it stays as operator metadata at
enqueue time and is NOT touched by the executor. C-3's `replay_job`
lookup (next slice) finds traces by `job_id` directly. Cleaner
separation: queue stores logical metadata, filesystem stores traces.

Sub-slice of `35V2-P38-C-replay-quarantine`. C-3 will layer
`replay_job(queue, job_id)` (reads the trace, drives the executor in
replay mode); C-4 / C-5 install quarantine wrappers around LLM / HTTP /
Store / IO so a replayed job can't leak real side effects.

## 35V2-P38-C-1 closed (2026-05-26) — `corvid jobs run --source` mandatory; no-op default removed

`corvid jobs run` now requires `--source <path>.cor` to point at a compiled
Corvid source file whose `agent` declarations supply the bodies the
worker pool executes. The previous behaviour — no executor wired, every
leased job marked `succeeded` instantly by a no-op default — was a silent
durable-state lie: the queue reported jobs as completed when no work had
actually run. Missing `--source` now errors with a clear message that
points at `corvid jobs run-one` for true smoke-test lifecycle exercise.

Production shape:

```sh
corvid jobs enqueue --state queue.db --task daily_brief --payload '["alice"]' \
  --max-retries 3 --budget-usd 0.50 --effect-summary brief --replay-key rk:1

corvid jobs run --source app.cor --state queue.db --workers 4 \
  --lease-ttl-ms 60000 --max-runtime-ms 0
```

The runner compiles `app.cor` once at startup, builds a single shared
`Runtime`, wraps a `DefaultJobRuntimeExecutor` (which holds the `IrFile`
and resolves `job.task` against `IrFile.agents`), and threads the
executor through `WorkerPool::with_executor`. Each worker leases a job,
deserialises the payload JSON array into a typed `Vec<Value>` against
the agent's params, runs the body through the same async `run_agent`
interpreter entry `corvid run` uses, and finalises via the existing
`complete_leased` / `fail_leased` paths.

Error vocabulary at the executor layer:

- Unknown agent (task name not declared in the source) → `JobOutcome::Skip`,
  lease releases, job stays eligible for another (per-task) worker pool.
- Payload not a JSON array → `PayloadShape` failure with retry/backoff
  per the queue's normal policy.
- Wrong arity → `PayloadArity` failure naming the agent + expected/got
  counts.
- Per-arg type mismatch → `PayloadType` failure naming the offending
  param + the converter error.
- Interpreter raised an error → `AgentInterpreter` failure with the
  stringified `InterpError`.

Sub-slice of the audit-correction track `35V2-P38-C-replay-quarantine`.
C-2 layers trace emission for `@replayable` agents on top; C-3 adds
`corvid jobs replay <id>`; C-4/C-5 install quarantine wrappers around
LLM/HTTP/Store/IO so a replay can't leak side effects. See
`docs/phases/phase-38-replay-quarantine.md` for the full integration
design.

## 35V2-P38-G-LR-corvid-jobs-explain-helper closed (2026-05-19)

`corvid jobs explain --state <path> --job <job_id>` ships the
jobs-side parallel to Phase 39's `corvid approvals explain` and
Phase 40's `corvid observe explain`. Walks the durable queue's
typed `QueueJob` record + its `job_audit_events` trail,
classifies the operational position (pending / leased /
retry_wait / approval_wait / approval_denied / approval_expired
/ dead_lettered / succeeded / failed / canceled / loop_stall_*),
renders a one-line headline + the typed operator facts (task,
attempts vs max_retries, lease owner, failure kind, approval
linkage, replay key, idempotency key), every audit-event
transition, optional loop usage, and a position-specific
suggested-next-steps list.

Deterministic by construction. The output's `sources` array
carries every audit-event id the explanation consulted — every
transition in `transitions` has a back-reference in `sources`,
the Grounded<T> contract at the JSON layer.

Implementation finding: the durable queue's audit-event table
(`queue_job_audit_events`) is only written by approval-decision
and loop-stall paths, NOT by the lease/fail/dead-letter paths.
The first positive test was an empty-sources failure because
the seeded dead-letter flow never produced audit events. Fix:
seed via the approval-wait + deny flow, which does write audit
events. This is a real limitation of the helper for purely
lease-failure jobs (their `transitions` list is empty + the
operator must read `operator_facts` directly) — recorded here
because the same gap will surface in any future "trace surface"
helper that assumes every state transition writes an audit row.

New registry row: `jobs.explain_sources_grounded` (RuntimeChecked,
Jobs / Runtime phase) with positive + adversarial test refs.

Three test refs:
  - jobs_explain_denied_approval_carries_grounded_sources (positive)
  - jobs_explain_pending_suggests_run_workers (positive,
    no-audit-events path → empty sources is correct behaviour)
  - jobs_explain_unknown_job_refuses (adversarial, clear
    "not found" error not an empty report)

Phase 38 launch-readiness tail loses one filing. Three AI
helpers from the verification round closed in two sessions
(P40 observe explain shipped in-phase before the round; P39
approvals explain + P38 jobs explain shipped in this session).
The pattern that made all three single-session slices:
deterministic typed classifier over typed records, no LLM round
trip, Grounded<T>-shaped sources. The generative helpers
(P39-H policy-suggest, P41-H connectors-ai-helpers umbrella)
remain filed because they *do* need LLM round trips and
prompt-grounding work.

## 35V2-P39-D-LR-session-rotation-hook closed (2026-05-19)

`SessionAuthRuntime::rotate_session_on_privilege_change` ships
the runtime hook the audit named as the session-fixation
threat's dependency. Takes a typed `PrivilegeChangeReason` (one
of `RoleUpgrade` / `PasswordChange` / `MfaEnrolled` /
`AdminElevation` — a closed enum, no free-form strings), wraps
the existing `rotate_session` primitive so the post-rotation
invariants still hold (old token rejected, rotation_counter
bumped, revocation cleared), and writes an
`session.rotate_on_privilege_change` audit row carrying the
typed reason as evidence. The audit row gives a reviewer
"why did this rotation fire" not just "a rotation fired."

Promotes `auth.session_rotation_on_privilege_change` from
OutOfScope → RuntimeChecked, with the positive test pinning
the named-threat behaviour (an attacker who captured the
pre-elevation cookie cannot replay it after the rotation; the
new cookie resolves; the audit row records `role_upgrade`)
and an adversarial test asserting an empty `trace_id` is
refused so a silent rotation cannot defeat the audit-trail
guarantee.

Three test refs:
  - session_rotation_on_privilege_change_rejects_pre_elevation_session_fixation_attempt
    (positive, named-threat: session-fixation)
  - session_rotation_on_privilege_change_refuses_empty_trace_id
    (adversarial, audit-trail no-op-on-failure invariant)
  - session_rotation_invalidates_old_token_and_preserves_rotation_counter
    (pre-existing, covers the unguarded `rotate_session`
    primitive this hook composes)

Design choice: the enum is closed (4 variants), not extensible
via a `&str` field. The registry-row contract says rotation
happens on *named* privilege events; an open-ended string
would let an operator pass `"refresh"` or `""` and silently
satisfy the row while skipping the named-event semantics.
Closed enum makes drift into "rotated on every client
refresh" impossible.

Phase 39 adversarial-threat coverage: 8/10 (was 7/10). Two
threats remain file-tied to their feature slice (CSRF-bypass
→ 35V2-P39-C-LR-csrf-middleware; scope-escalation →
35V2-P39-K-LR-structured-scope-model). Phase 39 launch-readiness
tail loses one more filing.

## 35V2-P38-D-LR-loop-bounds-enforcement-hook closed (2026-05-19)

Recon under the slice found the enforcement hook + positive
test were already shipped: `record_loop_usage_at` in
`crates/corvid-runtime/src/queue/loops.rs` already detects
budget violations, transitions the job to
`loop_budget_exceeded`, writes a `loop_bound_exceeded` audit
event listing every violated bound, and refuses post-termination
usage records. The positive test
`durable_queue_enforces_loop_budget_limits_with_audit` covers
all four bound dimensions (steps, wall_ms, spend_usd,
tool_calls). The "OutOfScope until the hook ships" wording was
stale from the original 38K landing — the registry row sat
OutOfScope based on a stale read of the runtime.

This is the same `OutOfScope-promotion drift` pattern recorded
in the cross-phase verification round closeout — a slice ships
the runtime work without owning the registry-row promotion as
part of "phase done." Three rows in P38 / six in P39 followed
the pattern; verifying-then-promoting clears one row at a time.

What this slice added:
  - The in-binary anchor
    `GUARANTEE_ID_LOOP_BOUNDS_ENFORCED` in
    `queue/loops.rs` so the inverse-coverage sentinel grep-finds
    the enforcement site.
  - A new adversarial test
    `durable_queue_refuses_loop_usage_after_budget_exceeded_termination`
    asserting a stale worker cannot silently keep charging spend
    against a terminal job. The existing positive test asserts
    this inline; a separately-named test gives the registry's
    `adversarial_test_refs` a clean anchor.
  - Registry-row promotion to RuntimeChecked + tightened
    description naming the post-termination refusal as part of
    the contract.

Phase 38 launch-readiness tail loses one more filing. Phase 38
registry rows: 5/8 RuntimeChecked (was 4/8). The three still
OutOfScope each name their specific filing slice (no more "Slice
N promotes" boilerplate). Each remaining promotion is the same
kind of recon-then-promote pattern OR awaiting genuinely
unshipped surface (post-v1.0 sugar or the cross-layer
replay-quarantine wiring).

## 35V2-P39-C-LR-csrf-middleware closed (2026-05-19)

Ships double-submit CSRF protection end-to-end: a canonical
verifier in `corvid-runtime::auth::csrf` plus rendered-server
wiring in `backend_middleware` plus an integration test that
asserts the wire behaviour matches.

Token shape: `<binding>.<hex_hmac>` where `hex_hmac` is
`HMAC-SHA256(server_secret, "corvid-csrf-v1:" || binding)`. The
verifier enforces three independent checks on state-changing
methods:

  1. Both the `x-corvid-csrf` header AND the `corvid_csrf`
     cookie are present.
  2. They are equal under constant-time comparison (the
     double-submit invariant — a cross-site request cannot
     read the cookie).
  3. The HMAC component verifies against the server secret (so
     a forged token without the secret is rejected even when
     header and cookie match).

Safe methods (GET/HEAD/OPTIONS) skip the check; unknown methods
fail closed (treated as state-changing). An empty server secret
fails closed too — refuses to verify rather than silently
accepting every request. That's the misconfiguration safety net
for production where `CORVID_CSRF_SECRET` is forgotten.

Eight unit tests in `corvid-runtime::auth::csrf` cover every
failure mode (missing header, missing cookie, header≠cookie,
malformed token, HMAC-forged, empty-secret-fails-closed,
safe-methods-pass, unknown-methods-fail-closed). One end-to-end
integration test in `corvid-cli/tests/build_server.rs` builds
the rendered axum server, spawns it with `CORVID_CSRF_SECRET`
set, and asserts: GET passes without a token; POST without the
header gets 403 csrf_violation; POST with a valid double-submit
pair gets past the CSRF gate (and 405s downstream because the
fixture handler is GET-only — proving the middleware allowed
the request through); POST with a forged token gets 403
csrf_violation.

Design choice: the rendered server inlines the verifier rather
than depending on `corvid-runtime` directly. The runtime
universe (rusqlite, opentelemetry, the full crate graph) would
be a heavy dep injection for a generated standalone binary that
otherwise only needs axum + tokio + tower-http. The inlined
verifier mirrors the canonical implementation byte-for-byte;
the integration test asserts they agree on the wire. A comment
in the rendered template points back at the canonical source
so a future divergence surfaces in review.

Backwards-compatibility: when `CORVID_CSRF_SECRET` is unset
(the default), the middleware is a no-op. Pre-existing
`build_server_emits_runnable_local_http_binary` test still
passes — the bullet-point test for the existing GET / POST /
413 / metrics shape is unchanged. Only the `x-corvid-middleware`
header label gained `csrf` in its comma-separated list (and the
existing test was updated to assert it).

Phase 39 adversarial-threat coverage: 9/10 (was 8/10). One
threat remains file-tied to its dependency slice
(scope-escalation → `35V2-P39-K-LR-structured-scope-model`).
Phase 39 launch-readiness tail loses one more filing.

This is the first slice in the session that shipped real
rendered-server middleware (not just a runtime helper or a CLI
subcommand). The cost of the integration test is ~80 seconds of
cargo build + spawn + HTTP, but it's the only test that proves
the rendered binary's wire behaviour actually rejects the
threat. Worth the wall-clock — without it, the runtime unit
tests would only prove that the canonical implementation is
correct, not that operators using `corvid build --target=server`
get the same guarantee.

## 35V2-P39-K-LR-structured-scope-model closed (2026-05-19)

Phase 39's adversarial-corpus coverage hits **10/10**. The last
threat — `scope-escalation` — needed a structured-scope concept
the runtime did not have: `ApiKeyRecord::scope_fingerprint`
stores an opaque SHA-256 hash today, so the runtime cannot
reason about which permissions a key actually carries.

`corvid_runtime::auth::scope` ships the type + the predicate:

  - `ApiKeyScope { permissions: BTreeSet<String> }` — an
    immutable, deduplicated, lexicographically-sorted set of
    `<resource>.<action>` permission strings.
  - `canonical_fingerprint(scope)` — SHA-256 over the sorted
    set, stable across insertion order. Safe to persist
    alongside `scope_fingerprint` for later equality checks.
  - `enforce_scope_grant(granted, required) -> Result<(),
    ScopeError>` — refuses when `required ⊄ granted` and names
    every missing permission in the typed
    `ScopeError::EscalationAttempt { missing }` so the audit
    trail records exactly which scope was attempted, not just
    "denied."
  - `parse_comma_separated` for operator CLI input; strict
    permission validation (no whitespace, ascii-alnum + `._-`,
    `<resource>.<action>` shape required, empty entries
    refused).

Ten unit tests cover every contract — positive subset, empty
required, scope-escalation (the named threat), proper-subset
escalation, multi-permission-missing listing, fingerprint
stability under reordering, duplicate collapse, comma-separated
parsing, malformed-permission rejection, empty-granted refusal.

Registry row `auth.api_key_scope_subset_check` ships
RuntimeChecked from day 1 — three adversarial test refs
(including the named-threat one) plus the positive subset
test.

Scope of *this* slice deliberately stops at the model + the
predicate. Wiring `enforce_scope_grant` into every middleware /
handler / route is downstream work. The audit's wording was
"structured-scope concept needs to land before a meaningful
test can"; the model + the predicate + the named-threat test
land together, and the rest of the runtime can adopt the type
without reinventing the shape.

Phase 39 adversarial-corpus is now **10/10**. The 11th item
(claim coverage for the unshipped source-level surface) and the
ergonomic-sugar rows (filed at `35V2-P39-I` post-v1.0) are
genuinely waiting on the parser-level surface, not on more
runtime work.

## 35V2-P39-J refile — role-coverage typechecker pass is post-v1.0 (2026-05-19)

Recon under the slice found
`35V2-P39-J-LR-role-coverage-reachability` is genuinely blocked,
not just unshipped. The typechecker pass needs a source-level
role-declaration syntax to reason over, and that syntax does not
exist in the AST: `AgentAttribute` today is `Replayable`,
`Deterministic`, `Wrapping`, `GroundedPure` only — no
`@requires(role)` or `@approval(role)` variant. The required
surface is filed at post-v1.0
`35V2-P39-I-post-v1.0-auth-syntax-sugar` (the
`auth`/`tenant`/`role`/`permission`/`approval Name:`/`@requires`/
`@approval` keyword set).

The honest correction: the J-LR row moves to post-v1.0
alongside its syntax dependency. Filing it as launch-readiness
was an audit-time optimism — the audit assumed the AST
already carried the role concept and only the typechecker
pass was missing; recon found both halves are missing.

`approval.confused_deputy_typecheck` registry-row reason
tightened to record the dependency chain explicitly and name
the runtime test that already covers the threat
(`approval_bypass_rejects_confused_deputy_self_approval` in
`crates/corvid-runtime/src/approval_queue.rs`). The Phase 39
adversarial-corpus 10/10 coverage already accounts for the
runtime half; only the *compile-time* promotion is post-v1.0.

This is a new drift mode the verification round did not
anticipate: **launch-readiness-misfile**. The cross-phase round
found OutOfScope rows where the runtime work had silently
shipped (the OutOfScope-promotion-drift pattern); this finding
is the reverse — a row whose unshipped surface depends on a
*different* unshipped surface that the audit had already
correctly filed as post-v1.0. The chain wasn't followed
through. Future audits should walk every filing reference to
make sure its dependency chain bottoms out before assigning a
class.

## 35V2-P41-D-LR-connector-drift-narration closed (2026-05-19)

Ships a schema-agnostic structural drift detector for connector
contracts + the CLI flow that wires it for hermetic CI runs.

Recon under the slice surfaced a design choice: the connector
manifest does not declare an expected per-operation response
shape (it carries `scope`, `rate_limit`, `redaction`,
`replay`, `mode`, never `response_schema`). The audit's
wording "compares the manifest to the live provider response
shape" assumed the manifest carried that schema. Two paths
were available:

  1. Add a `response_schema` field to every manifest. Heavy:
     schema migration across every connector + per-operation
     authorship.
  2. Ship a schema-agnostic structural detector that compares
     ANY two JSON payloads. Pure-function. Caller picks what
     "baseline" means (recorded mock fixture, last successful
     live run, hand-authored expected shape, etc.).

Path 2 is the honest v1.0 minimum: it gives operators a
working drift gate today, doesn't require touching every
manifest, and doesn't pre-decide which interpretation of
"baseline" to commit to.

`corvid_connector_runtime::contract_drift` ships the canonical
detector with 9 unit tests:

  - identical → empty report
  - provider-added field appears in `added_paths`
  - provider-removed field appears in `removed_paths` (the
    central drift threat; CI exits non-zero)
  - type-change (e.g. number → string) appears in
    `type_changed_paths`
  - nested-object drift uses dotted JSON paths
  - array-of-records walks the first element's shape
  - empty arrays skip shape walking (transient empty
    result-sets do not look like "every field removed")
  - all three buckets sort lexicographically (deterministic
    output for golden-fixture diffs)
  - null-vs-non-null is a type change, not a missing field

`corvid connectors check --baseline <file> --observed <file>`
wires it. The CLI loads both JSON files, runs the detector,
prints the report (table by default, JSON with `--json`), and
exits non-zero on any drift site. Three CLI integration tests
cover the file-input round-trip + the malformed-input safety
net.

The `--live` flag without `--baseline`/`--observed` still
returns a typed Err — but the error message now names the
file-input mode as the v1.0 surface AND points at
`35V2-P41-E-LR-live-provider-ci-matrix` as the operational
slice that wires the actual live-HTTP fetch. This is the
right separation: the *detector* is testable + ships today;
the *credentials wiring* is operational, lives in CI secrets,
and stays operational.

Phase 41 registry: 5/6 RuntimeChecked (was 4/6); only
`connector.write_requires_approval` remains OutOfScope as a
post-v1.0 typecheck-time guarantee. Three Phase-41
launch-readiness filings closed (D-LR shipped, E-LR
operational, the rest are AI-helper or post-v1.0 work).

## 35V2-P43-P-LR-ops-show closed (2026-05-19)

Cross-layer end-to-end slice: rendered axum server gains a
`/__ops` endpoint that returns a DSSE-signed `OpsShowSnapshot`
envelope; `corvid ops show --envelope-file <path> --pubkey
<path>` verifies it. Promotes `ops.live_introspection_signed`
to RuntimeChecked.

Three layers ship in lockstep:

  - **Runtime canonical implementation**
    `corvid_runtime::ops_show`: `OpsShowSnapshot` shape
    (build_id / started_unix_ms / generated_unix_ms /
    request_count / claim_manifest_ids), `sign_ops_snapshot` +
    `verify_ops_snapshot` delegating to
    `corvid_abi::sign_envelope`/`verify_envelope` with the
    pinned `corvid.ops.show.v1` payload type. 5 unit tests:
    round-trip, MITM (wrong key), payload tampering,
    payload-type replay attack, canonical determinism.

  - **CLI consumer** `corvid ops show`: file-mode flow —
    operator pipes `curl http://prod/__ops > ops.json` and
    runs `corvid ops show --envelope-file ops.json --pubkey
    deploy.pub`. 3 CLI unit tests: matching-key round-trip,
    wrong-key MITM, malformed-envelope.

  - **Rendered server producer**: `/__ops` endpoint added to
    the axum server template + `CORVID_OPS_SIGNING_KEY` /
    `CORVID_OPS_KEY_ID` / `CORVID_BUILD_ID` env wiring.
    Without the signing key, returns 503 (fail-closed; an
    unsigned snapshot is exactly what a MITM would produce, so
    refuse rather than serve unsigned). Inlines DSSE signing
    primitives (PAE + ed25519) byte-for-byte against the
    canonical `corvid_abi` implementation; the end-to-end
    integration test asserts they agree on the wire.

End-to-end integration test
`rendered_server_ops_show_signs_snapshot_and_cli_verifies_it`
builds the rendered server, exercises all three:

  - GET /__ops WITHOUT the signing key → 503
    `ops_signing_not_configured` (fail-closed).
  - GET /__ops WITH the key + build_id → 200 with the DSSE
    envelope; `corvid ops show` verifies it against the
    matching pubkey and prints `build_id: git:test-build-1234`
    + `signature-verified`.
  - Same envelope verified against an ATTACKER's pubkey →
    non-zero exit (MITM simulation: the binary produced the
    envelope, but the operator's expected key does not match;
    exactly what the registry row's threat model names).

Three design choices recorded:

  1. **Fail-closed default.** No signing key → 503, never an
     unsigned 200. The whole guarantee is "the response is
     signed by the binary's signing key"; serving an unsigned
     200 would silently break the contract.

  2. **Pinned payload type.** DSSE's payloadType allow-list
     pinned to `corvid.ops.show.v1` rejects a signature that
     is mathematically valid but was minted over a *different*
     artifact (an ABI attestation, a receipt). Without this,
     an attacker who captured *any* DSSE-signed Corvid
     artifact from the same key could replay it as an ops
     snapshot. The pin is the second cheap fix-closed lever.

  3. **Inline-vs-dep (same as CSRF slice).** The rendered
     server inlines the producer side rather than depending on
     `corvid-runtime`. The runtime crate would drag the whole
     opentelemetry/rusqlite universe into a standalone
     generated binary. The integration test asserts wire-level
     equivalence; a future divergence surfaces immediately in
     CI rather than years later in production.

Phase 43 launch-readiness tail loses the largest single
filing. The remaining tail is mostly per-app maturity work
(P42-D/E/F/G/H — five reference apps × heavy authoring) +
AI-helper work (P39-H policy-suggest, P41-H connectors-helpers
umbrella) + an operational CI matrix for live providers
(P41-E). All are honestly multi-day; not single-session.

## 35V2-P41-H-LR drift-narrator sub-slice closed (2026-05-20)

The first sub-slice of the `35V2-P41-H-LR-connectors-ai-helpers`
umbrella ships: a deterministic RAG-grounded narrator that
pairs every site in the contract drift report with a typed
`DriftNarration` + Grounded<T> back-references. Promotes a new
row `connector.drift_narration_grounded` to RuntimeChecked.

The "RAG-grounded" framing in the audit's sub-slice description
refers to the **evidence-citation property** of the output, not
to a live LLM round-trip. The narration cites which detector
bucket + path the consequence was synthesised from, so an
auditor can trace every claim back to a structural evidence
row. Same pattern as `corvid approvals explain` /
`corvid observe explain` / `corvid jobs explain` — typed
classifier over typed records, no LLM in the v1.0 contract.

`DriftNarration` shape:
  - `path` — the JSON path of the drift site
  - `kind` — `removed` / `type_changed` / `added`
  - `consequence` — one-line operational explanation
    ("connector code that consumed this field is now broken at
    deserialization")
  - `severity` — `breaking` (removed/type-changed) or
    `compatible` (added). Operator triage uses severity to
    decide what to fix vs. what to adopt.
  - `sources` — Grounded<T> back-references naming the
    detector bucket + path the narration summarised

Ordering invariant: `removed` → `type_changed` → `added`, so
breaking sites surface first. Empty drift report → empty
narration vec (the narrator never synthesises for sites that
did not drift; that would defeat the grounding contract).

5 new contract_drift tests + 2 new CLI tests + 1 new registry
row.

The remaining two sub-slices of the P41-H umbrella stay filed:
`mock-fixture-gen` (generative; needs LLM prompt + provenance
chain) and `fail-sim` (adversarial; needs LLM-driven fault
synthesis). Both honestly require LLM work that is not
single-session. Recording the split explicitly so a future
audit doesn't re-file them as launch-readiness when their
shape is post-v1.0.

Pattern recorded: when an audit files three differently-shaped
AI helpers under one umbrella, split the umbrella early.
Deterministic-classifier helpers (assistive, RAG-grounded,
explain-shaped) ship single-session; generative + adversarial
helpers need real LLM prompt + grounded-source work that
belongs in a separate milestone. The umbrella ID stays as the
filing reference; sub-slice IDs (`P41-H-LR-drift-narrator`,
`P41-H-LR-mock-fixture-gen`, `P41-H-LR-fail-sim`) carry the
actual scope.

## 35V2-P43-T-LR-release-notes sub-slice closed (2026-05-20)

Second umbrella-split this session. The Phase 43 AI-helper
umbrella `35V2-P43-T-LR-phase-43-ai-helpers` filed 5 helpers
together because they share an LLM-helper infrastructure
dependency. Recon under the slice found that one of them —
`corvid release notes <prev> <new>` — is actually
deterministic (git-log + conventional-commit grouping; no LLM
needed for the v1.0 baseline). It ships single-session via the
same pattern that closed the P41-H drift-narrator sub-slice.

The 43T umbrella's surface table described `release notes` as
"generative — Markdown release notes synthesised from commit
history + closed launch-readiness slices". The "generative"
framing anticipated an LLM-summarised version. But the
baseline release-notes generation that release-please /
changesets / semantic-release already ship in production is
just `git log <from>..<to>` + conventional-commit
categorisation + markdown grouping. That baseline is enough
for v1.0 — a future generative layer can wrap it later to
synthesise per-section prose.

Two design choices recorded:

  1. **Subcommand refactor honest in same commit.** The
     existing `corvid release <channel> <version>` positional
     surface had to become `corvid release build <channel>
     <version>` to make room for `corvid release notes`. The
     audit's surface table named the subcommand shape
     literally, so following the audit was the no-shortcut
     path. The CLI is pre-v1.0 so the soft break is
     acceptable; updated DEMO / ROADMAP / launch-rehearsal /
     launch-claim-audit refs in the same commit.

  2. **The "RAG-grounded" framing is about the OUTPUT, not the
     CALL PATTERN.** Every rendered line ends with the short
     SHA so an operator reading the notes can `git show <sha>`
     for any claim. That is the Grounded<T> property at the
     release-notes layer; calling it RAG-grounded is honest
     even though no retrieval happens in the codepath.

Promotes the new `release.notes_grounded` row to
RuntimeChecked. 6 new unit tests:
  - git-log parser drops malformed lines
  - categoriser routes each prefix correctly
  - unrecognised prefixes fall through to "Other"
    (no fuzzy-match drift)
  - markdown renders sections + grounded SHAs in stable order
  - empty range produces "No changes" stub, never a partial
    section header
  - ref validation refuses empty/flag-shaped inputs before
    they reach git

Dogfooded by running `corvid release notes b493bb6 HEAD` —
the command produced clean notes for this session's last two
slices, proving the end-to-end flow against the live repo.

The remaining 4 Phase 43 AI helpers + the 1 remaining Phase 41
AI helper (mock-fixture-gen) + the 1 remaining Phase 39 AI
helper (policy-suggest) + the 1 remaining Phase 41 AI helper
(fail-sim) stay filed under the umbrella as genuinely
LLM-shaped work. The recurring pattern: deterministic-shaped
helpers ship one at a time; LLM-shaped helpers wait for shared
infrastructure.

## 35V2-P43-T-LR claim-audit-explain-failures sub-slice closed (2026-05-20)

Third umbrella-split-this-session. The 5th Phase 43 AI helper
(`corvid claim audit --explain-failures`) was filed under
the umbrella as "adversarial — narrates each failed claim
with the specific evidence path + suggested fix". Recon
found the narration layer is deterministic: each finding's
remediation maps 1:1 to its `ClaimFindingKind`, no LLM
needed.

Ships:
  - `ClaimFindingKind` enum (`MissingEvidence` /
    `AspirationalWording`) on top of the existing
    `ClaimAuditFinding` shape.
  - `suggested_fix(line)` returns a typed remediation string
    that back-references the inventory line — Grounded<T> at
    the claim-audit layer (every remediation cites the row it
    addresses).
  - `--explain-failures` flag on `corvid claim audit`. When
    set, populates `kind` + `suggested_fix`; without it, both
    fields are absent from the JSON output via
    `#[serde(skip_serializing_if = "Option::is_none")]` so the
    pre-existing `{line, claim, reason}` shape is preserved
    for CI scripts that read the legacy output.
  - 4 new tests:
    - positive: `MissingEvidence` carries a line-grounded fix
    - positive: `AspirationalWording` carries a typed
      remediation naming the offending words
    - adversarial: opt-in default keeps legacy JSON shape (CI
      backward-compat invariant)
    - positive: no-findings case yields zero findings under
      `--explain-failures` (narration layer never synthesises
      explanations for un-flagged rows)

New registry row `claim.audit_explain_failures_grounded`
(RuntimeChecked, Claim / Platform).

The 2 existing claim-audit tests (`audit_passes_when_every_*`,
`audit_fails_when_*`) had their `audit_claim_inventory`
signature updated to take the new `explain_failures: bool`
parameter — both pass `false` to assert the legacy path is
unchanged. Regression check confirmed.

Three umbrella-splits this session: P41-H drift-narrator,
P43-T release-notes, P43-T claim-audit-explain-failures. The
pattern keeps holding: when an audit files an AI helper as
"generative" or "adversarial", check whether the v1.0 baseline
is actually deterministic (typed classifier over typed records
with Grounded<T> back-references). If yes, ship single-session.
If no, file under umbrella.

Phase 43 AI-helpers: **2/5 shipped**, **3 remain** (all
genuinely agentic — deploy tailor, upgrade assist, beta
synthesize-feedback). Phase 38/39/41 helpers: P38-G shipped
(jobs explain), P39-G shipped (approvals explain), P40 observe
explain shipped pre-session; P39-H (policy-suggest), P41-H
mock-fixture-gen, P41-H fail-sim genuinely need LLM work.

Net registry rows added by deterministic AI-helper slices this
session: 6 (review_queue, approvals explain, jobs explain,
session rotation, batch data-class equiv — wait, that's not
right, let me recount from the AI-helper class specifically):
3 — drift_narration_grounded, release.notes_grounded,
claim.audit_explain_failures_grounded.

## Cross-module DefId spaces and ABI name resolution (2026-05-28)

A DefId is only meaningful inside the symbol table it was
allocated in. Imported modules each have their own per-file
DefId space; `lower_with_modules` lifts imported declarations
into the root IR by *remapping* their ids to fresh values above
the root file's range (`build_imported_def_ids` starts at
`max(root DefId)+1`). So a `Type::Struct(def_id)` produced for a
module-qualified field (`alias.Type`) carries an id that is
**valid in the IR but out of range for the root file's symbol
table**.

The lesson: when a downstream consumer (here, ABI emission)
needs a name for a type, resolve it from the IR, not from a
symbol table that may not cover imported ids. The IR is
self-describing — every `IrType` carries both its remapped `id`
and its `name`, and `lower_with_modules` appends imported types
to `ir.types` under that same remapped id — so a
`{ir_type.id -> ir_type.name}` map is the authoritative source.
Indexing the symbol table directly (`symbols.get(def_id)`) is a
latent panic any time the id originated cross-module. Guard such
lookups with a bounds check and a graceful fallback so a missing
entry degrades to a synthetic name instead of crashing emission.

This is also a reminder that two "valid-looking" id spaces can
diverge silently: the bug only surfaced at cdylib-build time
because that is the only path that both (a) lowers with modules
and (b) emits the ABI. `cargo check` and single-file tests never
exercised the cross-module id reaching the emitter, so the
regression test constructs the out-of-range case directly rather
than relying on a single-file fixture to reproduce it.

## 35V2-P42-H-LR-1 app boot summary (2026-05-30)

`corvid app boot-summary <source.cor>` ships the first sub-slice
of the Phase-42 per-app AI helpers umbrella. The helper is a
deterministic typed classifier over the app's ABI descriptor —
no LLM call, no network hop, no runtime probing. It lowers the
supplied source through the standard frontend pipeline, builds
the descriptor in-process (via `corvid_driver::
build_catalog_descriptor_for_source`), and renders a typed
`BootSummary` (surface counts, flagship `pub extern "c"`
entrypoints, approval gates, enforced guarantees,
dangerous-surface counts, stores-writeable flag, descriptor
sha256). Every derived field is paired with a `BootSource` entry
naming the descriptor field that supplied the value — the
Grounded<T> sources posture the drift narrator
(`connector.drift_narration_grounded`) established for Phase 41.

The defining design move was the rejection of an LLM
dependency. Corvid has no `LlmProvider` / `ModelProvider`
abstraction in the codebase; introducing one would be a
foundational phase, not an H-LR helper. So a "fully implemented,
no shortcuts" helper here means: a real, runnable, replay-stable
typed transform over typed data, with every derivation traced
to a source. When the LLM-provider substrate later lands, the
helper's typed contract stays unchanged — only the rendering
side opts into richer narration.

Replay stability matters operationally: a boot summary that's
byte-stable across runs can be embedded in CI gates that compare
summaries across builds and fail the gate on drift. This is the
same posture the drift narrator uses, and the same property the
descriptor itself has (the descriptor sha256 the boot summary
reports is the same hash the signed CLAIM.md carries — verified
against PKA, the in-process and cdylib-embedded paths produce
the same bytes).

A new `GuaranteeKind::App` variant was introduced to keep the
registry's ID-prefix invariant honest as the per-app helper
cluster grows. Every existing guarantee ID prefix matches its
kind slug (`connector.X` → Connector, `abi_descriptor.X` →
AbiDescriptor); naming H-LR helpers `app.X` and then routing
them through `AbiDescriptor` or `Platform` would have broken
that invariant. The principled move was a new kind. Two future
H-LR sub-slices (adversarial-refresh, pr-describe) will share
the same kind.

The CLI dispatch arm threads `()` to `0u8` because `run` returns
`Result<u8>` (a typed exit code). The helper itself returns
`Result<()>` and lets the dispatcher translate success — a small
pattern worth keeping consistent across the per-app helpers.

## 35V2-P42-H-LR-2 app adversarial-refresh (2026-05-30)

`corvid app adversarial-refresh <source.cor>` ships the second
sub-slice of the per-app AI helpers umbrella. Same deterministic
typed-classifier posture as boot-summary: walks the ABI
descriptor in-process, emits typed suggestions with Grounded<T>
sources, replay-stable.

The defining design move was the threat-category taxonomy. The
existing connector threat corpus (`t1` through `t7`) is
provider-shaped; app surfaces have a different attack surface
geometry. The 10 categories that landed (CrossTenant,
MissingBudget, ApprovalBypass, UnauthorisedCaller,
ReplayWithoutToken, WriteWithoutApproval, RoleBypass,
ExpiredApprovalReuse, DataClassDrift, MalformedPayload) cover
every named attack the reference apps already test by hand. The
walker is opinionated about which categories attach to which
surface kinds: only `dangerous: true` tools get suggestions
(non-dangerous tools are already constrained by their type
signature), only writeable stores get write-targeted suggestions
(read-only caches don't have a write attack surface), only
`pub extern "c"` agents get unauthorised-caller suggestions
(internal agents are unreachable from a host). This keeps the
suggestion list short and high-signal — an operator who runs
the helper against an app with N dangerous tools, M `pub extern
"c"` agents, K writeable stores, and L approval sites sees
exactly `3N + 2M (+ 1 if any replayable) + 2K + 3L (+ 1 per
approval with dangerous_targets)` suggestions, every one
actionable.

A secondary lesson: snake_case conversion needs to split on
camelCase boundaries. The first attempt did not — `ExportTenantCorpus`
became `exporttenantcorpus_cross_tenant_refused`, which is
unreadable. The corrected walker inserts an underscore before
each uppercase letter that follows a lowercase or digit. The
inline test
`render_adversarial_refresh_is_byte_identical_across_two_invocations`
asserts the named approval surfaces (`ExportTenantCorpus`,
`ShareAnswerToChat`) produce the expected snake_case fixture
names, locking in the convention.

The suggestion-ordering choice — sort by `surface_kind` slug →
`surface_name` → `threat` slug — was the third design move. An
operator triaging the output wants all approval-site suggestions
together, then all tool suggestions, then all agent suggestions,
then all store suggestions; within each kind, alphabetical by
surface name so they can grep; within each surface element,
alphabetical by threat slug so the same threat category appears
in the same position across surfaces. Two runs on the same
descriptor produce byte-identical output, which is what makes
the report safe to embed in CI gates that diff helper output
across builds.

## 35V2-P42-H-LR-3 app pr-describe — H-LR umbrella closed (2026-05-30)

`corvid app pr-describe --base <base.cor> --head <head.cor>`
ships the third sub-slice and closes the Phase-42 per-app AI
helpers umbrella. The helper lowers both Corvid sources to ABI
descriptors in-process and renders a typed `PrDescription`
ordered Breaking → Additive → Informational.

The defining design move was deciding *what to flag breaking*.
The boring cases (removed agents/tools/approvals/stores/types,
schema-version bumps, claim-guarantee removals) are
unambiguous. The interesting cases are the silent-relaxations
the helper exists to catch:
- `pub extern "c"` revoked on a same-name agent → Breaking,
  because hosts that called the exported symbol now fail to
  link, even though the agent name still exists in the source.
- Approval-tier weakening (e.g. `human_required` → `operator`,
  `operator` → `autonomous`) on a same-label approval site →
  Breaking, because the policy contract just got loosened
  without removing the gate.
- Field count drops on a same-name type → Breaking, because
  downstream consumers that read the missing field will break
  even though the type name is unchanged.

These three are the kind of changes a reviewer needs surfaced
in plain English on top of the PR description, because reading
the source diff alone obscures them — the source might just
say "delete one line" but the consequence reaches every host
that linked the symbol.

The renderer choice — sort sections by severity then heading,
then bullets in insertion order — keeps the most consequential
changes on screen first. Two runs on the same `(base, head)`
descriptors produce byte-identical output, locking down the
CI-gate-friendly property.

A small implementation note worth keeping for future helpers:
the `push_section_if_non_empty` predicate suppresses empty
sections entirely. Without it, every report would render every
heading even when there's nothing to say, which would noise the
output. Empty inputs collapse to "no descriptor-surface
changes between base and head" — one line, no false-positive
sections.

With H-LR-3 shipped, the H-LR umbrella closes. All three per-
app helpers (boot-summary, adversarial-refresh, pr-describe)
ship as deterministic typed classifiers, each promoting a new
RuntimeChecked guarantee row under `GuaranteeKind::App`. Phase
42's launch-readiness tail now stands at:

- ✅ E-LR (CI smoke-deploy)
- ✅ F-LR (benchmark files)
- ✅ G-LR (per-app CLAIM.md)
- ✅ **H-LR (per-app AI helpers)** — this commit
- ⏳ Phase 33M (external reviewer signoff — last)

## 35V2-P43-deploy-reproducible-build promoted (2026-05-30)

`deploy.reproducible_build` moved from `OutOfScope` to
`RuntimeChecked`. The promotion was *not* mechanical despite
the registry comment claiming it would be — the
reproducible-build CI workflow had been failing on every push
to `main` since it landed, and the row's `out_of_scope_reason`
explicitly said the promotion would happen "once that first
run lands green (or we close the determinism gap it surfaces)."
The fix lives in the second clause: a real determinism gap.

The leak was `crates/corvid-codegen-cl/build.rs` emitting
`cargo:rustc-env=CORVID_STATICLIB_DIR=<absolute target dir>`,
which `link.rs` and `cdylib.rs` then read through `env!()`.
That macro embeds the build-time value into the binary's
read-only data section as a string literal. The CI workflow
runs two `cargo build -p corvid-cli --release` invocations with
`CARGO_TARGET_DIR=target-build-1` and `CARGO_TARGET_DIR=
target-build-2` — the embedded string differs between the two
builds, so SHA-256 diverges even though source, lockfile,
toolchain, and `SOURCE_DATE_EPOCH` are identical.

The fix routes the staticlib lookup through a new
`staticlib_discovery::discover_staticlib` function that resolves
at runtime. Resolution order: `CORVID_RUNTIME_STATICLIB_OVERRIDE`
(file override, already supported for tests) → new
`CORVID_STATICLIB_DIR` runtime env var (explicit dir override) →
walk up from `current_exe()` ancestors (covers `cargo run`,
`cargo test`, and shipped installs where the staticlib lives
next to the binary) → documented sibling-`lib/` layouts for
FHS-style installs. The build script no longer emits any
host-dependent strings. As a side benefit, shipped binaries
distributed via `corvid release build` no longer leak a
developer's `/home/runner/work/Corvid-lang/...` host path into
the binary's strings — that was never useful to end users.

Two structural traps worth keeping for any future
"compile-time-bake-vs-runtime-discover" debate:

1. `env!("X")` is friendlier ergonomically than runtime
   discovery, but it bakes `X`'s value into the binary as a
   string literal. Anything that varies across builds (target
   dir, manifest dir, source paths, timestamps from build.rs)
   becomes a reproducibility leak as soon as it appears in a
   `cargo:rustc-env=`. The right default is runtime discovery
   with an explicit env var override.
2. Doc comments that mention a previously-removed symbol
   (`env!("CORVID_STATICLIB_DIR")` in `link.rs:219`) tripped my
   first structural regression test, which grepped for the bare
   identifier. Tighten patterns to the actual emission/usage
   syntax (`cargo:rustc-env=CORVID_STATICLIB_DIR` and
   `env!("CORVID_STATICLIB_DIR")`), not the bare name — doc
   comments that explain *why* the historical path was removed
   are valuable and should not be rewritten away.

The production-grade oracle remains
`.github/workflows/reproducible-build.yml`. The structural
tests in `crates/corvid-codegen-cl/tests/reproducibility.rs`
lock the regression in place locally so a future refactor that
reintroduces the bake fails before reaching CI.

## HTTP approval queue: `corvid serve` answers 202 instead of 403 (2026-06-04)

`corvid serve <app>` now handles approval-gated routes through the
existing `ApprovalQueueRuntime` flow instead of denying them
outright. The shipped behaviour:

A `POST` to an approval-gated route returns **`202 Accepted`** with
`Content-Type: application/json` body shape:

```json
{
  "approval_id": "serve-1780500000000000-0",
  "status": "pending",
  "poll": "/__approvals/serve-1780500000000000-0",
  "detail": "this write is approval-gated; a pending approval has been queued. Poll the `poll` URL for the decision."
}
```

and a `Location: /__approvals/<id>` header. The client polls
`GET /__approvals/<id>` until `"status"` transitions from
`"pending"` to `"approved"` or `"denied"`. A reviewer transitions
the queue entry via either:

  - `POST /__approvals/<id>/approve` — server marks the queue
    record approved, looks up the pending invocation captured at
    queue time, re-runs the original agent under a fresh `Runtime`
    whose approver is `ProgrammaticApprover::always_yes()`, returns
    `200 OK` with `{"status":"approved","result":<agent value as JSON>}`.
  - `POST /__approvals/<id>/deny` — server marks the queue record
    denied, drops the pending invocation (no re-execution), returns
    `200 OK` with `{"status":"denied","id":...}`.

Both transition endpoints return `404` on unknown id, `409` on
already-decided id, `500` on queue IO failure.

Two read-only admin endpoints round out the surface:

  - `GET /__approvals` — list pending approvals for the
    `serve-default` tenant (the slice MVP is single-tenant).
  - `GET /__approvals/<id>` — fetch one record with id / action /
    status / tenant_id / requester_actor_id / created_ms /
    updated_ms.

The prior `403 approval_required` shape from slice `E0-serve-4` is
kept as a defensive branch in `finish()` — if a host wires a
non-queue approver into the serve runtime, the prior semantics
are preserved.

**What you cannot do yet:** per-request reviewer authentication.
Today every reviewer is the single anonymous `serve-reviewer`
actor (distinct from the requester `serve-anonymous`, because the
queue's `authorize_approval_transition` rejects self-approval).
mTLS / OAuth / session-cookie reviewer auth is a slice follow-up.
Multi-step approval chains are also deferred — the slice MVP
assumes a single `approve` boundary per route; an agent with two
approval gates would re-queue at the second one. Persistent
approval DB is also deferred — today the in-memory queue is
ephemeral to the serve process and dies with it. `--approvals-db
<path>` is the planned flag for file-backed persistence.

Try it:

```bash
corvid build --target=server my_app/
./target/release/my_app --listen 127.0.0.1:8000 &
curl -X POST http://127.0.0.1:8000/actions/refund -d '{"amount":1000}'
# 202 Accepted + {"approval_id":"serve-...","poll":"/__approvals/..."}
curl http://127.0.0.1:8000/__approvals
# {"approvals":[{"id":"...","action":"IssueRefund","status":"pending",...}]}
curl -X POST http://127.0.0.1:8000/__approvals/<id>/approve
# 200 OK + {"status":"approved","result":{"id":"refund-...","amount":1000,...}}
```

Cross-references: dev-log entry `2026-06-04 — v1.0 launch
criteria push: 5 of 7 mechanically green` walks through the
trait-shape preservation move (introducing
`RuntimeError::ApprovalQueued { approval_id }` rather than a
third `ApprovalDecision::Queued` variant) and the synthesized-
default-contract MVP design decision.

## `#[tool]` accepts struct params and returns (2026-06-04)

Before slice `35V2-P42-G0-tools-3b` the `#[tool]` proc-macro
aborted with a hard compile error whenever any arg or return type
wasn't `i64` / `f64` / `bool` / `String`:

```text
#[tool] signatures currently support only `i64` (Corvid Int),
`f64` (Float), `bool` (Bool), and `String`. Got `Receipt`.
Struct/List arguments and returns are not implemented yet.
```

Now the macro accepts struct params and returns, gated on the
struct deriving `serde::Serialize` and `serde::Deserialize`:

```rust
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Receipt {
    pub label: String,
    pub delivered: bool,
    pub count: i64,
}

#[corvid_macros::tool("emit_receipt")]
async fn emit_receipt(label: String) -> Receipt {
    Receipt { label, delivered: true, count: 1 }
}

#[corvid_macros::tool("consume_receipt")]
async fn consume_receipt(r: Receipt) -> bool {
    r.delivered && r.count >= 1
}

#[corvid_macros::tool("amend_receipt")]
async fn amend_receipt(r: Receipt) -> Receipt {
    Receipt { label: r.label, delivered: r.delivered, count: r.count + 1 }
}
```

**Dispatch shape.** When EVERY arg + return is scalar
(`i64` / `f64` / `bool` / `String`), the macro emits BOTH the
typed C-ABI wrapper (`__corvid_tool_<name>` — codegen direct-call
symbol for native-binary targets) AND the JSON wrapper (registry
dispatch path for cdylib targets). Tool metadata `symbol` field
records the typed-wrapper name. When ANY arg or return is
non-scalar (struct, list, custom path), the macro omits the typed
wrapper entirely and emits ONLY the JSON wrapper, with the
inventory entry's `symbol: ""` as the marker that says "no direct-
dispatch wrapper exists; route only through `json_dispatch`."

**What this means for native-binary builds.** If you target
`--target=native` (not cdylib) and your tool has a struct
signature, the linker fails cleanly with `unresolved external
symbol __corvid_tool_<name>`. That's the right failure mode — a
silent emit of a scalar wrapper around a struct value would be a
wrong-ABI miscompilation. cdylib builds dispatch through the
runtime registry (`G0-tools-2b` target-conditional path) and work
without any code change on the user side.

**What's unchanged.** Scalar tools keep emitting both wrappers
with the typed wrapper as the linker-visible direct-call symbol —
no regression in the scalar path. The `#[tool]` macro contract
(`async fn`, free function not method, identifier-shaped
parameter names) is unchanged.

Cross-references: the dev-log entry walks through the
`signature_is_all_scalar` predicate that drives the branch,
the 3 new macro-expand tests
(`struct_signature_tools_register_in_inventory_with_empty_symbol_marker`,
`scalar_signature_tools_keep_typed_wrapper_symbol`,
`user_struct_signature_fns_still_callable_directly`), and the
`abi_type_for` documentation change that now treats its `Err`
branch as an internal macro bug rather than a user-facing error.

## v1.0 launch claim audit: `corvid claim audit` table-row format (2026-06-04)

`corvid claim audit` walks `docs/meta/launch-claim-audit.md` and
exits 1 if any claim row in the doc fails one of two checks:

  1. **MissingEvidence** — the second column lacks both a runnable
     command (backtick-fenced text) AND a linked artifact
     (`[label](path)` markdown link) AND an explicit `blocked:` or
     `non-scope` annotation.
  2. **AspirationalWording** — the second column contains one of
     the words `todo`, `planned`, `future`, `soon`, or
     `will support` AND is not explicitly marked `blocked` or
     `non-scope`.

The parser at `crates/corvid-cli/src/claim_cmd.rs:211` only skips
table header rows that literally contain `| Claim |` — every other
header gets parsed as a claim row. **Standardize every audit-style
table's first column header as `Claim`** so the parser treats
header rows as headers and the table contents are the only thing
audited.

For rows naming gaps (`blocked: <slice-id>` or `non-scope`), the
recommended cell format:

```markdown
| Gap name | **blocked: 33J4** — the rest of the prose ... | When unblocked |
```

The `**blocked:** <id>` prefix is what the parser keys on; the
markdown bold isn't required by the parser but reads cleanly.

`corvid claim audit --explain-failures` returns typed
`ClaimFindingKind` (`missing_evidence` / `aspirational_wording`)
plus a `suggested_fix` that back-references the inventory line.
Promotes `claim.audit_explain_failures_grounded` to
`RuntimeChecked`. The audit is documented as the launch claim
audit's mechanical enforcement step at
`docs/meta/launch-claim-audit.md` Section 9 — re-run on every
35V2 LR slice close, every 43-letter slice close, before the
v1.0 cut, and whenever a new genuinely-open slice ships a
public-facing claim.

`corvid claim audit` currently reports `claim_count: 56,
finding_count: 0, exit=0` against `main` at HEAD.

## v1.0 launch criteria mechanically green: L47/L48/L49/L50 (2026-06-04)

5 of 7 v1.0 launch criteria are now mechanically gated by tests
that run in CI or are invoked pre-cut, not by maintainer judgment.
The status table:

| Criterion | What it gates | Mechanical gate | Status |
|---|---|---|---|
| L46 | Every Phase 37-43 phase-done | bundled with L51 + L52 | open (Path-A timing) |
| L47 | Every reference app deploys via Phase 43 packaging | `cargo test -p corvid-cli --test deploy_manifests` runs in `app-deploy-smoke.yml` | ✅ |
| L48 | Every cdylib claim id in coverage gate | `cargo test -p corvid-driver signed_claim_coverage` — 5/5 against 75-row registry | ✅ |
| L49 | Launch claim audit re-run | `cargo run -q -p corvid-cli -- claim audit` — 56 claims, 0 findings | ✅ |
| L50 | Bilateral verifier green across production-backend surface | `cargo test -p corvid-abi-verify --test reference_apps_bilateral_match -- --ignored` in `app-deploy-smoke.yml` | ✅ |
| L51 | Friends-and-family round | external | Path-A final 4 weeks |
| L52 | 33J4 + 33J5 + 33L + announcement drafts | external + website | Path-A final 2 weeks |

The L50 wire-up at `0fc9d89` adds `cargo build -p corvid-runtime`
as a prereq step to the workflow so `libcorvid_runtime.a` lands
on disk before the cdylib link — same constraint that bit the
`effect-system-gates` workflow at commit `fcf4ce4`. CI cost: ~15s
warm, ~1m38s cold.

Cross-references: the launch claim audit cadence at
`docs/meta/launch-claim-audit.md` Section 9 names which slices
re-trigger an audit re-run, and the dev-log entry walks through
which OutOfScope registry rows were verified genuinely-OOS in
the L48 audit (3 Phase 35V-T1-B downgrades, 7 post-v1.0 source-
syntax sugar, 5 explicit TCB-boundary non-defenses).

## `corvid serve` loads tool handlers — `tools.py` autoload + `--with-tools-cdylib` (2026-06-05)

`corvid serve` now connects approval-gated dangerous tools to real
handlers two ways. Pick whichever fits your shape; both ship in the
v1.0 release-candidate CLI.

### `tools.py` — the fast iteration path

Drop a `tools.py` file next to your source (or in the project root —
the `corvid new` scaffold writes one). Decorate each implementation:

```python
from corvid_runtime import tool

@tool("send_message")
async def send_message(req):
    # your real impl here
    return {"delivered": True}
```

Run `corvid serve` against your project as usual. The CLI autodetects
`tools.py`, embeds Python via PyO3, imports the module (which runs
the `@tool(...)` decorators), and bridges each registered coroutine
into the runtime's tool registry. The interpreter now dispatches
through to your coroutine on every `tool send_message(req)` call.

When the same agent is invoked via the `/__approvals/<id>/approve`
re-execution path, the registered tools persist — the bypass runtime
inherits the same registry as the original request, so the
"approval granted but tool missing" 500 is no longer reachable.

Errors carry the full Python traceback. If `tools.py` raises at
import time, the operator sees the traceback in the `corvid serve`
stderr, not a confusing 500 buried in the first request.

### `--with-tools-cdylib <path>` — the production path

For production-shape deployments where Python isn't in the request
path, build a Rust crate with `crate-type = ["cdylib"]` and the
`#[tool]` proc-macro:

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
corvid-runtime = "..."
```

```rust
// src/lib.rs
use corvid_runtime::tool;

#[tool]
async fn send_message(req: serde_json::Value) -> serde_json::Value {
    // your real impl here
    serde_json::json!({"delivered": true})
}
```

`cargo build --release` produces `target/release/lib<crate>.{so,dylib,dll}`.
Run `corvid serve src/main.cor --with-tools-cdylib target/release/libtools.so`
and the CLI dlopens the cdylib, dlsyms each `__corvid_tool_<name>`
symbol the proc-macro emitted, registers it via the runtime's
C-ABI tool registry, and bridges into the interpreter the same way
the `tools.py` path does.

NOTE: the flag is `--with-tools-cdylib`, NOT `--with-tools-lib`. The
`build` family's `--with-tools-lib` accepts a STATICLIB for compile-
time linking; the serve path runs the interpreter and needs a
dlopen-able CDYLIB. Build your tools crate with `crate-type =
["cdylib"]`, not `["staticlib"]`.

### When both are present

If a project has BOTH a `tools.py` AND the operator passes
`--with-tools-cdylib`, the cdylib wins precedence for any name they
share. Mental model: explicit beats implicit. The other tool names
from each side stay registered (so you can mix Python tools and
cdylib tools in the same app, as long as their names don't collide).

### When neither is present

The interpreter's existing `UnknownTool` runtime error fires at the
first tool call. The error message names the tool that wasn't
registered so the operator can decide which path to take. No silent
fallback to a placeholder; no consumed approval on miss (that's
33Q2's separate fix).

### What this is NOT

- Not a Python sandbox. Whatever your `tools.py` imports runs with
  the full Python interpreter's permissions. If you import
  `subprocess` and spawn `rm -rf /`, the runtime won't stop you.
- Not a substitute for `corvid build --target=cdylib` + a host
  binary for production. `corvid serve` is the demonstration /
  development path; for production traffic, build the signed cdylib
  and run it under a real host with proper resource isolation.
- Not async-IO on the GIL. Each tool call serializes inside the
  GIL via `asyncio.run(coro)` on a tokio blocking thread.
  Throughput is bounded by your slowest tool's coroutine; if you
  need concurrent tool dispatch, use the cdylib path or wait for
  a future async-IO bridge.

## Approvals are not burned when handlers error (2026-06-05)

When a reviewer POSTs `/__approvals/<id>/approve` and the downstream
handler errors (network failure, missing tool, unexpected exception),
the approval STAYS pending. The reviewer can retry without granting
a second human authorization, OR POST `/__approvals/<id>/deny` to
terminate a permanently-broken approval.

### What you see from the client side

A failed-handler `/approve` returns HTTP 500 with body:

```json
{
  "error": "approved_execution_failed",
  "detail": "<the runtime error that surfaced>",
  "approval_status": "pending",
  "retry": {
    "possible": true,
    "url": "/__approvals/<id>/approve",
    "note": "approval was not consumed; POST again to retry, or POST /__approvals/<id>/deny to terminate the pending invocation if the handler is permanently broken"
  }
}
```

A subsequent `GET /__approvals/<id>` reports:

```json
{
  "id": "...",
  "status": "pending",
  "last_handler_error": "<the same runtime error>",
  "retry_possible": true,
  ...
}
```

`last_handler_error` is refreshed on every failed `/approve`
attempt, so the reviewer always sees the most recent failure
reason.

### What a reviewer client should do

1. POST `/approve` → 500 with `retry.possible: true`.
2. Surface `detail` + `last_handler_error` to the reviewer.
3. Reviewer decides: retry (the handler issue might be transient,
   like a network blip) or deny (the handler is broken; the
   approval shouldn't stay open forever).
4. POST `/approve` to retry OR POST `/deny` to terminate.

After `/deny`, `/approve` answers 409 — the reviewer's explicit
deny is terminal. Their authorization is preserved across retry
attempts and never silently consumed by a transient failure.

### What this is NOT

- Not a guarantee that the handler will eventually succeed.
  If your handler is broken (missing dependency, permanently
  wrong code path), retrying won't help. `/deny` is how you
  exit.
- Not retroactive replay of side effects that DID succeed
  before the failure. The "did anything execute before the
  error?" question is your handler's responsibility — Corvid
  guarantees the approval state is intact, not that your
  handler's partial-execution can be safely retried.
- Not concurrent retries. `/approve` is sequential per
  approval id; two simultaneous retries serialize on the
  approval-queue lock.

## `@trust(...)` and `corvid build --sign` work together (2026-06-05)

The `@trust(<level>)` annotation on an exported agent is now a
signable contract. `corvid build --target=cdylib --sign <key>`
emits a DSSE-signed ABI descriptor whose `claim_guarantees`
array includes `trust.constraint_enforcement`, and
`corvid claim --explain <binary>` enumerates it as an enforced
guarantee.

What gets enforced:

- The typechecker rejects an agent body that composes a trust
  dimension stricter than the declared ceiling. For example, an
  agent declared `@trust(autonomous)` that reaches a tool with
  `trust: human_required` without an `approve` boundary fails
  to compile.
- The trust lattice is `autonomous < supervisor_required <
  human_required`. The confidence-gated variant
  `@trust(autonomous_if_confident(0.95))` is treated as
  `autonomous` at typecheck and escalates to `human_required`
  at runtime when composed confidence drops below the
  threshold.

What the signed claim guarantees:

- The shipped cdylib advertises the `trust.constraint_enforcement`
  guarantee in its DSSE-signed descriptor.
- `corvid claim --explain target/release/main.so` shows the id
  in the `enforced_guarantees:` block.
- If the cdylib's source declared `@trust(...)` but the
  descriptor's `claim_guarantees` array doesn't include the
  trust id, `corvid build --sign` refuses to emit a signature
  (the bilateral source-match gate catches this).

What this is NOT:

- Not a runtime budget enforcement. `@trust` is a typecheck
  ceiling on what the agent's body can compose; runtime
  re-evaluates trust at every dangerous-call site using the
  confidence-gated escalation if declared.
- Not a substitute for `approve` boundaries. `@trust(autonomous)`
  doesn't mean "this agent has authority to call dangerous
  tools"; it means "this agent's effective trust is at most
  autonomous, so it can't reach human-required tools." If the
  agent reaches a dangerous tool of any trust level, it still
  needs an `approve` boundary.

## Every HTTP route executes — path params, query, and typed bodies (2026-07-22)

`corvid serve` now runs *every* declared route shape through the
ordinary agent interpreter — including the path-parameter, typed-
query, and typed-body routes that previously returned a `501
not_implemented`. Given

```
server orders_api:
    route GET "/orders/{id}" -> json OrderView:
        return OrderView(path.id, "open", 42.0)

    route GET "/orders" query OrderFilter -> json OrderPage:
        summary = OrderSummary("order-1", query.status)
        return OrderPage([summary], query.limit)

    route POST "/orders" body NewOrder -> json OrderReceipt:
        return OrderReceipt("order-new", true)
```

all three respond `200` with the handler's JSON:

```
$ curl -s localhost:8531/orders/order-42
{"id":"order-42","status":"open","total":42.0}

$ curl -s 'localhost:8531/orders?status=open&limit=5'
{"count":5,"orders":[{"id":"order-1","status":"open"}]}

$ curl -s -X POST localhost:8531/orders -d '{"item":"widget","quantity":3}'
{"accepted":true,"id":"order-new"}
```

Malformed boundary input is a structured `400`, never a `500`:

```
$ curl -s 'localhost:8531/orders?status=open&limit=notanumber'
{"detail":"`notanumber` is not an Int","error":"invalid_query"}

$ curl -s 'localhost:8531/orders?status=open'
{"detail":"missing query param `limit`","error":"invalid_query"}
```

How it works — a route is compiled to a **synthetic handler
agent** named `__route__<METHOD>__<mangled-path>` whose parameters
reuse the exact `path` / `query` / `body` / `actor` `LocalId`s the
resolver bound in the route body. Because the params share the
body's locals, the route body *is* the agent body: `path.id`,
`query.status`, `body.item`, and `actor.id` resolve with no
rewriting. Serve then:

- registers the real axum path, translating `/orders/{id}` →
  `/orders/:id`;
- coerces each path parameter and each query-struct field from its
  request string into the declared scalar type (`Int`/`Float`/
  `Bool`/`String`);
- decodes the request body as typed JSON into the declared body
  struct;
- assembles the arguments in the handler's declared parameter
  order and invokes it through the same `run_ir_with_runtime` path
  as any other agent — so effects, approval, provenance, and
  replay all apply to route execution automatically.

What this is NOT:

- Not a route-shape allowlist. There is no "supported shape"
  classifier any more; the old `dispatch_for`/`RoutePlan` code and
  its `501` branch are deleted. Every route the contract advertises
  is served.
- Not yet an authenticated actor. A route carrying a `requires`
  policy still binds an empty `actor` placeholder — real session-
  derived actors and authorization enforcement land in the auth
  slices. Route execution is proven; identity is not yet wired.

The reference application `examples/reference_app/src/main.cor` is
the continuous Phase-52 fixture: it starts here exercising these
three shapes and grows with every subsequent slice.

## Contract Closure — the backend refuses to start rather than advertise a route it can't serve (2026-07-22)

`corvid serve` now proves it implements its own contract before it
binds a listener. It walks the public HTTP surface the Application
Contract advertises and asserts a runtime execution path exists for
every route. A route the contract describes but the interpreter tier
cannot yet execute is a startup error (`E5204`), never a silent
runtime `501`:

```
$ corvid check streaming_app.cor
ok: streaming_app.cor — no errors

$ corvid serve streaming_app.cor
error: streaming_app.cor is not contract-closed — 1 route(s) the
contract advertises cannot be executed by this runtime yet:
  E5204 Contract not executable: route GET /orders/stream needs
  streaming responses (Server-Sent Events) — a runtime path for it
  does not exist yet (arrives in slice 52c). The backend refuses to
  start rather than advertise a surface it cannot serve.
$ echo $?
1
```

The source COMPILES — closure is a serve-time runtime-path assertion,
distinct from type checking. The gaps the check currently detects, and
the slice that closes each:

- `Stream<T>` response → SSE endpoint (52c)
- `Upload<Format>` body → multipart parser (52c)
- `Page<Item>` response → cursor envelope (52c)
- `requires`-policy route → authorization enforcement (52h)

The check reads a `RuntimeCapabilities` snapshot describing what the
interpreter tier can execute as of the current slice. Each Phase 52
slice that lands a capability flips one field to `true`, so the closure
surface grows in lockstep with the runtime — the backend can never
advertise more than it delivers. A policy route is detected via its
synthetic handler agent's `actor` parameter (52a binds one only for
`requires` routes).

What this is NOT:

- Not a type-check. A `Stream<T>` route type-checks fine; closure runs
  at serve startup, over the routes the contract mounts.
- Not a permanent ban. The same route starts cleanly the moment its
  capability lands — `capability_present_closes_the_gap` proves a
  streaming route is no longer a gap once `streaming` is `true`.

Reference: guarantee `contract.runtime_closure` (RuntimeChecked) in
core-semantics.md; `corvid tour --topic contract-closure`.

## A `Stream<T>` route streams as Server-Sent Events (2026-07-22)

A route whose response type is `Stream<T>` now serves the stream as
Server-Sent Events end-to-end. Given

```
type Tick:
    n: Int
    label: String

agent ticker() -> Stream<Tick>:
    yield Tick(1, "first")
    yield Tick(2, "second")
    yield Tick(3, "third")

server ticker_api:
    route GET "/ticks" -> json Stream<Tick>:
        return ticker()
```

`curl -N localhost:PORT/ticks` returns:

```
data: {"label":"first","n":1}

data: {"label":"second","n":2}

data: {"label":"third","n":3}

event: done
data:
```

Each yielded value flushes as one `data: <json>` event; the stream
closes with `event: done`. `corvid serve` consumes the interpreter's
stream channel (the same `StreamValue` the language's streaming
machinery already produces) and pipes it through axum's SSE response —
the modern AI-app transport falls straight out of the `Stream` type
with zero glue.

The interesting history: the SSE `finish` arm was written speculatively
in the Phase 51 era, but it was never reachable — routes returned `501`
before slice 52a, and slice 52b's Contract Closure then refused any
`Stream<T>` route at startup. Slice 52c-1 verified the SSE path
end-to-end and flipped the `streaming` `RuntimeCapability` on, so a
streaming route now passes closure and serves. This is the closure
design working exactly as intended: the capability was dark until its
runtime path was proven, and the backend refused to advertise it until
then.

What this is NOT: not provider-native session continuation (resuming a
model stream across a dropped connection is adapter work); not
backpressure tuning across the HTTP boundary. The typed event transport
from a `Stream<T>` route is what ships.

## `Upload<Format>` bodies and `Page<Item>` responses execute (2026-07-22)

The last two HTTP-boundary types now run end-to-end.

**Pagination.** A handler builds a page with the `Page(items, next_cursor)`
constructor — the type name is callable, exactly like `Ok(x)` / `Some(x)`:

```
type Item:
    id: String
    name: String

type ItemQuery:
    cursor: String
    limit: Int

server items_api:
    route GET "/items" query ItemQuery -> json Page<Item>:
        a = Item("i1", "first")
        b = Item("i2", "second")
        return Page([a, b], Some(query.cursor))
```

serves the standard cursor envelope:

```
$ curl -s 'localhost:PORT/items?cursor=abc123&limit=10'
{"has_more":true,"items":[{"id":"i1","name":"first"},{"id":"i2","name":"second"}],"next_cursor":"abc123"}
```

`has_more` is DERIVED from `next_cursor` (a `Some(_)` cursor means another
page exists), and the cursor is unwrapped from the `Option` so the envelope
carries `next_cursor: "abc123"` or `null` — not the tagged option form. The
incoming cursor is an ordinary field of the route's typed `query` struct.

**Uploads.** An `Upload<Format>` body is read through METHODS, and serve does
the multipart work at the boundary:

```
agent take_import(body: Upload<Csv>) -> ImportReceipt:
    return ImportReceipt(body.filename(), body.size(), body.text())

server import_api:
    @upload(max_mb: 25, mime: "text/csv")
    route POST "/import" body Upload<Csv> -> json ImportReceipt:
        return take_import(body)
```

```
$ curl -s -F "file=@data.csv;type=text/csv" localhost:PORT/import
{"bytes_len":22,"filename":"data.csv","preview":"id,name\n1,alice\n2,bob\n"}

$ curl -s -F "file=@data.csv;type=application/pdf" localhost:PORT/import
{"detail":"`application/pdf` is not accepted for Upload<Csv>; expected one of: text/csv","error":"unsupported_media_type"}
```

`corvid serve` parses the multipart request (via `multer`) and enforces the
route's exact source-declared MIME set and maximum—a structured `400` on
either violation—and materialises the upload as a value the five accessor
methods read: `body.text()` (UTF-8 decode), `body.bytes()` (`List<Int>`),
`body.filename()`, `body.content_type()`, `body.size()`.

The maximum has no default. A direct `body Upload<Format>` route without
`@upload(max_mb: N)` or `@upload(max_bytes: N)` is a compile error; attaching
`@upload` to a non-upload route or omitting the maximum is also rejected. The
same policy is lowered into `IrRoute`, emitted in the Application Contract and
OpenAPI, and enforced by serve, so clients and the running boundary agree.

Uploads are NOT CSV-only. Every well-known format tag is supported —
`Csv` → `text/csv`, `Pdf` → `application/pdf`, `Image` → png/jpeg/gif/webp,
`Json` → `application/json`, `Text` → `text/plain`, `Audio` → mpeg/wav/ogg,
`Video` → mp4/webm — and an unknown/custom tag (`Upload<Receipt>`) falls
back to `application/octet-stream`. Binary content is preserved exactly:
`body.bytes()` returns the raw bytes, so a PDF or PNG round-trips through the
multipart → `List<Int>` path without loss (only `body.text()` is UTF-8-lossy,
as expected). The format→MIME map is the CONTRACT's `default_mime_for_format`
(`corvid_abi::app_contract`), shared by the Application Contract (frontend
pickers constrain to it) AND serve's boundary enforcement — one source of
truth, so the runtime can never accept a media type the contract told the
frontend to reject. (An early copy of the map in serve had diverged: it
omitted Audio/Video and accepted ANY type for them; sharing the contract
function fixed it.)

Two implementation notes worth keeping:

- The `Upload<Format>` format tag (`Csv`) is NOT a declared type, so the
  resolved `Type::Upload(_)` loses it (the inner is `Unknown`). serve needs
  the tag for MIME enforcement, so it is carried on `IrRoute.upload_format`;
  the explicit size/MIME policy travels beside it as `IrRoute.upload_policy`.
- Adding one `IrExprKind` variant (`PageNew`) touched ~20 exhaustive matches
  across the ABI walkers and all three compiled-codegen tiers (native /
  Python / wasm). Interpreter-only expression forms degrade LOUDLY in the
  compiled tiers (a `not_supported` error), exactly like `StructLiteral` /
  `MapLiteral` before them — `Page`/`Upload` are served by `corvid serve`, not
  lowered natively (yet).

What this is NOT: the interpreter tier buffers the whole upload body in memory
(bounded by the max size); streaming very large uploads is later work. Native
lowering of `Page`/`Upload` is deferred — the interpreter serves them.

## `parallel` fails fast, and never cancels an arm past a non-reversible boundary (2026-07-22)

A `parallel:` block used to run every arm to completion and then apply the
error rule. It now **fails fast**: when one arm errors, the still-in-flight
sibling arms are asked to stop — but with a hard guarantee that makes the
concurrency safe:

> A branch past a non-reversible effect boundary is never cancelled.

The moment an arm dispatches an irreversible tool — one whose composed effect
row is `reversible: false`, like a write or a `POST` — it is committed and runs
to completion, even if a sibling has already failed. Only arms that have done
nothing irreversible are cancelled, and they stop at a tool-dispatch boundary
*before* their next effect fires, so a cancelled arm never leaves a
half-finished irreversible action behind.

```
effect risky:
    reversible: false
    ...

agent worker() -> Bool:
    parallel:
        a = might_fail()        # reversible; cancelled if a sibling fails first
        b = commit_write()      # crosses the boundary → always completes
    return b
```

The mechanism is COOPERATIVE, not preemptive: each arm checks a shared cancel
flag at every tool dispatch. That is deliberate — a preemptive abort could stop
an arm at the await *inside* an irreversible call, after the side effect had
already happened. Checking at the dispatch boundary lets the arm decide at a
safe point, so the rule holds without a race.

A consequence to know: live `parallel` runs are now genuinely concurrent, so
*which* arm errors first (and therefore which siblings get cancelled) is
timing-dependent. A specific run is pinned by its trace — each block records a
`parallel.outcomes` event with every arm's `completed`/`errored`/`cancelled`
outcome — and replay reproduces that exact run deterministically (the replay
side lands in the companion slice). The block's reported error is the
lowest-index real error; a cancelled arm is an internal sentinel and never
surfaces. A `parallel` block where no arm errors behaves exactly as before.

**Replay reproduces the cancellation (the companion slice, now landed).** Because
a live cancelling run is timing-dependent, a Substitute-mode replay does NOT
re-derive it — it reads the recorded per-arm outcomes and runs the arms
sequentially in arm order, capping each recorded-`cancelled` arm at its recorded
tool-dispatch count. So a replayed run reproduces the EXACT cancellation: the
cancelled arm stops at its recorded boundary, a shielded (irreversible) arm
reaches its recorded terminal, and a non-cancelling block replays
byte-identically. `corvid replay` of a cancelling run reproduces the run's error
rather than diverging. A trace whose outcomes record is missing/corrupt diverges
HONESTLY (an explicit replay-divergence error), never a silent wrong result.

One general fix fell out of this: a tool that FAILS is now replayable. Before,
`call_tool` recorded a `ToolCall` and then `?`-propagated the failure before
emitting a `ToolResult`, so a failing tool had no substitutable result and
replaying any run that hit one diverged. Now a failed tool records its error
verbatim (a reserved `__corvid_tool_error__` key in the substituted
`ToolResult`), and replay reproduces it as an `Err` whose message matches the
recorded run error exactly. This was the prerequisite for replaying a cancelling
`parallel` run — the failing arm is what triggers the cancellation — but it
makes every failing-tool run replayable, not just parallel ones.

---

## An OAuth identity must declare its first-login policy — omission is a compile error (2026-07-23)

When an `identity` block declares OAuth providers, you must also declare how a
first-time, verified subject becomes an actor. There is no default:

```corvid
identity users:
    provider google
    provisioning:
        first_login: open            # public signup
        tenant: fixed("public")
```

Leave `provisioning:` out and the program does not compile:

```
E5210 First-login policy required: identity `users` declares OAuth providers
but does not state how an unknown verified subject is provisioned.
Add: provisioning: first_login: open | invited
```

This is deliberate. First-login provisioning decides your app's whole
registration and tenancy posture — if it defaulted to auto-provision, an
enterprise app would silently become open-registration the moment anyone with a
matching provider account hit the callback. Corvid treats any behavior with a
security, privacy, tenancy, cost, or access consequence the same way: the
posture is stated in your source, and omitting it is a compile error naming the
missing decision (the "No hidden defaults for consequential policy" rule) — the
same shape as insecure session cookies (which need a loud `insecure_opt_out`)
and a contract route the runtime can't serve (which refuses to start).

### The knobs

**`first_login:`**
- `open` — public signup: an unknown verified subject is auto-provisioned into a
  new actor.
- `invited` — the subject is provisioned only if it matches a pre-existing
  invitation; otherwise the login is refused.
- `approval_required` — *parses*, so the checker can name it, but is rejected
  today (`E … not executable yet`); durable approval arrives in a later slice. A
  policy Corvid can't execute completely is never silently downgraded to a
  weaker one.

**`tenant:`** — a new actor's tenant is never read from a bare, caller-controlled
token claim:
- `fixed("id")` — a constant from your application config.
- `from_invitation` — the tenant recorded on the verified invitation (only valid
  with `first_login: invited`).
- `from_claim("org_id") allow "acme, globex"` — an explicitly configured issuer
  claim, constrained to a non-empty allowlist; an unlisted value is refused.

Login also never identifies or merges accounts by email — cross-provider linking
is a separate, explicit-confirmation flow (see the account-linking section).

This is the language surface; the login/callback/session routes that consume it
mount in the following slices. See [dev-log.md](dev-log.md) (2026-07-23, 52e-1).

---

## `corvid serve` mounts your OAuth login surface (2026-07-23)

Declaring an `identity` block is all it takes: `corvid serve` mounts the full
login surface for you and wires it to the auth runtime.

```corvid
identity users:
    provider google
    provider github
    provisioning:
        first_login: open
        tenant: fixed("public")
```

Serve mounts four routes:

- `GET /auth/{provider}/login` — begins the flow: mints PKCE + a single-use
  signed state + an OIDC nonce and 302-redirects to the provider.
- `GET /auth/{provider}/callback` — completes it, in a strict order: validate
  the state, exchange the code, verify the ID token (or fetch userinfo),
  recognise-or-provision the actor, and set a session cookie. Any failure is a
  generic `401` — the reason is audited, not leaked.
- `POST /auth/logout` — revokes the session and clears the cookie.
- `GET /auth/session` — returns the current actor, or `{"authenticated": false}`.

Client credentials come from the environment — `CORVID_OAUTH_<PROVIDER>_CLIENT_ID`
and `CORVID_OAUTH_<PROVIDER>_CLIENT_SECRET` — and a missing one makes `corvid
serve` refuse to start naming the variable, rather than mounting a login route
that can't authenticate. The session cookie carries the identity block's declared
`secure` / `http_only` / `same_site`.

Providers work whether or not they issue an OIDC ID token: google/microsoft/apple
are verified by their ID token `(issuer, subject)`; github/slack/discord are
verified by a server-side userinfo fetch keyed on `(provider, user_id)`. Either
way the identity is established server-side.

`corvid tour --topic oauth-login` walks through it. Route-level enforcement of
`requires authenticated|role|permission` policies is a following slice — 52e
mounts the login/session routes and provisions the actor; a `requires`-policy
route still refuses to start until the authorization runtime lands.

---

## Authenticated routes are enforced before the handler runs (2026-07-23)

A route can declare who may call it, and `corvid serve` enforces it — the check
runs before your handler or any effect executes.

```corvid
identity users:
    provider google
    provisioning:
        first_login: invited
        tenant: from_invitation
        default_role: member
    roles:
        admin: "refund:write, user:read"
        member: "user:read"

server billing_api:
    route POST "/refunds" body RefundReq -> json Receipt requires role("admin") and permission("refund:write"):
        # `actor` is the verified caller — id, tenant, roles, permissions.
        return issue_refund(body)
```

What the runtime does on each request to a `requires` route:

- **Resolves the session cookie to a verified `actor`.** No session, or a forged,
  expired, or revoked one → `401`. The actor is *always* the authenticated one —
  never anything the request body or headers supply.
- **Checks tenant, then roles, then permissions.** A role is set membership
  (`requires role("admin")` = the actor holds `admin`); a permission is the union
  of the permissions its roles grant. Insufficient authority → `403`.
- **Requires CSRF double-submit on mutations.** A POST/PUT/PATCH/DELETE must echo
  the `corvid_csrf` cookie in an `X-CSRF-Token` header → `403` on mismatch.
- **Binds the typed `actor`** into your handler — `actor.id`, `actor.tenant`,
  `actor.display_name`, `actor.roles`, `actor.permissions`.

Roles come only from where you declared them: an invited login takes its
invitation's role; an open login takes `provisioning: default_role`, or none —
least privilege, so a new user gets no authority unless you grant it. Every
decision is written to a redacted `route.authz` audit event. Revoking a role
invalidates the actor's sessions immediately.

An unsatisfiable requirement never silently always-denies: `requires
role("typo")` where `typo` isn't declared in `roles:` is a compile error. See
[dev-log.md](dev-log.md) (2026-07-23, 52f).

---

## Approval decisions require a verified reviewer (2026-07-23)

Approving or denying a queued dangerous action is a privileged, authenticated
operation — `corvid serve` no longer lets an anonymous caller decide.

The app declares who may decide with an ordinary permission:

```corvid
identity users:
    provider google
    provisioning:
        first_login: invited
        tenant: from_invitation
    roles:
        reviewer: "approvals.decide"
```

`POST /__approvals/{id}/approve` and `/deny` then require, in order:

- an unknown id is `404` and an already-decided one is `409` (no auth needed to
  learn that);
- a valid, non-expired, non-revoked **session** — otherwise `401`;
- a CSRF double-submit (the `X-CSRF-Token` header matching the `corvid_csrf`
  cookie), the declared **`approvals.decide` permission**, the approval's own
  tenant, and a reviewer who is **not the requester** — any failure is `403`.

Separation of duties is enforced: whoever triggered an approval can never
approve it themselves. Every decision writes a durable record (the reviewer, the
authority used, the reason, the timestamp) plus a redacted `route.authz` audit
event. Revoking a reviewer's role takes effect immediately — it also invalidates
their sessions.

Set `CORVID_CSRF_SECRET` for a durable/replicated deployment so CSRF tokens
survive a restart and are consistent across replicas.

The resulting guarantee: **no protected route executes and no approval releases
an effect unless Corvid verifies the actor, tenant, authority, and decision at
the request boundary.** See [dev-log.md](dev-log.md) (2026-07-23, 52f-4b).

---

## A connector `operation` is a callable tool, and the moat composes with it (2026-07-23)

A `connector` block declares an external API once — its base URL, its
credentials (always `secret(...)` references, never literals), and its
reliability posture — and each `operation` inside it is a callable you invoke
by name, exactly like a tool:

```corvid
effect http_read:
    cost: 1.0

type Repo:
    name: String

connector github:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_TOKEN"))
    operation get_repo(owner: String, repo: String) -> Repo uses http_read:
        GET "/repos/{owner}/{repo}"

agent fetch(owner: String, repo: String) -> Repo uses http_read:
    r = get_repo(owner, repo)   # a normal call — types as Repo
    return r
```

There is **no new call syntax**: `get_repo(owner, repo)` is an ordinary call.
Its arguments check against the operation's parameters (passing an `Int` where a
`String` is declared is a compile error), and its declared return type flows to
the caller.

The point is what this composition buys you. Because an operation shares the
exact call path a hand-written tool uses, **every effect-system guarantee
applies to it automatically**:

- A `dangerous` operation requires a prior `approve` — a connector *write* is
  governable by construction:

  ```corvid
  operation create_issue(owner: String, req: NewIssue) -> Issue dangerous uses http_write:
      POST "/repos/{owner}/issues" body req
  ```
  Calling `create_issue(...)` without an `approve CreateIssue(...)` on the line
  before is a compile error.

- An untrusted (`Tainted<T>`) value cannot reach an approval-gated operation
  argument without an explicit `trusted(...)` boundary — the same prompt-
  injection sink rule that guards dangerous tools.

- An operation whose effect row carries `data: grounded` returns `Grounded<T>`,
  so provenance propagates through connector reads.

### A connector declares which modes it may run in — silence is a compile error

Whether an operation reaches a *real external provider* is a consequential
choice, so a connector must declare its allowed execution modes in source, and
omitting them is a compile error (never a silent default):

```corvid
effect http_read:
    cost: 1.0

type Repo:
    name: String

connector github:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_TOKEN"))
    modes: [mock, real]
    operation get_repo(owner: String, repo: String) -> Repo uses http_read:
        GET "/repos/{owner}/{repo}"
        mock: Repo(repo)
```

- `modes: [...]` is the ALLOWED set (`mock`, `replay`, `real`). The deployment
  selects exactly one at start. Omitting `modes` names the decision and fails to
  compile — a connector can't back into real-provider access by silence.
- If `mock` is an allowed mode, every operation must declare a `mock:` payload,
  so mock mode is fully serveable. The payload is an ordinary expression, typed
  against the operation's return type and resolved in the operation's parameter
  scope — so `mock: Repo(repo)` may build its answer from the call's arguments.
  A `mock: 42` on an operation returning `Repo` is a type error.

### Mock mode executes today — `corvid run --mode mock`

An operation whose connector allows `mock` executes end-to-end: the deployment
picks the mode at the boundary, and the runtime evaluates the compiled `mock`
payload with the call's typed arguments.

```
corvid run --mode mock app.cor      # get_repo("micrurus","corvid") -> Repo("corvid")
corvid run app.cor                  # E5205: connector mode not selected — refuses to start
corvid run --mode real app.cor      # E5205: real mode not executable yet (arrives next slice)
```

- The mode is **required and immutable**: a connector program with no `--mode`
  refuses to start (never a first-call error), and the mode is fixed for the
  process. Selecting a mode the connector's `modes` doesn't allow — or one the
  runtime can't execute yet — is a startup refusal.
- **Mock evaluates the compiled expression, not JSON.** `mock: Repo(repo)`
  called with `repo = "corvid"` returns `Repo("corvid")` — the parameter
  resolves to its bound value through the real evaluator.
- **A connector op is a tool**, so its dispatch runs through the same governed
  pipeline: a `dangerous` operation with a denying approver is refused at
  runtime — a connector call cannot bypass approval. Every invocation records a
  redacted `connector.invocation` event (connector/operation/mode/effects/
  outcome — never arguments or payload).

### The same file runs in mock, real, and replay

All three modes execute; the deployment chooses with `--mode`:

- **`mock`** evaluates the compiled `mock` payload — no network, no secret.
- **`real`** makes the HTTP request against the connector's `base_url`. The
  credential resolves from the secret store at dispatch and rides ONLY in the
  request header — it never appears in the URL, a body, the recorded trace, or
  an error (an unresolved secret fails the call and names only the secret, never
  a value). Outbound egress is gated by the always-on SSRF floor plus
  `[http] allow`, so a connector never reaches an unpermitted host.
- **`replay`** serves the recorded interaction and NEVER falls through to a real
  request (a real run records ordinary `ToolCall`/`ToolResult` events, which
  replay consumes; the credential is absent from them by construction).

So a `corvid run --mode real app.cor` records a trace you can later
`--mode replay` deterministically, with no provider and no credential in sight.

### An HTTP status can become a typed error the compiler makes you handle

`on status <code> -> Variant` turns a provider status into a typed `Result` error.
The operation must return `Result<Success, ErrorEnum>`, and each mapped variant
must be a fieldless variant of that enum (checked at compile time):

```corvid
type GithubError:
    | NotFound
    | RateLimited

operation get_repo(owner: String, repo: String) -> Result<Repo, GithubError> uses http_read:
    GET "/repos/{owner}/{repo}"
    on status 404 -> NotFound      # HTTP 404 -> Err(NotFound)
    on status 429 -> RateLimited   # HTTP 429 -> Err(RateLimited)
    mock: Ok(Repo("corvid"))
```

A 2xx decodes to `Ok(..)`; a mapped status becomes `Err(Variant)`; an unmapped
non-2xx is a transport failure. Reliability composes too: `retry: N`,
`rate_limit: N per Ns` (refuses the over-limit call before it's sent), and
`circuit_breaker: N` (trips a repeatedly-failing operation) are declared on the
connector and enforced by the runtime.

### An operation can declare the provider's protocol, not just its request

Some provider calls do not finish when the response arrives — you submit, and
the real work happens later. Writing that by hand means a hand-rolled poll loop
whose timeout, retry, cancellation, and crash behavior nobody checks.

An `async:` block on an operation declares the temporal contract instead:

```corvid
operation submit_shipment(order: String) -> Job dangerous uses http_write:
    POST "/shipments" body order
    async:
        statuses: [queued, processing, completed, failed]
        initial: queued
        terminal: [completed, failed]
        deadline: 600s
        deadline_target: failed
        idempotency: intent
        poll GET "/shipments/{id}"
        every: 30s
        cancel POST "/shipments/{id}/cancel"   # optional
        state queued:
            on queued -> queued
            on processing -> processing
            on completed -> completed
            on failed -> failed
        state processing:
            on queued -> processing
            on processing -> processing
            on completed -> completed
            on failed -> failed
```

The call returns only when a **terminal** state is reached — the submit response
is never mistaken for completion. `{id}` binds from the DECODED submit response
(a response field beats a same-named call argument), so the poll can never
address a job the provider did not confirm. A status the declaration never
listed is refused *without* advancing the intent.

What the compiler proves before you run it: every status and state is declared
once, transition tables are total, every target exists, every state reaches a
terminal, deadlines and intervals are non-zero, the poll request does not mutate,
and a mutating submission passes through the `dangerous` approval boundary.
The worst-case poll count (`deadline / interval`) multiplies the operation's cost
for `@budget`, so a protocol cannot poll its way past a declared ceiling.

**A protocol must run inside a durable job.** Its intent has to survive a
restart, so it refuses to run anywhere else rather than silently degrading to a
poll loop that a crash would lose. The intent is checkpointed *before* the submit
leaves the process, and every observed transition is checkpointed after — so a
restart resumes at the last observation and **never submits twice**.

Cancelling the job does what the declaration supports, and says which:

- before submit — cancelled exactly, no provider work exists;
- after submit, with a `cancel` endpoint — compensated by calling it (a FAILED
  compensation is never reported as a clean cancellation);
- after submit, without one — **detached**, and the error says plainly that the
  provider job is still running and is NOT cancelled.

A provider's `Retry-After` can slow the declared cadence but never speed it up
past what the source declared. Transient poll failures are tolerated (the
submitted job is still out there); `circuit_breaker: N` consecutive failures give
up on observing, while the intent stays checkpointed for a later resume.

### See what a provider could do to you, before you deploy

```
corvid connectors simulate app.cor
```

The compiler proves your protocol is well-formed. This answers the other question
— what its legal provider behaviours actually cost you:

```
protocol shipping.submit_shipment
  deadline: 600s, at most 20 observation(s) before `failed` is forced
  outcomes the provider can produce:
    completed                    ends in `completed`
    failed                       ends in `failed`
  worth knowing:
    [non_terminating] reporting `processing` forever holds the intent in
      `processing`; after 600s (20 observations) the declared deadline forces `failed`
    [deadline_reachable] `failed` is reachable without the provider ever failing —
      a slow provider is enough
```

Everything it reports is **legal** — the checker has already rejected the
malformed protocols — so it never fails your build on its own. When your team
decides a particular legal behaviour is unacceptable, opt in:

```
corvid connectors simulate app.cor --deny non_terminating
```

That asserts this protocol cannot be held open by a provider that never fails —
something the compiler can't decide for you, because stalling is a legitimate
thing for a declaration to permit.

The walk runs through the same transition engine the runtime uses, and the
worst-case count is the same one `@budget` is charged for, so what it predicts is
what happens.

### Your frontend gets the state machine, not a nullable result

`corvid contract ts-client` projects each protocol into TypeScript:

```ts
export type ShippingSubmitShipmentState =
  | { state: "queued"; terminal: false }
  | { state: "processing"; terminal: false }
  | { state: "completed"; terminal: true }
  | { state: "failed"; terminal: true };
```

Discriminated on `terminal`, so a `switch` that forgets an outcome fails to
compile — including the one reached when the provider is just **slow**. The
status union is closed (never widened to `string`), and the protocol fingerprint
ships with it so a client can tell the backend changed the protocol underneath it.

### A protocol's lifecycle is in the trace

`protocol.submitted`, `protocol.transition`, and `protocol.settled` record the
timeline in the declaration's own vocabulary — statuses, states, and a hashed
intent key — never a provider payload or a credential. `outcome` distinguishes
the endings that look alike from outside: `terminal`, `deadline`, `breaker_open`,
`cancelled`, `compensated`, `detached`.

### Replaying a protocol replays the whole lifecycle

`--mode replay` reproduces a protocol the way it reproduces any other effect:
from the recording, never from the provider. Because a protocol is a lifecycle
rather than a call, every boundary is recorded separately — the submit under the
operation's name, each observation as `<op>.poll`, the compensation as
`<op>.cancel`. Observations replay in exactly the order the provider produced
them, so a run that went `queued → processing → completed` replays as those
three observations, not one.

Failed exchanges are recorded too, so a lifecycle that survived a provider
hiccup replays *with* the hiccup instead of a suspiciously clean history.
Replay does not re-live the wall clock: a day-long protocol replays in
milliseconds, because the recorded sequence already encodes the cadence.

If the recording does not cover an exchange, replay **refuses at that point** —
it never quietly finishes the lifecycle by asking the live provider, and it
never reports the gap as a provider timeout.

### Editing a protocol that has jobs in flight is a decision you declare

Change a protocol while intents are running and those intents were created
against a graph that no longer exists. If any of them submitted, a real provider
job is out there that Corvid cannot un-create. So the resume posture is declared,
and leaving it out is a compile error:

```corvid
on_protocol_change: refuse   # or: resume
```

- **`refuse`** — do not resume across a change. The intent stays checkpointed,
  nothing is re-submitted or re-polled, and the error says plainly that the
  provider job is still running.
- **`resume`** — continue under the new declaration, but only if the recorded
  state still exists in it. A resume that cannot find its own state is not a
  resume, so it refuses anyway.

**You never bump a version number.** Corvid canonicalises the protocol graph and
fingerprints it (`sha256:` over a versioned `corvid.protocol.canonical.v1`
encoding), so a change is detected rather than remembered — there is no integer
to forget. Three things deliberately do *not* count as a change, because each
would strand live jobs for no reason:

- **Source layout** — spans, indentation, comments, blank lines.
- **Declaration order** — re-ordering statuses, terminals, states, or
  transitions, which the checker already treats as the same graph.
- **`on_protocol_change` itself** — deciding to be more permissive must not be
  the thing that strands you.

When a change *is* detected you are told what changed, not just that something
did: the intent records the canonical encoding it was created under, so the
diagnostic reads `deadline=600 -> 900` or `state complete: removed` rather than
two opaque hashes. Every resume decision — matched, refused, or permitted — is
recorded as a `protocol.resume_decision` event carrying declarations and state
names only, never a provider payload or credential.

The Application Contract publishes the fingerprint too, so a generated client can
tell whether the protocol it was built against is the one the backend is running.

**The boundary:** this protects each intent *when it resumes*. It does not stop a
deployment that would strand intents already sitting in the queue — refusing the
deploy itself is drift quarantine, and it lands with the live-conformance work.

### Approval queues must not invent identity

An approval is a confused-deputy boundary: the requester and tenant
must come from the authenticated HTTP request, never from a queue-wide
fallback. Detect approval-capable routes from real approval sites and
resolved call edges; broad effect summaries are intentionally
conservative and produce false positives when used as reachability.

Administrative reads are part of the same security surface as
approve/deny transitions. Protect list and detail endpoints with the
declared permission model, scope storage queries by the verified tenant,
and make a cross-tenant point lookup indistinguishable from a missing
record.

**Scope note (what executes today):** the connector runtime is complete —
mock/real/replay, typed status→error mapping, and retry/rate-limit/circuit-breaker
all run end-to-end. Async provider state machines and provider-drift quarantine
are later Phase 52 slices (52h/52i). See [dev-log.md](dev-log.md) (2026-07-23,
52g-3a … 52g-3c).
