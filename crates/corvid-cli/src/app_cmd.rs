//! Per-app assistive helpers for `corvid app *`.
//!
//! Lives under the Phase-42 launch-readiness umbrella
//! `35V2-P42-H-LR-per-app-ai-helpers`. Each helper is a
//! deterministic typed classifier over the app's ABI descriptor
//! — built in-process from a `.cor` source, no link step.
//!
//! In-binary anchor for the boot-summary launch-readiness row.
//! The constant is also re-exported from `corvid-abi` so the
//! runtime, CLI, and coverage gate share one source of truth.
//! Mirrors the drift-narrator pattern in
//! `connectors_cmd/check.rs`.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use corvid_abi::{boot_summary_from_descriptor, descriptor_from_json, render_boot_summary};
use corvid_driver::build_catalog_descriptor_for_source;

/// Anchor for the boot-summary launch-readiness row. The CLI's
/// `corvid app boot-summary` command delegates to
/// [`corvid_abi::boot_summary_from_descriptor`] and prints the
/// Grounded<T>-shaped result. The launch-readiness coverage gate
/// refuses to promote `app.boot_summary_grounded` from declared
/// to runtime-checked unless the corpus carries a positive row
/// asserting non-empty sources and an adversarial row asserting
/// the empty-surface case stays grounded.
#[allow(dead_code)]
pub const GUARANTEE_ID_APP_BOOT_SUMMARY_GROUNDED: &str =
    corvid_abi::GUARANTEE_ID_APP_BOOT_SUMMARY_GROUNDED;

/// Runs `corvid app boot-summary <source.cor>`. Lowers the
/// source file in-process to an ABI descriptor and prints the
/// typed boot summary. Returns a typed error rather than
/// panicking if the source fails to compile.
pub fn run_boot_summary(source_path: &Path) -> Result<()> {
    let descriptor_json = build_descriptor_json(source_path)?;
    let descriptor = descriptor_from_json(&descriptor_json)
        .with_context(|| format!("parse descriptor for `{}`", source_path.display()))?;
    let summary = boot_summary_from_descriptor(&descriptor);
    let rendered = render_boot_summary(&summary);
    print!("{rendered}");
    Ok(())
}

fn build_descriptor_json(source_path: &Path) -> Result<String> {
    let output = build_catalog_descriptor_for_source(source_path)
        .with_context(|| format!("build descriptor for `{}`", source_path.display()))?;
    if !output.diagnostics.is_empty() {
        let mut buf = String::new();
        for diag in &output.diagnostics {
            let _ = writeln!(&mut buf, "  {}", diag);
        }
        return Err(anyhow!(
            "cannot summarise `{}` — frontend rejected the source:\n{}",
            source_path.display(),
            buf.trim_end()
        ));
    }
    output.descriptor_json.ok_or_else(|| {
        anyhow!(
            "no descriptor produced for `{}` despite clean diagnostics",
            source_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Positive: a real reference-app surface produces a
    /// grounded boot summary. Asserts the rendered output
    /// carries the descriptor sha + a non-empty sources block,
    /// matching the launch-readiness contract that
    /// `app.boot_summary_grounded` promotes.
    #[test]
    fn boot_summary_for_minimal_app_renders_grounded_block() {
        let dir = tempdir().expect("tempdir");
        let src = dir.path().join("main.cor");
        fs::write(
            &src,
            "agent ask() -> Int:\n    return 42\n",
        )
        .expect("write source");

        let descriptor_json = build_descriptor_json(&src).expect("descriptor");
        assert!(!descriptor_json.is_empty(), "descriptor JSON non-empty");
        let descriptor = descriptor_from_json(&descriptor_json).expect("parse descriptor");
        let summary = boot_summary_from_descriptor(&descriptor);
        assert!(summary.surface_counts.agents >= 1);
        assert!(!summary.descriptor_sha256.is_empty());
        assert!(!summary.sources.is_empty());
        let rendered = render_boot_summary(&summary);
        assert!(rendered.contains("app_name:"));
        assert!(rendered.contains("descriptor_sha256:"));
        assert!(rendered.contains("sources:"));
        assert!(rendered.contains("descriptor.source_path"));
    }

    /// Adversarial: a source that fails to compile must surface
    /// a typed error naming the file and the frontend
    /// diagnostics, NOT panic. The boot summary itself is never
    /// rendered for an unparseable input.
    #[test]
    fn boot_summary_for_unparseable_source_returns_typed_error_not_panic() {
        let dir = tempdir().expect("tempdir");
        let src = dir.path().join("broken.cor");
        fs::write(&src, "this is not corvid syntax !!!").expect("write source");
        let err = run_boot_summary(&src).expect_err("must reject unparseable source");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("broken.cor"),
            "error must name the offending file, got: {msg}"
        );
    }
}
