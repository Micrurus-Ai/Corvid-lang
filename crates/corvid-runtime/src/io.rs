use crate::errors::RuntimeError;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    pub path: PathBuf,
    pub contents: String,
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub effect: FileSystemEffect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWrite {
    pub path: PathBuf,
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub effect: FileSystemEffect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub effect: FileSystemEffect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSystemEffect {
    pub effect_tag: String,
    pub approval_label: String,
    pub replay_key: String,
}

pub struct TextLineStream {
    pub path: PathBuf,
    lines: Lines<BufReader<File>>,
    pub lines_read: u64,
    pub effect: FileSystemEffect,
}

#[derive(Clone, Default)]
pub struct IoRuntime {
    /// Slice `35V2-P38-C-5`: when `true`, `write_text` and
    /// `write_text_with_effect` short-circuit with
    /// `RuntimeError::QuarantineViolation { surface: "io", .. }`.
    /// Reads (`read_text`, `list_dir`, `open_line_stream`) pass
    /// through — they don't escape the process. Set by
    /// `RuntimeBuilder::build` when entering Substitute-mode replay.
    /// The runtime's own JSONL trace writer uses `JsonlTraceWriter`
    /// (not `IoRuntime`), so trace emission is unaffected.
    write_quarantined: bool,
}

impl IoRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// Flip write-quarantine on. Subsequent `write_text*` calls fail
    /// closed with `QuarantineViolation`.
    pub fn quarantine_writes(&mut self) {
        self.write_quarantined = true;
    }

    /// True when this IoRuntime refuses file writes. Test helper.
    pub fn is_write_quarantined(&self) -> bool {
        self.write_quarantined
    }

    fn quarantine_violation(path: &Path, op: &str) -> RuntimeError {
        RuntimeError::QuarantineViolation {
            surface: "io".to_string(),
            detail: format!(
                "blocked an unrecorded `{op}` on `{}` during replay-mode \
                 quarantine. Filesystem writes through `IoRuntime` cannot escape \
                 a replayed run; trace emission uses its own writer and is \
                 unaffected.",
                path.display()
            ),
        }
    }

    pub fn join_path(&self, base: impl AsRef<Path>, child: impl AsRef<Path>) -> PathBuf {
        base.as_ref().join(child.as_ref())
    }

    pub fn parent_path(&self, path: impl AsRef<Path>) -> Option<PathBuf> {
        path.as_ref().parent().map(Path::to_path_buf)
    }

    pub fn file_name(&self, path: impl AsRef<Path>) -> Option<String> {
        path.as_ref()
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
    }

    pub fn extension(&self, path: impl AsRef<Path>) -> Option<String> {
        path.as_ref()
            .extension()
            .map(|ext| ext.to_string_lossy().to_string())
    }

    pub fn with_extension(&self, path: impl AsRef<Path>, extension: &str) -> PathBuf {
        let mut out = path.as_ref().to_path_buf();
        out.set_extension(extension);
        out
    }

    pub fn normalize_path(&self, path: impl AsRef<Path>) -> PathBuf {
        normalize_path(path.as_ref())
    }

    pub async fn read_text(&self, path: impl AsRef<Path>) -> Result<FileRead, RuntimeError> {
        self.read_text_with_effect(path, Self::read_effect())
            .await
    }

    pub async fn read_text_with_effect(
        &self,
        path: impl AsRef<Path>,
        effect: FileSystemEffect,
    ) -> Result<FileRead, RuntimeError> {
        let path = path.as_ref().to_path_buf();
        let started = Instant::now();
        let contents = tokio::fs::read_to_string(&path).await.map_err(|err| {
            RuntimeError::ToolFailed {
                tool: "std.io".to_string(),
                message: format!("failed to read `{}`: {err}", path.display()),
            }
        })?;
        Ok(FileRead {
            bytes: contents.len() as u64,
            contents,
            path,
            elapsed_ms: elapsed_ms(started),
            effect,
        })
    }

    pub async fn write_text(
        &self,
        path: impl AsRef<Path>,
        contents: &str,
    ) -> Result<FileWrite, RuntimeError> {
        self.write_text_with_effect(path, contents, Self::write_effect())
            .await
    }

    pub async fn write_text_with_effect(
        &self,
        path: impl AsRef<Path>,
        contents: &str,
        effect: FileSystemEffect,
    ) -> Result<FileWrite, RuntimeError> {
        let path = path.as_ref().to_path_buf();
        if self.write_quarantined {
            return Err(Self::quarantine_violation(&path, "write_text"));
        }
        let started = Instant::now();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                RuntimeError::ToolFailed {
                    tool: "std.io".to_string(),
                    message: format!("failed to create `{}`: {err}", parent.display()),
                }
            })?;
        }
        tokio::fs::write(&path, contents).await.map_err(|err| {
            RuntimeError::ToolFailed {
                tool: "std.io".to_string(),
                message: format!("failed to write `{}`: {err}", path.display()),
            }
        })?;
        Ok(FileWrite {
            path,
            bytes: contents.len() as u64,
            elapsed_ms: elapsed_ms(started),
            effect,
        })
    }

    pub async fn list_dir(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<DirectoryEntry>, RuntimeError> {
        self.list_dir_with_effect(path, Self::list_effect()).await
    }

    pub async fn list_dir_with_effect(
        &self,
        path: impl AsRef<Path>,
        effect: FileSystemEffect,
    ) -> Result<Vec<DirectoryEntry>, RuntimeError> {
        let path = path.as_ref().to_path_buf();
        let mut entries = tokio::fs::read_dir(&path).await.map_err(|err| {
            RuntimeError::ToolFailed {
                tool: "std.io".to_string(),
                message: format!("failed to list `{}`: {err}", path.display()),
            }
        })?;
        let mut out = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|err| {
            RuntimeError::ToolFailed {
                tool: "std.io".to_string(),
                message: format!("failed to read directory entry in `{}`: {err}", path.display()),
            }
        })? {
            let file_type = entry.file_type().await.map_err(|err| RuntimeError::ToolFailed {
                tool: "std.io".to_string(),
                message: format!("failed to stat `{}`: {err}", entry.path().display()),
            })?;
            out.push(DirectoryEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                path: entry.path(),
                is_dir: file_type.is_dir(),
                effect: effect.clone(),
            });
        }
        out.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(out)
    }

    pub async fn open_line_stream(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<TextLineStream, RuntimeError> {
        self.open_line_stream_with_effect(path, Self::stream_effect())
            .await
    }

    pub async fn open_line_stream_with_effect(
        &self,
        path: impl AsRef<Path>,
        effect: FileSystemEffect,
    ) -> Result<TextLineStream, RuntimeError> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).await.map_err(|err| RuntimeError::ToolFailed {
            tool: "std.io".to_string(),
            message: format!("failed to open `{}` for streaming: {err}", path.display()),
        })?;
        Ok(TextLineStream {
            path,
            lines: BufReader::new(file).lines(),
            lines_read: 0,
            effect,
        })
    }

    pub fn read_effect() -> FileSystemEffect {
        FileSystemEffect {
            effect_tag: "std.io.read".to_string(),
            approval_label: String::new(),
            replay_key: "std.io.read".to_string(),
        }
    }

    pub fn write_effect() -> FileSystemEffect {
        FileSystemEffect {
            effect_tag: "std.io.write".to_string(),
            approval_label: "filesystem.write".to_string(),
            replay_key: "std.io.write".to_string(),
        }
    }

    pub fn list_effect() -> FileSystemEffect {
        FileSystemEffect {
            effect_tag: "std.io.list".to_string(),
            approval_label: String::new(),
            replay_key: "std.io.list".to_string(),
        }
    }

    pub fn stream_effect() -> FileSystemEffect {
        FileSystemEffect {
            effect_tag: "std.io.stream".to_string(),
            approval_label: String::new(),
            replay_key: "std.io.stream".to_string(),
        }
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                if !parts.is_empty() {
                    parts.pop();
                } else if normalized.as_os_str().is_empty() {
                    parts.push(component.as_os_str().to_os_string());
                }
            }
            Component::Normal(part) => parts.push(part.to_os_string()),
        }
    }

    for part in parts {
        normalized.push(part);
    }
    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

impl TextLineStream {
    pub async fn next_line(&mut self) -> Result<Option<String>, RuntimeError> {
        let line = self.lines.next_line().await.map_err(|err| RuntimeError::ToolFailed {
            tool: "std.io".to_string(),
            message: format!("failed to read streamed line from `{}`: {err}", self.path.display()),
        })?;
        if line.is_some() {
            self.lines_read += 1;
        }
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slice 35V2-P38-C-5: a write-quarantined `IoRuntime` refuses
    /// `write_text` with `QuarantineViolation { surface: "io", .. }`.
    /// Reads (`read_text`) continue to work — only writes escape.
    #[tokio::test]
    async fn quarantined_io_refuses_write_but_passes_through_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seeded.txt");
        let mut io = IoRuntime::new();
        // Seed a file BEFORE quarantining so the read finds it.
        io.write_text(&path, "seed").await.expect("seed write");
        io.quarantine_writes();
        assert!(io.is_write_quarantined());

        // read passes through.
        let read = io.read_text(&path).await.expect("read after quarantine");
        assert_eq!(read.contents, "seed");

        // write refuses.
        let new_path = dir.path().join("new.txt");
        let err = io
            .write_text(&new_path, "should not write")
            .await
            .expect_err("quarantined write must error");
        match err {
            RuntimeError::QuarantineViolation { surface, detail } => {
                assert_eq!(surface, "io");
                assert!(
                    detail.contains("new.txt"),
                    "detail should name the path: {detail}"
                );
                assert!(detail.contains("write_text"), "detail should name op: {detail}");
            }
            other => panic!("expected io QuarantineViolation, got {other:?}"),
        }
        // The new file should NOT exist on disk.
        assert!(
            !new_path.exists(),
            "quarantined write must not touch the filesystem: {new_path:?}"
        );
    }

    #[tokio::test]
    async fn io_runtime_writes_reads_and_lists_text_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("note.txt");
        let io = IoRuntime::new();

        let write = io.write_text(&path, "hello").await.unwrap();
        assert_eq!(write.bytes, 5);
        assert_eq!(write.effect.effect_tag, "std.io.write");

        let read = io.read_text(&path).await.unwrap();
        assert_eq!(read.contents, "hello");
        assert_eq!(read.bytes, 5);
        assert_eq!(read.effect.effect_tag, "std.io.read");

        let entries = io.list_dir(path.parent().unwrap()).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "note.txt");
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].effect.effect_tag, "std.io.list");
    }

    #[tokio::test]
    async fn io_runtime_streams_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let io = IoRuntime::new();
        let mut stream = io.open_line_stream(&path).await.unwrap();

        assert_eq!(stream.next_line().await.unwrap().as_deref(), Some("alpha"));
        assert_eq!(stream.next_line().await.unwrap().as_deref(), Some("beta"));
        assert_eq!(stream.next_line().await.unwrap(), None);
        assert_eq!(stream.lines_read, 2);
        assert_eq!(stream.effect.effect_tag, "std.io.stream");
    }

    #[test]
    fn io_runtime_manipulates_paths() {
        let io = IoRuntime::new();
        let joined = io.join_path("alpha", Path::new("beta").join("note.txt"));
        assert_eq!(joined, PathBuf::from("alpha").join("beta").join("note.txt"));
        assert_eq!(io.parent_path(&joined), Some(PathBuf::from("alpha").join("beta")));
        assert_eq!(io.file_name(&joined).as_deref(), Some("note.txt"));
        assert_eq!(io.extension(&joined).as_deref(), Some("txt"));
        assert_eq!(
            io.with_extension(&joined, "md"),
            PathBuf::from("alpha").join("beta").join("note.md")
        );
        assert_eq!(
            io.normalize_path(Path::new("alpha").join(".").join("beta").join("..").join("note.txt")),
            PathBuf::from("alpha").join("note.txt")
        );
    }
}
