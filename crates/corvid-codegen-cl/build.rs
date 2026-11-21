//! Identify the native backend that produced a cached executable.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn collect_sources(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_sources(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    // Watching the directory also covers files added or removed after a build.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
    let mut files = vec![PathBuf::from("Cargo.toml"), PathBuf::from("build.rs")];
    collect_sources(Path::new("src"), &mut files)?;
    files.sort();

    let mut hash = Sha256::new();
    hash.update(std::env::var("TARGET").expect("Cargo target triple"));
    for file in files {
        // Only crate-relative paths enter the digest.
        let path = file
            .to_str()
            .expect("UTF-8 backend source path")
            .replace('\\', "/");
        let bytes = std::fs::read(file)?;
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    println!(
        "cargo:rustc-env=CORVID_NATIVE_BACKEND_ID={:x}",
        hash.finalize()
    );
    Ok(())
}
