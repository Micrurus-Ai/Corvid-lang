# Batch scoring from the command line

This comparison measures how long a caller waits for a fresh command-line process to return a result. Each implementation generates three integer features per record, computes a weighted score, and sums the scores that reach the same threshold. The input is deterministic and fits exactly in all four number representations.

The zero-record case measures launch, argument parsing, printing and exit. The million-record case includes that same overhead plus the scoring loop. A persistent server or a program waiting for a model response has a different cost profile.

## Reproduce

Install Corvid with native linking support, Python, Node.js and Rust. From the repository root run:

```sh
python benches/process_latency/run.py /path/to/corvid --output session.json
```

The runner builds Corvid with `corvid build --target native` and Rust with `rustc -O` before timing. Python runs under CPython and JavaScript under Node.js with their default settings. Each measured sample starts a new process. Node's JIT warmup inside that process counts toward its time. Each language gets three discarded runs and 30 measured runs per input, interleaved in the order recorded in the report. Each run must return the checksum calculated from the scoring function's 257-record period.

The report includes the binary version, host, raw nanosecond samples, and ratios of median elapsed times. A Corvid-to-language ratio below 1 means Corvid finished sooner in that case. The interval uses 10,000 paired bootstrap resamples of the same-session trials. Repeat on your target hardware before using the ratios to make a deployment decision.
