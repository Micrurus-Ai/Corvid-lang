//! Canonical registry of every public Corvid guarantee.
//!
//! This crate is the single source of truth for what Corvid promises,
//! who enforces it, and where in the pipeline that enforcement lives.
//! Every later Phase 35 artifact derives from this registry:
//!
//!   * `corvid contract list` prints the registry.
//!   * `docs/core-semantics.md` is generated from it.
//!   * The bilateral verifier cross-checks against it.
//!   * `corvid claim --explain` reports per-binary which entries
//!     were enforced.
//!   * `corvid build --sign` refuses to ship unless every declared
//!     contract maps to a registry entry.
//!
//! No public guarantee is anonymous. If a check exists in the
//! compiler or runtime that backs a public claim, it must register
//! here. If a behaviour is documented but not enforced, it must
//! register here as `GuaranteeClass::OutOfScope` with an explicit
//! `out_of_scope_reason` — that is how the registry stays honest.

#![forbid(unsafe_code)]

pub mod render;

pub use render::render_core_semantics_markdown;

pub mod types;
pub mod validate;
pub use types::*;
pub use validate::*;

pub mod registry;
pub use registry::*;


pub mod signed_claim;
pub use signed_claim::*;


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_well_formed() {
        validate_slice(GUARANTEE_REGISTRY).expect("registry well-formed");
    }

    #[test]
    fn lookup_finds_known_entry() {
        let g = lookup("approval.dangerous_call_requires_token").expect("entry exists");
        assert_eq!(g.kind, GuaranteeKind::Approval);
        assert_eq!(g.class, GuaranteeClass::Static);
    }

    #[test]
    fn lookup_misses_unknown_entry() {
        assert!(lookup("nope.does_not_exist").is_none());
    }

    #[test]
    fn by_class_static_excludes_out_of_scope() {
        for g in by_class(GuaranteeClass::Static) {
            assert_ne!(g.class, GuaranteeClass::OutOfScope);
        }
        let static_count = by_class(GuaranteeClass::Static).count();
        assert!(
            static_count >= 5,
            "expected at least 5 static guarantees in seed, got {static_count}"
        );
    }

    #[test]
    fn by_kind_partitions_registry() {
        let mut total = 0;
        for kind in GuaranteeKind::ALL {
            total += by_kind(*kind).count();
        }
        assert_eq!(
            total,
            GUARANTEE_REGISTRY.len(),
            "every entry must belong to exactly one kind"
        );
    }

    #[test]
    fn signed_cdylib_claim_ids_resolve_to_enforced_guarantees() {
        let mut seen = std::collections::BTreeSet::new();
        for id in SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS {
            assert!(seen.insert(*id), "duplicate signed cdylib claim id `{id}`");
            let guarantee = lookup(id)
                .unwrap_or_else(|| panic!("signed cdylib claim id `{id}` is not registered"));
            assert_ne!(
                guarantee.class,
                GuaranteeClass::OutOfScope,
                "signed cdylib claim id `{id}` must be enforced"
            );
        }
    }

    #[test]
    fn out_of_scope_entries_carry_reasons() {
        let mut found = 0;
        for g in by_class(GuaranteeClass::OutOfScope) {
            assert!(
                !g.out_of_scope_reason.trim().is_empty(),
                "OutOfScope guarantee `{}` has no reason",
                g.id
            );
            found += 1;
        }
        assert!(
            found >= 1,
            "registry should explicitly enumerate at least one OutOfScope honest non-defense"
        );
    }

    #[test]
    fn duplicate_id_rejected() {
        let entries = [GUARANTEE_REGISTRY[0], GUARANTEE_REGISTRY[0]];
        let err = validate_slice(&entries).expect_err("duplicate must fail");
        assert!(matches!(err, RegistryError::DuplicateId(_)));
    }

    #[test]
    fn out_of_scope_without_reason_rejected() {
        let bad = Guarantee {
            id: "test.no_reason",
            kind: GuaranteeKind::Platform,
            class: GuaranteeClass::OutOfScope,
            phase: Phase::Platform,
            description: "demo",
            out_of_scope_reason: "",
            positive_test_refs: &[],
            adversarial_test_refs: &[],
        };
        let err = validate_slice(&[bad]).expect_err("missing reason must fail");
        assert!(matches!(err, RegistryError::OutOfScopeMissingReason(_)));
    }

    #[test]
    fn enforced_with_reason_rejected() {
        let bad = Guarantee {
            id: "test.spurious_reason",
            kind: GuaranteeKind::Approval,
            class: GuaranteeClass::Static,
            phase: Phase::TypeCheck,
            description: "demo",
            out_of_scope_reason: "should not be set",
            positive_test_refs: &[],
            adversarial_test_refs: &[],
        };
        let err = validate_slice(&[bad]).expect_err("enforced + reason must fail");
        assert!(matches!(err, RegistryError::EnforcedHasReason(_)));
    }

    #[test]
    fn malformed_id_rejected() {
        let bad = Guarantee {
            id: "NoDot",
            kind: GuaranteeKind::Approval,
            class: GuaranteeClass::Static,
            phase: Phase::TypeCheck,
            description: "demo",
            out_of_scope_reason: "",
            positive_test_refs: &[],
            adversarial_test_refs: &[],
        };
        let err = validate_slice(&[bad]).expect_err("malformed id must fail");
        assert!(matches!(err, RegistryError::MalformedId { .. }));
    }

    #[test]
    fn slugs_round_trip_through_display() {
        for kind in GuaranteeKind::ALL {
            assert_eq!(format!("{kind}"), kind.slug());
        }
        for class in GuaranteeClass::ALL {
            assert_eq!(format!("{class}"), class.slug());
        }
        for phase in Phase::ALL {
            assert_eq!(format!("{phase}"), phase.slug());
        }
    }

    // ----------------------------------------------------------------
    // Phase 35-E: cross-reference enforcement.
    //
    // Every Static / RuntimeChecked guarantee must have at least one
    // positive test ref AND at least one adversarial test ref. Every
    // populated test ref must follow the format
    // `<file_path>::<fn_name>` and refer to a function that actually
    // exists in the named file.
    //
    // OutOfScope guarantees are exempt from the test-ref requirement
    // — they are explicit non-defenses; the `out_of_scope_reason` is
    // their proof. Slice 35-A's `validate_slice` already enforces
    // that exemption is honest.
    // ----------------------------------------------------------------

    fn split_test_ref(test_ref: &str) -> Option<(&str, &str)> {
        let mut parts = test_ref.rsplitn(2, "::");
        let fn_name = parts.next()?;
        let file_path = parts.next()?;
        if file_path.is_empty() || fn_name.is_empty() {
            return None;
        }
        Some((file_path, fn_name))
    }

    /// Read the file at `file_path` (interpreted relative to the
    /// workspace root, which is the `corvid-guarantees` crate's
    /// great-grandparent dir during tests).
    fn read_file_under_workspace(file_path: &str) -> Result<String, String> {
        // CARGO_MANIFEST_DIR is .../crates/corvid-guarantees during
        // `cargo test`. Walk up two levels to hit the workspace root.
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .ok_or_else(|| {
                format!(
                    "could not derive workspace root from CARGO_MANIFEST_DIR `{}`",
                    manifest_dir.display()
                )
            })?;
        let abs = workspace_root.join(file_path);
        std::fs::read_to_string(&abs).map_err(|e| {
            format!(
                "could not read `{}` (resolved to `{}`): {e}",
                file_path,
                abs.display()
            )
        })
    }

    #[test]
    fn every_enforced_guarantee_has_positive_and_adversarial_test_refs() {
        let mut missing: Vec<String> = Vec::new();
        for g in GUARANTEE_REGISTRY {
            if g.class == GuaranteeClass::OutOfScope {
                continue;
            }
            if g.positive_test_refs.is_empty() {
                missing.push(format!(
                    "guarantee `{}` (class {}) has zero positive_test_refs",
                    g.id,
                    g.class.slug()
                ));
            }
            if g.adversarial_test_refs.is_empty() {
                missing.push(format!(
                    "guarantee `{}` (class {}) has zero adversarial_test_refs",
                    g.id,
                    g.class.slug()
                ));
            }
        }
        assert!(
            missing.is_empty(),
            "phase 35-E test-coverage gap:\n  - {}\n\nEither downgrade the \
             guarantee to OutOfScope with an explicit reason or add tests \
             before promoting it back.",
            missing.join("\n  - ")
        );
    }

    #[test]
    fn every_test_ref_has_well_formed_path() {
        let mut malformed: Vec<String> = Vec::new();
        for g in GUARANTEE_REGISTRY {
            for r in g
                .positive_test_refs
                .iter()
                .chain(g.adversarial_test_refs.iter())
            {
                if split_test_ref(r).is_none() {
                    malformed.push(format!(
                        "guarantee `{}`: test_ref `{}` is not in `<file>::<fn>` form",
                        g.id, r
                    ));
                }
            }
        }
        assert!(
            malformed.is_empty(),
            "phase 35-E malformed test refs:\n  - {}",
            malformed.join("\n  - ")
        );
    }

    #[test]
    fn every_test_ref_resolves_to_a_real_test_function() {
        // Group refs by file so each file is read once.
        use std::collections::BTreeMap;
        let mut by_file: BTreeMap<&'static str, Vec<(&'static str, &'static str)>> =
            BTreeMap::new();
        for g in GUARANTEE_REGISTRY {
            for r in g
                .positive_test_refs
                .iter()
                .chain(g.adversarial_test_refs.iter())
            {
                let (file, func) = split_test_ref(r).expect(
                    "every_test_ref_has_well_formed_path enforces the shape; \
                     this should already pass before reaching here",
                );
                by_file.entry(file).or_default().push((g.id, func));
            }
        }

        let mut missing: Vec<String> = Vec::new();
        for (file, refs) in &by_file {
            let body = match read_file_under_workspace(file) {
                Ok(s) => s,
                Err(e) => {
                    for (gid, func) in refs {
                        missing.push(format!(
                            "guarantee `{gid}`: cannot read `{file}` to verify \
                             `{func}` exists ({e})"
                        ));
                    }
                    continue;
                }
            };
            for (gid, func) in refs {
                let needle = format!("fn {func}(");
                if !body.contains(&needle) {
                    missing.push(format!(
                        "guarantee `{gid}`: test function `{func}` not found in `{file}` \
                         (looked for literal `{needle}`)"
                    ));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "phase 35-E unresolved test refs:\n  - {}",
            missing.join("\n  - ")
        );
    }

    // ----------------------------------------------------------------
    // Phase 35V-T1-A clean-signal sentinels.
    //
    // The 35V-T1-A verification (2026-05-08) re-checked Phase 35-A's
    // narrow claim — that the registry is internally well-formed,
    // every row has the required fields, and every test_ref resolves
    // to a real fn — and found it clean. The existing tests above
    // already pin most of that surface. The two sentinels below add
    // the property gaps the verification surfaced as still-unpinned:
    // exhaustive class-axis partitioning of the registry (mirror of
    // `by_kind_partitions_registry`) and a row-count regression
    // canary against the 2026-05-08 baseline.
    //
    // Note: a separate inverse-coverage finding (every Static /
    // RuntimeChecked id should appear in non-test workspace source)
    // surfaced 18 unwired ids during the 35V-T1-A run. That belongs
    // to slice 35V-T1-Drift / 35V-T1-B (35-B's "no anonymous
    // contract enforcement" claim), not to 35V-T1-A — the registry
    // itself is well-formed; the gap is downstream tagging. See
    // `docs/phase-35V-pre-launch-audit.md` for the slice plan.
    // ----------------------------------------------------------------

    /// Phase 35V-T1-A: every registry row belongs to exactly one
    /// `GuaranteeClass`. Mirror of `by_kind_partitions_registry`
    /// for the orthogonal class axis. Catches bugs where a future
    /// row's class is read incorrectly by `by_class` (filter
    /// regression) or where an entire class slot drops out of the
    /// public iter helpers (registry-API regression).
    #[test]
    fn by_class_partitions_registry() {
        let total: usize = GuaranteeClass::ALL
            .iter()
            .map(|c| by_class(*c).count())
            .sum();
        assert_eq!(
            total,
            GUARANTEE_REGISTRY.len(),
            "every registry row must belong to exactly one class; \
             by_class partition sum {total} != registry len {}",
            GUARANTEE_REGISTRY.len()
        );
    }

    /// Phase 35V-T1-A row-count canary. Pins the registry size at
    /// or above the verified-clean baseline so an accidental
    /// `git rebase` drop or a refactor that elides rows can't
    /// silently shrink the public guarantee surface. Promotion to
    /// a higher floor is welcome as the registry grows; the
    /// floor is monotonically increasing, so this canary doesn't
    /// fight legitimate adds.
    ///
    /// 2026-05-08 baseline: 56 rows (39 Static/RuntimeChecked + 17
    /// OutOfScope) verified clean for 35-A's internal honesty
    /// claims by slice 35V-T1-A.
    ///
    /// `#[allow(non_snake_case)]` because "35V" is the canonical
    /// phase identifier (matches `ROADMAP.md` and
    /// `docs/phase-35V-pre-launch-audit.md`); rendering it as
    /// `35_v` would diverge from how the phase is referenced
    /// everywhere else.
    #[test]
    #[allow(non_snake_case)]
    fn registry_row_count_at_or_above_phase_35V_t1_a_baseline() {
        const BASELINE: usize = 56;
        assert!(
            GUARANTEE_REGISTRY.len() >= BASELINE,
            "registry has {} rows; phase 35V-T1-A 2026-05-08 baseline \
             is {BASELINE}. Rows being removed unintentionally is the \
             drift mode this canary catches; if a row is being removed \
             intentionally (downgrade to private, deprecation), bump the \
             BASELINE constant in the same commit.",
            GUARANTEE_REGISTRY.len()
        );
    }

    // ----------------------------------------------------------------
    // Phase 35V-T1-Drift sentinel.
    //
    // Inverse-coverage check that 35V-T1-A's draft surfaced and
    // 35V-T1-Drift's tagging commits (A through E) corrected:
    // every Static / RuntimeChecked guarantee row's id must appear
    // as a literal in at least one non-test, non-registry workspace
    // source file. The forward direction (every diagnostic id is
    // registered) is enforced by `TypeError::with_guarantee`'s
    // debug_assert; this is the inverse — every registered
    // enforced row is wired to at least one anchor in production
    // code. A row failing this check is "claimed but not wired" —
    // either the implementation is missing or the contract is
    // anonymous in code. The 18-row finding from the draft sentinel
    // surfaced "implementation exists, missing tag" drift; commits
    // A-E added the tags. This permanent sentinel pins the
    // property going forward so future regressions can't reopen
    // the drift silently.
    //
    // `OutOfScope` rows are exempt: by definition they have no
    // enforcement; their `out_of_scope_reason` is the proof.
    // ----------------------------------------------------------------

    /// Walk every `.rs` source file under `crates/` looking for a
    /// literal occurrence of `needle`. Returns true if found in at
    /// least one non-skip file; false otherwise.
    ///
    /// Skip list (a guarantee row's id only counts as "wired" when
    /// it appears in real enforcement code, not in the registry
    /// definition or test-reference paperwork):
    ///
    /// - The registry file itself (`crates/corvid-guarantees/src/registry.rs`),
    ///   which by construction names every id.
    /// - The signed-claim whitelist file (`crates/corvid-guarantees/src/signed_claim.rs`),
    ///   which is itself a curated subset of registry ids.
    /// - The render module (`crates/corvid-guarantees/src/render.rs`),
    ///   which formats the registry into Markdown.
    /// - Files whose path contains `/tests/` or ends in
    ///   `tests.rs` — these are the test-reference targets the
    ///   registry already names in `positive_test_refs` /
    ///   `adversarial_test_refs`.
    ///
    /// Anywhere else the id appears (typechecker `with_guarantee`
    /// calls, runtime const declarations, codegen diagnostic
    /// emission, doc-comment anchors at enforcement sites) is
    /// treated as evidence the guarantee is wired.
    fn id_is_wired_in_workspace_source(needle: &str) -> bool {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root");
        let crates_dir = workspace_root.join("crates");
        let mut stack = vec![crates_dir];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let path_str = path.to_string_lossy();
                let path_lower = path_str.replace('\\', "/");
                if path_lower.ends_with("crates/corvid-guarantees/src/registry.rs")
                    || path_lower.ends_with("crates/corvid-guarantees/src/signed_claim.rs")
                    || path_lower.ends_with("crates/corvid-guarantees/src/render.rs")
                {
                    continue;
                }
                if path_lower.ends_with("/tests.rs") || path_lower.contains("/tests/") {
                    continue;
                }
                if let Ok(body) = std::fs::read_to_string(&path) {
                    if body.contains(needle) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Phase 35V-T1-Drift permanent sentinel. Every `Static` /
    /// `RuntimeChecked` registry row must have its `id` appear as
    /// a literal in at least one non-test, non-registry source
    /// file. A failing row is claimed-but-not-wired drift — either
    /// the implementation is missing (downgrade to OutOfScope with
    /// an explicit reason) or the enforcement site needs a literal
    /// anchor (add `pub const GUARANTEE_ID_<NAME>: &str = "<id>"`
    /// near the enforcement code, with a doc comment, mirroring
    /// `ReplayDivergence::guarantee_id` in
    /// `corvid-runtime/src/replay/diverge.rs`).
    ///
    /// Established 2026-05-08 by 35V-T1-Drift after the draft of
    /// this test surfaced 18 unwired ids (commits A-E added their
    /// anchors). The sentinel pins the property going forward so
    /// future regressions can't reopen the drift silently.
    #[test]
    fn every_enforced_guarantee_id_is_wired_to_workspace_source() {
        let mut unwired: Vec<&'static str> = Vec::new();
        for g in GUARANTEE_REGISTRY {
            if g.class == GuaranteeClass::OutOfScope {
                continue;
            }
            if !id_is_wired_in_workspace_source(g.id) {
                unwired.push(g.id);
            }
        }
        assert!(
            unwired.is_empty(),
            "phase 35V-T1-Drift: registry rows enforced \
             (Static/RuntimeChecked) but not anchored in any workspace \
             source file:\n  - {}\n\nEither downgrade the row to \
             OutOfScope with an explicit `out_of_scope_reason`, or add \
             a literal anchor near the enforcement site \
             (`pub const GUARANTEE_ID_<NAME>: &str = \"<id>\"` plus a \
             doc comment naming the contract). See \
             docs/phase-35V-pre-launch-audit.md for the corrective \
             pattern.",
            unwired.join("\n  - ")
        );
    }

    // ----------------------------------------------------------------
    // Phase 35V-T1-B sentinel.
    //
    // Verifies 35-B's claim: "Every contract-enforcing diagnostic
    // in resolve / typecheck / IR-lower / codegen carries its
    // `guarantee_id`. No contract enforcement is anonymous." This
    // sentinel narrows that claim to its mechanically-checkable
    // core: every `Static` guarantee whose enforcement phase is
    // typecheck / resolve / IR-lower / codegen must be constructed
    // through `TypeError::with_guarantee(...)` somewhere — the
    // canonical tagged-diagnostic API. A row with no
    // `with_guarantee("<id>")` site is one where either the
    // diagnostic exists but uses the un-tagged `TypeError::new`
    // (drift to fix by retagging the call site) or the diagnostic
    // doesn't exist at all (drift to fix by downgrading the row).
    //
    // `RuntimeChecked` rows are exempt because runtime enforcement
    // uses different patterns (constants + `*_guarantee_id()`
    // methods, anchored by 35V-T1-Drift's sentinel above).
    //
    // The forward direction (every `with_guarantee` id resolves in
    // the registry) is enforced by the debug_assert in
    // `TypeError::with_guarantee` in `corvid-types/src/errors.rs`.
    // The 35V-T1-A row-count canary + 35-E test-ref resolution +
    // 35V-T1-Drift workspace-source anchor + this T1-B
    // `with_guarantee` anchor together pin the registry's honesty
    // surface.
    // ----------------------------------------------------------------

    /// Walk every `.rs` file under `crates/` looking for a
    /// `with_guarantee(..., "<needle>"...)`-style call. Returns
    /// true if found in at least one non-skip file.
    ///
    /// Same skip list as `id_is_wired_in_workspace_source`: registry
    /// definition / signed-claim whitelist / render module / test
    /// files are excluded.
    fn id_appears_in_with_guarantee_call(needle: &str) -> bool {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root");
        let crates_dir = workspace_root.join("crates");
        let mut stack = vec![crates_dir];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let path_str = path.to_string_lossy();
                let path_lower = path_str.replace('\\', "/");
                if path_lower.ends_with("crates/corvid-guarantees/src/registry.rs")
                    || path_lower.ends_with("crates/corvid-guarantees/src/signed_claim.rs")
                    || path_lower.ends_with("crates/corvid-guarantees/src/render.rs")
                {
                    continue;
                }
                if path_lower.ends_with("/tests.rs") || path_lower.contains("/tests/") {
                    continue;
                }
                if let Ok(body) = std::fs::read_to_string(&path) {
                    // Look for either (a) a tagged-diagnostic call
                    // that mentions the id, or (b) the
                    // language-VM's equivalent helper that takes a
                    // guarantee_id parameter. Since matching the
                    // multi-line `with_guarantee(...)` syntactic
                    // shape exactly would require parsing, we
                    // approximate: the body contains both
                    // `with_guarantee` AND the id literal.
                    if body.contains("with_guarantee") && body.contains(needle) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Phase 35V-T1-B permanent sentinel. For every `Static`
    /// guarantee whose enforcement phase is one of `TypeCheck`,
    /// `Resolve`, or `IrLower`, the registry id must appear in a
    /// `with_guarantee(...)` call site somewhere in non-test
    /// workspace source. These three phases' canonical
    /// tagged-diagnostic API is `TypeError::with_guarantee`; the
    /// forward direction is enforced by its debug_assert in
    /// `corvid-types/src/errors.rs`; this test pins the inverse.
    ///
    /// `Codegen` is intentionally NOT in scope here because
    /// codegen-phase contract enforcement uses `CodegenError`,
    /// not `TypeError`, and the project does not currently ship a
    /// tagged-diagnostic constructor for `CodegenError` analogous
    /// to `with_guarantee`. Codegen-phase rows are still anchored
    /// in workspace source via the broader 35V-T1-Drift sentinel
    /// (literal id appears somewhere in non-test code), so they
    /// are not anonymous; they just don't go through this
    /// specific construct. A separate slice could add a
    /// `CodegenError::with_guarantee` constructor and tighten
    /// this test's scope; that work is filed for a future T1-B
    /// follow-up rather than blocking the current launch claim.
    ///
    /// `RuntimeChecked` rows are also exempt because runtime
    /// enforcement uses module-level constants (anchored by
    /// 35V-T1-Drift's sentinel), not `TypeError::with_guarantee`.
    #[test]
    fn every_typecheck_phase_static_guarantee_uses_with_guarantee_constructor() {
        let mut untagged: Vec<&'static str> = Vec::new();
        for g in GUARANTEE_REGISTRY {
            if g.class != GuaranteeClass::Static {
                continue;
            }
            let phase_in_scope = matches!(
                g.phase,
                Phase::TypeCheck | Phase::Resolve | Phase::IrLower
            );
            if !phase_in_scope {
                continue;
            }
            if !id_appears_in_with_guarantee_call(g.id) {
                untagged.push(g.id);
            }
        }
        assert!(
            untagged.is_empty(),
            "phase 35V-T1-B: Static guarantees in TypeCheck / \
             Resolve / IrLower phase but not constructed via \
             `TypeError::with_guarantee(...)`:\n  - {}\n\n\
             Either retag the diagnostic site to use \
             `with_guarantee` instead of `TypeError::new`, or \
             downgrade the registry row to OutOfScope with an \
             explicit `out_of_scope_reason`. The forward direction \
             is enforced by `with_guarantee`'s debug_assert in \
             `corvid-types/src/errors.rs`; this test pins the \
             inverse.",
            untagged.join("\n  - ")
        );
    }
}
