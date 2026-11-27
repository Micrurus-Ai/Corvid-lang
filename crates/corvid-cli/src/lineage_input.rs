//! Reading lineage traces from disk: the one reader `corvid observe`,
//! `corvid trace lineage` and `corvid eval promote` share, so they agree
//! on which files are traces and how a run id resolves to a file.

use anyhow::{Context, Result};
use corvid_runtime::LineageEvent;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn read_lineage_events(path: &Path) -> Result<Vec<LineageEvent>> {
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

pub(crate) fn read_lineage_input(path: &Path) -> Result<Vec<LineageEvent>> {
    if path.is_dir() {
        let mut events = Vec::new();
        let mut paths = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if is_lineage_jsonl(&path) {
                events.extend(read_lineage_events(&path)?);
            }
        }
        return Ok(events);
    }
    read_lineage_events(path)
}

pub(crate) fn resolve_lineage_path(id_or_path: &str, trace_dir: Option<&Path>) -> PathBuf {
    let direct = PathBuf::from(id_or_path);
    if direct.exists() || direct.extension().is_some() {
        return direct;
    }
    trace_dir
        .unwrap_or_else(|| Path::new("target/trace"))
        .join(format!("{id_or_path}.lineage.jsonl"))
}

pub(crate) fn is_lineage_jsonl(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.ends_with(".lineage.jsonl"))
        .unwrap_or(false)
}
