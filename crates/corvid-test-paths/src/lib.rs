//! Where the workspace and Cargo's build output live, for integration
//! tests.
//!
//! Integration tests that link Corvid-compiled programs need artifacts
//! Cargo built, such as the `corvid-test-tools` staticlib. Those tests
//! used to look for them under `<workspace>/target`, which is wrong
//! whenever the build directory is elsewhere: `CARGO_TARGET_DIR`, a
//! `build.target-dir` setting, or a checkout on a slow filesystem built
//! into a fast one. A `cargo build` the test starts honors those
//! settings, so the test must look where that build actually wrote.
//!
//! [`cargo_target_dir`] resolves the directory in this order:
//!
//! 1. `CARGO_TARGET_DIR`, which a child `cargo build` inherits. A
//!    relative value is taken from the workspace root, because the tests
//!    run their child builds there.
//! 2. The nearest ancestor of the running test binary that holds
//!    `CACHEDIR.TAG`, the marker Cargo writes at the root of every
//!    target directory. This covers `build.target-dir` in a Cargo
//!    config, which no environment variable reports.
//! 3. `<workspace>/target`, Cargo's default.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The workspace root: the directory holding the top-level `Cargo.toml`.
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("corvid-test-paths lives at <workspace>/crates/corvid-test-paths")
        .to_path_buf()
}

/// Cargo's target directory for this build. See the module docs for
/// the resolution order.
pub fn cargo_target_dir() -> PathBuf {
    resolve_target_dir(
        std::env::var_os("CARGO_TARGET_DIR"),
        std::env::current_exe().ok(),
        &workspace_root(),
    )
}

/// The release profile directory, where `cargo build --release` writes.
pub fn release_dir() -> PathBuf {
    cargo_target_dir().join("release")
}

/// The platform's file name for the `corvid-test-tools` staticlib.
pub fn test_tools_staticlib_name() -> &'static str {
    if cfg!(windows) {
        "corvid_test_tools.lib"
    } else {
        "libcorvid_test_tools.a"
    }
}

/// Where `cargo build -p corvid-test-tools --release` writes its
/// staticlib.
pub fn test_tools_staticlib() -> PathBuf {
    release_dir().join(test_tools_staticlib_name())
}

fn resolve_target_dir(
    env_value: Option<OsString>,
    current_exe: Option<PathBuf>,
    workspace_root: &Path,
) -> PathBuf {
    if let Some(value) = env_value.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        return if path.is_absolute() {
            path
        } else {
            workspace_root.join(path)
        };
    }
    if let Some(exe) = current_exe {
        if let Some(dir) = exe
            .ancestors()
            .skip(1)
            .find(|dir| dir.join("CACHEDIR.TAG").is_file())
        {
            return dir.to_path_buf();
        }
    }
    workspace_root.join("target")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("corvid-test-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn an_absolute_cargo_target_dir_is_used_as_given() {
        let root = scratch("absolute");
        let chosen = root.join("elsewhere");
        let resolved = resolve_target_dir(Some(chosen.clone().into()), None, &root);
        assert_eq!(resolved, chosen);
    }

    #[test]
    fn a_relative_cargo_target_dir_is_taken_from_the_workspace_root() {
        let root = scratch("relative");
        let resolved = resolve_target_dir(Some(OsString::from("build-out")), None, &root);
        assert_eq!(resolved, root.join("build-out"));
    }

    #[test]
    fn without_the_variable_the_marked_ancestor_of_the_binary_wins() {
        let root = scratch("marker");
        let target = root.join("custom-target");
        let deps = target.join("debug").join("deps");
        std::fs::create_dir_all(&deps).expect("create deps");
        std::fs::write(target.join("CACHEDIR.TAG"), b"Signature: 8a477f597d28d172789f06886806bc55").expect("tag");
        let resolved = resolve_target_dir(None, Some(deps.join("some_test-0123")), &root);
        assert_eq!(resolved, target);
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        let root = scratch("empty");
        let resolved = resolve_target_dir(Some(OsString::new()), None, &root);
        assert_eq!(resolved, root.join("target"));
    }

    #[test]
    fn with_nothing_to_go_on_the_default_is_the_workspace_target() {
        let root = scratch("default");
        let resolved = resolve_target_dir(None, Some(root.join("bin").join("tool")), &root);
        assert_eq!(resolved, root.join("target"));
    }

    #[test]
    fn this_test_binary_runs_inside_the_resolved_target_dir() {
        let exe = std::env::current_exe().expect("current exe");
        let target = cargo_target_dir();
        let exe = exe.canonicalize().unwrap_or(exe);
        let target = target.canonicalize().unwrap_or(target);
        assert!(exe.starts_with(&target), "{} is not under {}", exe.display(), target.display());
    }
}

/// The `corvid-test-tools` staticlib, built for this checkout, set as the
/// runtime archive native builds in this process link
/// (`CORVID_RUNTIME_STATICLIB_OVERRIDE`). It bundles the runtime, so a
/// native test links the runtime its compiler was built from.
///
/// Built at most once per test process, on first use. Before, each test
/// file carried its own copy of this helper: some built the library and
/// some did not, so a test could link a stale one (which the runtime stamp
/// now refuses). And tests in one process each ran `cargo build`, so one
/// could link while another's build was rewriting the library.
pub fn linked_test_tools_staticlib() -> PathBuf {
    static BUILT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let status = std::process::Command::new(cargo)
            .args(["build", "-p", "corvid-test-tools", "--release"])
            .current_dir(workspace_root())
            .status()
            .expect("run cargo build -p corvid-test-tools");
        assert!(status.success(), "building corvid-test-tools failed");
    });
    let path = test_tools_staticlib();
    // SAFETY: tests set this before linking; every caller sets the same value.
    unsafe {
        std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", &path);
    }
    path
}
