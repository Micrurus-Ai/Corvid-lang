# Browser refund approval

This example compiles one Corvid source file to
WASM, loads the generated ES module in a browser, supplies typed host
capabilities, displays the approval boundary, records replay-compatible trace
events from the generated loader, and persists run state through IndexedDB.

## Build

```powershell
cargo run -q -p corvid-cli -- build examples/browser_refund_approval/src/refund_gate.cor --target=wasm
```

Generated artifacts land in `examples/browser_refund_approval/target/wasm/`.

## Run

Serve the demo directory, then open `http://localhost:8000/web/`:

```powershell
python -m http.server 8000 -d examples/browser_refund_approval
```

The page imports `../target/wasm/refund_gate.js`, which loads
`refund_gate.wasm` beside it. The host object implements:

- `prompts.refund_score(amount): bigint`
- `approvals.IssueRefund(amount): boolean`
- `tools.issue_refund(amount): bigint`
- `store.get` / `store.put` through the generated IndexedDB helper

The trace panel shows `schema_header`, `run_started`, `llm_call/result`,
`approval_request/decision/response`, `tool_call/result`, and `run_completed`
events produced by the generated loader. The status line shows the persisted
run count and last result after a reload.

## Verify

```powershell
examples/browser_refund_approval/verify.ps1
```

On Unix-like shells:

```sh
examples/browser_refund_approval/verify.sh
```
