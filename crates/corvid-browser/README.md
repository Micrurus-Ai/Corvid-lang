# corvid-browser

WASM-targeting typechecker entry point for the Corvid playground
(slice 33J7 at <https://corvid-lang.org/playground>).

This crate exposes two functions:

- **`check(source: &str) -> CheckResult`** — single-file typecheck
  (slice 33J7-prereq).
- **`check_project(files: &HashMap<String, String>, entry: &str) ->
  CheckResult`** — multi-file typecheck (slice 33J7a). Resolves
  imports through the in-memory file map instead of the
  filesystem, so the playground's multi-tab editor can typecheck a
  real project shape.

Both run the same Corvid pipeline (lex → parse → resolve →
typecheck) and return a flat-wire-format result that the browser
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
  path: string | null;               // source file (multi-file only; null for `check`)
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

The `path` field is `null` for single-file `check()` results
(the diagnostic is unambiguously about the one source) and `Some`
for multi-file `check_project()` results, where the playground
must route the squiggle to the right editor tab. The field was
added in the 33J7a slice; older renderers safely ignore it.

## Multi-file API (slice 33J7a)

```ts
// JS-side signature exposed via wasm-bindgen as `checkProject`:
function checkProject(
  files: { [path: string]: string },
  entry: string
): CheckResult;
```

The `files` argument maps path-keyed strings (e.g. `"src/main.cor"`)
to source. The `entry` argument names the file to typecheck against;
imports inside it are resolved against the same map.

Resolution semantics:

- Only `import "./..." as alias` (`ImportSource::Corvid`) imports
  load. Python (`import python "..."`), remote
  (`import "https://..."`), and package (`import "corvid://..."`)
  imports refuse with a playground-sandbox diagnostic.
- Paths normalize to web-style canonical form (`./` segments
  dropped, `..` resolved, `/` separator, `.cor` extension implicit).
  So `import "./policy"` from `src/main.cor` looks up
  `src/policy.cor` in the file map.
- Cycles surface as a single diagnostic at the import that closes
  the back-edge.
- Module not found → diagnostic anchored at the import site in
  the importing file.
- A file with a parse error in it still contributes its parse
  errors to `diagnostics`, anchored to its own path.

Cross-file moat property: a dangerous tool defined in module A,
called from module B, still requires an `approve` token at the
call site. The compile-time guarantee `approval.dangerous_call_
requires_token` fires across file boundaries the same way it
fires within one file.

## Examples API (slice 33J7-playground)

The playground's examples picker + terminal panel call two more
entries. Both are tier 1 (typecheck / analyze) — they ship today
on the existing `check` pipeline. Tier 2 (`runExample` — actual
agent execution) lands when the wasm-clean runtime exists; see
[`docs/meta/playground-examples-contract.md`](../../docs/meta/playground-examples-contract.md).

```ts
// JS-side signatures exposed via wasm-bindgen:
function listExamples(): ExampleCatalog;
function checkExample(name: string): CheckResult;

interface ExampleCatalog {
  version: "v1";              // versions independently of CheckResult
  examples: ExampleMeta[];
}

interface ExampleMeta {
  name: string;               // stable kebab-case id, e.g. "approve-gates"
  title: string;              // "Approve Before Dangerous"
  category: string;           // "Safety at compile time" — picker groups by this
  pitch: string;              // one-paragraph why-this-matters
  source: string;             // the baked .cor program
  spec_path: string;          // docs link, e.g. "docs/internals/effect-spec/03-typing-rules.md"
  non_scope: string;          // what the demo deliberately does not prove
  tier: number;               // 1 = typecheck-demo (today), 2 = needs execution
}
```

The catalog is the `corvid tour` topic set — one source of truth,
shared with the native CLI via the wasm-clean `corvid-tour-catalog`
crate. `checkExample(name)` is a thin wrapper over `check` using
the topic's baked `source`; an unknown name fails closed with a
`CheckResult { ok: false }` carrying one error diagnostic. The
approve-refusal demo edits `ExampleMeta.source` in-place and routes
the edited text through `check` directly, not back through
`checkExample`.

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
# Pin wasm-bindgen-cli to the same version Cargo.lock pins for the
# `wasm-bindgen` crate. A default `cargo install -f wasm-bindgen-cli`
# may pull a newer version that errors with a schema-version
# mismatch against the .wasm artifact. Read the version out of
# Cargo.lock and install the matching CLI:
WASM_BINDGEN_VERSION=$(grep -A1 'name = "wasm-bindgen"' Cargo.lock \
    | head -2 | grep version | sed -E 's/.*"([^"]+)".*/\1/')
cargo install -f wasm-bindgen-cli --version "$WASM_BINDGEN_VERSION"

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
