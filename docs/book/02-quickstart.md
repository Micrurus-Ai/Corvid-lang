# Quickstart

## Goal

Five minutes from zero to a Corvid program that calls an LLM, refuses to
compile when you remove a safety check, and produces a deterministic
replay you can show your team.

## Step 1 — Make a project

```sh
corvid new hello-corvid
cd hello-corvid
```

This creates:

```
hello-corvid/
├── corvid.toml         # project manifest
├── src/
│   └── main.cor        # entry point
└── tests/
    └── main_test.cor   # one passing test
```

## Step 2 — Read the entry point

`src/main.cor`:

```corvid
effect llm_call:
    cost: $0.005
    latency: medium
    confidence: 0.9

prompt summarize(text: String) -> String uses llm_call:
    "Summarize the following in one sentence: " + text

agent main() -> String:
    article = "The compiler should see what your AI is doing."
    return summarize(article)
```

What's happening:

- `effect llm_call` declares a named effect with three dimensions: cost,
  latency, confidence. Every prompt that uses this effect inherits these
  bounds.
- `prompt summarize` is a function backed by an LLM call. Its return type
  is `String`, its effect row says it uses `llm_call`.
- `agent main` is the program entry. Agents compose prompts and tools.

## Step 3 — Run it

```sh
corvid run src/main.cor
```

You should see something like:

```
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

```
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

Open `src/main.cor` and add a refund tool:

```corvid
effect refund_effect:
    cost: $50.00
    trust: supervisor_required
    reversible: false

tool refund(amount: Float, customer_id: String) -> String uses refund_effect:
    @host.payment.refund(customer_id, amount)

agent main() -> String:
    article = "..."
    summary = summarize(article)
    return refund(50.0, "cust_123")
```

Run it again:

```sh
corvid check src/main.cor
```

The compiler refuses:

```
error[E0301]: dangerous tool `refund` called without `approve`
  --> src/main.cor:14:12
   |
14 |     return refund(50.0, "cust_123")
   |            ^^^^^^ this tool requires `approve` because its effect
   |                   row carries `trust: supervisor_required`
   |
   = help: add `approve Refund(amount, customer_id)` before this call,
           or downgrade the effect row's trust dimension if the call
           is genuinely safe.
   = guarantee: approval.dangerous_call_requires_token
```

This is the load-bearing claim: a dangerous tool call without `approve`
does not compile. Not "produces a runtime warning." Not "fails a lint."
Does not compile.

## Step 6 — Add `approve`, watch it pass

```corvid
agent main() -> String:
    article = "..."
    summary = summarize(article)
    approve Refund(50.0, "cust_123")
    return refund(50.0, "cust_123")
```

```sh
corvid check src/main.cor
```

```
ok. 1 file checked, 0 errors.
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
