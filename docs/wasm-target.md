# Corvid WASM Target

Phase 23 begins with a real deployable foundation rather than a placeholder.
`corvid build --target=wasm <file>` emits four artifacts under `target/wasm/`:

- `<name>.wasm` - a valid WebAssembly module for scalar runtime-free agents.
- `<name>.js` - an ES module loader using `WebAssembly.instantiateStreaming`.
- `<name>.d.ts` - TypeScript declarations for the exported agents.
- `<name>.corvid-wasm.json` - a manifest describing exports and the current host ABI boundary.

## Current Boundary

Agents whose parameters and return values are `Int`, `Float`, `Bool`,
`Nothing`, or `String` cross the WASM boundary cleanly. Agent-to-agent
calls and scalar arithmetic lower into the module directly. The
`String` agent boundary shipped in Phase 20n-B (slice L-4); it is
documented in detail under [String ABI](#string-abi) below.

Scalar prompts, tools, and approvals lower to typed imports from the
`corvid:host` module:

- `prompt.<name>` for prompt calls.
- `tool.<name>` for tool calls.
- `approve.<Label>` for approval gates.

The generated JS loader exposes `adaptImports(host)` so browser and edge hosts
can provide `{ prompts, tools, approvals }` maps without writing raw
`WebAssembly.Imports` objects by hand. It also exports
`createIndexedDbStoreHost(options)`, a typed `store.get` / `store.put` /
`store.delete` host helper backed by IndexedDB for browser-side durable state.

`instantiate(host, { trace })` records Phase 21-style trace events while the
WASM module runs. `trace` may be an array, a callback, or an object with an
`events` array. The loader emits schema-v2 `schema_header`, `run_started`,
`llm_call/result`, `tool_call/result`, `approval_request/decision/response`,
and `run_completed` events for scalar host imports. BigInt values are converted
to JSON-safe numbers when possible, or strings when they exceed JavaScript's
safe integer range.

Unsupported AI-native features still fail loudly. Structs, provenance
handles, stream callbacks, direct in-WASM asynchronous store calls, and
`String` parameters or returns on host imports (only the agent
boundary supports `String` in v1) require later host ABI slices.
Browser and edge deployment must preserve Corvid's effect, approval,
provenance, budget, and replay contracts instead of compiling those
features away.

## String ABI

Phase 20n-B ships a real WASM-side string ABI: `String` parameters and
returns on agents cross the boundary as UTF-8 byte spans in linear
memory, addressed by `(ptr, len)` pairs.

### Memory layout

Every emitted module exports a single linear memory and a small
heap allocator. Layout, lowest address first:

```
[ 0 ..  8)        null-pointer sentinel          (corvid_alloc never returns 0..8)
[ 8 .. 8+P)       compile-time string-literal pool   (P = sum of unique literal byte lengths)
[ 8+P .. )        runtime heap, managed by corvid_alloc / corvid_free
```

The literal pool is emitted as a single active `DataSection` segment
with offset `8`. The runtime heap starts immediately past the pool,
so literal addresses and runtime allocations never alias.

### Exports added to every module

The compiler always emits these three exports, even for modules that
don't use `String`:

| Export | Type | Purpose |
|---|---|---|
| `memory` | linear memory | Raw byte access for the JS loader. |
| `corvid_alloc` | `(size: i32) -> i32` | Allocate a payload of at least `size` bytes; returns the payload pointer, or `0` on out-of-memory. |
| `corvid_free` | `(ptr: i32, size: i32) -> ()` | Return a previously-`alloc`'d block to the free list. `size` is accepted for ABI compatibility; the allocator reads the actual size from the per-block 4-byte header. |

The allocator is a real free-list implementation with adjacent-block
coalescing on free. Repeated alloc/free cycles reuse memory; the
allocator integration tests at
`crates/corvid-codegen-wasm/tests/allocator.rs` verify that 1000
churn iterations stay at the initial 1-page memory footprint.

### Calling convention

For each agent parameter:

- `Int` lowers to a single `i64`.
- `Float` lowers to a single `f64`.
- `Bool` lowers to a single `i32`.
- `Nothing` is represented by zero `WasmType`s in the function
  signature.
- `String` lowers to a pair of `i32`s: `(<name>_ptr, <name>_len)`.
  Both are absolute byte offsets into the module's linear memory.

For an agent return:

- `Int`, `Float`, `Bool` lower to single-value `i64` / `f64` / `i32`.
- `Nothing` lowers to a zero-result function type.
- `String` lowers to a multi-value `(result i32 i32)` carrying the
  return's `(ptr, len)`.

Multi-value returns are part of WebAssembly's stage-4 specification
and are supported by every modern engine; no feature flags required.

### Ownership

Phase 20n-B v1 fixes the ownership convention so the allocator can
support repeated calls without leaks:

- The host (the JS loader, or a Rust caller via wasmtime) **allocates
  every input `String`** by calling `corvid_alloc` and writing UTF-8
  bytes into the returned slot.
- The host **passes the `(ptr, len)` pair** as the agent's two `i32`
  arguments.
- The agent **returns a `(ptr, len)` pair** that may point to:
  (a) an input span the host allocated (pass-through),
  (b) a literal in the data section, or
  (c) a fresh allocation the agent made internally (future slices).
- The host **decodes the return bytes immediately** via
  `TextDecoder` (or equivalent), producing a host-owned string copy.
- The host **frees only the inputs it allocated**. The agent's
  return is not freed by the host because it may alias an input or
  a const-memory literal — distinguishing the cases would require
  an extra return field, deferred to a later slice.

The `finally` block in the generated JS loader's wrapper ensures
input frees run even when the agent throws mid-call.

### Generated JS loader for a `String` agent

For:

```corvid
agent shout(msg: String) -> String:
    return msg
```

…the generated `shout.js` exposes:

```js
import { instantiate } from './shout.js';

const module = await instantiate();
const result = await module.shout('hello, world');
// result === 'hello, world'
```

Internally the wrapper:

1. Encodes `msg` via `TextEncoder` to a `Uint8Array`.
2. Calls `exports.corvid_alloc(bytes.length)` to reserve linear memory.
3. Writes the encoded bytes via `new Uint8Array(memory.buffer, ptr, len).set(bytes)`.
4. Calls `exports.shout(ptr, len)` and destructures the multi-value
   return as `[resultPtr, resultLen]`.
5. Decodes the return bytes via `TextDecoder` into a JS string.
6. Calls `exports.corvid_free(ptr, len)` in a `finally` block.

The TypeScript declaration uses idiomatic `string`:

```ts
export interface CorvidWasmModule {
  shout(msg: string): string;
}
```

### String literals

String literals like `return "hello"` are interned at compile time
into the per-module literal pool and lowered to two `i32.const`
instructions producing the literal's `(absolute_addr, byte_len)`
pair. No runtime allocation is performed for literal-only String
returns. Repeated identical literals across multiple agents
de-duplicate to a single pool entry.

### Manifest `kind` discriminator

Every parameter and return in the emitted
`<name>.corvid-wasm.json` manifest carries a `kind` field that
records the WASM-level boundary shape:

- `"i64"` — `Int`
- `"f64"` — `Float`
- `"i32"` — `Bool`
- `"void"` — `Nothing`
- `"string"` — `String` (multi-value `(ptr, len)`)

Downstream tooling (registry consumers, alternative loaders,
analysis tools) should switch on `kind` for ABI shape rather than
parsing the human-readable `ty` field.

### What's not in v1

Filed as out-of-scope for the 20n-B slice and tracked as future
follow-ups:

- **`String` parameters or returns on `corvid:host` imports.** Tools,
  prompts, and approvals stay scalar in v1; the closing-loop here
  requires a richer host-import contract.
- **Struct ABI.** Phase 20n-C ships native-target struct returns;
  the WASM struct ABI is a separate follow-up.
- **`Stream<String>` and other streaming String surfaces.** Streaming
  is its own boundary work.
- **Multi-string return tuples.** Single-`String` returns are
  sufficient for v1; multi-return shapes would need an explicit
  return-record convention.
- **WASM Component Model adapters.** The bare `(ptr, len)` ABI is the
  v1 choice; Component Model adapters can layer on top later
  without changing the underlying convention.

## Example

```corvid
agent add_one(x: Int) -> Int:
    y = x + 1
    return y
```

```text
corvid build src/math.cor --target=wasm
```

The generated TypeScript surface is:

```ts
export interface CorvidWasmModule {
  add_one(x: bigint): bigint;
}

export type CorvidWasmTraceSink =
  | Array<Record<string, unknown>>
  | ((event: Record<string, unknown>) => void)
  | { events: Array<Record<string, unknown>> };

export function instantiate(
  hostOrImports?: WebAssembly.Imports | CorvidWasmHost,
  options?: { trace?: CorvidWasmTraceSink },
): Promise<CorvidWasmModule>;

export function createIndexedDbStoreHost(
  options?: { dbName?: string; storeName?: string },
): Promise<CorvidWasmHost & { close(): void }>;
```

## Browser Demo

The committed browser smoke demo lives in
`examples/wasm_browser_demo`. It compiles `src/refund_gate.cor` to WASM,
loads the generated ES module from `target/wasm/refund_gate.js`, supplies typed
prompt/tool/approval host capabilities, displays the approval decision, and
renders the generated trace events. The page uses the generated
`createIndexedDbStoreHost` helper to persist run count and last result across
reloads through IndexedDB; the Phase 23 browser CI test covers that persistence
path.

```powershell
examples/wasm_browser_demo/verify.ps1
python -m http.server 8000 -d examples/wasm_browser_demo
```

Open `http://localhost:8000/web/` after the build finishes.

## Wasmtime Parity Harness

`cargo test -p corvid-codegen-wasm --test wasmtime_parity` runs generated WASM
under Wasmtime. The harness compares the current WASM-supported scalar parity
subset against the interpreter, then separately exercises typed scalar
prompt/approval/tool imports through a Wasmtime host.

The harness is intentionally fail-loud about the current boundary. Native parity
families that require strings, structs, lists, provenance handles, or streaming
callbacks cannot enter the WASM parity matrix until those ABI slices exist.
That keeps the deployment story honest: Phase 23 proves the browser/edge target
for scalar AI-native host capabilities without pretending the whole native
surface has already crossed the WASM boundary.

## Next Slice

Phase 24 starts the LSP and diagnostics track. WASM-side expansion continues
through the later ABI slices for strings, structs, provenance handles, and
streaming host callbacks.
