# Offline refund bot

A self-contained Corvid program with mocked tools and a fake LLM, so you
can run it end-to-end without an API key.

## Run natively (no Python required)

```bash
cargo run -p offline_refund_bot
```

Expected output:

```
refund_bot decided: should_refund=true reason="user reported legitimate complaint"
trace written under examples/offline_refund_bot/target/trace
```

The runner registers mock `get_order` / `issue_refund` tools, an
always-yes approver, and a `MockAdapter` returning a canned decision —
all in Rust, all through `corvid-runtime`.

## Run via Python (legacy, still works)

```bash
cd examples/offline_refund_bot
corvid build src/refund_bot.cor
python3 tools.py
```

Same agent, dispatched through the Python codegen path (`--target=python`).

## What's going on

- `src/refund_bot.cor` — the Corvid source. The killer feature lives here:
  delete the `approve IssueRefund(...)` line and `corvid check` refuses
  to compile.
- `runner/main.rs` — the native runner: builds a `Runtime`, registers
  tools and the mock LLM, and calls into `corvid-driver::run_with_runtime`.
- `tools.py` — Python equivalent for the legacy `--target=python` flow.
- `target/trace/*.jsonl` — JSONL trace events written by the native run.
