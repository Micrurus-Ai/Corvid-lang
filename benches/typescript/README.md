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

The default browser benchmark compares the packaged Corvid pricing handlers forced to Wasm with TypeScript compiled to ES2020. Use the four-way comparison below to include the compiler's automatic JavaScript choice and quantity bindings. Both use the storefront's generated DOM controls, HTML, styles, prices and checkout endpoint. Each visit appends a single `2` to a notebook quantity of `1`, waits for the visible line total of 5,400 cents, and checks out for 5,525 cents with one pencil.

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

For the Wasm preload experiment, add `--compare-preload --pairs 42`. A third copy of the Corvid package includes a fetch preload for the content-hashed Wasm URL at the start of the cart content. The generated JavaScript and Wasm are identical across both Corvid variants. All six visit orders repeat seven times. The runner checks for exactly one resource timing entry per expected asset on each visit. The entries record the initiating element or API, request start, response end and transferred bytes.

Use `--compare-handlers --pairs 24` for the four-way compiler comparison. The variants are the generated loader forced to Wasm, the same loader with automatic JavaScript, the packaged quantity-specific entrypoint, and the TypeScript reference with the general DOM bindings. Each four-round block balances visit order. Repeat with `--cpu-slowdown 4` for Chrome's simulated slower CPU. Each visit checks the expected resources, line total and checkout receipt.
