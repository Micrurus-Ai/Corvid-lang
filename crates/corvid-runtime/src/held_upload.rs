//! Uploads held on disk (large uploads held on disk, U2).
//!
//! A route declared `@upload(..., disk_over_mb: N)` holds a file larger
//! than N MB in a private temporary file instead of memory, so memory per
//! request is bounded by the threshold, not the maximum. Writing a
//! person's file to disk is a privacy choice the source makes; without
//! `disk_over_mb` nothing reaches this module.
//!
//! The rules the files keep:
//!
//! - **Private.** The directory is the server's own: on Unix created
//!   with mode `0700`, owned by the server's user, and refused when it is
//!   not (another user's, or readable by others). Each file is `0600` and
//!   named at random, never with the person's filename. On a platform
//!   where the server cannot yet prove a directory private, holding a
//!   file on disk refuses to start rather than assume.
//! - **Short-lived.** A file is deleted when the last value holding it is
//!   dropped, which for a request is when it ends, whether it succeeded,
//!   failed or was cancelled. A file being written is deleted if writing
//!   stops. Each server holds a lock on its own directory, and at start
//!   removes the directories whose server is gone, so a crash leaves
//!   nothing behind for long and two servers sharing a data directory
//!   never delete each other's files.
//! - **Read once.** The SHA-256 a trace records is computed while the
//!   file is written, so a large file is never read again to digest it.

use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const LOCK_FILE: &str = ".lock";

/// This server's private directory for held uploads.
#[derive(Debug)]
pub struct UploadDir {
    dir: PathBuf,
    /// Held for the server's life: the sweep of another server skips a
    /// directory whose lock is held.
    _lock: File,
}

impl UploadDir {
    /// Open this server's directory under `base`, made private, after
    /// removing the directories of servers that are gone. `base` is
    /// `CORVID_SERVE_DATA_DIR/uploads` when a data directory is
    /// configured, otherwise a per-user directory under the system's
    /// temporary directory ([`UploadDir::default_base`]).
    pub fn open(base: &Path) -> Result<Self, String> {
        private_dir(base)?;
        sweep(base);
        let dir = base.join(random_name()?);
        private_dir(&dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK_FILE))
            .map_err(|e| format!("cannot create `{}`: {e}", dir.join(LOCK_FILE).display()))?;
        lock.try_lock().map_err(|e| format!("cannot lock `{}`: {e}", dir.display()))?;
        Ok(UploadDir { dir, _lock: lock })
    }

    /// The base used without a data directory: a per-user directory
    /// under the system's temporary directory, so users never share one.
    pub fn default_base() -> Result<PathBuf, String> {
        let temp = std::env::temp_dir();
        Ok(temp.join(format!("corvid-uploads-{}", current_user_tag(&temp)?)))
    }

    /// The directory this server's files are written in.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// A new file to write one upload into.
    pub fn writer(&self) -> io::Result<HeldUploadWriter> {
        let path = self.dir.join(format!("{}.upload", random_name().map_err(io::Error::other)?));
        let file = private_file(&path)?;
        Ok(HeldUploadWriter { file: Some(file), path, hasher: Sha256::new(), size: 0 })
    }
}

impl Drop for UploadDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An upload being written. Dropped before [`HeldUploadWriter::finish`],
/// its file is deleted, so a refused or cancelled upload leaves nothing.
#[derive(Debug)]
pub struct HeldUploadWriter {
    file: Option<File>,
    path: PathBuf,
    hasher: Sha256,
    size: u64,
}

impl HeldUploadWriter {
    pub fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        let file = self.file.as_mut().ok_or_else(|| io::Error::other("the upload is already finished"))?;
        file.write_all(chunk)?;
        self.hasher.update(chunk);
        self.size += chunk.len() as u64;
        Ok(())
    }

    /// The bytes written so far.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The finished upload, which owns its file from here on.
    pub fn finish(mut self) -> io::Result<HeldUpload> {
        let mut file = self.file.take().ok_or_else(|| io::Error::other("the upload is already finished"))?;
        file.flush()?;
        drop(file);
        let sha256_hex = corvid_hex::encode(&std::mem::take(&mut self.hasher).finalize());
        Ok(HeldUpload { path: std::mem::take(&mut self.path), size: self.size, sha256_hex })
    }
}

impl Drop for HeldUploadWriter {
    fn drop(&mut self) {
        if self.file.take().is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A finished upload held on disk. Its file is deleted when the last
/// reference drops.
pub struct HeldUpload {
    path: PathBuf,
    size: u64,
    sha256_hex: String,
}

impl HeldUpload {
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The SHA-256 of the content, computed while it was written.
    pub fn sha256_hex(&self) -> &str {
        &self.sha256_hex
    }

    /// The content, read into memory because the program asked for it.
    pub fn read(&self) -> io::Result<Vec<u8>> {
        let mut content = Vec::with_capacity(usize::try_from(self.size).unwrap_or(0));
        File::open(&self.path)?.read_to_end(&mut content)?;
        Ok(content)
    }

    /// The file, for a caller that streams it instead of reading it whole.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Never prints the path: it names nothing the person sent, but a log
/// line has no use for it either.
impl std::fmt::Debug for HeldUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldUpload").field("size", &self.size).finish()
    }
}

impl Drop for HeldUpload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Remove the directories under `base` whose server is gone: one whose
/// lock can be taken, or that has no lock file at all.
fn sweep(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let gone = match OpenOptions::new().write(true).open(dir.join(LOCK_FILE)) {
            Ok(lock) => lock.try_lock().is_ok(),
            Err(e) => e.kind() == io::ErrorKind::NotFound,
        };
        if gone {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

fn random_name() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    corvid_crypto::fill_random(&mut bytes).map_err(|e| format!("cannot draw a random file name: {e}"))?;
    Ok(corvid_hex::encode(&bytes))
}

#[cfg(unix)]
fn private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let show = dir.display();
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("cannot create `{show}` for uploads held on disk: {e}"))?;
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("cannot read `{show}`: {e}"))?;
    if !meta.is_dir() {
        return Err(format!("`{show}` is not a directory, so uploads cannot be held in it privately"));
    }
    let owner = meta.uid();
    let mine = current_uid(dir.parent().unwrap_or(dir))?;
    if owner != mine {
        return Err(format!(
            "`{show}` belongs to another user, so uploads held there would not be private; give the server its own directory"
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot make `{show}` private: {e}"))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn private_dir(dir: &Path) -> Result<(), String> {
    Err(format!(
        "holding uploads on disk needs a directory the server can prove private, and on this platform it cannot check `{}` yet; remove `disk_over_mb` or serve from Linux or macOS",
        dir.display()
    ))
}

#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// The server's own user id: the owner of a file it creates. Std has no
/// call for the effective user id, and a file the process just made
/// cannot be owned by anyone else.
#[cfg(unix)]
fn current_uid(writable: &Path) -> Result<u32, String> {
    use std::os::unix::fs::MetadataExt;
    let probe = writable.join(format!(".corvid-probe-{}", random_name()?));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| format!("cannot create a file in `{}`: {e}", writable.display()))?;
    let uid = file.metadata().map(|m| m.uid());
    drop(file);
    let _ = std::fs::remove_file(&probe);
    uid.map_err(|e| format!("cannot read `{}`: {e}", probe.display()))
}

#[cfg(unix)]
fn current_user_tag(temp: &Path) -> Result<String, String> {
    current_uid(temp).map(|uid| uid.to_string())
}

#[cfg(not(unix))]
fn current_user_tag(_temp: &Path) -> Result<String, String> {
    Ok(std::env::var("USERNAME").unwrap_or_else(|_| "user".into()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_held_upload_is_private_digested_once_and_deleted_when_dropped() {
        let base = tempfile::tempdir().unwrap();
        let dir = UploadDir::open(&base.path().join("uploads")).unwrap();
        assert_eq!(mode(dir.path()), 0o700);
        let mut writer = dir.writer().unwrap();
        writer.write(b"hello ").unwrap();
        writer.write(b"world").unwrap();
        let held = writer.finish().unwrap();
        assert_eq!(mode(held.path()), 0o600);
        assert_eq!(held.size(), 11);
        assert_eq!(held.read().unwrap(), b"hello world");
        assert_eq!(held.sha256_hex(), corvid_crypto::sha256(&corvid_crypto::Bytes::new(b"hello world")).to_hex());
        let path = held.path().to_path_buf();
        drop(held);
        assert!(!path.exists(), "the file is gone with its last reference");
    }

    #[test]
    fn an_unfinished_upload_leaves_no_file() {
        let base = tempfile::tempdir().unwrap();
        let dir = UploadDir::open(&base.path().join("uploads")).unwrap();
        let mut writer = dir.writer().unwrap();
        writer.write(b"half a file").unwrap();
        let path = writer.path.clone();
        drop(writer);
        assert!(!path.exists());
    }

    #[test]
    fn start_removes_a_gone_servers_files_but_never_a_running_ones() {
        let base = tempfile::tempdir().unwrap();
        let uploads = base.path().join("uploads");
        let running = UploadDir::open(&uploads).unwrap();
        let crashed = uploads.join("crashed");
        std::fs::create_dir_all(&crashed).unwrap();
        std::fs::write(crashed.join("left.upload"), b"from a crash").unwrap();
        let next = UploadDir::open(&uploads).unwrap();
        assert!(!crashed.exists(), "a directory with no live server is swept");
        assert!(running.path().exists() && next.path().exists(), "a running server's directory is kept");
        let gone = running.path().to_path_buf();
        drop(running);
        assert!(!gone.exists(), "a server removes its own directory when it stops");
    }

    #[test]
    fn a_directory_open_to_others_is_made_private() {
        let base = tempfile::tempdir().unwrap();
        let uploads = base.path().join("uploads");
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::set_permissions(&uploads, std::fs::Permissions::from_mode(0o777)).unwrap();
        UploadDir::open(&uploads).unwrap();
        assert_eq!(mode(&uploads), 0o700);
    }
}
