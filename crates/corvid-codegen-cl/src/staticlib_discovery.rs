//! Runtime discovery of the `corvid-runtime` staticlib.
//!
//! For a long time the staticlib path was baked into the corvid
//! binary at compile time via
//! `cargo:rustc-env=CORVID_STATICLIB_DIR=<absolute path>` emitted
//! by `build.rs` and read through `env!()`. That worked for dev
//! workflows but had two real problems:
//!
//! 1. **It broke bit-identical rebuilds.** Two `cargo build` runs
//!    with different `CARGO_TARGET_DIR` values (the reproducible-
//!    build CI workflow uses `target-build-1` and `target-build-2`
//!    on purpose) baked two different absolute paths into the
//!    binary's read-only data section, so the SHA-256 differed
//!    even though the source, lockfile, and toolchain were
//!    identical. This is exactly what the `deploy.reproducible_build`
//!    runtime-checked guarantee promises against.
//! 2. **It baked a developer's host path into shipped binaries.**
//!    A user who installed the corvid CLI from a release artifact
//!    would have e.g. `/home/runner/work/Corvid-lang/Corvid-lang/target/release`
//!    in their binary's strings — meaningless on their host and a
//!    minor information leak.
//!
//! The discovery here resolves the staticlib path at *runtime*
//! using the binary's actual location, with a documented override
//! env var for explicit configuration. The build script no longer
//! emits any host-dependent strings.
//!
//! ## Resolution order
//!
//! 1. `CORVID_RUNTIME_STATICLIB_OVERRIDE` — explicit path to a
//!    staticlib file. Used by tests that need to substitute a
//!    bundled-runtime lib (e.g. `corvid_test_tools.lib`) to avoid
//!    the MSVC `LNK2005` duplicate-`std` failure when pairing two
//!    Rust staticlibs.
//! 2. `CORVID_STATICLIB_DIR` — explicit directory containing the
//!    expected staticlib filename. Used by integration tests and
//!    operators who ship the staticlib in a non-standard layout.
//! 3. Walk up from `current_exe().parent()`, checking each
//!    ancestor directory for the staticlib. Covers:
//!    - `cargo run --bin corvid` (binary at `target/<profile>/corvid`,
//!      staticlib at `target/<profile>/corvid_runtime.lib`).
//!    - `cargo test -p corvid-cli` (test binary at
//!      `target/<profile>/deps/<name>-<hash>`, staticlib at
//!      `target/<profile>/corvid_runtime.lib` one level up).
//!    - Shipped installs that place the binary and staticlib in
//!      the same directory (`<prefix>/bin/corvid` +
//!      `<prefix>/bin/corvid_runtime.lib`, or any layout that
//!      keeps them in the same dir).
//! 4. Documented sibling-dir layouts: `<exe_parent>/lib/`,
//!    `<exe_parent>/../lib/`, and `<exe_parent>/../lib/corvid/`.
//!    Covers shipped installs that follow the FHS convention of
//!    `<prefix>/bin/<exe>` + `<prefix>/lib/<lib>`.

use std::path::{Path, PathBuf};

/// Resolution outcome — successful path + which strategy it came
/// from, so callers can include the strategy in error messages
/// if the file later fails to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaticlibLocation {
    pub path: PathBuf,
    pub strategy: ResolutionStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolutionStrategy {
    OverrideEnvVar,
    DirEnvVar,
    WalkExeAncestors,
    SiblingLibDir,
}

impl ResolutionStrategy {
    pub(crate) fn description(self) -> &'static str {
        match self {
            ResolutionStrategy::OverrideEnvVar => "CORVID_RUNTIME_STATICLIB_OVERRIDE",
            ResolutionStrategy::DirEnvVar => "CORVID_STATICLIB_DIR",
            ResolutionStrategy::WalkExeAncestors => {
                "walked up from `current_exe()` ancestors"
            }
            ResolutionStrategy::SiblingLibDir => "sibling `lib/` directory next to `current_exe()`",
        }
    }
}

/// Discovers a path to the `corvid-runtime` staticlib given its
/// platform-specific filename (`corvid_runtime.lib` on MSVC,
/// `libcorvid_runtime.a` on Unix). Returns `None` if no strategy
/// finds it.
pub(crate) fn discover_staticlib(staticlib_name: &str) -> Option<StaticlibLocation> {
    // Override env var: takes a full path to a *file*.
    if let Some(override_path) = std::env::var_os("CORVID_RUNTIME_STATICLIB_OVERRIDE") {
        let path = PathBuf::from(override_path);
        if path.exists() {
            return Some(StaticlibLocation {
                path,
                strategy: ResolutionStrategy::OverrideEnvVar,
            });
        }
    }
    // Dir env var: takes a *directory* expected to contain the staticlib.
    if let Some(dir) = std::env::var_os("CORVID_STATICLIB_DIR") {
        let candidate = Path::new(&dir).join(staticlib_name);
        if candidate.exists() {
            return Some(StaticlibLocation {
                path: candidate,
                strategy: ResolutionStrategy::DirEnvVar,
            });
        }
    }
    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return None,
    };
    // Walk ancestors of the binary directory.
    let exe_parent = exe_path.parent()?;
    for ancestor in exe_parent.ancestors() {
        let candidate = ancestor.join(staticlib_name);
        if candidate.exists() {
            return Some(StaticlibLocation {
                path: candidate,
                strategy: ResolutionStrategy::WalkExeAncestors,
            });
        }
        // Stop walking after a reasonable depth to avoid touching
        // root-level directories.
        if ancestor.parent().is_none() {
            break;
        }
    }
    // Documented sibling-`lib/` layouts for shipped installs.
    let sibling_candidates = [
        exe_parent.join("lib").join(staticlib_name),
        exe_parent
            .parent()
            .map(|p| p.join("lib").join(staticlib_name))
            .unwrap_or_default(),
        exe_parent
            .parent()
            .map(|p| p.join("lib").join("corvid").join(staticlib_name))
            .unwrap_or_default(),
    ];
    for candidate in sibling_candidates {
        if candidate.as_os_str().is_empty() {
            continue;
        }
        if candidate.exists() {
            return Some(StaticlibLocation {
                path: candidate,
                strategy: ResolutionStrategy::SiblingLibDir,
            });
        }
    }
    None
}

/// Best-effort guess at the directory where a freshly-built
/// staticlib *should* land in a dev workflow, used as a write
/// target when `build_runtime_staticlib` auto-builds the lib
/// because it isn't already on disk. Walks up from
/// `current_exe()` looking for a `target/<profile>/` segment,
/// since that's the canonical Cargo layout. Falls back to the
/// nearest ancestor of `current_exe()` that has a `target`
/// sibling. Returns `None` if no plausible target dir is found.
pub(crate) fn guess_dev_target_profile_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut cur: Option<&Path> = exe.parent();
    while let Some(dir) = cur {
        if let Some(name) = dir.file_name() {
            let name_str = name.to_string_lossy();
            if name_str == "debug" || name_str == "release" {
                if let Some(parent) = dir.parent() {
                    if let Some(parent_name) = parent.file_name() {
                        if parent_name.to_string_lossy().starts_with("target") {
                            return Some(dir.to_path_buf());
                        }
                    }
                }
            }
        }
        cur = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Override env var takes precedence over every other strategy.
    /// This is the production override path tests rely on; we lock
    /// it down explicitly.
    #[test]
    fn override_env_var_wins_when_path_exists() {
        let temp = std::env::temp_dir().join("corvid_staticlib_override_test.lib");
        std::fs::write(&temp, b"stub").unwrap();
        // SAFETY: tests in this crate are single-threaded; we
        // restore the env var after the assertion.
        std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", &temp);
        std::env::remove_var("CORVID_STATICLIB_DIR");
        let resolved = discover_staticlib("corvid_runtime.lib");
        std::env::remove_var("CORVID_RUNTIME_STATICLIB_OVERRIDE");
        let _ = std::fs::remove_file(&temp);
        let resolved = resolved.expect("override env var should resolve");
        assert_eq!(resolved.strategy, ResolutionStrategy::OverrideEnvVar);
        assert_eq!(resolved.path, temp);
    }

    /// Dir env var resolves when set + the named file exists in
    /// that dir. Mirrors how integration tests or operators set
    /// the path explicitly.
    #[test]
    fn dir_env_var_resolves_when_file_exists_in_named_dir() {
        let dir = std::env::temp_dir().join("corvid_staticlib_dir_test");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("corvid_runtime.lib");
        std::fs::write(&file, b"stub").unwrap();
        std::env::remove_var("CORVID_RUNTIME_STATICLIB_OVERRIDE");
        std::env::set_var("CORVID_STATICLIB_DIR", &dir);
        let resolved = discover_staticlib("corvid_runtime.lib");
        std::env::remove_var("CORVID_STATICLIB_DIR");
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&dir);
        let resolved = resolved.expect("dir env var should resolve");
        assert_eq!(resolved.strategy, ResolutionStrategy::DirEnvVar);
    }

    /// Adversarial: an override pointing at a non-existent file
    /// must NOT count as resolved — discovery falls through to
    /// the next strategy rather than returning a path that won't
    /// open.
    #[test]
    fn override_env_var_pointing_at_missing_file_falls_through() {
        std::env::set_var(
            "CORVID_RUNTIME_STATICLIB_OVERRIDE",
            "/nonexistent/path/to/corvid_runtime.lib",
        );
        std::env::remove_var("CORVID_STATICLIB_DIR");
        let resolved = discover_staticlib("corvid_runtime.lib");
        std::env::remove_var("CORVID_RUNTIME_STATICLIB_OVERRIDE");
        // Whatever happens after fall-through is workspace-state-
        // dependent — we only care that the override didn't claim
        // success.
        if let Some(loc) = resolved {
            assert_ne!(loc.strategy, ResolutionStrategy::OverrideEnvVar);
        }
    }

    /// The `ResolutionStrategy::description` strings are stable
    /// human-readable identifiers that go into error messages —
    /// changing them silently is a regression. This locks down
    /// the strings.
    #[test]
    fn resolution_strategy_descriptions_are_stable() {
        assert_eq!(
            ResolutionStrategy::OverrideEnvVar.description(),
            "CORVID_RUNTIME_STATICLIB_OVERRIDE"
        );
        assert_eq!(
            ResolutionStrategy::DirEnvVar.description(),
            "CORVID_STATICLIB_DIR"
        );
        assert!(ResolutionStrategy::WalkExeAncestors
            .description()
            .contains("current_exe"));
        assert!(ResolutionStrategy::SiblingLibDir
            .description()
            .contains("lib/"));
    }
}
