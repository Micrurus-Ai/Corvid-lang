# Pre-phase design — Provenance Propagation

**Status:** DECISIONS RECORDED — **revised 2026-05-12 after slice-2
recon** (see "Design correction" below). Slice 0 of the Provenance
Propagation phase. CTO delegated the scope call; this doc is the
contract the multi-tier work executes against. Same role
`runtime-split-design.md` played for 33J7b.

**Filed:** 2026-05-12. **Revised:** 2026-05-12 (slice-2 recon
corrected a load-bearing premise — D1, D5, R6, and the slice plan
changed; the original framing is preserved struck-through where it
matters so the correction is auditable).

**Origin:** the `corvid-differential-verify` test
`corpus_scan_includes_deliberate_failure` is red. Investigation
traced it to a real tier-soundness divergence and
`tests/corpus/combined_all.cor` has been byte-identical since Phase
20g, where it was committed as a program that *should* agree across
all four tiers. The CTO call: don't paper over it — close it as an
**invention**, not a fix.

---

## Design correction — the typechecker is grounded-blind (recon, 2026-05-12)

The slice-2 recon pass falsified this doc's original premise before
any code was written. **What the original draft assumed:** the
typechecker sees `Grounded<String>` for a `data: grounded` prompt's
return and accepts `String + Grounded<String>` via the "legacy
compatibility" assignability rule at `types.rs:153-156`.

**What the recon found:** `Type::Grounded` is *only ever constructed
from an explicit `Grounded<T>` type annotation* a user writes by
hand (`checker/expr.rs:536`, `checker/types.rs:261`). There is **no
path** from a `data: grounded` effect to `Type::Grounded` in the
checker. So:

- `audit() -> String uses audit_effect` (effect carries `data:
  grounded`) returns plain `Type::String` to the typechecker. The
  grounding lives **only in the effect row**, never in the type.
- `first + second` in `combined_all.cor` is `String + String` to the
  checker → it typechecks clean (confirmed: differential-verify's
  error is "interpreter run failed", not "typecheck failed" — so
  `typecheck` passed).
- At runtime the interpreter wraps `audit()`'s result in
  `Value::Grounded` *because of the effect* → `eval_arithmetic` sees
  `Value::String + Value::Grounded` → `TypeMismatch`.

The divergence is not "checker permissive, interpreter strict." It
is **the checker and the runtime disagree about whether the value is
grounded at all.** The type system is *unsound* here — it says
`String` where the runtime produces `Grounded<String>`.

**Consequence for the moat.** `@grounded_pure` (D6) needs the
*compiler* to reason about grounded-ness. A grounded-blind checker
can never prove "no laundering." So the real fix is not "teach the
interpreter to tolerate `Value::Grounded`" (that would be Design Y —
the rejected shortcut, since it leaves the moat impossible). The fix
is **Design X: make `data: grounded` a type-system property** — a
prompt/tool/agent whose effect row carries `data: grounded` returns
`Type::Grounded<T>`. CTO call (delegated 2026-05-12): Design X, it
is the only design that fixes the soundness hole *and* enables the
moat. D1, D5, R6 and the slice plan below are revised accordingly.

## The problem, precisely (revised)

`Grounded<T>` is Corvid's provenance-carrying type — a value plus a
`ProvenanceChain` proving where it came from. Today:

1. **Typechecker is grounded-blind for effect-induced grounding.**
   `data: grounded` never produces `Type::Grounded`; only an
   explicit `Grounded<T>` annotation does. The `types.rs:153-156`
   "legacy compatibility: Grounded<T> assignable to T" rule exists
   but only fires for hand-annotated grounded values — it is *not*
   what lets `combined_all.cor` pass (that passes because the
   checker never sees grounding at all).
2. **IR lowering** (`corvid-ir/src/lower.rs:1172-1187`) only emits
   `UnwrapGrounded` for an *explicit* `.unwrap_discarding_sources()`
   call. Implicit `Grounded<T> → T` coercions produce no IR node.
3. **Interpreter** (`corvid-vm/src/interp/expr.rs`) — `eval_arithmetic`
   pattern-matches concrete `Value::Int/Float/String/List`. A
   `Value::Grounded` falls through to `TypeMismatch`.
4. **Native codegen / replay** — the `UnwrapGrounded` IR node *is*
   handled, but since lowering never emits one for implicit
   coercions, those tiers face the same raw-grounded-operand gap.

The gap: `data: grounded` is a runtime-value fact the type system
cannot see, so the four tiers cannot be made to agree on it — and
the moat cannot be built on a fact the compiler is blind to.

## The reframe: invention, not fix

A fix makes the tiers agree. An **invention** makes grounded values
do something no other language or framework can. The phase ships:

**Provenance Propagation** — `Grounded<T>` is *contagious* through
every operator, *preserved* end-to-end across all four tiers, and
*never silently dropped*. The compiler can prove a boundary launders
nothing.

The principled core: `Grounded<T>` is an **applicative functor** and
provenance-merge is its **monoid**. State that law once and every
pure operation lifts for free — it is one algebra, not a pile of
per-operator special cases.

The pitch: *in Corvid, "did this value come from an AI, how was it
computed, and how confident is it" is a compile-time-decidable
property that survives every operation. You cannot accidentally
launder a model output into trusted data.*

## Scope — what ships, what defers

CTO call (delegated 2026-05-12). Ship the moat with discipline; do
not bundle three inventions into one phase on an already-slipped
launch track.

**Ships in this phase:**
- **Base contagion** — `Grounded<T>` contagious through all
  operators (arithmetic, concat, comparison), all four tiers.
- **`@grounded_pure`** — the compiler-enforced no-laundering
  boundary. The moat.
- **The `Derived` provenance representation** — base contagion uses
  it from day one so how-provenance's *data model* ships with the
  base and no migration is needed later.
- The invention-shipping contract (README / tour / inventions.md /
  spec / tests).

**Defers to follow-up phases (sequencing, not shortcutting — each
ships fully later):**
- **Pillar 1 polish — how-provenance DAG rendering.** The `Derived`
  *data* ships here; the rich `corvid trace dag` rendering of the
  operation tree is a follow-up.
- **Pillar 2 — compile-time confidence budgets.** Runtime confidence
  *composition* ships here (it has to — `GroundedValue` carries a
  confidence field). Compile-time `@confidence(>= X)` enforcement
  *through arithmetic*, and the question of whether `*` deserves a
  composition rule other than `Min`, is its own focused phase.

---

## Decisions

### D1 — Grounding is a type-system property + the contagion law (revised 2026-05-12)

**Decision, part A — `data: grounded` produces `Type::Grounded<T>`
(Design X).** A prompt / tool / agent whose effect row carries
`data: grounded` has its declared return type `T` wrapped to
`Type::Grounded<T>` by the checker. The type system stops being
blind to effect-induced grounding; the checker, interpreter, native
codegen, and replay finally agree on *which values are grounded*.
This is the soundness fix the recon surfaced as mandatory — without
it the contagion law has nothing to operate on and `@grounded_pure`
(D6) is uncheckable.

**Decision, part B — the contagion law (uniform, all operators).**
Any operator application with at least one `Grounded` operand
produces a `Grounded` result. No exceptions:

- `Grounded<T> ⊕ T` → `Grounded<T>`
- `T ⊕ Grounded<T>` → `Grounded<T>`
- `Grounded<T> ⊕ Grounded<T>` → `Grounded<T>`
- comparisons: `Grounded<T> ⊗ T` → `Grounded<Bool>` (and the two
  other operand shapes)

`⊕` ranges over arithmetic (`+ - * / %`) and string/list concat;
`⊗` over comparisons (`== != < <= > >=`). `&&` / `||` are
short-circuit and out of scope (they already route specially).

**Ordering note.** Parts A and B are coupled: shipping A without B
would *break* `combined_all.cor` at the checker — `check_binop`
matches concrete `Type::Int`/`Type::String` arms, so a
`Type::Grounded` operand would fall through to its error arm. B
(contagion in `check_binop` / `check_unop`) must land first (dormant
— nothing produces `Type::Grounded` from effects yet), then A
activates it. Both are `corvid-types` changes; the slice plan lands
them as one slice for that reason.

**Why uniform.** A per-operator carve-out is the pile-of-special-
cases anti-pattern. The applicative-functor framing demands one
law. A stated law that the implementation only partially honors is
exactly the spec/behavior drift the project forbids — so the phase
implements the *complete* law, comparisons included.

### D2 — `Grounded<Bool>` in control-flow conditions

**Decision.** `if` / `while` / `match` conditions accept
`Grounded<Bool>`. The branch decision implicitly unwraps the bool —
this implicit unwrap is **recorded** (the trace shows "branch taken
under grounded condition") and is **IR-visible** (see D5). It does
NOT violate `@grounded_pure`: branching consumes the bool to pick a
path, it does not emit a laundered *value*; the returned value's
provenance still tracks through whichever branch produced it.

**Why.** Contagion that stops at control flow is not contagion.
"This code path was taken because of an AI output" is precisely the
kind of fact provenance should capture. "Powerful, not powerless" —
grounded values stay usable in control flow.

### D3 — The `Derived` provenance representation

**Decision.** Add a fifth `ProvenanceKind` variant:

```rust
ProvenanceKind::Derived {
    op: String,                  // "add", "concat", "lt", ...
    inputs: Vec<ProvenanceChain>, // recursive — a real tree
}
```

When an operator produces a grounded result, the result's
`ProvenanceChain` is a single `ProvenanceEntry` whose kind is
`Derived { op, inputs }`. `inputs` holds the operand chains in
operand order; an ungrounded operand contributes an empty
`ProvenanceChain` (self-describing as "(ungrounded)" to renderers —
no new `Literal` variant needed).

**Mechanical consequence — shipped in slice 1.** `ProvenanceKind`
derived `Eq`; `ProvenanceChain` / `ProvenanceEntry` did not. The
recursive `Vec<ProvenanceChain>` payload needs the whole tree `Eq`.
Slice 1 added `Eq` to the chain types — sound because they carry no
floats (confidence lives on the VM's `GroundedValue`, not core's
`ProvenanceChain`). Also shipped: a `ProvenanceChain::derived(op,
inputs, timestamp_ms)` constructor as the single mint point all four
tiers call. ✅ Done — see slice 1 in the plan.

**Why a tree, not a flat merge.** `ProvenanceChain` already has a
flat `merge()` — it stays, for cases that legitimately accumulate
sources (agent handoffs). But operator results want *how*-provenance:
the operation tree, not just the leaf set. Flat-merging at operator
sites would lose which sources fed which operand — and recovering
that later would be a representation migration. Shipping `Derived`
now means how-provenance's data model is permanent from day one;
only the rich DAG rendering defers.

### D4 — Runtime confidence composition: `Min`, uniform

**Decision.** A grounded operator result's `confidence` =
`Min` over the operands' confidences; an ungrounded operand
contributes `1.0`. Uniform across all operators in this phase.

**Why.** `GroundedValue` already documents confidence as "composed
via Min through the call graph" — this extends the *existing,
shipped* rule to operators rather than inventing a new one. Whether
specific operators (notably `*`, where independent confidences might
*multiply*) deserve a different rule is the deferred Pillar 2 design
question. Using the shipped rule is not a shortcut; refining it is
the deferred pillar's job, and refining it later is non-breaking
(it only tightens runtime confidence values, never types).

### D5 — The legacy `Grounded<T> → T` rule: load-bearing now, keep it, make it visible (revised 2026-05-12)

**Decision.** Keep the `types.rs:153-156` legacy assignability rule.
Under Design X (D1 part A) it is no longer "legacy cruft to retire"
— it is **load-bearing**: when `data: grounded` starts producing
`Type::Grounded<T>`, *every* existing consumer of a grounded
prompt/tool result suddenly sees `Grounded<T>` where it saw `T`. The
legacy rule is what keeps that from breaking every program — it lets
`Grounded<T>` keep flowing into `T` positions (return, args,
bindings, struct fields). Removing it would turn Design X into a
workspace-wide breaking change. It stays.

But it must not stay *invisible*. Make every implicit `Grounded<T>
→ T` coercion **IR-visible**: when the legacy coercion fires, IR
lowering inserts an explicit discard node (reuse `UnwrapGrounded`,
or a dedicated `CoerceGroundedDiscard` — slice 3 decides which is
cleaner). Operator operands do *not* need this — D1's contagion law
covers them (the result stays grounded; nothing is dropped).

Consequence:
- **Normal agents:** implicit coercion still compiles and runs.
  Near-zero blast radius on existing code — the coercion still
  *works*, it is just no longer *invisible*. (Exact blast radius is
  measured in slice 2 — see R6.)
- **`@grounded_pure` agents:** the checker sees the discard node and
  refuses to compile (D6).
- The provenance drop becomes greppable in the IR and visible in
  traces — closing the "silent" part of the gap without a breaking
  removal.

**Why.** This is the powerful-and-safe resolution. Removing the rule
outright (the option the chat rejected as "powerless") would now —
post-Design-X — be a workspace-wide break, not just "ceremony on
existing code." Keeping it invisible (the rejected "shortcut")
leaves silent laundering. Keeping it *visible* gives normal code its
ergonomics back AND gives `@grounded_pure` something concrete to
forbid AND makes every drop auditable. Best of three.

### D6 — `@grounded_pure` — the moat attribute

**Decision.** A new agent attribute, sibling of `@deterministic` /
`@replayable`. `@grounded_pure agent foo(...)` compiles iff the
checker proves foo's body contains **no provenance-stripping site**:
no `UnwrapGrounded` node and no implicit-coercion discard node (D5)
is reachable. It is a reachability check over the IR — the same
proof shape `@deterministic` already uses for "no nondeterministic
source reachable."

Composition with sibling attributes is **orthogonal**: an agent may
be any subset of `@deterministic` / `@replayable` / `@grounded_pure`;
each is an independent proof obligation. `@grounded_pure` says
nothing about determinism and vice versa.

**Why this is the moat.** Base contagion alone is "provenance
propagates" — nice. `@grounded_pure` makes it *load-bearing*: mark a
boundary and the compiler **guarantees no AI output is laundered into
trusted data across it**. That is the `approve`-moat shape — a
compile-time refusal — applied to data flow. It is the most on-thesis
thing in the whole set, and it is cheap once contagion and D5's
visible-discard node exist.

### D7 — `combined_all.cor` becomes idiomatic

**Decision.** Change `tests/corpus/combined_all.cor`'s `main()`
signature from `-> String` to `-> Grounded<String>`. The program
genuinely produces a grounded value (`audit()` is grounded, `+`
propagates per D1); the signature should say so. The corpus shows
*idiomatic* Corvid. The legacy-coercion-discard path (D5) gets its
own dedicated fixture rather than being demonstrated ambiguously by
`combined_all.cor`.

### D8 — Effect rows unchanged

**Decision.** Contagion is a **type-and-value-level** property. It
does not alter effect rows. `data: grounded` on an effect is what
*originates* a `Grounded<T>` value; operator contagion *propagates*
it downstream. The agent's effect profile — what the differential-
verify tool compares across tiers — is unchanged by this phase. The
tiers must agree on *types and values*, which is what D1–D6 deliver.

---

## Risk mitigations

- **R1 — native codegen is the hardest tier.** Grounded values
  through the C ABI. *Mitigation:* the `corvid-differential-verify`
  4-tier check IS the mitigation — if native diverges from
  interpreter, the tool catches it. Slice 5 is gated by re-running
  the differential verifier; native ships only when it agrees with
  interpreter byte-for-byte.
- **R2 — recursive `Derived` chains grow with expression depth.**
  *Mitigation:* depth is bounded by source expression nesting — not
  unbounded. Note it; do not optimize during the feature. If it ever
  becomes a perf problem, that is a separate, measured slice.
- **R3 — the `Grounded<Bool>` control-flow cascade may ripple
  wider than expected.** *Mitigation:* sequence it (slice 6) behind
  the load-bearing arithmetic/concat slices. If it ripples past its
  slice, it splits — it does not block the rest of the phase.
- **R4 — D5's visible-discard change could surface existing silent
  coercions.** *Mitigation:* D5 is *additive* — the coercion still
  compiles and runs, it just gains an IR node. Only `@grounded_pure`
  agents are affected, and there are none until slice 8. Confirmed
  by the full workspace test run + corpus baseline staying green
  through slices 2–5.
- **R5 — `@grounded_pure` interaction with `@deterministic` /
  `@replayable`.** *Mitigation:* D6 specifies orthogonal composition
  up front; slice 8 includes a test matrix over the attribute
  subsets.
- **R6 — Design X blast radius (added 2026-05-12).** Making `data:
  grounded` produce `Type::Grounded<T>` means every consumer of a
  grounded prompt/tool result now sees `Grounded<T>`. The legacy
  rule (D5) absorbs *most* positions (return / args / bindings /
  fields — anything governed by `is_assignable_to`). The positions
  it does NOT absorb are operator operands (`check_binop` /
  `check_unop` match concrete arms) — which D1 part B covers — and
  any position with a hard concrete-type match elsewhere in the
  checker. *Mitigation:* slice 2 includes an explicit blast-radius
  measurement — after wiring Design X, run the full workspace test
  suite + `verify --corpus` + every `examples/` program, and
  enumerate every new failure. The expectation is that legacy-rule
  + D1-contagion cover essentially all of it and the residue is a
  countable handful of sites needing explicit `Grounded<T>`
  annotations or `.unwrap_discarding_sources()`. **If the residue is
  large or structural, slice 2 stops and the phase re-scopes** —
  that is the recon gate, not a thing to patch around.

---

## Slice plan

One commit per slice. Validation gate between every commit (workspace
check + targeted tests + `corvid-runtime-core` wasm32 build + corpus
baseline). Push every slice. Wait for acknowledgement at slice
boundaries.

- **Slice 0 — design doc.** This document. Commit:
  `docs(grounded-prop-0): record provenance-propagation design`.
- **Slice 1 — `Derived` representation. ✅ shipped.**
  `ProvenanceKind::Derived { op, inputs: Vec<ProvenanceChain> }`
  landed in `corvid-runtime-core/src/provenance.rs`. `Eq` added to
  `ProvenanceChain` + `ProvenanceEntry` (the recursive tree needs it;
  no floats in core's chain types so total equality is sound).
  `ProvenanceChain::derived(op, inputs, timestamp_ms)` constructor —
  the single mint point all four tiers will use. 5 round-trip tests
  (incl. recursive-tree survival + empty ungrounded-operand chain +
  `Eq`). Gate green: wasm32 build clean, core 20/20, runtime 257/257,
  workspace check clean, corpus baseline unchanged.
- **Slice 2 — typechecker: contagion law + Design X (revised
  2026-05-12).** `corvid-types`, landed in two coupled steps within
  one slice per D1's ordering note:
  - **2a — contagion law, dormant.** `check_binop` / `check_unop`
    handle `Type::Grounded` operands: result is `Grounded<T>` /
    `Grounded<Bool>` per D1 part B. Nothing produces `Type::Grounded`
    from effects yet, so this is dormant — corpus + workspace stay
    green. Checker tests for every operand shape.
  - **2b — Design X.** `data: grounded` on an effect row wraps the
    prompt/tool/agent return type to `Type::Grounded<T>` (D1 part A).
    This *activates* 2a. The legacy rule (D5) absorbs the
    return/arg/binding/field positions.
  - **2c — blast-radius measurement (R6).** Full workspace test
    suite + `verify --corpus` + every `examples/` program; enumerate
    every new failure. If the residue is a countable handful → note
    them, they get explicit annotations in later slices. **If it is
    large or structural → stop, re-scope the phase.**
  - The legacy rule is also made *detectable* for lowering here (D5
    groundwork) so slice 3 can insert the visible discard node.
- **Slice 3 — IR.** `corvid-ir`: operator nodes carry/derive
  grounded-result info; lowering emits the visible discard node at
  implicit-coercion sites (D5). IR lowering tests.
- **Slice 4 — interpreter.** `corvid-vm`: `eval_arithmetic` /
  comparison eval handle `Grounded` operands — unwrap, operate,
  re-wrap with the `Derived` chain (D3) and `Min` confidence (D4).
- **Slice 5 — native codegen.** `corvid-codegen-cl`: same semantics
  through the C runtime. Gated by the differential verifier
  re-agreeing interpreter ≡ native (R1).
- **Slice 6 — control-flow conditions.** `Grounded<Bool>` accepted
  by `if` / `while` / `match` across all tiers, with the recorded
  implicit condition-unwrap (D2).
- **Slice 7 — `@grounded_pure` front end.** `corvid-syntax` /
  `corvid-ast` / `corvid-resolve`: parse + represent + resolve the
  attribute.
- **Slice 8 — `@grounded_pure` proof.** `corvid-types` / `corvid-ir`:
  the reachability proof obligation (D6) + the attribute-composition
  test matrix (R5).
- **Slice 9 — corpus + differential-verify.** `combined_all.cor` →
  `Grounded<String>` (D7); new dedicated legacy-coercion fixture;
  the `corpus_scan_includes_deliberate_failure` test goes green
  *because the four tiers genuinely agree*.
- **Slice 10 — invention-shipping contract.** README catalog entry,
  `corvid tour --topic provenance-propagation` demo, `inventions.md`
  proof-matrix row, spec section, `learnings.md` closeout, ROADMAP
  tick.

Estimated ~3 weeks. The phase is large because it is honest: a
language capability that is not end-to-end across all four tiers is
not shipped.

## Invention-shipping contract (CLAUDE.md)

Provenance Propagation is a nameable Corvid capability. Before the
phase closes, slice 10 must deliver:

- **README catalog entry** — "Provenance Propagation" with the
  contagion law + the `@grounded_pure` guarantee.
- **`corvid tour --topic provenance-propagation`** — a demo whose
  source compiles through the normal driver pipeline, showing
  `Grounded<T>` flowing through operators and a `@grounded_pure`
  boundary refusing a laundering attempt.
- **`docs/reference/inventions.md` row** — shipped status, runnable
  command, test coverage, spec link, explicit non-scope (Pillars 1
  & 2 deferred).
- **Spec section** — defines the contagion law, the `Derived`
  representation, `@grounded_pure`'s proof obligation.
- **Tests** — validate every claim the catalog entry makes.

## Acceptance criteria for closing the phase

- [ ] All 11 slices (0–10) on `main`.
- [ ] `Grounded<T>` contagious through arithmetic, concat, and
      comparison — verified across checker, interpreter, native,
      replay.
- [ ] `@grounded_pure` compiles clean agents and refuses laundering
      agents; orthogonal composition with `@deterministic` /
      `@replayable` tested.
- [ ] `tests/corpus/combined_all.cor` compiles + runs consistently
      across all four tiers; `corpus_scan_includes_deliberate_failure`
      green.
- [ ] `cargo test --workspace` green; corpus baseline otherwise
      unchanged.
- [ ] `corvid-runtime-core` still builds to `wasm32-unknown-unknown`
      (the `Derived` addition must stay wasm-clean).
- [ ] Invention-shipping contract delivered (slice 10).
- [ ] `learnings.md` + ROADMAP updated.

## What's out of scope

- **Pillar 1 polish** — rich how-provenance DAG rendering in
  `corvid trace dag`. Data model ships here; rendering is a
  follow-up.
- **Pillar 2** — compile-time `@confidence` budget enforcement
  through arithmetic, and per-operator confidence-composition rules.
  Runtime `Min` composition ships here; the compile-time pillar is
  its own phase.
- **`&&` / `||` contagion** — short-circuit booleans route
  specially today; folding them into the contagion law is a small
  follow-up if it proves worth it.
- **Performance** — the `Derived` chain's memory footprint is
  measured-not-optimized; a separate slice if it becomes real.
