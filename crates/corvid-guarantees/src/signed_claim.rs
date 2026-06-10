//! The signed-cdylib claim whitelist — slice 35-J / signed
//! attestation surface, decomposed in Phase 20j-A8.
//!
//! Every guarantee id in [`SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS`]
//! is asserted by the build's DSSE-signed ABI descriptor for
//! every signed cdylib artifact. The build signing gate
//! (`corvid build --sign`) refuses to emit a signature unless
//! every declared contract in the source maps to a registry
//! entry whose id is in this list — so the signed binary
//! advertises only enforced claims, never aspirational ones.
//!
//! Excluded from the list:
//! - guarantees whose subject is not a cdylib (e.g.,
//!   receipt-envelope verification, observability sink shape)
//! - explicit non-defenses (the `OutOfScope` rows in
//!   [`super::registry::GUARANTEE_REGISTRY`])

use super::registry::lookup;
use super::types::Guarantee;

/// Guarantee ids carried by every signed cdylib ABI descriptor.
///
/// This list excludes guarantees whose subject is not a cdylib
/// artifact, such as receipt-envelope verification, and excludes
/// explicit non-defenses. The build signing gate checks source
/// declarations against this set before it emits a DSSE attestation.
pub const SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS: &[&str] = &[
    "approval.dangerous_call_requires_token",
    "approval.token_lexical_only",
    // `approval.dangerous_marker_preserved` removed 2026-05-08 by
    // Phase 35V-T1-B — downgraded to OutOfScope because the
    // property is structural (Corvid source has no alias-effect-
    // override surface that could erase a `@dangerous` marker), so
    // there's no separately-tagged diagnostic to anchor a
    // signed-claim assertion against. The parent
    // `approval.dangerous_call_requires_token` (above) fires
    // through the alias path and that firing is what the registry
    // test_refs verify.
    "approval.reachable_entrypoints_require_contract",
    "effect_row.body_completeness",
    // `effect_row.caller_propagation` removed 2026-05-08 by Phase
    // 35V-T1-B — downgraded to OutOfScope because the analyzer's
    // unified `ConstraintViolation` doesn't distinguish
    // body-internal from callee-contributed violations; the parent
    // `effect_row.body_completeness` (above) fires a single
    // diagnostic that enforces both perspectives.
    "effect_row.import_boundary",
    // Slice 33S1c — the three executing file-I/O guarantees
    // (from the 33S/33S1 executing-I/O-surfaces phase). A
    // signed cdylib whose source uses `io.read_text` /
    // `io.write_text` / `io.list_dir` (declared in std/io.cor)
    // asserts these three RuntimeChecked properties in its
    // claim manifest: path confinement against `[io] root`,
    // write quarantine on replay, and read gate on replay.
    // See crates/corvid-runtime/src/io.rs for the enforcement
    // anchors and crates/corvid-guarantees/src/registry.rs for
    // the rows + test refs.
    "io_source.fs_path_confinement",
    "io_source.fs_write_quarantine_on_replay",
    "io_source.fs_read_quarantine_on_replay",
    // Slice 33S2c — the three executing HTTP-client guarantees.
    // A signed cdylib whose source uses `http_get` /
    // `http_post_json` (declared in std/http.cor) asserts these
    // three RuntimeChecked properties in its claim manifest:
    // structural SSRF block (refuses private / loopback / link-
    // local hosts regardless of allowlist), required `[http] allow`
    // allowlist enforcement, and replay quarantine that refuses
    // POST escapes and gates GETs through the substitution path.
    // See crates/corvid-runtime/src/http.rs for the enforcement
    // anchors and crates/corvid-guarantees/src/registry.rs for
    // the rows + test refs.
    "io_source.http_ssrf_structural_block",
    "io_source.http_allowlist_enforcement",
    "io_source.http_quarantine_on_replay",
    // Slice 33S3d — the three executing SQLite guarantees.
    // A signed cdylib whose source uses `db_open` / `db_query`
    // / `db_execute` (declared in std/db.cor) asserts these
    // three RuntimeChecked properties in its claim manifest:
    // structural parameter-binding-only (no SQL interpolation
    // path exists; the typechecker's List<DbParam> signature +
    // rusqlite::params_from_iter together prevent injection),
    // write quarantine on replay (db_execute refuses during
    // Substitute-mode), and read passthrough on replay (db_query
    // is not blocked; trace-substitution upper gate lands in a
    // follow-up slice). Path confinement reuses the existing
    // io_source.fs_path_confinement guarantee — db_open routes
    // through IoToolPolicy::resolve the same way the io tools
    // do, so a signed cdylib that uses db_open inherits the
    // fs_path_confinement claim automatically.
    "io_source.sqlite_parameter_binding_only",
    "io_source.sqlite_write_quarantine_on_replay",
    "io_source.sqlite_read_passthrough_on_replay",
    // Slice 33R5b-c — the two executing JSON guarantees.
    // A signed cdylib whose source uses `json_parse` / `json_get_*`
    // / `json_object_*` (declared in std/json.cor) asserts these
    // two RuntimeChecked properties in its claim manifest:
    // parse-safety (malformed input is recoverable, never
    // panics) and field-type-safety (typed-accessor mismatches
    // return Err, never coerce or panic). The typed-decoder
    // convention (`decode_<X>_from_json`) inherits the same
    // properties because it routes through the same parse +
    // json_to_value path.
    "json.parse_safety_no_panic",
    "json.field_type_safety_at_access_boundary",
    "grounded.provenance_required",
    // `grounded.propagation_across_calls` removed 2026-05-08 by
    // Phase 35V-T1-B — downgraded to OutOfScope because the
    // grounded-return analysis fires a single `UngroundedReturn`
    // diagnostic regardless of whether the missing provenance
    // came from direct construction or from a callee boundary;
    // the parent `grounded.provenance_required` (above) covers
    // both perspectives.
    "budget.compile_time_ceiling",
    "confidence.min_threshold",
    // Slice 33Q3 (2026-06-05): `@trust(<level>)` on an agent is now a
    // signable claim. The typechecker rejects bodies that violate the
    // declared ceiling; this id advertises that enforcement in the
    // signed cdylib's claim manifest. See
    // `crates/corvid-guarantees/src/registry.rs::trust.constraint_enforcement`
    // for the row + test refs, and `collect_constraint_claims` in
    // `corvid-driver/src/build/claim_coverage.rs` for the require-site.
    "trust.constraint_enforcement",
    "replay.deterministic_pure_path",
    "abi_descriptor.cdylib_emission",
    "abi_descriptor.byte_determinism",
    "abi_descriptor.bilateral_source_match",
    "abi_attestation.envelope_signature",
    "abi_attestation.descriptor_match",
    "abi_attestation.sign_requires_claim_coverage",
    "jobs.cron_schedule_durable",
    "jobs.idempotency_key_uniqueness",
    "jobs.lease_exclusivity",
    "jobs.durable_resume",
    "jobs.cron_dst_correct",
    // Slice 35V2-P38-C-6: every `@replayable` agent in a signed
    // cdylib promises that its side effects are quarantined during
    // replay (LLM via QuarantinedLlmAdapter, plus HTTP / store /
    // file-write refusals). The claim-coverage walker requires this
    // id for `AgentAttribute::Replayable` and
    // `AgentAttribute::Deterministic`.
    "jobs.replayable_side_effects",
    "auth.api_key_at_rest_hashed",
    "auth.jwt_kid_rotation",
    "auth.oauth_pkce_required",
    "connector.scope_minimum_enforced",
    "connector.rate_limit_respects_provider",
    "connector.webhook_signature_verified",
    "connector.replay_quarantine",
];

pub fn signed_cdylib_claim_guarantees() -> impl Iterator<Item = &'static Guarantee> {
    SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS
        .iter()
        .filter_map(|id| lookup(id))
}
