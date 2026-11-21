//! Build script — emits `CORVID_BUILD_SHA` and `CORVID_BUILD_DATE`
//! into the compiled binary so `corvid --version` can report which
//! commit the user actually has.
//!
//! Slice `35V2-P33-version-output-sha` — first of four release-
//! pipeline slices that close the gap between `main` HEAD and
//! whatever the install script delivers. Before this slice
//! `corvid --version` printed only `corvid 0.0.1` and reviewers
//! had no way to verify which commit their binary was at,
//! surfaced by the first 33M friends-and-family trial's follow-up
//! audit (see
//! `docs/external-trials/33m-friends-and-family-followup-prompt.md`
//! "Transparency: this follow-up itself had bugs"). After this
//! slice the version line becomes
//! `corvid 0.0.1 (<short-sha>, <commit-date>)` — e.g.
//! `corvid 0.0.1 (e8efa23, 2026-06-04)`.
//!
//! Direct `git` invocation rather than the `vergen` crate, because
//! the dep tree should not pay for a build helper to run two
//! commands. The script handles three failure modes the way the
//! reviewer would expect:
//!
//!   - **No git on PATH** — fall back to `unknown` for both fields.
//!     Release tarballs are built in a workflow that DOES have git,
//!     so this only fires for users building inside a stripped
//!     environment (e.g. a docker stage with `--no-install-recommends`
//!     that didn't pull git).
//!   - **Not a git checkout** — same fall back. Users who download
//!     the source tarball from a GitHub Release and `cargo build`
//!     from it land here.
//!   - **Env-var override** — if `CORVID_BUILD_SHA` and / or
//!     `CORVID_BUILD_DATE` are already set in the environment, the
//!     script preserves them and skips the git lookup. Lets the
//!     release workflow inject values for the published artifact
//!     even when the workflow's checkout-action SHA strategy
//!     differs from what `git rev-parse` would surface.

use std::process::Command;

fn main() {
    // The build script is also an input when building outside a Git checkout.
    println!("cargo:rerun-if-changed=build.rs");
    watch_git_path("HEAD");
    if let Some(branch) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        if !watch_git_path(&branch) {
            watch_git_path("packed-refs");
        }
    }
    println!("cargo:rerun-if-env-changed=CORVID_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=CORVID_BUILD_DATE");

    let sha = std::env::var("CORVID_BUILD_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string())
        });

    let date = std::env::var("CORVID_BUILD_DATE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            git_output(&["log", "-1", "--format=%cd", "--date=short"])
                .unwrap_or_else(|| "unknown".to_string())
        });

    println!("cargo:rustc-env=CORVID_BUILD_SHA={sha}");
    println!("cargo:rustc-env=CORVID_BUILD_DATE={date}");
}

// Git resolves per-worktree HEAD and shared branch refs to different directories.
// Packed refs can become loose after a commit. Watch the nearest existing
// directory for a missing path, since a missing Cargo input is always dirty.
fn watch_git_path(name: &str) -> bool {
    if let Some(path) = git_output(&["rev-parse", "--path-format=absolute", "--git-path", name]) {
        if let Some(existing) = std::path::Path::new(&path).ancestors().find(|p| p.exists()) {
            println!("cargo:rerun-if-changed={}", existing.display());
            return existing == std::path::Path::new(&path);
        }
    }
    false
}

/// Run `git <args>` and return trimmed stdout if the command
/// succeeded with non-empty output. Returns `None` on any failure
/// mode (missing git binary, non-zero exit, empty output) so the
/// caller's `unwrap_or_else` lands on the documented `unknown`
/// fallback.
fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
