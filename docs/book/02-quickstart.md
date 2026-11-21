# Quickstart

Run a Corvid application, replay its first result, then write a typed model call and read a file through the standard library. You can complete the starter, replay and file examples offline.

## Step 1 — Make a project

```sh
corvid new hello-corvid
cd hello-corvid
```

The project manifest is `corvid.toml`. Your entry point is `src/main.cor`, and the standard library lives under `src/std/`. Use `corvid new --with-python-tools` if your project also needs a Python tool module.

`corvid new` copies the standard library into the project's `src/std/`. If the installed library is missing or incomplete, the CLI supplies its bundled modules. Extra modules in the installation are also copied.

The scaffolded `corvid.toml` declares the executing-I/O security
boundaries from day one: `[io] root = "."` confines file access to the
project directory, and `[http] allow = []` fails HTTP egress closed
until you name trusted hosts.

Check and run the starter before editing it.

```sh
corvid check
corvid run
corvid trace list
```

The starter greets you with the current UTC time. Copy the JSONL path from the last column of the trace list and pass it to replay while the original source is still in place. Replace `TRACE_PATH` below with that path.

```sh
corvid replay TRACE_PATH --source src/main.cor
```

The replay prints the greeting with the **same timestamp**. It reads the clock result recorded by the first run.

## Step 2 — Write the entry point

Replace
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

- `effect llm_call` declares a named effect with three dimensions: cost,
  latency, confidence. Every prompt that uses this effect inherits these
  bounds.
- `prompt summarize` is a function backed by an LLM call. Its body is a
  single template string — `{text}` interpolates the parameter. Its
  return type is `String`, its effect row says it uses `llm_call`.
- `agent main` is the program entry. Agents compose prompts and tools.

## Step 3 — Run the prompt

Select your provider and model through `CORVID_MODEL` and configure its credential in the environment. For an OpenAI-compatible local server, use `CORVID_MODEL=openai-compat:http://127.0.0.1:PORT/v1:MODEL`, replacing `PORT` and `MODEL` with the server's values. To check the program offline, use `corvid check src/main.cor`.

```sh
corvid run src/main.cor --target interp
```

The interpreter displays the generated text with its source information. The text depends on the model's response.

```text
Grounded("The compiler can track what an AI application does.", sources: [])
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
printf '%s' 'Corvid ships a typed effect system.' > note.txt
```

In PowerShell use `Set-Content -NoNewline note.txt 'Corvid ships a typed effect system.'`. Edit `src/main.cor` to read it:

```corvid
import "./std/io" use io_read_text

agent main() -> Result<String, String>:
    file = io_read_text("note.txt")?
    return Ok(file.contents)
```

Run it:

```sh
corvid run src/main.cor
```

Output:

```text
Ok("Corvid ships a typed effect system.")
```

What you just got, with zero glue:

- The `[io] root = "."` boundary in `corvid.toml` confined the read
  to your project directory.
- `io_read_text("../etc/passwd")` would return an `Err` value with a
  structured diagnostic naming the offending path AND the configured
  root — a refusal your program observes, not a crash.
- `io_read_text` returns `Result<FileReadEnvelope, String>`: a missing
  file or a policy refusal is an Err VALUE the `?` propagates (or you
  `match` on); malformed argument shapes are still compile-time errors.
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

The last command typechecks the refund example. To execute it you would also need a registered implementation for the refund tool.

To build a browser interface for a Corvid service, continue with [Browser views](19-browser-views.md).
