# Performance

The application benchmark measures costs a developer encounters when starting the CLI, compiling a program, manipulating collections, serving a request, and streaming model output. Earlier workflow comparisons measured different programs and execution tiers. Their raw samples remain available in the [benchmark archives](../../benches/results).

For an ordinary program, measure the execution tier and input sizes the application uses. A CLI startup sample includes process launch and exit. A request sample uses an already-running server. For model calls, use a local provider to measure Corvid's overhead and production telemetry to measure provider latency.

## Budgets are real

`@budget($X)` is a compile-time ceiling. The compiler computes the
worst-case composed cost; the binary refuses to ship if it exceeds X.
Use this to keep production agents from drifting upward in cost as
they accrete features.

## Concurrent jobs

The durable runner (`corvid jobs run --source <path>.cor --workers=N`) scales to N async
workers per process. Past a few thousand jobs/second per host, scale
horizontally — multiple runner processes against one queue.

## Measuring an application release

Build the CLI and runtime with the workspace release profile. From the repository root, run the benchmark with Python 3.11 or later.

```sh
cargo build --release -p corvid-runtime
cargo build --release -p corvid-cli
python benches/analysis/application.py target/release/corvid --output baseline.json
```

The two Cargo invocations mirror release packaging. Building the standalone runtime first keeps the CLI's optional Python dependencies out of the static library used by the native linker. On Windows, use `target/release/corvid.exe`. The native compilation workload requires the platform C linker and the matching Corvid runtime static library. The harness creates its projects in a temporary directory. It warms each workload three times, then saves 15 wall-clock samples, their median in milliseconds, and the CLI binary's SHA-256.

| Measurement | Work inside the timer |
|---|---|
| CLI startup | Start a new process, print the version, and exit, with the filesystem cache warm. |
| Frontend | Start the CLI and check a file containing 100 functions and one agent. |
| Native compilation | Compile and link the same file with a fresh output executable. The harness also runs it and checks the result outside the timer. |
| Interpreter collections | Start the interpreter, build a 16,384-element list, take 1,024 four-element slices, and sum their first elements. |
| HTTP request | Open a loopback connection to an already-running Corvid server, request a two-byte response, and read it completely. |
| Model streaming | Read 16 fixed chunks from a local model stub, both directly and through a Corvid streaming route, alternating the order for each pair. The median paired difference estimates the additional end-to-end overhead of this local route. Both measurements include client-side reading and validation. |

To compare a change, run the same command on the same machine with `--baseline baseline.json --output candidate.json`. Choose one group with `--workload collections`, `startup`, `compiler`, or `service`. A workload flags a regression if its median rises by more than 30%, clears its absolute noise floor, and exceeds the reference median by three robust standard deviations. The harness estimates that spread as 1.4826 times the median absolute deviation. The floors are 3 ms for startup, 5 ms for checking and collections, 50 ms for native building, 0.5 ms for plain HTTP and the direct provider, and 1 ms for the Corvid stream. Repeat a flagged result with the same toolchain and machine load before attributing it to a code change.

### Windows x64 baseline

On the Core Ultra 7 155U test machine, the release CLI started and printed its version in 26.3 ms. Checking the 100-function program took 29.9 ms, and compiling and linking it took 380.9 ms. The interpreter's list-slicing workload took 42.2 ms. Each networking sample is the mean time per request in a batch of 16. The median batch measured 5.0 ms per plain HTTP request and 7.1 ms per completed Corvid stream. The median paired difference from the local provider was 3.7 ms. A second run with the same binary measured 11.5 ms per Corvid stream and 7.7 ms paired overhead. Use these figures as a local reference. [Raw samples and binary identifiers](../../benches/analysis/baselines/windows-x64.json)

In a separate development-build comparison on the same machine, copying only the requested slice reduced the collection workload median from 768.7 ms to 49.2 ms across nine interleaved pairs. The program returned the same sum in every run.

## Comparing workflow archives

`corvid bench compare python` and `corvid bench compare typescript` read the existing workflow comparisons. Use `--session` to select an archive. The benchmark protocol and raw measurements are in [benches](../../benches/README.md).
