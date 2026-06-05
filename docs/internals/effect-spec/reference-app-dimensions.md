# Reference-app dimension values catalog

> Companion to [`04-builtin-dimensions.md`](./04-builtin-dimensions.md).
> Lists every `trust:`/`data:`/etc value that appears in any shipped
> reference app under `examples/backend/`. The catalog is the source
> of truth for the drift gate at
> `crates/corvid-types/tests/reference_app_dimensions_gate.rs` — when
> a reference app introduces a new value, the test fails until the
> value is documented here.
>
> **Why this exists.** Slice `33Q7a` (maintainer-as-reviewer-2026-06-05
> P1.2) caught that the v1.0 typechecker accepts any string as a
> `trust:`/`data:` value via `DimensionValue::Name(String)` —
> reference apps use 8+ distinct trust values where the spec's
> stated lattice has only 3, and the typechecker doesn't reject
> the non-canonical ones. The spec sections 4.2 and 4.4 now name
> this honestly; this file catalogs what's actually in use so a
> trial reviewer can recognize a non-canonical value as
> "domain extension shipped in the reference apps" rather than
> "secret spec extension I missed."
>
> The post-v1.0 slice `33Q7b` tightens the typechecker to require
> `corvid.toml`-declared custom dimensions for non-canonical values.
> When `33Q7b` ships, the reference apps either move to canonical
> values OR declare their domain extensions explicitly, and this
> file becomes purely historical.

---

## Trust dimension

### Canonical (in spec)

- `autonomous`
- `supervisor_required`
- `human_required`
- `autonomous_if_confident(<threshold>)` (confidence-gated)

### Reference-app extensions (NOT in spec lattice, accepted as `Name(String)`)

| Value | Used by | Intended semantics (annotation only) |
|-------|---------|---------------------------------------|
| `bounded` | All 5 reference apps in various `support_ai`/`policy_search`/`ingest`/etc effects | "Trust is bounded by the agent's declared @budget and effect row" — annotation that the action is non-dangerous |
| `grounded` | Reference apps using grounded provenance | "Trust derived from `Grounded<T>` provenance — the LLM output is anchored to a citable source" |
| `local` | Reference apps using local-only operations | "Trust does not require network egress" |
| `readonly` | Reference apps using read-only DB / read-only connector calls | "Trust is read-only; no mutation" |
| `workspace` | Reference apps using workspace-scoped operations | "Trust is bounded by the workspace authentication context" |

### Spec values NOT currently used by reference apps

- `autonomous`
- `supervisor_required`

---

## Data dimension

### Canonical (in spec §4.4)

- `none`
- `public`
- `pii`
- `financial`
- `medical`
- `grounded`

### Reference-app extensions

| Value | Used by | Intended semantics (annotation only) |
|-------|---------|---------------------------------------|
| `code` | Code Maintenance Agent | "Source code; sensitivity depends on repo visibility" |
| `customer` | Customer Support Agent + others | "Customer-facing data; analogous to `pii` but more specific to a tenant context" |
| `external` | Reference apps using outbound integrations | "Data flowing to external systems; equivalent to a write-side `external_io` classification" |
| `internal` | Reference apps using internal-only data | "Tenant-internal data; not shared across tenants" |
| `private` | Personal Knowledge Agent + others | "User-private data; analogous to `pii` for personal-knowledge use cases" |

### Spec values currently used by reference apps

- `financial` (only one)

---

## How the drift gate uses this catalog

The Rust test at
`crates/corvid-types/tests/reference_app_dimensions_gate.rs`:

1. Walks every `examples/backend/*/src/main.cor`.
2. Parses `trust:` / `data:` declarations.
3. Asserts every distinct value is in one of the two sets per
   dimension (canonical OR reference-app-extension) defined in
   constants at the top of this file's Markdown — the Rust test
   has the same lists hardcoded.

When the lists drift (a new reference app introduces a new value,
OR the spec adds a new canonical value, OR a reference app stops
using one of the listed extensions), the test fails with a
diagnostic naming the drifted value and the file that introduced
it. Resolving the drift means either updating this catalog +
the matching Rust constants OR rewriting the reference app to
use a documented value.

This is intentionally a soft gate: it catches drift but doesn't
reject the build. The post-v1.0 slice `33Q7b` promotes this to
hard typechecker enforcement.
