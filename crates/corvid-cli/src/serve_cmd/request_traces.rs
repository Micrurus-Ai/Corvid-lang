//! One run trace per served request (lineage L3b).
//!
//! A program that declares
//!
//! ```text
//! traces:
//!     location: env("TRACE_DIR")
//!     keep: 720 hours
//! ```
//!
//! has each request recorded as its own run under `location:`. The run's
//! first event names the route (`ROUTE_REQUEST_EVENT`), so its lineage is
//! rooted at `POST /cards` rather than the handler agent the route runs
//! through. The run id is returned in the `Corvid-Run` response header,
//! so a slow save seen in the browser can be looked up as the run that
//! served it.
//!
//! A trace holds what people sent, so it is kept no longer than `keep:`:
//! files past it are deleted when the server starts and every hour after.
//! The directory is created private to the server's user on Unix.
//! `traces: none` records nothing, and this module is never built.

use corvid_runtime::tracing::{fresh_run_id, Tracer};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The response header carrying the request's run id.
pub(super) const RUN_HEADER: &str = "corvid-run";

/// Where served requests are recorded, and for how long.
#[derive(Debug)]
pub(super) struct RequestTraces {
    dir: PathBuf,
    keep: Duration,
}

impl RequestTraces {
    /// The source's `traces:` block as the server records it, or `None`
    /// for `traces: none` (the checker requires a `traces:` with any
    /// `server`). Fails when the location cannot be read or made.
    pub(super) fn from_source(source: &str) -> Result<Option<Self>, String> {
        let Ok(tokens) = corvid_syntax::lex(source) else { return Ok(None) };
        let (file, _errors) = corvid_syntax::parse_file(&tokens);
        let recording = file.decls.iter().find_map(|decl| match decl {
            corvid_ast::Decl::Traces(traces) => traces.recording.clone(),
            _ => None,
        });
        let Some(recording) = recording else { return Ok(None) };
        let (Some(location), Some((count, unit, _))) = (&recording.location, &recording.keep) else {
            return Err("the `traces:` block has no `location:` or `keep:`".into());
        };
        let keep = Duration::from_secs(corvid_types::traces::keep_seconds(count, unit)?);
        let dir = corvid_runtime::tables::resolve_location(location)
            .map(PathBuf::from)
            .map_err(|_| match location {
                corvid_ast::DatabaseLocation::Env(name) => {
                    format!("traces are kept at the directory `{name}` names, but `{name}` is not set")
                }
                corvid_ast::DatabaseLocation::Path(path) => format!("the traces location `{path}` cannot be read"),
            })?;
        private_dir(&dir)?;
        Ok(Some(RequestTraces { dir, keep }))
    }

    /// A new run's tracer, writing one file under the location.
    pub(super) fn open_run(&self) -> Tracer {
        Tracer::open(&self.dir, fresh_run_id())
    }

    /// Delete the trace files older than `keep:`. Returns how many went.
    pub(super) fn sweep(&self) -> usize {
        sweep_older_than(&self.dir, self.keep, SystemTime::now())
    }

    /// Sweep now, then every hour, for as long as the server runs. A plain
    /// thread rather than an async task: the server calls this before its
    /// async runtime starts, and a sweep is blocking file work anyway.
    pub(super) fn spawn_sweeper(self: std::sync::Arc<Self>) {
        let removed = self.sweep();
        if removed > 0 {
            eprintln!("corvid serve: removed {removed} trace file(s) older than the declared `keep:`");
        }
        std::thread::Builder::new()
            .name("corvid-trace-sweeper".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(60 * 60));
                self.sweep();
            })
            .expect("the trace sweeper thread starts");
    }
}

/// Delete each `.jsonl` file under `dir` last written before `now - keep`.
fn sweep_older_than(dir: &Path, keep: Duration, now: SystemTime) -> usize {
    let Some(cutoff) = now.checked_sub(keep) else { return 0 };
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .filter(|entry| entry.metadata().and_then(|m| m.modified()).is_ok_and(|written| written < cutoff))
        .filter(|entry| std::fs::remove_file(entry.path()).is_ok())
        .count()
}

#[cfg(unix)]
fn private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("cannot create the traces directory `{}`: {e}", dir.display()))?;
    }
    let meta = std::fs::metadata(dir).map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?;
    if !meta.is_dir() {
        return Err(format!("the traces location `{}` is not a directory", dir.display()));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot make `{}` private: {e}", dir.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create the traces directory `{}`: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sweep_removes_only_traces_past_their_keep() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.jsonl");
        let fresh = dir.path().join("fresh.jsonl");
        let other = dir.path().join("notes.txt");
        for path in [&old, &fresh, &other] {
            std::fs::write(path, b"{}").unwrap();
        }
        let two_days_ago = SystemTime::now() - Duration::from_secs(48 * 3600);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(two_days_ago).unwrap();
        std::fs::File::options().write(true).open(&other).unwrap().set_modified(two_days_ago).unwrap();
        let removed = sweep_older_than(dir.path(), Duration::from_secs(24 * 3600), SystemTime::now());
        assert_eq!(removed, 1);
        assert!(!old.exists() && fresh.exists(), "only the old trace goes");
        assert!(other.exists(), "a file that is not a trace is never touched");
    }
}
