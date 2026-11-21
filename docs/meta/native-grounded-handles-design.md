# Native grounded values

## Implemented behavior

Native strings use a reference-counted descriptor. Retrieval tools and grounded
prompts attach source information to that descriptor. The ownership pass tracks
`Grounded<String>` through local reads, operators and returns. The runtime
removes uncaptured source information on final release or collector sweep.

For string addition, the compiler tells the runtime which operands are grounded.
The runtime builds a `Derived` entry named `add`, retains the input chains in
operand order and takes the minimum confidence. A plain operand contributes an
empty chain and confidence 1. The result owns a fresh descriptor. A C host can
capture its sources before consuming the returned string. The handle owns a
separate reference to the attestation until the host releases it.

The native parity test `grounded_string_concat_native_matches_interpreter`
checks computed strings and allocation balance. The shared-library test
`cdylib_grounded_string_return_exposes_attestation_handle` retrieves the source
names for a concatenation from two tools. Runtime tests cover nested joins,
confidence, explicitly discarded sources and descriptor lifetime.

## Remaining representation work

String concatenation covers one operator on one heap type. Scalar attestations
use a thread-local last-value slot. Struct attestations use heap pointer keys.
Future changes need to address overlapping scalar lifetimes, explicit host
inputs, and other operators and containers. Shared or immortal string
descriptors also need coverage when distinct retrievals return the same
descriptor with different source information.

## Design questions

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
   thread-local last scalar attestation slot with real per-value
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

The active release-readiness queue is in [ROADMAP.md](../../ROADMAP.md).
