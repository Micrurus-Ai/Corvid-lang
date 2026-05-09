# Phase 35 — Defensible Core

The v1.0 launch gate. Phase 35 makes Corvid's launch claim defensible
under hostile public scrutiny: every public guarantee is enumerated
in a machine-readable manifest, every guarantee is backed by
adversarial tests, the ABI surface is bilaterally byte-checked, and
the launch wording is derivable from shipped artifacts rather than
aspirational.

After Phase 35, an outside reviewer can answer "what does Corvid
guarantee, what is checked statically, what is checked at runtime,
what is out of scope, and how do I verify each independently?" in
under ten minutes by running committed commands.

## Slice catalog

The 14 slices of Phase 35 plus the 35-N audit-correction extension
landed across separate commits. The canonical slice list lives in
[`ROADMAP.md`](../ROADMAP.md) under the Phase 35 section. This
document does not duplicate the slice descriptions; it records the
phase's design rationale, the deliverables that survived 35V
verification, and the cross-slice patterns worth preserving.

## Five concrete drivers (the original "why")

External review on the path to public launch identified five gaps
between Corvid's *implementation* (compiler, runtime, tests,
attestation) and its *publicly defensible core story*. Each Phase
35 slice clusters maps to closing one or more of these:

1. **Semantic contract not crisply enumerated.** Closed by 35-A
   (`corvid-guarantees` registry) + 35-C (`corvid contract list`)
   + 35-D (spec generation from the registry).

2. **Proof living in tests, not in a concise core spec.** Closed
   by 35-D (spec generation) + 35-E (test cross-references that
   tie every Static guarantee to real positive + adversarial
   tests).

3. **Trusted computing base broad.** Partially closed by 35-H
   (separate-binary descriptor verifier) + 35-I (`corvid claim
   --explain` provenance command) + 35-K (security model doc).
   *True second-implementation TCB shrinkage is post-v1.0; the
   shipped verifier defends post-link descriptor tampering and
   build-cache drift but not full TCB shrinkage.*

4. **Launch wording risking getting ahead of formal proof.** Closed
   by 35-J (`build --sign` refusal when claims aren't covered) +
   35-L (README launch wording derivable from runnable commands)
   + 35-M (CI enforces drift gates on every push). Phase 35V
   tightened residual aspirational wording in 35-H/35-K that the
   original phase shipped with.

5. **Adversarial coverage thin.** Closed by 35-F (ABI fuzz corpus,
   ≥256 mutants per gate) + 35-G (source-bypass fuzz corpus
   covering four documented attack classes).

## What Phase 35V verification confirmed (2026-05-08 / 09)

Phase 35V is the verification round modeled on the verifier-
correction pattern from Phase 20m. It treats every Phase 35 slice
as a claim to disprove, not a fact to trust. See
[`phase-35V-pre-launch-audit.md`](./phase-35V-pre-launch-audit.md)
for the full track structure and per-slice findings.

Track 1 (Phase 35 verification, 14 slices): **8 commits** of
corrective work + 6 clean signals via existing test surface.
Substantive findings:

- 35-A registry coverage was internally honest but missing two
  partition-axis sentinels and a row-count canary (35V-T1-A added
  them).
- The inverse-coverage property (every Static/RuntimeChecked id
  appears as a literal in non-test workspace source) was unpinned;
  18 enforced rows were claimed-but-not-anchored. 35V-T1-Drift
  added the literal anchors at each enforcement site (one per row
  across jobs / auth / observability / connector / receipts
  clusters) plus a permanent inverse-coverage sentinel that catches
  future regressions of this property.
- 35-B's "no contract enforcement is anonymous" claim had four
  typecheck-shaped Static rows whose diagnostic carried the parent
  id rather than the subsidiary id. 35V-T1-B implemented one
  genuine discrimination (`approval.token_lexical_only`,
  distinguishing "no approve" from "approve out of lexical scope"
  via a new `approvals_seen_in_agent` body-wide audit log) and
  honestly downgraded three rows to OutOfScope where the unified
  analyzer fired one diagnostic for both perspectives
  (`approval.dangerous_marker_preserved`,
  `effect_row.caller_propagation`,
  `grounded.propagation_across_calls`). The signed-claim whitelist
  shrank accordingly; the `validate_signed_claim_coverage`
  validator was aligned with the downgrades in 35V-T1-J.
- 35-H's ROADMAP slice text claimed "independent code path", "two
  implementations", and "TCB shrinkage" — none of those were
  shipped. The verifier links the same `corvid-syntax` /
  `corvid-resolve` / `corvid-types` / `corvid-ir` / `corvid-abi`
  libraries the main pipeline uses. 35V-T1-H tightened the wording
  in ROADMAP, README, and `docs/security-model.md` to match
  shipped behavior; the registry row's description was already
  honest.
- A pre-existing Windows linker baseline (missing `secur32.lib`
  for the bundled `whoami` crate) was filed by Phase 20n and
  closed by 35V-T1-H so the bilateral-verifier tests could
  actually run. Two MSVC linker invocation sites needed the lib
  added (`link.rs::link_binary` and
  `cdylib.rs::link_shared_library`).
- 35-I's claim-explain stability claim was shipped but unpinned;
  35V-T1-I added a byte-stability sentinel.

Track 2 (audit-correction completeness for Phases 36/38/39/41,
12 slices): **all 12 clean signal**. The 2026-04-29 audit's
filed corrective tracks (38K/38M/39K/39L/41K/41L/41M plus 36K/L/M)
all landed honestly. The rendered axum server's middleware
pipeline is wired and exercised end-to-end by
`build_server_emits_runnable_local_http_binary`; multi-worker job
runner + SIGKILL crash-recovery + 4-worker idempotency + DST cron
all pass; real JWT verification with kid/alg/jwks-fetch refusal
works; `corvid auth` / `corvid approvals` / `corvid connectors`
exist as top-level subcommands with rich surface; replay
quarantine fires for every connector type.

## Cross-slice patterns

Patterns that survive Phase 35's slice contexts and apply to
future launch-gate work:

**1. Registry as single source of truth.** Every later artifact
derives from `corvid_guarantees::GUARANTEE_REGISTRY`: `corvid
contract list`, `docs/core-semantics.md`, the bilateral verifier
descriptor inputs, `corvid claim --explain` cross-references, and
the `corvid build --sign` claim coverage gate. A drift-gate test
(`rendered_markdown_matches_committed_doc`) catches divergence
between the rendered spec and the committed doc; CI runs it on
every push. Lesson: when shipping multiple artifacts that must
agree, generate them all from one in-code source of truth and
gate divergence at CI rather than relying on human discipline.

**2. Forward + inverse + meta coverage for the registry's
honesty.** The registry's load-bearing claim ("every public
guarantee is enumerated, every enforcement is non-anonymous") is
pinned in three orthogonal directions: forward
(`with_guarantee` debug_assert verifies tagged ids resolve in
the registry), inverse-broad (35V-T1-Drift's sentinel verifies
every Static/RuntimeChecked id appears in non-test source),
inverse-narrow (35V-T1-B's sentinel verifies every typecheck-
shaped Static id goes through the tagged constructor). Each
sentinel catches a different drift mode. Phase 20m's verifier-
correction pattern recommends "verify the comparison site, not
the suggestion field" — the orthogonal sentinels make the
inverse-of-each-comparison-site testable.

**3. Honest classification beats optimistic tagging.** Phase 35V-
T1-B found four typecheck-shaped Static rows whose enforcement
mechanism didn't fire a separately-tagged diagnostic. The honest
options were (a) implement the discrimination, or (b) downgrade
to OutOfScope with explicit `out_of_scope_reason`. The
*shortcut* would be to invent fake "subsumed_by" relationships
that paper over the lack of separate diagnostics. The phase
chose discrimination where the typechecker had the information
(token_lexical_only — `approvals_seen_in_agent` extension), and
honest downgrade where the unified analyzer fundamentally fires
one diagnostic for both perspectives (the other three rows).
Lesson: registry claims must be classified by what's actually
shipped, not by what's documented. Downgrading is not a
shortcut; claiming Static when only the parent enforces is.

**4. Aspirational launch wording surfaces in the verification
round.** Phase 35V-T1-H found ROADMAP, README, and
docs/security-model.md all carried "bilateral verifier" / "two
implementations" / "TCB shrinkage" claims that the implementation
doesn't deliver. The shipped verifier IS useful (post-link
descriptor tampering, build-cache drift) but not at the level
the wording promised. The corrective work was to tighten the
wording, not to invent the missing implementation. Lesson:
launch surfaces (ROADMAP slice descriptions, README claim
boundaries, security model doc) are checkable against shipped
behavior; a verification round that doesn't audit them misses a
load-bearing class of drift.

**5. Cross-component coupling discovered at verification time.**
Phase 35V-T1-B downgraded three registry rows to OutOfScope.
Phase 35V-T1-J found that `validate_signed_claim_coverage` in
`crates/corvid-driver/src/build/claim_coverage.rs` still
required those ids in every signed claim set — without the
validator alignment, signed builds for any source touching those
surfaces would have rejected at sign time. The verification
round caught the coupling only because the existing test
surface (`signed_claim_coverage_*`) tripped after the downgrade.
Lesson: a registry change has cross-component consequences that
a phase-level verification audit catches but a slice-level
review does not.

**6. The verifier-correction pattern scales.** Phase 20m
formalised "first-round fix phase produces fixes; verification
round produces a scorecard; corrections phase addresses only
verifier-confirmed corrections" on a 6-slice 20l surface. Phase
35V applied the same pattern to a 14-slice launch-gate surface
plus 12 audit-correction slices and 4 closer slices (~30
verifications total). The pattern produced: 8 commits of
corrective work in Track 1, zero corrective work in Track 2,
4 closer commits in Track 3. The audit's value scales with the
breadth of what's claimed; the per-slice verification cost stays
roughly constant.

## Out of scope (post-v1.0)

These are explicit non-goals. Each describes a property the
launch claim deliberately does not promise:

- Formal mechanized proof of the type system. The
  `corvid-guarantees` registry is the v1.0 public-claim surface;
  formal proofs are post-v1.0 research.
- Cryptographic primitive proofs. Corvid uses ed25519, SHA-256,
  and DSSE as standardized primitives, not redesigns.
- Defense against compiler-toolchain compromise. Corvid trusts
  the rustc and Cranelift releases the user installs;
  reproducible builds are a post-v1.0 hardening goal.
- Defense against signing-key compromise. Key management is a
  host responsibility, explicitly delegated to the host's
  key-management practice in `docs/security-model.md`.
- Bug-bounty program, third-party audit contract, formal launch
  comms — those belong to the final market-launch phase.
- True second-implementation TCB shrinkage via a separate
  parser/resolver/typechecker reaching `AbiDescriptor`
  independently — Phase 35V-T1-H tightened the launch wording
  to stop promising this; future-phase consideration to
  implement.

## Closing audit

All 14 Phase 35 slices (`35-A` through `35-N`) plus the
`35-N` audit-correction extension are `[x]` in
[`ROADMAP.md`](../ROADMAP.md). Phase 35V verified all 14 against
shipped behavior; corrective work for the discoveries landed in
the 35V Track 1 commits. The phase's `✅ closed` marker is
added in this commit.

The launch claim's honesty is now pinned by:

- Forward direction: `TypeError::with_guarantee` debug_assert.
- Inverse-broad direction: `every_enforced_guarantee_id_is_wired_to_workspace_source`.
- Inverse-narrow direction: `every_typecheck_phase_static_guarantee_uses_with_guarantee_constructor`.
- Class-axis partitioning: `by_class_partitions_registry`.
- Row-count regression: `registry_row_count_at_or_above_phase_35V_t1_a_baseline`.
- JSON output coverage: `json_payload_contains_every_registry_id`.
- Claim-explain byte stability: `claim_explain_output_is_byte_stable_across_re_runs`.
- Spec drift: `rendered_markdown_matches_committed_doc`.
- Test-ref resolution: `every_test_ref_resolves_to_a_real_test_function`.
- ABI fuzz: 256 mutants per gate, all rejected, benign mutations
  round-trip.
- Source-bypass fuzz: AST mutators across 4 attack classes, each
  fails typecheck with the right `guarantee_id`.

A future external-reviewer round on the Phase 35 / 35V surface
slots into the same verifier-correction shape this phase
established. Each subsequent round should be cheaper than the
last as recurring drift modes get pinned by sentinels.
