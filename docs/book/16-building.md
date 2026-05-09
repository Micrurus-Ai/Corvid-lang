# Building and targets

## The four targets

```sh
corvid build src/main.cor                       # default: native
corvid build src/main.cor --target=native
corvid build src/main.cor --target=wasm
corvid build src/main.cor --target=server
corvid build src/main.cor --target=cdylib
```

## Native

A standalone binary you can drop on a server. Cranelift backend.
Cross-compiles via the standard rustc/cargo toolchain.

```sh
corvid build src/main.cor --target=native --optimize=release
ls target/release/main          # the binary
```

## WASM

A WASM module for the browser, edge runtimes, or any wasmtime/
wasmer host. Bare `(ptr, len)` UTF-8 ABI for strings; multi-value
returns for struct returns.

```sh
corvid build src/main.cor --target=wasm
ls target/wasm32-unknown-unknown/release/main.wasm
```

The website's playground uses this target. Host-mediated for filesystem
and network. See **[WASM target](/docs/wasm)**.

## Server

A rendered axum 0.7 backend binary with the full middleware pipeline
(auth, rate-limit, tracing, CORS, compression, request logging),
graceful shutdown, handler-isolation timeout, body-limit, and
`/healthz`/`/readyz`/`/metrics` endpoints. Generated as a Cargo
project; the user owns the project, Corvid owns the shape.

```sh
corvid build src/main.cor --target=server
cd target/server
cargo run                    # compiles to a production-shape binary
```

See **[Backend](/docs/backend)** for routes/middleware/etc.

## cdylib

A C-callable shared library. The signed-build path produces a
DSSE-attested cdylib with embedded `CORVID_ABI_DESCRIPTOR` that
`corvid-abi-verify` cross-checks against the source.

```sh
corvid build src/lib.cor --target=cdylib --sign
ls target/cdylib/libmy_app.{so,dylib,dll}
```

For C consumers the C header is generated with:

```sh
corvid generate c-header src/lib.cor
```

See **[FFI: C/Rust](/docs/ffi-c)**.

## Build flags

| Flag | Purpose |
|---|---|
| `--target=<t>` | native \| wasm \| server \| cdylib |
| `--optimize=<o>` | debug \| release |
| `--sign` | produce a DSSE-signed artifact + receipts |
| `--out-dir=<d>` | override default `target/` |
| `--check-only` | typecheck without codegen (alias: `corvid check`) |
| `--manifest=<f>` | use a non-default `corvid.toml` |

## Reproducible builds

```sh
corvid build src/main.cor --target=native --reproducible
```

Pins the timestamp, embedded paths, and codegen-RNG seed for
byte-stable outputs. Verifies via:

```sh
sha256sum target/release/main
# expect: matches the published hash for this version
```

Phase 22's bundle format ships every reproducible build with a
`MANIFEST.toml` of inputs (source SHAs, dep tree, toolchain).
