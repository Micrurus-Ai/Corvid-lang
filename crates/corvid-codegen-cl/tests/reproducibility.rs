//! Structural tests for the determinism fix that
//! `deploy.reproducible_build` promotes.
//!
//! The reproducible-build CI workflow
//! (`.github/workflows/reproducible-build.yml`) is the
//! production-grade oracle: it builds the corvid CLI twice with
//! two different `CARGO_TARGET_DIR` values
//! (`target-build-1` / `target-build-2`) and asserts the
//! resulting binaries are bit-identical. Until 2026-05-30 the
//! workflow failed on every push because
//! `crates/corvid-codegen-cl/build.rs` emitted
//! `cargo:rustc-env=CORVID_STATICLIB_DIR=<absolute target dir>`
//! and `link.rs` / `cdylib.rs` read it through `env!()`, baking a
//! `CARGO_TARGET_DIR`-dependent absolute path into the binary's
//! read-only data section. The fix replaced that compile-time
//! bake with runtime discovery
//! (`crate::staticlib_discovery::discover_staticlib`).
//!
//! The tests below assert the *structural* properties of the
//! fix — the build script no longer emits the host-dependent
//! env var, and the two consumer sites no longer read it via
//! `env!()`. They run in milliseconds and lock the regression
//! down: if a future change reintroduces the bake, these tests
//! fail before the CI workflow does.

use std::fs;
use std::path::PathBuf;

fn read_repo_relative(path: &str) -> String {
    // The corvid-codegen-cl crate sits at
    // `<workspace>/crates/corvid-codegen-cl`; walk up two levels
    // from CARGO_MANIFEST_DIR to reach the workspace root, then
    // join the requested path.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .ancestors()
        .nth(2)
        .expect("workspace root above crates/corvid-codegen-cl");
    let full = workspace.join(path);
    fs::read_to_string(&full).unwrap_or_else(|e| {
        panic!(
            "could not read `{}` for reproducibility regression check: {e}",
            full.display()
        )
    })
}

/// Positive: the build script's source MUST NOT emit
/// `cargo:rustc-env=CORVID_STATICLIB_DIR=...`. That emission
/// was the root cause of the CI mismatch — two builds with
/// different `CARGO_TARGET_DIR` values produced two different
/// embedded path strings. Promotes `deploy.reproducible_build`.
#[test]
fn build_script_emits_no_corvid_staticlib_dir_env_var() {
    let build_rs = read_repo_relative("crates/corvid-codegen-cl/build.rs");
    // We grep for the actual emission directive, not the bare
    // identifier, because the file's doc comment legitimately
    // mentions `CORVID_STATICLIB_DIR` when explaining why the
    // historical emission was removed.
    assert!(
        !build_rs.contains("cargo:rustc-env=CORVID_STATICLIB_DIR"),
        "`crates/corvid-codegen-cl/build.rs` emits \
         `cargo:rustc-env=CORVID_STATICLIB_DIR=…` — it must not, because \
         that bakes a CARGO_TARGET_DIR-dependent absolute path into the \
         corvid binary and breaks bit-identical rebuilds."
    );
    assert!(
        !build_rs.contains("cargo:rustc-env="),
        "`crates/corvid-codegen-cl/build.rs` emits at least one \
         `cargo:rustc-env=` directive — every such directive risks baking \
         host-dependent state into the binary. If a new compile-time env \
         var is required, route it through runtime discovery instead (see \
         `staticlib_discovery.rs`)."
    );
}

/// Positive: neither `link.rs` nor `cdylib.rs` reads
/// `CORVID_STATICLIB_DIR` via `env!()`. That macro embeds the
/// build-time value into the binary as a string literal; the
/// fix routes through runtime discovery instead. This test
/// locks the regression: if a future refactor reintroduces
/// `env!("CORVID_STATICLIB_DIR")` it fails immediately.
#[test]
fn link_and_cdylib_do_not_read_corvid_staticlib_dir_via_env_macro() {
    for source_rel in &[
        "crates/corvid-codegen-cl/src/link.rs",
        "crates/corvid-codegen-cl/src/cdylib.rs",
    ] {
        let body = read_repo_relative(source_rel);
        assert!(
            !body.contains("env!(\"CORVID_STATICLIB_DIR\")"),
            "`{source_rel}` reads CORVID_STATICLIB_DIR via env!() — it must \
             not, because env!() embeds the build-time value as a string \
             literal in the resulting binary and breaks bit-identical \
             rebuilds. Route through `staticlib_discovery::discover_staticlib` \
             at runtime instead."
        );
    }
}

/// Adversarial: discovery module is wired into `lib.rs` and the
/// two consumer sites both call into it. This is the positive
/// shape of the fix — without these wirings the previous tests
/// could pass while the runtime resolution path was effectively
/// dead. The test pins the call-site contract so a future
/// refactor that removes the wiring trips it.
#[test]
fn staticlib_discovery_module_is_wired_into_consumers() {
    let lib_rs = read_repo_relative("crates/corvid-codegen-cl/src/lib.rs");
    assert!(
        lib_rs.contains("staticlib_discovery"),
        "`crates/corvid-codegen-cl/src/lib.rs` does not declare \
         `staticlib_discovery` — the runtime resolution path is \
         unreachable from `link.rs` / `cdylib.rs`."
    );
    let link_rs = read_repo_relative("crates/corvid-codegen-cl/src/link.rs");
    assert!(
        link_rs.contains("staticlib_discovery::discover_staticlib"),
        "`link.rs` does not call `staticlib_discovery::discover_staticlib` — \
         the consumer site reverted to the baked-path path."
    );
    let cdylib_rs = read_repo_relative("crates/corvid-codegen-cl/src/cdylib.rs");
    assert!(
        cdylib_rs.contains("staticlib_discovery::discover_staticlib"),
        "`cdylib.rs` does not call `staticlib_discovery::discover_staticlib` — \
         the consumer site reverted to the baked-path path."
    );
}

/// Adversarial: the reproducible-build CI workflow file is
/// committed at the expected path. The registry's positive
/// test refs point at this test plus the structural ones
/// above, but the workflow remains the end-to-end SHA-256
/// oracle on Ubuntu; if someone deletes the workflow,
/// `deploy.reproducible_build` should not silently lose its
/// production-grade check.
#[test]
fn reproducible_build_workflow_file_exists_and_diffs_two_target_dirs() {
    let workflow = read_repo_relative(".github/workflows/reproducible-build.yml");
    assert!(workflow.contains("target-build-1"));
    assert!(workflow.contains("target-build-2"));
    assert!(
        workflow.contains("sha256sum"),
        "workflow does not invoke `sha256sum` — the bit-identity \
         oracle for `deploy.reproducible_build` is gone."
    );
}
