//! Build script for `corvid-codegen-cl`.
//!
//! Historically this script emitted `CORVID_STATICLIB_DIR=<absolute
//! target dir path>` as a compile-time env var so that `link.rs` and
//! `cdylib.rs` could find the `corvid-runtime` staticlib via
//! `env!()`. That worked for dev workflows but baked an absolute
//! path into the binary's read-only data section, which:
//!
//! 1. broke bit-identical rebuilds with different `CARGO_TARGET_DIR`
//!    values — the reproducible-build CI workflow uses two separate
//!    target dirs on purpose and would observe a SHA-256 mismatch
//!    from the embedded path alone (the
//!    `deploy.reproducible_build` runtime-checked guarantee), and
//! 2. shipped a developer's host path into release binaries.
//!
//! The staticlib path is now resolved at runtime via
//! `staticlib_discovery::discover_staticlib`. This build script
//! intentionally emits nothing host-dependent — it only declares
//! its own rerun dependency.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
