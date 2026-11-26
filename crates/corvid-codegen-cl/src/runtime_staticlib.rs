//! Locating the `corvid-runtime` staticlib a native build links,
//! building it in a development checkout when it is missing.

use crate::errors::CodegenError;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn runtime_staticlib_path(compiler: &cc::Tool) -> Result<PathBuf, CodegenError> {
    // Tests can route through `CORVID_RUNTIME_STATICLIB_OVERRIDE` to
    // pick a Rust staticlib that already bundles `corvid-runtime`
    // transitively (e.g. `corvid_test_tools.lib`). Linking the
    // override on its own avoids the duplicate-`std` LNK2005 that
    // pairing it with the standalone `corvid_runtime.lib` would
    // produce on MSVC. Outside tests this stays unset and the
    // default lib is built and used.
    if let Some(override_path) = std::env::var_os("CORVID_RUNTIME_STATICLIB_OVERRIDE") {
        let path = PathBuf::from(override_path);
        if !path.exists() {
            return Err(CodegenError::link(format!(
                "CORVID_RUNTIME_STATICLIB_OVERRIDE points at non-existent path `{}`",
                path.display()
            )));
        }
        return Ok(path);
    }
    let runtime_lib_name = if compiler.is_like_msvc() {
        "corvid_runtime.lib"
    } else {
        "libcorvid_runtime.a"
    };
    // Fast path: the staticlib is already on disk in a location we
    // can discover. Skips the dev-mode auto-build entirely when it
    // isn't needed.
    if let Some(location) = crate::staticlib_discovery::discover_staticlib(runtime_lib_name) {
        return Ok(location.path);
    }
    // Slow path: no staticlib on disk. Fall back to the dev-mode
    // auto-build, writing into the canonical Cargo
    // `target/<profile>/` layout discovered from `current_exe()`.
    let staticlib_dir = crate::staticlib_discovery::guess_dev_target_profile_dir()
        .ok_or_else(|| CodegenError::link(
            "could not locate `corvid-runtime` staticlib and no `target/<profile>/` \
             directory derivable from the running binary's path. Set \
             `CORVID_STATICLIB_DIR` to the directory containing the staticlib \
             or `CORVID_RUNTIME_STATICLIB_OVERRIDE` to its full path."
        ))?;
    let runtime_staticlib_path = staticlib_dir.join(runtime_lib_name);
    build_runtime_staticlib(&staticlib_dir, &runtime_staticlib_path)?;
    if !runtime_staticlib_path.exists() {
        return Err(CodegenError::link(format!(
            "corvid-runtime staticlib missing at `{}` after auto-build.",
            runtime_staticlib_path.display()
        )));
    }
    Ok(runtime_staticlib_path)
}

fn build_runtime_staticlib(
    staticlib_dir: &Path,
    runtime_staticlib_path: &Path,
) -> Result<(), CodegenError> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .ancestors()
        .nth(2)
        .ok_or_else(|| CodegenError::link("locate workspace root for corvid-runtime build"))?;
    let profile = staticlib_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("-p").arg("corvid-runtime");
    if profile.eq_ignore_ascii_case("release") {
        cmd.arg("--release");
    }
    let output = cmd.current_dir(workspace_root).output().map_err(|err| {
        CodegenError::link(format!("spawn `cargo build -p corvid-runtime`: {err}"))
    })?;
    if !output.status.success() {
        return Err(CodegenError::link(format!(
            "auto-build of corvid-runtime staticlib for `{}` failed: {}{}",
            runtime_staticlib_path.display(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        )));
    }
    Ok(())
}
