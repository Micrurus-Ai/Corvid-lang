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

// ============================================================================
// Phase 33S1a — IoToolPolicy.
//
// The executing `io.read_text` / `io.write_text` / `io.list_dir`
// stdlib tools (declared in `std/io.cor`) carry their security
// boundary in this policy struct. The `Runtime` holds a single
// `IoToolPolicy`; the `Runtime::call_tool` interception path for
// `io.*` tool names threads each call's `path` argument through
// `IoToolPolicy::resolve` before any `IoRuntime` method runs.
//
// The policy is built from `corvid.toml`'s `[io] root` field
// (parsed in 33S0). Two semantics:
//
//   * `root` is a relative path → resolves against the directory
//     containing `corvid.toml` (the loader passes that anchor in
//     via `IoToolPolicy::new`). Reproducible across hosts.
//   * `root` is absolute → taken as-is. Operators pointing at
//     `/var/lib/myapp/data` get exactly that.
//
// Inside `resolve`:
//   * Normalize `path` (collapse `.` and `..` segments).
//   * If the resolved path escapes `root` (lexical comparison),
//     reject with `RuntimeError::ToolFailed` naming the
//     attempted path AND the root so the operator can see both.
//   * Otherwise return the resolved absolute path the `IoRuntime`
//     methods can use directly.
//
// When `root` is `None` (no `[io] root` configured), every
// `resolve` call returns the "missing config" error. This is the
// fail-closed contract 33S0 prepared for.
// ============================================================================

/// Phase 33S1c — anchor for the path-confinement guarantee.
/// `IoToolPolicy::resolve` is the enforcement site: every
/// executing file-I/O call resolves through this policy and is
/// refused if it would escape the configured `[io] root` (or if
/// no root is configured). The `corvid-guarantees` inverse-
/// coverage sentinel uses this anchor to confirm the registry
/// row is wired to the enforcement code.
pub const GUARANTEE_ID_IO_SOURCE_FS_PATH_CONFINEMENT: &str =
    "io_source.fs_path_confinement";

/// Phase 33S1c — anchor for the write-quarantine guarantee.
/// `IoRuntime::write_text_with_effect` (low-level) AND the
/// `Runtime::call_tool("io.write_text", ...)` dispatch path BOTH
/// honour replay-mode write-quarantine: the low-level path
/// returns QuarantineViolation directly when `write_quarantined`
/// is set; the dispatch path goes through replay-substitution
/// first so any write either substitutes from the recorded trace
/// OR diverges — never reaching the live filesystem.
pub const GUARANTEE_ID_IO_SOURCE_FS_WRITE_QUARANTINE_ON_REPLAY: &str =
    "io_source.fs_write_quarantine_on_replay";

/// Phase 33S1c — anchor for the read-passthrough/gated guarantee.
/// `IoRuntime::read_text_with_effect` (low-level) passes
/// through during replay (reads don't escape the process);
/// `Runtime::call_tool("io.read_text", ...)` goes through
/// replay-substitution so dispatch-path reads either substitute
/// from the trace OR diverge when no event matches — they
/// never reach the live filesystem unless the trace prescribed
/// it.
pub const GUARANTEE_ID_IO_SOURCE_FS_READ_QUARANTINE_ON_REPLAY: &str =
    "io_source.fs_read_quarantine_on_replay";

/// Policy carrying the configured `[io] root` for the executing
/// file-I/O surface. Construct via `IoToolPolicy::new` once per
/// `Runtime` and store on the runtime; the `io.*` tool dispatch
/// interception calls `resolve` per request.
#[derive(Debug, Clone, Default)]
pub struct IoToolPolicy {
    /// Resolved root directory. `None` means no `[io] root` was
    /// configured in `corvid.toml`; every `resolve` call returns
    /// the missing-config error. `Some(path)` is an absolute,
    /// normalised directory paths must stay under.
    root: Option<PathBuf>,
}

impl IoToolPolicy {
    /// Build a policy from the parsed `[io] root` value + the
    /// directory containing the source `corvid.toml`. Relative
    /// roots resolve against `corvid_toml_dir`; absolute roots
    /// are taken as-is. Normalises the resolved path.
    pub fn new(root_value: Option<&str>, corvid_toml_dir: Option<&Path>) -> Self {
        let root = root_value.map(|raw| {
            let raw_path = Path::new(raw);
            let anchored = if raw_path.is_absolute() {
                raw_path.to_path_buf()
            } else {
                match corvid_toml_dir {
                    Some(anchor) => anchor.join(raw_path),
                    None => raw_path.to_path_buf(),
                }
            };
            // The root MUST end up absolute: `resolve`'s
            // confinement check is a component-prefix comparison,
            // and a still-relative root (a relative corvid.toml
            // anchor from `corvid run src/main.cor`, or a relative
            // CORVID_IO_ROOT) false-fires it on every path.
            let absolute = if anchored.is_absolute() {
                anchored
            } else {
                match std::env::current_dir() {
                    Ok(cwd) => cwd.join(&anchored),
                    Err(_) => anchored,
                }
            };
            normalize_path(&absolute)
        });
        Self { root }
    }

    /// Empty policy — no `[io] root` configured. Every call to
    /// `resolve` returns the missing-config error. Convenience
    /// for tests + the default Runtime construction path that
    /// hasn't been given a policy yet.
    pub fn unset() -> Self {
        Self { root: None }
    }

    /// True when the policy has a configured root. False when
    /// `[io] root` was absent from `corvid.toml`.
    pub fn is_configured(&self) -> bool {
        self.root.is_some()
    }

    /// Resolve a caller-supplied `path` against the configured
    /// root. Steps:
    ///
    ///   1. Refuse the call if no root is configured (fail-closed
    ///      contract for the missing-`[io] root` case).
    ///   2. Join the caller path onto the root (caller path is
    ///      treated as relative to the root regardless of its
    ///      absolute/relative shape — absolute paths in caller
    ///      input would escape confinement by construction, so
    ///      we strip the leading `/` before joining).
    ///   3. Normalise (collapse `.` / `..` segments).
    ///   4. Refuse if the normalised path escapes the root via
    ///      a lexical prefix check.
    ///   5. Return the resolved absolute path.
    pub fn resolve(&self, caller_path: &str) -> Result<PathBuf, RuntimeError> {
        let Some(root) = &self.root else {
            return Err(RuntimeError::ToolFailed {
                tool: "io".to_string(),
                message: "no `[io] root` is configured in this project's corvid.toml. \
                          Add `[io]\\nroot = \".\"` to corvid.toml to scope executing \
                          file-I/O to the project directory, or set CORVID_IO_ROOT. \
                          The executing file-I/O surface fails closed without an \
                          explicit root — this is the 33S0 security model."
                    .to_string(),
            });
        };

        // Strip leading separators from the caller path so an
        // absolute-looking input can't escape the root by being
        // joined as-is. After stripping, the caller path is
        // always treated as relative to `root`.
        let caller_stripped = Path::new(caller_path.trim_start_matches(['/', '\\']));
        let joined = root.join(caller_stripped);
        let normalised = normalize_path(&joined);

        // Confinement check: `normalised` must start with `root`
        // (as a path-component prefix). A simple `starts_with`
        // on the OsStr would false-positive on `/var/lib/foobar`
        // when root is `/var/lib/foo` — use `Path::starts_with`
        // which compares component-by-component.
        if !normalised.starts_with(root) {
            return Err(RuntimeError::ToolFailed {
                tool: "io".to_string(),
                message: format!(
                    "path `{caller_path}` escapes the configured `[io] root` (`{}`). \
                     The executing file-I/O surface refuses paths that resolve \
                     outside the root after `..` / absolute-prefix normalisation. \
                     Either move the file inside the root or widen the root in \
                     corvid.toml.",
                    root.display()
                ),
            });
        }

        Ok(normalised)
    }

    /// Return the configured root path. Used by tour topics + the
    /// debug rendering in `corvid doctor`. Returns `None` when no
    /// `[io] root` is configured.
    pub fn root_path(&self) -> Option<&Path> {
        self.root.as_deref()
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

    // -------- Slice 33S1a: IoToolPolicy plumbing tests --------

    /// 33S1a — Relative `[io] root` resolves against the supplied
    /// corvid.toml anchor directory; relative caller paths
    /// resolve against the resulting root.
    #[test]
    fn io_tool_policy_relative_root_resolves_against_corvid_toml_dir() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        let policy = IoToolPolicy::new(Some("./data"), Some(project));
        let resolved = policy
            .resolve("notes.txt")
            .expect("inside-root path resolves");
        let expected = normalize_path(&project.join("data").join("notes.txt"));
        assert_eq!(
            resolved, expected,
            "relative root should resolve against the corvid.toml dir"
        );
    }

    /// A RELATIVE corvid.toml anchor (what `corvid run
    /// src/main.cor` produces when invoked from the project dir)
    /// must still yield an absolute root — otherwise the
    /// component-prefix confinement check false-fires on every
    /// path and the whole executing io/db surface is unusable
    /// from the CLI. Pins the CWD-anchoring fallback.
    #[test]
    fn io_tool_policy_relative_anchor_still_produces_absolute_root() {
        let policy = IoToolPolicy::new(Some("."), Some(Path::new("")));
        let resolved = policy
            .resolve("notes.txt")
            .expect("inside-root path must resolve even with a relative anchor");
        assert!(
            resolved.is_absolute(),
            "resolved path must be absolute; got {}",
            resolved.display()
        );
        let cwd = std::env::current_dir().expect("cwd");
        let expected = normalize_path(&cwd.join("notes.txt"));
        assert_eq!(resolved, expected, "path should land under the CWD-anchored root");
    }

    /// 33S1a — Absolute `[io] root` is taken as-is; relative
    /// caller paths join cleanly under it.
    #[test]
    fn io_tool_policy_absolute_root_taken_as_is() {
        let tmp = tempfile::tempdir().expect("tmp");
        let absolute_root = tmp.path().to_path_buf();
        let policy = IoToolPolicy::new(Some(absolute_root.to_str().unwrap()), None);
        let resolved = policy
            .resolve("deep/path/file.txt")
            .expect("nested path resolves");
        let expected = normalize_path(&absolute_root.join("deep").join("path").join("file.txt"));
        assert_eq!(resolved, expected);
    }

    /// 33S1a — A `..` segment that would escape the root is
    /// rejected by `resolve`. This is the load-bearing security
    /// guard. The error message names both the offending caller
    /// path AND the configured root so an operator can see both.
    #[test]
    fn io_tool_policy_rejects_parent_traversal_escape() {
        let tmp = tempfile::tempdir().expect("tmp");
        let policy = IoToolPolicy::new(Some("data"), Some(tmp.path()));
        let err = policy
            .resolve("../../etc/passwd")
            .expect_err("traversal escape must be rejected");
        match err {
            RuntimeError::ToolFailed { tool, message } => {
                assert_eq!(tool, "io");
                assert!(
                    message.contains("../../etc/passwd"),
                    "diagnostic must name the offending path; got {message}"
                );
                assert!(
                    message.contains("escapes the configured"),
                    "diagnostic must name the violation kind; got {message}"
                );
            }
            other => panic!("expected ToolFailed, got {other:?}"),
        }
    }

    /// 33S1a — An absolute-looking caller path (`/etc/passwd`)
    /// is stripped of its leading separator and joined under the
    /// root. The normalisation should keep it inside the root —
    /// the test verifies that the absolute-input never escapes
    /// confinement.
    #[test]
    fn io_tool_policy_strips_leading_separator_to_confine_absolute_inputs() {
        let tmp = tempfile::tempdir().expect("tmp");
        let policy = IoToolPolicy::new(Some("data"), Some(tmp.path()));
        let resolved = policy
            .resolve("/notes.txt")
            .expect("absolute input gets confined under root");
        let expected = normalize_path(&tmp.path().join("data").join("notes.txt"));
        assert_eq!(
            resolved, expected,
            "absolute caller paths must be joined under root, not taken as-is"
        );
    }

    /// 33S1a — `IoToolPolicy::unset()` (or default construction)
    /// fails every `resolve` call with the missing-config
    /// diagnostic. This is the 33S0 fail-closed contract.
    #[test]
    fn io_tool_policy_unconfigured_fails_closed_on_resolve() {
        let policy = IoToolPolicy::unset();
        let err = policy
            .resolve("anything.txt")
            .expect_err("unconfigured policy must fail closed");
        match err {
            RuntimeError::ToolFailed { tool, message } => {
                assert_eq!(tool, "io");
                assert!(
                    message.contains("[io] root"),
                    "diagnostic must name [io] root; got {message}"
                );
                assert!(
                    message.contains("corvid.toml"),
                    "diagnostic must mention corvid.toml; got {message}"
                );
                assert!(
                    message.contains("33S0"),
                    "diagnostic must reference the 33S0 security model; got {message}"
                );
            }
            other => panic!("expected ToolFailed, got {other:?}"),
        }
        assert!(!policy.is_configured());
        assert!(policy.root_path().is_none());
    }

    /// 33S1a — A correctly-configured policy reports
    /// `is_configured() == true` and returns the resolved root.
    #[test]
    fn io_tool_policy_configured_reports_root_path() {
        let tmp = tempfile::tempdir().expect("tmp");
        let policy = IoToolPolicy::new(Some("."), Some(tmp.path()));
        assert!(policy.is_configured());
        let root = policy.root_path().expect("configured policy has a root");
        assert_eq!(root, normalize_path(tmp.path()));
    }
}
