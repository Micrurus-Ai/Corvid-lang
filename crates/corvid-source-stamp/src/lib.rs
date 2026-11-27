//! A stamp of the sources a crate is built from, for build scripts.
//!
//! [`closure_stamp`] hashes the calling crate and every workspace crate
//! it depends on, transitively: each one's `Cargo.toml`, `build.rs`,
//! `src` and `runtime` (C sources), with paths relative to the
//! workspace root, so two checkouts of the same sources give the same
//! stamp. Tests, examples and benchmarks are left out, so editing a
//! test does not change the stamp or rebuild the crate.
//!
//! `corvid-runtime` exports its stamp as a symbol, so a native build
//! can check that the archive it links holds the runtime the code
//! generator was built against. `corvid-codegen-cl` uses its own as the
//! native backend id, part of the native cache key.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The parts of a crate that go into the binaries built from it.
const STAMPED_ENTRIES: &[&str] = &["Cargo.toml", "build.rs", "src", "runtime"];

/// Stamp the crate whose build script is running and its workspace
/// dependencies, and tell Cargo to rerun the build script when any
/// stamped file changes. Returns the stamp as lowercase hex.
///
/// Panics, failing the build loudly, when the crate is not in a
/// workspace laid out as `crates/<name>` or a dependency's directory
/// is missing: a stamp that silently skipped a crate would vouch for
/// sources it never read.
pub fn closure_stamp() -> String {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let crates_dir = manifest_dir.parent().expect("crate inside `crates/`").to_path_buf();
    let workspace_root = crates_dir.parent().expect("`crates/` inside the workspace").to_path_buf();
    let own_name = manifest_dir.file_name().and_then(|name| name.to_str()).expect("UTF-8 crate directory name").to_string();

    let closure = dependency_closure(&crates_dir, &own_name);
    let mut hash = Sha256::new();
    hash.update(std::env::var("TARGET").unwrap_or_default());
    for name in &closure {
        let crate_dir = crates_dir.join(name);
        for entry in STAMPED_ENTRIES {
            let path = crate_dir.join(entry);
            if !path.exists() {
                continue;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            let mut files = Vec::new();
            collect_files(&path, &mut files);
            files.sort();
            for file in files {
                let relative = file
                    .strip_prefix(&workspace_root)
                    .expect("stamped file inside the workspace")
                    .to_str()
                    .expect("UTF-8 source path")
                    .replace('\\', "/");
                let bytes = std::fs::read(&file).unwrap_or_else(|err| panic!("read `{}` for the source stamp: {err}", file.display()));
                hash.update((relative.len() as u64).to_le_bytes());
                hash.update(relative.as_bytes());
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(&bytes);
            }
        }
    }
    format!("{:x}", hash.finalize())
}

/// The crate named `root` and every workspace crate it depends on
/// (normal and target-specific dependencies), in name order.
fn dependency_closure(crates_dir: &Path, root: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root.to_string()];
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let manifest = crates_dir.join(&name).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|err| panic!("workspace crate `{name}` has no manifest at `{}`: {err}", manifest.display()));
        pending.extend(workspace_dependencies(&text).into_iter().filter(|dep| !seen.contains(dep)));
    }
    seen
}

/// The `corvid-*` crates a manifest lists as normal or target-specific
/// dependencies. Development and build dependencies are not part of
/// what a binary is built from.
fn workspace_dependencies(manifest: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            let header = header.trim();
            let (section, table_dep) = match header.rsplit_once("dependencies.") {
                Some((prefix, dep)) => (format!("{prefix}dependencies"), Some(dep.trim().to_string())),
                None => (header.to_string(), None),
            };
            let dependency_section = section == "dependencies" || (section.starts_with("target.") && section.ends_with(".dependencies"));
            // A `[dependencies.name]` table names one dependency; its
            // lines are that dependency's keys, not further entries.
            in_dependencies = dependency_section && table_dep.is_none();
            if let Some(dep) = table_dep.filter(|dep| dependency_section && dep.starts_with("corvid-")) {
                deps.push(dep);
            }
            continue;
        }
        if !in_dependencies {
            continue;
        }
        let key = line.split(['=', '.', ' ']).next().unwrap_or_default();
        if key.starts_with("corvid-") {
            deps.push(key.to_string());
        }
    }
    deps
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        files.push(path.to_path_buf());
        return;
    }
    let entries = std::fs::read_dir(path).unwrap_or_else(|err| panic!("read `{}` for the source stamp: {err}", path.display()));
    for entry in entries {
        let entry = entry.expect("directory entry");
        collect_files(&entry.path(), files);
    }
}

#[cfg(test)]
mod tests {
    use super::workspace_dependencies;

    #[test]
    fn normal_and_target_dependencies_count_and_dev_and_build_dependencies_do_not() {
        let manifest = r#"
[package]
name = "corvid-runtime"

[build-dependencies]
corvid-build-only = { workspace = true }

[dependencies]
corvid-ast = { workspace = true }
corvid-ir.workspace = true
serde = "1"

[dependencies.corvid-tabled]
workspace = true

[dependencies.pyo3]
version = "0.22"

[target.'cfg(unix)'.dependencies]
corvid-unix-only = { workspace = true }

[dev-dependencies]
corvid-vm = { workspace = true }
"#;
        assert_eq!(
            workspace_dependencies(manifest),
            vec!["corvid-ast", "corvid-ir", "corvid-tabled", "corvid-unix-only"]
        );
    }
}
