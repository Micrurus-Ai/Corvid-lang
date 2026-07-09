# Quickstart

## Goal

Five minutes from zero to a Corvid program that calls an LLM, refuses to
compile when you remove a safety check, and produces a deterministic
replay you can show your team.

Every `corvid`-tagged code block in this chapter compiles through the
real driver in CI, and the deliberately-failing block is CI-pinned to
keep failing (`crates/corvid-driver/tests/book_snippets_compile.rs`).

## Step 1 — Make a project

```sh
corvid new hello-corvid
cd hello-corvid
```

This creates:

```text
hello-corvid/
├── corvid.toml         # project manifest + [io]/[http] security boundaries
├── .gitignore
├── src/
│   ├── main.cor        # entry point (a minimal echo tool)
│   └── std/            # vendored stdlib modules (io, http, db, json, …)
└── tools.py            # optional Python host tools for the starter echo
```

The scaffolded `corvid.toml` declares the executing-I/O security
boundaries from day one: `[io] root = "."` confines file access to the
project directory, and `[http] allow = []` fails HTTP egress closed
until you name trusted hosts.

## Step 2 — Write the entry point

The scaffold's starter program is a one-line echo tool. Replace
`src/main.cor` with an LLM-backed program:

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

prompt summarize(text: String) -> String uses llm_call:
    "Summarize the following in one sentence: {text}"

agent main() -> String:
    article = "The compiler should see what your AI is doing."
    return summarize(article)
```

What's happening:

- `effect llm_call` declares a named effect with three dimensions: cost,
  latency, confidence. Every prompt that uses this effect inherits these
  bounds.
- `prompt summarize` is a function backed by an LLM call. Its body is a
  single template string — `{text}` interpolates the parameter. Its
  return type is `String`, its effect row says it uses `llm_call`.
- `agent main` is the program entry. Agents compose prompts and tools.

## Step 3 — Run it

```sh
corvid run src/main.cor
```

You should see something like:

```text
The compiler is the AI's first line of defense.
```

## Step 4 — Read a real file (the executing-I/O surface)

The example above is an LLM call. Corvid also ships executing-I/O
surfaces — HTTP, JSON, SQLite, and file I/O — that run real I/O
through the Corvid interpreter without any Python glue.

The simplest is `io_read_text`: read a UTF-8 file from disk. The
`corvid new` scaffold wrote `[io] root = "."` to your `corvid.toml`,
which confines every executing file-I/O call to the project directory.
Path traversal (`..` escapes) is refused at the runtime boundary.

Create a small data file:

```sh
echo "Corvid ships a typed effect system." > note.txt
```

Edit `src/main.cor` to read it:

```corvid
import "./std/io" use io_read_text

agent main() -> Result<String, String>:
    file = io_read_text("note.txt")
    return Ok(file.contents)
```

Run it:

```sh
corvid run src/main.cor
```

Output:

```text
Corvid ships a typed effect system.
```

What you just got, with zero glue:

- The `[io] root = "."` boundary in `corvid.toml` confined the read
  to your project directory.
- `io_read_text("../etc/passwd")` would be refused at the runtime
  boundary with a structured diagnostic naming the offending path AND
  the configured root.
- The agent uses `Result<String, String>`; the call's typed envelope
  flows through Corvid's existing error-handling surface.
- A `@deterministic` agent calling this would be a typecheck error.
- A replay run refuses `io_write_text` calls — the filesystem is
  provably untouched.

See [`stdlib/io.md`](../reference/stdlib/io.md) for the full reference.
For the HTTP + JSON + SQLite story end-to-end, see
[Talking to the outside world](./18-talking-to-the-outside-world.md).

## Step 5 — Add a dangerous tool, watch the compiler refuse

Replace `src/main.cor` with a version that declares a refund tool and
calls it without approval. The `dangerous` marker on the tool
declaration is the compile-time approve gate; the effect's `trust:`
dimension records the trust tier the call carries (it feeds
`@trust(...)` constraints and runtime approval routing):

```corvid-error
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

effect refund_effect:
    cost: $50.00
    trust: supervisor_required
    reversible: false

prompt summarize(text: String) -> String uses llm_call:
    "Summarize the following in one sentence: {text}"

tool refund(amount: Float, customer_id: String) -> String dangerous uses refund_effect

agent main() -> String:
    article = "The compiler should see what your AI is doing."
    summary = summarize(article)
    return refund(50.0, "cust_123")
```

Check it:

```sh
corvid check src/main.cor
```

The compiler refuses:

```text
[E0101] error: dangerous tool `refund` called without a prior `approve`
    ╭─[src/main.cor:19:12]
    │
 19 │     return refund(50.0, "cust_123")
    │            ────────────┬───────────
    │                        ╰───────────── this call needs prior approval
    │
    │ Help: add `approve Refund(arg1, arg2)` on the line before this call
────╯

1 error(s) found.
```

This is the load-bearing claim: a dangerous tool call without `approve`
does not compile. Not "produces a runtime warning." Not "fails a lint."
Does not compile. (The failing block above is itself compiled in CI and
pinned to keep failing with exactly this error class.)

## Step 6 — Add `approve`, watch it pass

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

effect refund_effect:
    cost: $50.00
    trust: supervisor_required
    reversible: false

prompt summarize(text: String) -> String uses llm_call:
    "Summarize the following in one sentence: {text}"

tool refund(amount: Float, customer_id: String) -> String dangerous uses refund_effect

agent main() -> String:
    article = "The compiler should see what your AI is doing."
    summary = summarize(article)
    approve Refund(50.0, "cust_123")
    return refund(50.0, "cust_123")
```

```sh
corvid check src/main.cor
```

```text
ok: src/main.cor — no errors
```

## Step 7 — Replay it

Every Corvid run records a deterministic trace:

```sh
corvid trace list
corvid replay <trace-id>
```

Replay re-executes the recorded LLM responses and tool calls without
hitting the network. The replay output is byte-identical to the original.
This is what makes "what changed?" answerable in seconds when a model
upgrade lands.

## What you just shipped

In five minutes you wrote a program where the compiler enforced an
approval policy that a static analyzer in any other language would have
caught — at best — as a code smell. Now read **[The Moat](/docs/the-moat)**
to understand why this is the load-bearing thing the language does.
