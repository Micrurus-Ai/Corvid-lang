# TypeScript benchmark runner

Node-based benchmark runner for the shared AI-workflow fixtures.

- runtime style: direct async/await
- no orchestration framework dependency
- emits one JSON object per trial

Run with:

```powershell
./benches/typescript/run.ps1 -Fixture tool_loop -Trials 3
```

## Storefront interaction

The browser benchmark compares the packaged Corvid pricing handlers with TypeScript compiled to ES2020. Both use the storefront's generated DOM controls, HTML, styles, prices and checkout endpoint. Each visit appends a single `2` to a notebook quantity of `1`, waits for the visible line total of 5,400 cents, and checks out for 5,525 cents with one pencil.

From the repository root, with Node, Python Playwright, Chrome and a built Corvid CLI available, run:

```powershell
npm ci --prefix benches/typescript --ignore-scripts
python scripts/benchmark_storefront.py target/debug/corvid.exe --output storefront.jsonl
```

The default is 40 paired rounds after one discarded warm-up pair. Each cold visit starts with a fresh browser context. The warm visit returns from checkout in the same tab. Both use a running browser and loopback server. Corvid and TypeScript alternate going first. Every visit records three times in milliseconds.

- **Ready** measures navigation start to the initial line total becoming visible and ready to respond.
- **DOM update** measures the trusted keydown timestamp to observation of the correct output value.
- **Frame estimate** measures the same keydown to a task posted during the next animation frame after that update. This approximates a rendering opportunity. Physical display latency requires separate measurement.

The JSON Lines file starts with environment and build metadata, followed by one record per edit, including the browser's reported JavaScript and Wasm transfer sizes. The runner prints medians and nearest-rank 95th percentiles. The [performance guide](../../docs/guides/performance.md#storefront-quantity-edit) records the Windows run.
