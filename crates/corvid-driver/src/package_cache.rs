use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) fn offline() -> bool {
    std::env::var("CORVID_OFFLINE").as_deref() == Ok("1")
}

/// Callers supply a validated SHA-256 from the lock or the verified artifact.
pub(crate) fn artifact(project: &Path, digest: &str) -> PathBuf {
    project
        .join("target/cache/packages")
        .join(format!("{}.cor", digest.to_ascii_lowercase()))
}

pub(crate) fn store(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().expect("package cache directory");
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.persist(path)?;
    Ok(())
}
