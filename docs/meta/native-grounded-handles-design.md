# Pre-phase stub — Native Grounded-Handle Representation

**Status:** STUB. Not yet started. Filed 2026-05-12 as the
pre-phase placeholder for a follow-up phase split out of Provenance
Propagation slice 5 (CTO decision — see
`grounded-propagation-design.md`'s slice-5 entry).

This is a stub, not a full pre-phase design. It records *why* the
phase exists and *what the chat must settle*, so the work is queued
as real and scoped rather than hand-waved. The full pre-phase chat
happens in its own session before any code.

---

## Why this phase exists

Provenance Propagation made `Grounded<T>` contagious through
operators across the typechecker, the IR, and the interpreter — the
interpreter builds the byte-identical `Derived { op, inputs }`
provenance tree (D3). Native codegen could not follow, because its
grounded model is structurally degenerate:

- A native `Grounded<T>` is a **bare machine value** of `T`. The
  `corvid_grounded_attest_*` C-ABI functions take
  `(value, source_name, confidence)` and return `value` unchanged
  (`crates/corvid-runtime/src/catalog_c_api/grounded_bridge.rs`).
- Provenance is registered into a **single process-global
  "last scalar attestation" slot** (`set_last_scalar_attestation`).
  There is no per-value provenance tracking.

This works for the narrow case it was built for — "a tool returns
`Grounded<T>` → the agent returns it directly" — and nothing more.
It cannot:

- Track more than one live grounded value at a time.
- Recover an operand's provenance chain at an operator site (the
  operand is a bare value with no identity).
- Build the `Derived` tree the contagion law (D3) requires for
  byte-identical provenance across tiers.

Provenance Propagation slice 5 shipped native **value-correctness**
(grounded operators produce correct values — see that phase's
slice-5 entry). This phase ships native **provenance-correctness**:
the `Derived` DAG, byte-identical to the interpreter's.

## What the pre-phase chat must settle

1. **The native `Grounded<T>` representation.** Byte-identical
   `Derived` trees require grounded values to carry runtime
   identity. Options to weigh: a fat representation (value + handle)
   for every grounded-typed slot; a handle-only representation with
   the value behind the handle; or a hybrid. Each has an ABI cost
   and a codegen-complexity cost.
2. **Blast radius across native codegen.** Every site that
   produces, consumes, stores, or passes a grounded value — call
   results, locals, returns, struct fields, the C-ABI boundary.
   Enumerate before scoping.
3. **The C runtime + attestation store changes.** Replace the
   single global "last scalar attestation" slot with real per-value
   provenance; add a `Derived`-aware attestation entry point that
   takes operand identities + the operator label.
4. **The Phase 22 FFI surface.** `pub extern "c"` agents return
   `Grounded<T>` with provenance across the boundary
   (`22-F-grounded-return`). A native-repr change ripples here —
   confirm the C-ABI descriptor + host-binding story still holds.
5. **The replay tier.** Replay must produce the same `Derived`
   trees native produces; settle whether replay rides on the new
   native repr or stays separate.
6. **Verification.** The profile-level differential verifier does
   NOT catch grounded-value or grounded-provenance correctness
   (per D8, contagion does not touch effect profiles). This phase
   needs a value-and-provenance-level cross-tier check —
   interpreter ≡ native byte-for-byte on the `Derived` tree, not
   just the effect profile.

## Where this sits

Queued after Provenance Propagation closes. Not a launch blocker:
the moat (`@grounded_pure`, compile-time) and the launch-critical
interpreter contagion ship in Provenance Propagation. This phase
makes native a *full* tier for the contagion law rather than a
value-correct-but-provenance-degenerate one.

ROADMAP entry to be added when Provenance Propagation closes
(slice 11).
