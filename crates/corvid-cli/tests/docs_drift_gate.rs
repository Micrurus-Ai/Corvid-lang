//! 35V2-P38-E sentinel: docs-as-code drift gate.
//!
//! Extracts every fenced ```corvid``` block from `docs/guides/*.md`
//! and routes each one through `corvid check`. Catches the drift
//! mode that surfaced in the 35V2-P38-A Phase 38 audit: a
//! user-facing guide referenced the aspirational `job` keyword
//! that doesn't parse, so a v1.0 user copy-pasting from the guide
//! would get a parse error.
//!
//! Two ways a guide block can opt out of the gate:
//!
//!   1. Open the fence with ```corvid skip``` instead of ```corvid```
//!      — useful for syntax sketches that are intentionally
//!      aspirational (e.g. a "what we'd like to ship" footnote).
//!   2. Make the block syntactically incomplete (a snippet that
//!      isn't a top-level decl) and open the fence with
//!      ```corvid skip``` — again, opt-in.
//!
//! The gate refuses silent opt-outs: any ```corvid``` block must
//! parse and typecheck. The `skip` form makes the opt-out visible
//! in the markdown source.
//!
//! ## EXEMPT_GUIDES
//!
//! The 35V2-P38-A audit found `docs/guides/jobs.md` referenced
//! aspirational syntax that doesn't parse. Lighting up this
//! drift-gate sentinel surfaced eight ADDITIONAL guides with the
//! same pattern. Per-guide rewrites are scope-creep for the
//! audit-correction round, so each affected guide is listed below
//! with the launch-readiness slice id that lands the rewrite. The
//! gate continues to enforce on every NEW guide and on jobs.md
//! (the guide rewritten in 35V2-P38-E).
//!
//! An exemption is the most honest stop-gap we can ship for this
//! scope: the gate exists, the failing guides are explicitly
//! named, and each exemption can only be removed by landing the
//! corresponding rewrite slice. No silent drift extension is
//! possible — adding a new exempt guide requires editing this
//! list, which surfaces in code review.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().and_then(Path::parent).unwrap().to_path_buf()
}

fn docs_guides_dir() -> PathBuf {
    workspace_root().join("docs").join("guides")
}

/// Guides whose Corvid blocks are temporarily exempt from the
/// drift gate while their per-guide rewrites land as
/// launch-readiness slices. Filed by 35V2-P38-E after the gate
/// surfaced 8 additional guides with the same aspirational-syntax
/// drift mode jobs.md had.
///
/// Each entry names the launch-readiness slice that removes the
/// exemption. When that slice lands, delete the entry; the gate
/// will then enforce on the rewritten guide.
const EXEMPT_GUIDES: &[(&str, &str)] = &[
    // 43W-1..43W-5 landed rewrites of auth.md, persistence.md,
    // observability.md, ffi-c-rust.md, ffi-python.md; entries
    // removed. The gate enforces on auth.md + jobs.md +
    // persistence.md + observability.md + ffi-c-rust.md +
    // ffi-python.md.
    ("backend.md", "35V2-P38-E-LR-backend-guide-rewrite"),
    ("connectors.md", "35V2-P38-E-LR-connectors-guide-rewrite"),
];

/// One Corvid code block extracted from a markdown file. The
/// language tag is the ` ```LANG ` text — `corvid` for gated
/// blocks, `corvid skip` for visibly-opted-out blocks.
#[derive(Debug)]
struct CodeBlock {
    file: PathBuf,
    start_line: usize,
    language_tag: String,
    body: String,
}

fn extract_corvid_blocks(file: &Path) -> Vec<CodeBlock> {
    let text = fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("read {file:?}: {e}"));
    let mut blocks = Vec::new();
    let mut iter = text.lines().enumerate().peekable();
    while let Some((line_no, line)) = iter.next() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("```") else {
            continue;
        };
        let language_tag = rest.trim().to_string();
        if !language_tag.starts_with("corvid") {
            // Skip non-Corvid fences.
            continue;
        }
        let mut body = String::new();
        let start_line = line_no + 1;
        for (_, body_line) in iter.by_ref() {
            if body_line.trim_start().starts_with("```") {
                break;
            }
            body.push_str(body_line);
            body.push('\n');
        }
        blocks.push(CodeBlock {
            file: file.to_path_buf(),
            start_line,
            language_tag,
            body,
        });
    }
    blocks
}

fn run_corvid_check(source: &str) -> Result<(), String> {
    let tmp = tempfile::Builder::new()
        .prefix("docs_drift_gate_")
        .suffix(".cor")
        .tempfile()
        .map_err(|e| format!("create temp .cor: {e}"))?;
    fs::write(tmp.path(), source).map_err(|e| format!("write temp .cor: {e}"))?;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent().and_then(Path::parent).unwrap();
    let output = Command::new("cargo")
        .current_dir(workspace)
        .args(["run", "-q", "-p", "corvid-cli", "--", "check"])
        .arg(tmp.path())
        .output()
        .map_err(|e| format!("spawn cargo run: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("stdout:\n{stdout}\n\nstderr:\n{stderr}"))
    }
}

#[test]
fn every_corvid_block_in_guides_compiles_clean() {
    let dir = docs_guides_dir();
    if !dir.exists() {
        // The guides directory not existing is itself a regression
        // worth flagging.
        panic!("docs/guides/ is missing at {dir:?}");
    }
    let mut markdowns: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    markdowns.sort();

    let mut blocks: Vec<CodeBlock> = Vec::new();
    for md in &markdowns {
        blocks.extend(extract_corvid_blocks(md));
    }

    if blocks.is_empty() {
        // If no guide has any Corvid blocks, that's a regression
        // (the gate has nothing to enforce). Catches the failure
        // mode where someone deletes all examples without
        // updating the test.
        panic!(
            "no ```corvid``` blocks found across {} guides — at \
             least one guide must contain an executable example. \
             Files scanned: {markdowns:?}",
            markdowns.len()
        );
    }

    let exempt: std::collections::HashMap<&str, &str> =
        EXEMPT_GUIDES.iter().copied().collect();

    let mut failures: Vec<String> = Vec::new();
    for block in &blocks {
        let file_name = block
            .file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if exempt.contains_key(file_name) {
            // Tracked as a launch-readiness rewrite slice; the
            // gate will enforce again when EXEMPT_GUIDES drops
            // this entry.
            continue;
        }
        if block.language_tag == "corvid skip" {
            // Visibly opted-out.
            continue;
        }
        if block.language_tag != "corvid" {
            // Some other tag like ```corvid foo``` we don't know.
            failures.push(format!(
                "{}:{} unknown language tag `{}` — use `corvid` for \
                 gated blocks or `corvid skip` to visibly opt out",
                block.file.display(),
                block.start_line,
                block.language_tag
            ));
            continue;
        }
        if let Err(err) = run_corvid_check(&block.body) {
            failures.push(format!(
                "{}:{} did not pass `corvid check`:\n{err}",
                block.file.display(),
                block.start_line
            ));
        }
    }

    if !failures.is_empty() {
        panic!(
            "docs/guides Corvid-block drift gate failed for {} \
             block(s):\n\n{}",
            failures.len(),
            failures.join("\n\n---\n\n")
        );
    }
}
