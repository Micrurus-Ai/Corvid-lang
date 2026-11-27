use crate::lineage_input::{read_lineage_events, resolve_lineage_path};
use anyhow::{Context, Result};
use corvid_runtime::render_lineage_tree;
use std::path::Path;

pub fn run_lineage(id_or_path: &str, trace_dir: Option<&Path>) -> Result<u8> {
    let path = resolve_lineage_path(id_or_path, trace_dir);
    let events = read_lineage_events(&path)
        .with_context(|| format!("reading lineage trace `{}`", path.display()))?;
    print!("{}", render_lineage_tree(&events));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_runtime::{LineageEvent, LineageKind, LineageStatus};
    use std::fs;

    #[test]
    fn reads_lineage_jsonl_from_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.lineage.jsonl");
        let route = LineageEvent::root("trace-1", LineageKind::Route, "GET /", 1)
            .finish(LineageStatus::Ok, 2);
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&route).unwrap()),
        )
        .unwrap();
        let events = read_lineage_events(&path).unwrap();
        assert_eq!(events, vec![route]);
    }

    #[test]
    fn bare_id_resolves_to_lineage_jsonl_under_trace_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = resolve_lineage_path("trace-1", Some(dir.path()));
        assert_eq!(path, dir.path().join("trace-1.lineage.jsonl"));
    }
}
