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

## Corvid, CPython, Node.js and Rust on the same workload

One Windows x64 session compared fresh command-line processes on two inputs. Each cell is Corvid's median elapsed time divided by the other implementation's median, with a 95% paired bootstrap interval. A ratio of 0.50 means the Corvid command took half as long. Both native programs were compiled before measurement.

| Input | Corvid / CPython | Corvid / Node.js | Corvid / Rust |
|---|---:|---:|---:|
| Zero records | 0.36 (0.34–0.38) | 0.20 (0.20–0.22) | 1.08 (0.99–1.18) |
| Score one million records | 0.12 (0.12–0.13) | 0.28 (0.27–0.30) | 1.47 (1.38–1.53) |

For the million-record batch, native Corvid finished sooner than CPython and Node.js and took 47% longer than optimized Rust. Startup accounts for much of the elapsed time, particularly for Node.js. The scoring loop uses integer arithmetic with deterministic inputs and makes zero model or network calls. See the [implementations and reproduction command](../../benches/process_latency/README.md) and the [raw samples and tool versions](../../benches/results/process-latency-windows-x64/session.json).

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

### Windows x64 release baseline

The [candidate run](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/44866623552) measured the same release binary twice on one runner. Each column gives the median in milliseconds from 15 samples after three warmups. The HTTP workloads use a loopback provider and average 16 sequential requests per sample.

| Workload | First run (ms) | Repeat (ms) |
|---|---:|---:|
| CLI startup | 17.482 | 16.465 |
| Check 100 functions | 19.795 | 18.217 |
| Native build of 100 functions | 462.281 | 457.090 |
| Interpreter list slices | 25.713 | 25.502 |
| HTTP ping | 13.860 | 13.784 |
| Direct provider stream | 3.636 | 3.806 |
| Corvid stream | 10.248 | 12.005 |

The paired median overhead for a 16-chunk stream was 6.533 ms and 8.199 ms. The [first report](../../benches/results/application-windows-release/baseline.json) and [repeat report](../../benches/results/application-windows-release/repeat.json) include the raw samples and CLI hash.

On [candidate 05c20e08](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/44883568390), both profiles used the same Windows x64 executable. Median startup took 14.732 ms and 14.712 ms. The paired stream overhead was 6.138 ms and 6.431 ms. All seven repeat medians fell within the benchmark's same-runner thresholds. With performance verification enabled, the candidate workflow now checks the repeat against its first profile automatically and retains the measurements for inspection if the comparison fails. This comparison measures repeatability for one build.

## Reproducible benchmarks

```sh
corvid bench src/main.cor
corvid bench compare python --source src/main.cor target/python_traces
```

Published baselines live in `benches/` and ship via Phase 22's
reproducibility scripts.
