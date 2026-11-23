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

### Startup profile

A second 160-edit run recorded each browser asset's request start and response end. Cold readiness medians were 57.3 ms for Corvid and 43.8 ms for TypeScript. Both reached the DOM update in 1.65 ms on cold visits and 1.20 ms on warm visits.

The cold JavaScript request took about 7.5 ms for either implementation. After that response ended, Corvid took a median 34.85 ms to become ready and TypeScript took 22.80 ms. Within Corvid's interval, Chrome started the Wasm request a median 21.30 ms after the JavaScript response ended. Fetching Wasm took 2.00 ms. The final interval from Wasm response end to cart readiness was 11.65 ms and includes browser scheduling, Wasm startup and control setup. Each median was computed separately. The measurements identify the serial request as a candidate for an early-fetch experiment. The experiment below measures the effect of overlapping these requests. [Startup samples](../../benches/typescript/storefront-startup-windows-chrome.jsonl)

### Early Wasm fetch experiment

Chrome 157 on Windows completed 252 checked edits across 42 rounds, with a cold and a warm visit per implementation. Adding a cart preload moved the median cold Wasm request start from 43.55 ms to 14.20 ms after navigation. Chrome recorded one Wasm resource per Corvid visit. Every preloaded cold visit transferred 846 bytes for Wasm and every warm visit reused it with zero transferred bytes.

| Cart | Cold readiness median / p95 (ms) | Warm readiness median / p95 (ms) |
| --- | ---: | ---: |
| Corvid | 56.95 / 65.90 | 29.30 / 34.90 |
| Corvid with preload | 57.15 / 62.20 | 30.40 / 34.50 |
| TypeScript | 45.75 / 56.50 | 25.25 / 28.30 |

The preload achieved overlap without improving median readiness on loopback. It remains a benchmark variant. Slower networks could give a different result. A follow-up profile can time module evaluation, Wasm instantiation and control setup independently. [Raw observations](../../benches/typescript/storefront-preload-windows-chrome.jsonl)

### Handler startup profile

In a separate 160-edit run on the same Windows Chrome setup, timestamp probes in temporary loader copies measured these median cold-visit intervals. Each implementation had 40 cold and 40 warm visits.

| Interval (ms) | Corvid | TypeScript |
| --- | ---: | ---: |
| Loader response end to first module statement | 22.25 | 22.45 |
| Module entry to beginning handler initialization | 2.00 | 2.00 |
| Begin initialization to handlers available | 13.50 | 0.90 |
| Handlers available to quantity controls bound | 2.75 | 1.40 |
| Navigation start to observed cart readiness | 68.75 | 54.40 |

Corvid's streaming Wasm startup took 13.30 ms including the fetch. A median 8.70 ms elapsed between the Wasm response ending and the instance becoming available. This elapsed interval includes browser scheduling. On warm visits, handler initialization took 6.20 ms for Corvid and 0.70 ms for TypeScript. The measurement supports testing a compiler-generated JavaScript path for small, pure integer handlers. The prototype should preserve signed 64-bit behavior, diagnostics and tracing and be compared with the existing Wasm backend using the same inputs. Repeat the profile with `--profile-stages`. [Raw samples](../../benches/typescript/storefront-stages-windows-chrome.jsonl)

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
