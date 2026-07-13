//! Project-scaffolding helpers — `corvid new <name>` creates a minimal
//! Corvid project directory with `corvid.toml`, `src/main.cor`, and
//! a starter `.gitignore`.
//!
//! Extracted from `lib.rs` as part of Phase 20i responsibility
//! decomposition (20i-audit-driver-c).

use std::path::{Path, PathBuf};

/// Scaffold a new Corvid project at `<name>/` under the current directory.
pub fn scaffold_new(name: &str) -> anyhow::Result<PathBuf> {
    scaffold_new_in(&std::env::current_dir()?, name)
}

/// Scaffold with the opt-in Python tool template (slice 47a:
/// `corvid new --with-python-tools`). The DEFAULT scaffold is
/// pure Corvid — the hello-world runs an executing stdlib surface,
/// not a PyO3 round-trip.
pub fn scaffold_new_with_python(name: &str) -> anyhow::Result<PathBuf> {
    let root = scaffold_new(name)?;
    write_python_tool_template(&root)?;
    Ok(root)
}

pub(crate) fn write_python_tool_template(root: &Path) -> anyhow::Result<()> {
    std::fs::write(
        root.join("tools.py"),
        r#"# Implement your Corvid tools here.
from corvid_runtime import tool


@tool("echo")
async def echo(message: str) -> str:
    return message
"#,
    )?;
    std::fs::write(
        root.join("src").join("python_tools_example.cor"),
        r#"# Example: a tool implemented in Python (tools.py).
# Run with: corvid run src/python_tools_example.cor

tool echo(message: String) -> String

agent main(name: String) -> String:
    return echo(name)
"#,
    )?;
    Ok(())
}

/// Scaffold a new Corvid project named `<name>` under `parent`.
pub fn scaffold_new_in(parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let root = parent.join(name);
    if root.exists() {
        anyhow::bail!("directory `{}` already exists", root.display());
    }
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(
        root.join("corvid.toml"),
        format!(
            r#"name = "{name}"
version = "0.1.0"

[llm]
# No default model is set. Pick one explicitly:
#   default_model = "claude-opus-4-6"

# Phase 33S1/33S2 — executing I/O surfaces are gated by explicit
# config. Both sections fail closed when missing; the scaffolds
# below declare a sensible, narrow default so a fresh project runs
# the file-I/O surface out of the box while leaving HTTP egress
# explicitly empty until the developer names trusted hosts.

[io]
# File-I/O root. The executing `io_read_text` / `io_write_text` /
# `io_list_dir` stdlib tools resolve every caller-supplied path
# against this root. Path traversal (`..`) and absolute-path
# escapes outside the root are refused. `"."` keeps every read /
# write under the project directory; widen to `"./data"` etc. to
# narrow further. Set CORVID_IO_ROOT in the environment to
# override at run time.
root = "."

[http]
# HTTP egress allowlist. The executing `http_get` / `http_post_json`
# stdlib tools refuse any URL whose host is not in this list. The
# SSRF block (RFC1918 / loopback / link-local) is ALWAYS ON
# regardless of allowlist contents and is not configurable. An
# empty list (the default below) means HTTP egress fails closed
# until you add a host. Set CORVID_HTTP_ALLOW=host1,host2 in the
# environment to override at run time.
allow = []
"#
        ),
    )?;
    std::fs::write(root.join(".gitignore"), "/target\n__pycache__/\n*.pyc\n")?;
    std::fs::write(
        root.join("src").join("main.cor"),
        r#"# Your first Corvid agent — pure Corvid, no glue code.
#
# `time_now_utc` is an EXECUTING stdlib tool: the call below is
# traced, and `corvid replay` substitutes the recorded instant so
# a re-run reproduces this exact output. That is the language's
# whole promise, live in your first program.

import "./std/time" use time_now_utc

agent greet(name: String) -> String:
    now = time_now_utc()
    return "Hello, " + name + "! It is " + now.iso

agent main() -> String:
    return greet("Corvid")
"#,
    )?;
    Ok(root)
}

/// Locate the system stdlib directory shipped alongside the corvid binary.
/// The Corvid import resolver is purely relative to the importing `.cor`
/// file, so `std/` must be vendored into each project that uses it. This
/// helper finds the source copy in two places, in order:
///
/// 1. `$CORVID_HOME/std` — explicit override set by the installer.
/// 2. `<exe-dir>/../std` — the layout produced by the install bootstrap
///    (`~/.corvid/bin/corvid` → `~/.corvid/std`).
///
/// Returns `None` when neither candidate resolves to a directory, in
/// which case [`vendor_std`] becomes a no-op.
pub fn find_std_source() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CORVID_HOME") {
        let candidate = PathBuf::from(home).join("std");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.parent()?.join("std");
    candidate.is_dir().then_some(candidate)
}

/// Copy the system stdlib into a fresh project so `import "./std/foo"`
/// works without users having to clone the language repository. Returns
/// the source path that was vendored from, or `None` if nothing was done
/// (no source found, or the destination already exists). Errors propagate
/// as filesystem failures during the copy.
///
/// **Vendored location.** `std/` is dropped into `<project>/src/std/`,
/// not `<project>/std/`. The reason is Corvid's import resolver is
/// purely relative to the importing file (per
/// `crates/corvid-resolve/`'s lookup rules): `import "./std/effects"`
/// from `src/main.cor` resolves to `src/std/effects.cor`, NOT to a
/// project-root-relative path. Vendoring into `project/std/` left
/// every fresh `corvid new` project broken at first import —
/// surfaced by the corvid-installer maintainer's
/// `LIVE-TEST-GAPS.md` Gap #1 (handoff at
/// `docs/meta/corvid-installer-sync-handoff.md`) with the one-line
/// fix the maintainer literally wrote. Integration test
/// `vendor_std_from_corvid_new_scaffold_lets_src_main_import_std_effects`
/// in this file's `#[cfg(test)] mod tests` block catches the
/// regression by running the full scaffold + import + check
/// round-trip.
pub fn vendor_std(project_root: &Path) -> anyhow::Result<Option<PathBuf>> {
    let dst = project_root.join("src").join("std");
    if dst.exists() {
        return Ok(None);
    }
    let Some(src) = find_std_source() else {
        return Ok(None);
    };
    vendor_std_from(&src, &dst)?;
    Ok(Some(src))
}

/// Recursive directory copy used by [`vendor_std`]. Exposed separately so
/// tests can drive it without touching `$CORVID_HOME` or the executable
/// path (both of which are process-global and racy under parallel tests).
pub fn vendor_std_from(src: &Path, dst: &Path) -> anyhow::Result<()> {
    copy_dir_recursive(src, dst)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
