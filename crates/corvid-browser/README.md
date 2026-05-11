# corvid-browser

WASM-targeting typechecker entry point for the Corvid playground
(slice 33J7 at <https://corvid-lang.org/playground>).

This crate exposes one function — `check(source: &str) -> CheckResult` —
that runs the Corvid typecheck pipeline (lex → parse → resolve →
typecheck) and returns a flat-wire-format result that the browser
playground renders inline.

## What ships

- A `cdylib` library compilable to `wasm32-unknown-unknown` with no
  runtime dependencies (no tokio, no filesystem, no network).
- A flat diagnostic schema (`Diagnostic` + `BrowserSpan`) keyed by
  `guarantee_id` so the playground can link each compile-time refusal
  to its docs page at `/docs/reference/guarantees#<id>`.
- An `rlib` target so native Rust tests can validate the pipeline
  end-to-end without a WASM toolchain.
- A `wasm-bindgen` entry exposed under `js_name = check` for browser
  JS consumption.

## What this crate does NOT do

- **Runtime execution.** No `corvid run`, no agent execution.
  Typecheck only.
- **LLM provider calls.** The playground's user-supplied API key path
  runs directly from the browser to the provider; this crate never
  sees a key.
- **Connector OAuth flows** (Gmail, Slack, MS365). Out of scope.
- **Multi-file resolution.** Imports refuse with the
  documented browser-only message. v1 of the playground is single-
  file by design.
- **Code generation.** No Cranelift, no Python codegen, no WASM
  *output* target (that's `corvid-codegen-wasm`, the wrong
  direction).

## Wire schema (v1)

```ts
interface CheckResult {
  version: "v1";
  ok: boolean;             // true iff zero `severity: "error"` diagnostics
  diagnostics: Diagnostic[];
}

interface Diagnostic {
  guarantee_id: string | null;       // e.g. "approval.dangerous_call_requires_token"
  severity: "error" | "warning" | "info";
  message: string;                   // primary message, single line
  span: BrowserSpan;
  help: string | null;               // optional fix hint
}

interface BrowserSpan {
  start_line: number;                // 1-indexed
  start_col: number;                 // 1-indexed, counts Unicode chars
  end_line: number;
  end_col: number;
}
```

The schema is intentionally flat. Multi-span and related-info fields
can be added later as additive `Option<Vec<...>>` fields — older
renderers safely ignore unknown fields. The `version` field signals
non-additive changes.

## Building

```sh
# Native check (fast feedback during development):
cargo build -p corvid-browser
cargo test  -p corvid-browser --tests

# WASM artifact for the playground:
cargo build -p corvid-browser --target wasm32-unknown-unknown --release

# Output: target/wasm32-unknown-unknown/release/corvid_browser.wasm
```

Postprocess with `wasm-bindgen` for browser consumption:

```sh
wasm-bindgen target/wasm32-unknown-unknown/release/corvid_browser.wasm \
             --out-dir <website>/public/playground \
             --target web
```

## Size budget

The brief calls for ≤8 MB gzipped on the wire. As of writing:

- Raw wasm: ~1.2 MB
- Gzipped: ~300-400 KB (well under budget)

The CI step `browser-typechecker-wasm` in `.github/workflows/ci.yml`
enforces the budget on every push.

## Integration with the website

The Corvid website at `Micrurus-Ai/corvid-website` consumes the
artifact via:

1. A CI step that clones this repo and runs the WASM build.
2. A `repository_dispatch` event (`corvid-lang-wasm-changed`) on
   every push to `main` here, picked up by the website's CI.
3. A renderer that calls `check(source)` from JavaScript, parses the
   `CheckResult` JSON, and:
   - Renders inline squiggles using `span.start_line` / `start_col` /
     etc.
   - Renders each `guarantee_id` as a clickable badge linking to
     `/docs/reference/guarantees#<id>`.
   - Surfaces `help` text as a hover popup.

## Schema-change protocol

Once the website consumes the wire format, schema changes are
coordinated:

- **Additive changes** (new optional fields): land here, ping the
  website team. No rollout coordination needed; older renderers
  ignore unknown fields.
- **Non-additive changes** (renamed / removed fields, changed
  semantics): bump `SCHEMA_VERSION`, open a coordinated PR pair (one
  here, one on the website), land in lockstep.

## Provenance

- Filed by the website team on 2026-05-11 in
  [`docs/meta/remaining-slices-handoff.md`](../../docs/meta/remaining-slices-handoff.md)
  as slice 33J7-prereq.
- Pipeline mirrors `crates/corvid-driver/src/pipeline/compile.rs`
  steps 1–4. Steps 5 (lower) and 6 (codegen) are intentionally
  excluded.
- Diagnostic conversions mirror `crates/corvid-driver/src/diagnostic.rs`
  but lifted into this crate so the driver's wasm-incompatible deps
  (tokio, hyper through codegen targets) stay out of the playground
  artifact.
