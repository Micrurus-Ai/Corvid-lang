//! Reading lineage from disk: the one reader `corvid observe`,
//! `corvid trace lineage`, `corvid eval promote` and the observe helpers
//! share, so they agree on which files are traces and how a run id
//! resolves to a file.
//!
//! Two kinds of file hold lineage:
//! - a run trace, `<run_id>.jsonl`, which every run writes and replay
//!   reads. Its lineage is derived by `corvid_runtime::lineage_from_trace`,
//!   the one projection.
//! - a lineage trace, `<run_id>.lineage.jsonl`, which is read as it is.

use anyhow::{Context, Result};
use corvid_runtime::{lineage_from_trace, LineageEvent};
use std::fs;
use std::path::{Path, PathBuf};

/// Which kind of trace a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TraceFileKind {
    /// A run's trace, as replay reads it.
    Run,
    /// Lineage events, as written.
    Lineage,
}

/// Which kind of trace `path` holds, judged by its name and first line,
/// or `None` for a file that is neither (any other JSON lines file).
pub(crate) fn trace_file_kind(path: &Path) -> Option<TraceFileKind> {
    if is_lineage_jsonl(path) {
        return Some(TraceFileKind::Lineage);
    }
    let text = fs::read_to_string(path).ok()?;
    let first = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let value: serde_json::Value = serde_json::from_str(first).ok()?;
    if value.get("schema").and_then(|s| s.as_str()) == Some(corvid_runtime::lineage::LINEAGE_SCHEMA) {
        Some(TraceFileKind::Lineage)
    } else if value.get("kind").is_some_and(|k| k.is_string()) && value.get("run_id").is_some_and(|r| r.is_string()) {
        Some(TraceFileKind::Run)
    } else {
        None
    }
}

/// The lineage a file holds: a lineage trace as written, or a run
/// trace's derived lineage. A file that is neither is refused by name.
pub(crate) fn read_lineage_events(path: &Path) -> Result<Vec<LineageEvent>> {
    match trace_file_kind(path) {
        Some(TraceFileKind::Lineage) => {}
        Some(TraceFileKind::Run) => {
            let events = corvid_trace_schema::read_events_from_path(path)
                .map_err(|error| anyhow::anyhow!("{error}"))
                .context("reading the run trace")?;
            return Ok(lineage_from_trace(&events));
        }
        None => anyhow::bail!(
            "`{}` is neither a run trace (as `corvid run` records) nor a lineage trace (`*.lineage.jsonl`)",
            path.display()
        ),
    }
    let text = fs::read_to_string(path)?;
    let mut events = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event: LineageEvent = serde_json::from_str(trimmed)
            .with_context(|| format!("line {} is not a lineage event", index + 1))?;
        events.push(event);
    }
    Ok(events)
}

/// The lineage of one file, or of every trace file in a directory, in
/// file-name order.
pub(crate) fn read_lineage_input(path: &Path) -> Result<Vec<LineageEvent>> {
    if path.is_dir() {
        let mut events = Vec::new();
        let mut paths = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if is_trace_jsonl(&path) {
                events.extend(
                    read_lineage_events(&path)
                        .with_context(|| format!("reading `{}`", path.display()))?,
                );
            }
        }
        return Ok(events);
    }
    read_lineage_events(path)
}

/// A path as given, or a bare run id under the trace directory: its
/// lineage trace when one exists, otherwise its run trace.
pub(crate) fn resolve_lineage_path(id_or_path: &str, trace_dir: Option<&Path>) -> PathBuf {
    let direct = PathBuf::from(id_or_path);
    if direct.exists() || direct.extension().is_some() {
        return direct;
    }
    let dir = trace_dir.unwrap_or_else(|| Path::new("target/trace"));
    let lineage = dir.join(format!("{id_or_path}.lineage.jsonl"));
    let run = dir.join(format!("{id_or_path}.jsonl"));
    if !lineage.exists() && run.exists() {
        run
    } else {
        lineage
    }
}

pub(crate) fn is_lineage_jsonl(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.ends_with(".lineage.jsonl"))
        .unwrap_or(false)
}

/// A file that holds lineage: a run trace or a lineage trace. Any other
/// JSON lines file in a trace directory is skipped.
pub(crate) fn is_trace_jsonl(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") && trace_file_kind(path).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_trace_schema::TraceEvent;

    fn write_run_trace(path: &Path) {
        let events = [
            TraceEvent::SchemaHeader {
                version: corvid_trace_schema::SCHEMA_VERSION,
                writer: "corvid-vm".into(),
                commit_sha: None,
                source_path: None,
                tzdb_version: None,
                ts_ms: 1,
                run_id: "run-7".into(),
            },
            TraceEvent::RunStarted { ts_ms: 2, run_id: "run-7".into(), agent: "main".into(), args: vec![] },
            TraceEvent::ToolCall { ts_ms: 3, run_id: "run-7".into(), tool: "table:Shelf.get".into(), args: vec![] },
            TraceEvent::ToolResult {
                ts_ms: 5,
                run_id: "run-7".into(),
                tool: "table:Shelf.get".into(),
                result: serde_json::Value::Null,
            },
            TraceEvent::RunCompleted { ts_ms: 6, run_id: "run-7".into(), ok: true, result: None, error: None },
        ];
        let text: String = events.iter().map(|e| serde_json::to_string(e).unwrap() + "\n").collect();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn a_run_trace_is_read_as_its_derived_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run-7.jsonl");
        write_run_trace(&path);
        let events = read_lineage_events(&path).unwrap();
        let names: Vec<_> = events.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["main", "Shelf.get"]);
        assert_eq!(events, read_lineage_input(dir.path()).unwrap(), "a directory reads the same file the same way");
    }

    #[test]
    fn a_file_that_is_no_trace_is_skipped_in_a_directory_and_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.jsonl");
        fs::write(&plain, "{\"level\":\"info\"}\n").unwrap();
        write_run_trace(&dir.path().join("run-7.jsonl"));
        assert_eq!(trace_file_kind(&plain), None);
        assert_eq!(trace_file_kind(&dir.path().join("run-7.jsonl")), Some(TraceFileKind::Run));
        assert_eq!(read_lineage_input(dir.path()).unwrap().len(), 2, "only the run trace is read");
        let refused = read_lineage_events(&plain).unwrap_err().to_string();
        assert!(refused.contains("neither a run trace"), "{refused}");
    }

    #[test]
    fn a_bare_run_id_finds_the_run_trace_when_no_lineage_trace_exists() {
        let dir = tempfile::tempdir().unwrap();
        write_run_trace(&dir.path().join("run-7.jsonl"));
        assert_eq!(resolve_lineage_path("run-7", Some(dir.path())), dir.path().join("run-7.jsonl"));
        fs::write(dir.path().join("run-7.lineage.jsonl"), "").unwrap();
        assert_eq!(resolve_lineage_path("run-7", Some(dir.path())), dir.path().join("run-7.lineage.jsonl"));
    }
}
