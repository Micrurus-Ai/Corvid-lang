# Playground examples + terminal — wire contract

**Status:** DRAFT. Pre-code artifact for the scope-reduced 33J7
playground. Not yet agreed. This doc is the thing to chat about
before any `corvid-browser` code lands — same role
`runtime-split-design.md` played for 33J7b.

**Filed:** 2026-05-12, after the scope reduction from "full cloud
IDE editor" to "curated examples + a terminal output panel where
users see what runs."

**Audience:** two readers — the website designer (builds the UI
shell) and the Corvid Rust side (builds the `corvid-browser` API
behind it). The doc exists so both can work in parallel against a
stable contract instead of guessing at each other.

---

## The scope, restated

Not a code editor. The playground is:

1. **An examples picker** — the user browses a curated set of
   Corvid programs.
2. **A terminal / output panel** — the user picks one, and the
   panel shows what that program does: its compile-time analysis
   first (tier 1), and eventually its actual run output (tier 2).

The marquee interaction is the moat demo: a user looks at an
example that calls a `dangerous` tool, sees the `approve`
statement, deletes it, and watches the panel show the compiler
*refusing to compile* with `approval.dangerous_call_requires_token`.
Add it back — green again. That is the whole Corvid pitch in one
five-second loop, and it works **today** with the typecheck-only
surface already shipped.

## The core decision: examples ARE `corvid tour` topics

Corvid already has a curated demo corpus:
[`crates/corvid-cli/src/tour.rs`](../../crates/corvid-cli/src/tour.rs)
ships `TOPICS` — 20 `TourTopic` entries, each with:

- `name` — stable kebab-case id (`approve-gates`, `dimensional-effects`, …)
- `title` — human title ("Approve Before Dangerous")
- `category` — grouping ("Safety at compile time")
- `pitch` — one-paragraph why-this-matters
- `spec` — link into `docs/internals/effect-spec/…`
- `roadmap` — which phase shipped it
- `test` — the test file that backs the claim
- `non_scope` — what the demo deliberately does NOT prove
- `source` — the baked `.cor` program, compiles through the normal driver

That is *exactly* an examples corpus. The CLAUDE.md invention-
shipping contract already *requires* every invention to ship a
`corvid tour --topic <name>` demo, so `TOPICS` is the canonical,
test-backed, spec-linked curated set. **Do not build a parallel
examples corpus.** The playground picker renders `TOPICS`; the
terminal panel runs each topic's `source`.

### What this requires: extract the catalog to a shared crate

`tour.rs` lives in `corvid-cli`. `corvid-browser` cannot depend on
`corvid-cli` (wrong direction in the dep graph). To share one
source of truth between the native `corvid tour` command and the
browser playground, the catalog data moves:

- **New crate `corvid-tour-catalog`** — wasm-clean, holds the
  `TourTopic` struct + the `TOPICS` const + `find_topic` /
  `render_tour_list`. Data is pure `&'static str`; trivially
  compiles to `wasm32-unknown-unknown`. No deps beyond `serde`
  for the wire-format derive.
- `corvid-cli` depends on `corvid-tour-catalog`, keeps `cmd_tour`
  (the CLI rendering) but pulls `TOPICS` from the new crate.
- `corvid-browser` depends on `corvid-tour-catalog`, exposes it
  to the playground.

This is one small extraction commit, same shape as the
33J7b runtime-split moves: `git mv` the data, re-export to
preserve `corvid_cli`'s internal paths, validate.

**This extraction needs sign-off before code.** It is the one
load-bearing structural decision in this contract. Everything
else is additive.

## Two tiers

| | Tier 1 — Analyze | Tier 2 — Run |
|---|---|---|
| What the panel shows | Compile-time analysis: diagnostics, effect rows, the approve-refusal demo | Actual agent execution output, streamed |
| Corvid surface needed | `corvid-browser::check()` — **ships today** | wasm-clean runtime execution — **needs 33J7b (3f–3h) + 33J7c + 33J7d** |
| LLM calls | none — pure typecheck | replayed by default (baked trace), real via BYO key (optional toggle) |
| When | now | weeks out |

Tier 1 is the whole marquee demo and is shippable immediately.
Tier 2 is the "see it actually run" experience and waits on the
runtime + vm split finishing. Ship tier 1, design tier 2's panel
to be forward-compatible, do not block tier 1 on tier 2.

## `corvid-browser` API surface

### Tier 1 — available after the catalog extraction

```rust
/// The curated examples the playground picker renders.
pub fn list_examples() -> ExampleCatalog;

/// Typecheck one example by name. Thin wrapper over `check()`
/// using the topic's baked `source`. Returns the existing
/// `CheckResult` wire format unchanged.
pub fn check_example(name: &str) -> CheckResult;
```

The website never needs the raw `.cor` source to call
`check_example` — but `list_examples()` returns it anyway so the
picker can show the program and the user can edit it in-place
for the approve-refusal demo (a `<textarea>`, not a full editor).
An edited example routes through the existing
`check(source: &str)` entry.

### Tier 2 — sketch only, do not build yet

```rust
/// Run an example. Streams structured events the terminal panel
/// renders. Replayed by default; `live: true` uses a BYO key
/// through the 33J7d suspend/resume bridge.
pub fn run_example(name: &str, opts: RunOptions) -> RunStream;
```

`RunStream` shape is deliberately unspecified in this draft —
it depends on the 33J7d suspend/resume bridge that does not exist
yet. Tier 2 gets its own contract addendum when the runtime
split finishes.

## Wire schemas (all `version: "v1"`)

Same discipline as the existing `CheckResult`: flat, versioned,
additive-changes-don't-bump, non-additive-changes-bump-and-
coordinate.

```ts
interface ExampleCatalog {
  version: "v1";
  examples: ExampleMeta[];
}

interface ExampleMeta {
  name: string;          // stable id, e.g. "approve-gates"
  title: string;         // "Approve Before Dangerous"
  category: string;      // "Safety at compile time" — picker groups by this
  pitch: string;         // one-paragraph why-this-matters
  source: string;        // the baked .cor program
  spec_path: string;     // docs link, e.g. "docs/internals/effect-spec/03-typing-rules.md"
  non_scope: string;     // what the demo deliberately does not prove
  tier: 1 | 2;           // 1 = typecheck-demo-only, 2 = needs execution
}
```

`CheckResult` is unchanged — see
[`crates/corvid-browser/README.md`](../../crates/corvid-browser/README.md)
"Wire schema (v1)". The terminal panel renders it for tier 1:
`ok` drives the green/red state, `diagnostics[]` render as the
output lines, each `guarantee_id` is a clickable badge.

## The marquee demo, spelled out

Example `approve-gates`. Picker shows it under "Safety at compile
time". Terminal panel renders:

1. Initial state — `check_example("approve-gates")` → `ok: true`,
   zero diagnostics. Panel shows a green "compiles ✓" line.
2. User deletes the `approve IssueRefund(id)` line in the
   `<textarea>`.
3. Panel re-runs `check(editedSource)` → `ok: false`, one
   diagnostic with `guarantee_id:
   "approval.dangerous_call_requires_token"`. Panel shows a red
   line + the diagnostic message + a badge linking to
   `/docs/reference/guarantees#approval.dangerous_call_requires_token`.
4. User re-adds the line → green again.

No runtime, no LLM, no API key. Pure typecheck. This is the demo
that sells Corvid and it is buildable the day the catalog
extraction lands.

## What the website designer builds NOW vs waits for

**Build now (independent of the real API — use mocked data
matching the schemas above):**

- Page layout — picker on one side, terminal panel on the other.
- The examples-picker component — grouped by `category`, each
  entry shows `title` + `pitch`.
- The terminal/output panel — renders `CheckResult`: the
  green/red state line, diagnostic lines, `guarantee_id` badges,
  `help` text on hover.
- The editable `<textarea>` for the approve-refusal demo —
  debounced re-check on edit.
- Styling, responsive breakpoints, the whole visual + interaction
  layer.

**Wait for (the real `corvid-browser` API):**

- Final wiring of `list_examples()` / `check_example()` /
  `check()` — swap the mock for the real wasm calls.
- Tier 2 anything — `run_example`, the streaming run panel.

~80% of the designer's work is the shell and is contract-
independent. The mock data this doc's schemas describe is enough
to build all of it. Final wiring is a swap, not a rebuild.

## Open questions — settle before code

1. **Catalog extraction sign-off.** Is `corvid-tour-catalog` as a
   new wasm-clean crate the right call, or should the data land
   in an existing crate? (Recommendation: new crate — single
   responsibility, clean dep arrow, ~one extraction commit.)
2. **Editable examples.** Tier 1's approve-refusal demo needs an
   editable `<textarea>`. Confirmed in scope? (Recommendation:
   yes — it IS the marquee demo, and it rides entirely on the
   existing `check()` entry, zero new Corvid surface.)
3. **Tier-1 launch set.** All 20 tour topics, or a curated
   subset for launch? (Recommendation: ship all 20 — they are
   already test-backed and spec-linked; the picker's `category`
   grouping keeps 20 navigable.)
4. **Category ordering.** `category` is a free-text string today.
   Does the picker need an explicit order, or is alphabetical-
   by-category fine? (Recommendation: add an optional
   `category_order: u8` to `ExampleMeta` if the designer wants
   control; defer until they ask.)

## How to start

**Rust side (me), once open questions 1–2 are settled:**

1. Extract `corvid-tour-catalog` — `git mv` the catalog data,
   re-export to preserve `corvid-cli` internal paths, validate
   gate. One commit.
2. Add `list_examples()` + `check_example()` to `corvid-browser`
   + their `wasm-bindgen` entries. One commit.
3. Add `ExampleCatalog` / `ExampleMeta` to
   `crates/corvid-browser/README.md`'s wire-schema section.
4. Tier 2 (`run_example`) gets its own contract addendum after
   the runtime split finishes — out of scope here.

**Website designer, now:**

1. Read this doc.
2. Build the UI shell against the schemas above with mocked
   data — picker + terminal panel + editable textarea.
3. Final-wire to `corvid-browser` once steps 1–2 land (a swap).

## What's out of scope for this contract

- Tier 2 execution — separate addendum after 33J7b/c/d.
- BYO API key plumbing — rides on tier 2.
- Multi-file examples in the picker — tour topics are single-file
  by construction; `check_project` exists if a future multi-file
  example needs it, but no launch topic needs it.
- A real code editor — the playground is examples + a small
  editable textarea for the one demo, not a Monaco/CodeMirror
  IDE. That was the old scope; this contract is the reduced one.
