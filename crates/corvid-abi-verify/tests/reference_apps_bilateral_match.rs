//! v1.0 launch criterion `ROADMAP.md` L50 — bilateral verifier
//! (Phase 35-H) green across the production-backend surface.
//!
//! Every Phase 37-43 contract id reachable from a built cdylib must
//! reconstruct from source through the descriptor-relevant frontend
//! pipeline and byte-match the descriptor JSON embedded as
//! `CORVID_ABI_DESCRIPTOR` in the cdylib. The verifier at
//! `crates/corvid-abi-verify/src/lib.rs::verify_source_matches_cdylib`
//! is the source-of-truth comparison; this integration test runs it
//! across all 5 reference apps so the L50 gate is mechanically
//! provable rather than asserted in prose.
//!
//! ## Why this is in `tests/` rather than `src/`
//!
//! The unit tests in `src/lib.rs` exercise the verifier against toy
//! programs (2-line `agent answer(x: Int) -> Int`). Those tests are
//! fast and run in the default `cargo test` pass. The reference apps
//! ARE production-shape — each main.cor is >600 lines and links the
//! ~100 MB runtime staticlib through Cranelift, so the per-app
//! cdylib build takes ~30 seconds. Sequenced across 5 apps the test
//! takes ~2-3 minutes on a warm cache, which is too long for the
//! default per-commit pass.
//!
//! The test is therefore marked `#[ignore]` so it doesn't bloat
//! `cargo test --workspace`. To run it explicitly:
//!
//! ```text
//! cargo test -p corvid-abi-verify --test reference_apps_bilateral_match -- --ignored
//! ```
//!
//! Per the L50 verification cadence (see
//! `docs/meta/launch-claim-audit.md` Section 9), this test is run
//! before every release-channel cut. The CI `app-deploy-smoke.yml`
//! workflow already builds all 5 cdylibs for its smoke-deploy gate,
//! so a CI invocation of this test reuses the same warm build state.

use corvid_driver::{build_target_to_disk, BuildTarget};
use std::path::PathBuf;

/// All 5 reference apps that ship as Phase 42 deliverables. Each
/// declares at least one `pub extern "c"` agent (verified at
/// `35V2-P42-G0-reprobe` close 2026-05-29 — every app builds as a
/// cdylib end-to-end).
const REFERENCE_APPS: &[&str] = &[
    "personal_executive_agent",
    "personal_knowledge_agent",
    "finance_operations_agent",
    "customer_support_agent",
    "code_maintenance_agent",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn main_cor(app: &str) -> PathBuf {
    repo_root()
        .join("examples")
        .join("backend")
        .join(app)
        .join("src")
        .join("main.cor")
}

#[test]
#[ignore = "Slow: builds 5 production-shape cdylibs (~2-3 min on a warm cache). \
            Run explicitly with `cargo test -p corvid-abi-verify --test reference_apps_bilateral_match -- --ignored` \
            before every release-channel cut per the L50 verification cadence."]
fn every_reference_app_cdylib_bilaterally_matches_its_source() {
    let mut failures: Vec<String> = Vec::new();

    for app in REFERENCE_APPS {
        let source = main_cor(app);
        assert!(
            source.exists(),
            "{app}: source missing at `{}`",
            source.display()
        );

        // Build the cdylib through the standard driver path —
        // identical to what `corvid build --target=cdylib` does for
        // an operator. emit_header=false / emit_abi_descriptor=false
        // because the descriptor symbol is always embedded
        // (`CORVID_ABI_DESCRIPTOR`); the flags only affect the
        // ergonomic side-outputs the operator doesn't need here.
        let build = match build_target_to_disk(
            &source,
            BuildTarget::Cdylib,
            false,
            false,
            &[],
            None,
        ) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!(
                    "{app}: cdylib build failed before reaching the verifier: {e}"
                ));
                continue;
            }
        };

        if !build.diagnostics.is_empty() {
            failures.push(format!(
                "{app}: cdylib build emitted {} diagnostic(s) — must be clean before \
                 bilateral verify: first diagnostic = {:?}",
                build.diagnostics.len(),
                build.diagnostics.first().unwrap()
            ));
            continue;
        }
        let cdylib = match build.output_path {
            Some(p) => p,
            None => {
                failures.push(format!(
                    "{app}: cdylib build returned no output path"
                ));
                continue;
            }
        };

        // The actual L50 check: rebuild the descriptor JSON from
        // source through the descriptor-relevant frontend
        // (lex/parse/resolve/typecheck/IR-lower/ABI-emit), read the
        // embedded `CORVID_ABI_DESCRIPTOR` symbol from the cdylib,
        // assert byte-equality.
        match corvid_abi_verify::verify_source_matches_cdylib(&source, &cdylib) {
            Ok(report) => {
                if !report.matches() {
                    failures.push(format!(
                        "{app}: bilateral mismatch — source_hash={} embedded_hash={} \
                         source_len={} embedded_len={}",
                        corvid_abi_verify::hex_hash(&report.source_json_hash),
                        corvid_abi_verify::hex_hash(&report.embedded_json_hash),
                        report.source_json_len,
                        report.embedded_json_len,
                    ));
                }
            }
            Err(e) => {
                failures.push(format!("{app}: verifier returned an error: {e}"));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "v1.0 launch criterion L50 (bilateral verifier green) failed for {} of {} reference apps:\n  - {}",
        failures.len(),
        REFERENCE_APPS.len(),
        failures.join("\n  - "),
    );
}
