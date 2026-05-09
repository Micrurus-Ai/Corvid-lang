# WASM target

## What ships

Corvid compiles to WebAssembly via `corvid build --target=wasm`. The
WASM binary uses a bare `(ptr, len)` UTF-8 ABI for strings, multi-value
returns for struct returns, and a uniform `kind` discriminator for
manifest entries. The runtime ships a real free-list-with-coalescing
allocator (no leaky bump allocator).

## What works in WASM

- Pure agents and prompts (with the LLM call routed through the host).
- Struct returns from prompts (per-struct codegen-emitted decoders).
- Grounded values (provenance metadata serialized through the bridge).
- Effect rows and approve tokens (compile-time, no runtime cost).

## What doesn't work in WASM

- Direct filesystem access (host must mediate).
- Direct database access (host must mediate; use a connector
  abstraction).
- Long-running loops without browser tab activity (browser engines
  may suspend WASM modules).

## The deep doc

[`docs/internals/wasm-abi.md`](https://github.com/Corvid-lang/Corvid-lang/blob/main/docs/internals/wasm-abi.md)
is the spec; Phase 20n-B and Phase 20n-C closing audits document the
ABI history.

## In the playground

The website's WASM playground (slice 33J5) embeds the same WASM
target. Code typed in the browser compiles in the browser; the
typecheck errors you'd see locally show up the same way in the
playground.
