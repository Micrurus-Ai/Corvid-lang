//! Locating the `corvid-runtime` staticlib a native build links, and
//! checking that it holds the runtime this compiler was built against.
//!
//! Every archive holding the runtime exports
//! `corvid_runtime::runtime_stamp::RUNTIME_STAMP_SYMBOL`, named after a
//! stamp of the runtime's sources. Before a link, the archive's symbol
//! index is searched for this compiler's stamp. A stale archive in the
//! development target directory is rebuilt; any other stale archive is
//! refused, never linked, because a runtime built from other sources
//! can lack an export the generated code calls or disagree with it.
//! Binary and library links both resolve the archive here.

use crate::errors::CodegenError;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::SystemTime;

/// The runtime archive to link, checked against this compiler's stamp.
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
        if !holds_this_runtime(&path)? {
            return Err(CodegenError::link(stale_override_diagnostic(&path)));
        }
        return Ok(path);
    }
    let runtime_lib_name = if compiler.is_like_msvc() {
        "corvid_runtime.lib"
    } else {
        "libcorvid_runtime.a"
    };
    let dev_dir = crate::staticlib_discovery::guess_dev_target_profile_dir();
    // Fast path: the staticlib is already on disk in a location we
    // can discover, and holds this compiler's runtime.
    if let Some(location) = crate::staticlib_discovery::discover_staticlib(runtime_lib_name) {
        if holds_this_runtime(&location.path)? {
            return Ok(location.path);
        }
        let in_dev_dir = dev_dir.as_deref().is_some_and(|dir| location.path.parent() == Some(dir));
        if !in_dev_dir {
            return Err(CodegenError::link(stale_archive_diagnostic(&location.path, location.strategy.description())));
        }
    }
    // Slow path: no staticlib on disk, or a stale one in the
    // development target directory. Build it into the canonical Cargo
    // `target/<profile>/` layout discovered from `current_exe()`.
    let staticlib_dir = dev_dir.ok_or_else(|| {
        CodegenError::link(crate::link::missing_staticlib_diagnostic(Path::new(runtime_lib_name)))
    })?;
    let runtime_staticlib_path = staticlib_dir.join(runtime_lib_name);
    build_runtime_staticlib(&staticlib_dir, &runtime_staticlib_path)?;
    if !runtime_staticlib_path.exists() {
        return Err(CodegenError::link(format!(
            "corvid-runtime staticlib missing at `{}` after auto-build.",
            runtime_staticlib_path.display()
        )));
    }
    if !holds_this_runtime(&runtime_staticlib_path)? {
        return Err(CodegenError::link(format!(
            "the corvid-runtime staticlib rebuilt at `{}` still holds a different runtime than this compiler was built with, so the compiler is older than this checkout's runtime sources. Rebuild the compiler too (`cargo build -p corvid-cli`).",
            runtime_staticlib_path.display()
        )));
    }
    Ok(runtime_staticlib_path)
}

fn stale_override_diagnostic(path: &Path) -> String {
    format!(
        "`{}` (CORVID_RUNTIME_STATICLIB_OVERRIDE) holds a different corvid-runtime than this compiler was built with, so linking it could miss or misuse a runtime export. Rebuild the library that bundles the runtime (for the test tools, `cargo build -p corvid-test-tools --release`).",
        path.display()
    )
}

fn stale_archive_diagnostic(path: &Path, found_by: &str) -> String {
    format!(
        "the corvid-runtime staticlib at `{}` (found through {found_by}) holds a different runtime than this compiler was built with, so linking it could miss or misuse a runtime export. Install the staticlib that ships with this compiler, or rebuild it from the same sources (`cargo build -p corvid-runtime`).",
        path.display()
    )
}

/// Archives already checked in this process, by path, size and change
/// time, so a test that links many binaries reads each index once.
static CHECKED: Mutex<Option<HashMap<PathBuf, (u64, Option<SystemTime>, bool)>>> = Mutex::new(None);

/// Whether the archive at `path` holds the runtime this compiler was
/// built against.
fn holds_this_runtime(path: &Path) -> Result<bool, CodegenError> {
    let metadata = std::fs::metadata(path)
        .map_err(|err| CodegenError::link(format!("read `{}`: {err}", path.display())))?;
    let identity = (metadata.len(), metadata.modified().ok());
    let mut checked = CHECKED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let checked = checked.get_or_insert_with(HashMap::new);
    if let Some(&(len, modified, holds)) = checked.get(path) {
        if (len, modified) == identity {
            return Ok(holds);
        }
    }
    let holds = archive_holds_symbol(path, corvid_runtime::runtime_stamp::RUNTIME_STAMP_SYMBOL)
        .map_err(|err| CodegenError::link(format!("read the symbol index of `{}`: {err}", path.display())))?;
    checked.insert(path.to_path_buf(), (identity.0, identity.1, holds));
    Ok(holds)
}

/// Whether an `ar` archive (Unix `.a` or MSVC `.lib`) names `symbol`.
/// Reads only the archive's first member when it is the symbol index
/// (`/`, `/SYM64/` or `__.SYMDEF`), which lists every symbol the
/// archive defines; otherwise searches the whole archive.
pub(crate) fn archive_holds_symbol(path: &Path, symbol: &str) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0u8; 8];
    if file.read_exact(&mut magic).is_err() || &magic != b"!<arch>\n" {
        return Ok(false);
    }
    let mut header = [0u8; 60];
    file.read_exact(&mut header)?;
    let name = String::from_utf8_lossy(&header[..16]).trim_end().to_string();
    let size: u64 = String::from_utf8_lossy(&header[48..58]).trim().parse().unwrap_or(0);
    let mut member = Vec::new();
    (&mut file).take(size).read_to_end(&mut member)?;
    let is_index = name == "/" || name == "/SYM64/" || name.starts_with("__.SYMDEF") || (name.starts_with("#1/") && member.starts_with(b"__.SYMDEF"));
    if !is_index {
        file.read_to_end(&mut member)?;
    }
    let needle = symbol.as_bytes();
    Ok(member.windows(needle.len()).any(|window| window == needle))
}

#[cfg(test)]
mod tests {
    use super::archive_holds_symbol;

    fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = b"!<arch>\n".to_vec();
        for (name, data) in members {
            bytes.extend(format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", 0, 0, 0, 644, data.len()).as_bytes());
            bytes.extend(*data);
            if data.len() % 2 == 1 {
                bytes.push(b'\n');
            }
        }
        bytes
    }

    fn holds(bytes: &[u8], symbol: &str) -> bool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.a");
        std::fs::write(&path, bytes).unwrap();
        archive_holds_symbol(&path, symbol).unwrap()
    }

    #[test]
    fn the_symbol_index_decides() {
        let current = archive(&[("/", b"\0\0\0\x01corvid_runtime_stamp_new\0"), ("alloc.o/", b"code")]);
        assert!(holds(&current, "corvid_runtime_stamp_new"));
        assert!(!holds(&current, "corvid_runtime_stamp_old"));
    }

    #[test]
    fn an_archive_without_an_index_is_searched_whole() {
        let unindexed = archive(&[("alloc.o/", b"code"), ("stamp.o/", b"corvid_runtime_stamp_new")]);
        assert!(holds(&unindexed, "corvid_runtime_stamp_new"));
    }

    #[test]
    fn a_file_that_is_not_an_archive_holds_nothing() {
        assert!(!holds(b"not an archive", "corvid_runtime_stamp_new"));
    }
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
