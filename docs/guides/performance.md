# Performance

## What's slow, what's fast

Corvid's optimization target is the AI-orchestration surface, not raw
throughput. Honest numbers (Phase 17 baselines):

- **Orchestration overhead** (no model call): ~25–36× slower than
  Python LangChain on equivalent code paths. ~1.7–2.6× slower than
  TypeScript on `refund_bot`, `rag_qa_bot`, `support_escalation_bot`.
- **Compile-time guarantee surface**: zero runtime cost. `approve`,
  `Grounded<T>`, and effect rows are typecheck-time constructs.
- **Replay**: byte-identical re-execution at memory speed (no
  network).
- **Codegen**: Cranelift backend; comparable to debug-mode rustc for
  agent-shaped programs.

The "honest slower" framing is deliberate — Corvid's value is the
correctness surface, not raw orchestration speed. See the
[benchmarks](https://corvid-lang.org/benchmarks) page for full archives.

## When to optimize

In order:

1. The model. Most of the latency and cost is the LLM call. Provider
   routing (`requires` clauses) and prompt tightening dominate.
2. The retrieval. RAG quality > LLM quality > orchestration speed.
3. The agent shape. Agents that loop over `n` calls cost `n` × per-
   call cost; loop bounds and budget annotations are cheap to add.
4. The orchestration. If after the above three you're still
   bottlenecked on Corvid's orchestration overhead, drop the inner
   loop into Rust via the FFI.

## Budgets are real

`@budget($X)` is a compile-time ceiling. The compiler computes the
worst-case composed cost; the binary refuses to ship if it exceeds X.
Use this to keep production agents from drifting upward in cost as
they accrete features.

## Concurrent jobs

The durable runner (`corvid jobs run --source <path>.cor --workers=N`) scales to N async
workers per process. Past a few thousand jobs/second per host, scale
horizontally — multiple runner processes against one queue.

## Profiling an application

Build a release CLI and its matching runtime static library, then run the local benchmark from the repository root with Python 3.11 or later.

```sh
cargo build --release --locked -p corvid-runtime
cargo build --release --locked -p corvid-cli
python benches/analysis/application.py target/release/corvid --output baseline.json
```

On Windows use `target/release/corvid.exe`. The harness records the CLI hash, host, raw samples and medians. It checks the native executable's result, the list checksum, the HTTP response and the streamed output. Model measurements use a loopback provider with 16 chunks. Use `--baseline baseline.json` for a comparison on the same host and build profile.

### Short list slices

On Windows x64 a development-profile CLI took 918.326 ms (median) to run a program that takes 1,024 four-item slices from a 16,384-item list. Copying just the selected elements reduced the median to 59.005 ms. Both runs returned 523776. These are whole-process timings with two warmups and seven measured trials. The JSON files in `benches/results/list-slice-windows-dev/` contain the samples and binary hashes.

## Profiling

```sh
corvid run src/main.cor --profile
```

Emits a flamegraph for the run. Useful for finding hot paths in
agent composition (rare) or in custom Rust FFI calls.

## Reproducible benchmarks

```sh
corvid bench src/main.cor
corvid bench compare python --source src/main.cor target/python_traces
```

Published baselines live in `benches/` and ship via Phase 22's
reproducibility scripts.
