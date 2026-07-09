//! Phase 44a — CI guard for the book's syntax/types chapters.
//!
//! The 2026-07-09 language gap audit found docs/book chapters
//! teaching at least 20 features that do not parse. The locked
//! remediation ("implement the book") keeps every chapter honest
//! in the interim via a fence-tag convention this test enforces:
//!
//! - ```corvid           — MUST compile through `corvid_driver::compile`.
//! - ```corvid-planned   — designed-but-unimplemented syntax; MUST
//!                         sit under a nearby "Planned" marker that
//!                         names its roadmap slice, and MUST NOT
//!                         compile-check (it wouldn't).
//! - ```corvid-error     — deliberately-failing example (e.g. the
//!                         quickstart's dangerous-call-without-approve
//!                         program); MUST FAIL to compile. Pins the
//!                         book's "does not compile" claims so a
//!                         checker regression that silently ACCEPTS
//!                         the program breaks CI.
//! - ```corvid-fragment  — illustrative fragment, not a standalone
//!                         program; skipped.
//!
//! When a Phase 45/46 slice ships its feature, the chapter's
//! `corvid-planned` block flips to `corvid` and this guard starts
//! compiling it — the doc and the language re-converge with zero
//! extra test wiring.
//!
//! Scope today: chapters 04 + 05 (the 44a slice). 44d extends the
//! same pattern to the quickstart; further chapters can join by
//! adding to `GUARDED_CHAPTERS`.

use std::fs;
use std::path::PathBuf;

const GUARDED_CHAPTERS: &[&str] = &[
    "docs/book/02-quickstart.md",
    "docs/book/04-syntax.md",
    "docs/book/05-types.md",
    "docs/book/11-prompts.md",
    "docs/book/13-pattern-matching.md",
];

/// How far above a `corvid-planned` fence the word "Planned" must
/// appear (in lines). Generous enough for a marker + blank line +
/// prose sentence; tight enough that a stray planned block can't
/// borrow a marker from an unrelated section.
const PLANNED_MARKER_WINDOW: usize = 12;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("corvid-driver sits two levels below the repo root")
        .to_path_buf()
}

struct Fence {
    tag: String,
    start_line: usize,
    body: String,
}

/// Extract fenced code blocks whose info string starts with `corvid`.
fn corvid_fences(markdown: &str) -> Vec<Fence> {
    let mut fences = Vec::new();
    let mut in_fence: Option<(String, usize, Vec<String>)> = None;
    for (idx, line) in markdown.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some((tag, start, body)) = in_fence.as_mut() {
            if trimmed.starts_with("```") {
                fences.push(Fence {
                    tag: tag.clone(),
                    start_line: *start,
                    body: body.join("\n"),
                });
                in_fence = None;
            } else {
                body.push(line.to_string());
            }
            continue;
        }
        if let Some(info) = trimmed.strip_prefix("```") {
            let info = info.trim();
            if info.starts_with("corvid") {
                in_fence = Some((info.to_string(), idx + 1, Vec::new()));
            } else if !info.is_empty() {
                // Non-corvid language fence (sh, toml, …) — skip until close.
                in_fence = Some(("__skip__".to_string(), idx + 1, Vec::new()));
            } else {
                // Bare ``` fence — also skip, but flag via tag so the
                // convention test can reject untagged corvid-looking code.
                in_fence = Some(("__bare__".to_string(), idx + 1, Vec::new()));
            }
        }
    }
    fences.retain(|f| f.tag.starts_with("corvid") || f.tag == "__bare__");
    fences
}

#[test]
fn every_corvid_block_in_guarded_chapters_compiles() {
    let root = repo_root();
    let mut failures = Vec::new();
    for chapter in GUARDED_CHAPTERS {
        let path = root.join(chapter);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {chapter}: {e}"));
        for fence in corvid_fences(&text) {
            match fence.tag.as_str() {
                "corvid" => {
                    let compiled = corvid_driver::compile(&fence.body);
                    if !compiled.ok() {
                        failures.push(format!(
                            "{chapter}:{} — `corvid` block fails to compile:\n{:#?}",
                            fence.start_line, compiled.diagnostics
                        ));
                    }
                }
                "corvid-error" => {
                    let compiled = corvid_driver::compile(&fence.body);
                    if compiled.ok() {
                        failures.push(format!(
                            "{chapter}:{} — `corvid-error` block COMPILED CLEAN. \
                             The book claims this program does not compile; either \
                             a checker regression now accepts it (fix the checker) \
                             or the example is stale (fix the book).",
                            fence.start_line
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    assert!(
        failures.is_empty(),
        "book blocks tagged `corvid` must compile through the driver \
         (retag deliberately-unimplemented syntax as `corvid-planned` \
         under a Planned marker, or fix the snippet):\n\n{}",
        failures.join("\n\n")
    );
}

#[test]
fn every_planned_block_sits_under_a_planned_marker() {
    let root = repo_root();
    let mut violations = Vec::new();
    for chapter in GUARDED_CHAPTERS {
        let path = root.join(chapter);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {chapter}: {e}"));
        let lines: Vec<&str> = text.lines().collect();
        for fence in corvid_fences(&text) {
            if fence.tag != "corvid-planned" {
                continue;
            }
            let fence_idx = fence.start_line - 1;
            let window_start = fence_idx.saturating_sub(PLANNED_MARKER_WINDOW);
            let has_marker = lines[window_start..fence_idx]
                .iter()
                .any(|l| l.contains("Planned"));
            if !has_marker {
                violations.push(format!(
                    "{chapter}:{} — `corvid-planned` block has no \"Planned\" \
                     marker within the preceding {PLANNED_MARKER_WINDOW} lines",
                    fence.start_line
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "planned blocks must be visibly marked so readers know the \
         syntax is not yet shipped:\n\n{}",
        violations.join("\n")
    );
}

#[test]
fn no_bare_fences_hide_corvid_code() {
    let root = repo_root();
    let mut violations = Vec::new();
    for chapter in GUARDED_CHAPTERS {
        let path = root.join(chapter);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {chapter}: {e}"));
        for fence in corvid_fences(&text) {
            if fence.tag == "__bare__" {
                violations.push(format!(
                    "{chapter}:{} — untagged ``` fence; tag it `corvid`, \
                     `corvid-planned`, `corvid-fragment`, or a non-corvid \
                     language so the guard knows how to treat it",
                    fence.start_line
                ));
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}
