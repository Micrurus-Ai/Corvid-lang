# Performance

## Storefront quantity edit

On November 23, 2026, Chrome 157 on Windows 11 completed 160 checked edits across the packaged Corvid storefront and a compiled TypeScript pricing handler. Each cell shows the median / 95th percentile in milliseconds, with 40 samples per row.

| Handler | Visit | Navigation to ready | Key to DOM update | Key to estimated frame |
| --- | --- | ---: | ---: | ---: |
| Corvid | Cold | 57.80 / 82.30 | 1.80 / 2.30 | 12.65 / 25.10 |
| TypeScript | Cold | 46.20 / 65.50 | 1.70 / 2.60 | 12.35 / 25.10 |
| Corvid | Warm | 29.95 / 36.40 | 1.30 / 1.80 | 11.10 / 22.40 |
| TypeScript | Warm | 25.95 / 35.40 | 1.30 / 2.10 | 7.90 / 19.10 |

The largest median difference was on the first visit. Corvid took 11.60 ms longer to become interactive. Chrome reported 16,700 transferred bytes for Corvid's cold browser assets and 11,438 for TypeScript's. All 40 warm Corvid visits reused the Wasm from cache. Both implementations updated the DOM in a median 1.30 ms on warm visits. Frame estimates also include browser scheduling.

Both sides used the same loopback Corvid server from a debug CLI build, generated DOM controls, HTML, CSS and checkout logic. The client used the packaged Wasm loader or TypeScript 5.9.3 compiled to JavaScript. These results cover one desktop browser and one interaction. See the [method and reproduction commands](../../benches/typescript/README.md#storefront-interaction) and [all 160 observations](../../benches/typescript/storefront-windows-chrome.jsonl).

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
