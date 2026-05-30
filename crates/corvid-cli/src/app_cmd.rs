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
use corvid_abi::{
    adversarial_refresh_from_descriptor, boot_summary_from_descriptor, descriptor_from_json,
    render_adversarial_refresh, render_boot_summary,
};
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

/// Anchor for the adversarial-refresh launch-readiness row.
#[allow(dead_code)]
pub const GUARANTEE_ID_APP_ADVERSARIAL_REFRESH_GROUNDED: &str =
    corvid_abi::GUARANTEE_ID_APP_ADVERSARIAL_REFRESH_GROUNDED;

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

/// Runs `corvid app adversarial-refresh <source.cor>`. Lowers
/// the source to a descriptor and prints a typed walker over
/// every surface element with one suggestion per (element,
/// threat) pair. Same diagnostics posture as boot-summary:
/// typed error rather than panic for unparseable sources.
pub fn run_adversarial_refresh(source_path: &Path) -> Result<()> {
    let descriptor_json = build_descriptor_json(source_path)?;
    let descriptor = descriptor_from_json(&descriptor_json)
        .with_context(|| format!("parse descriptor for `{}`", source_path.display()))?;
    let report = adversarial_refresh_from_descriptor(&descriptor);
    let rendered = render_adversarial_refresh(&report);
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

    /// Positive: a source with a `pub extern "c"` agent
    /// produces adversarial suggestions for that agent, and the
    /// rendered output names every suggestion's fixture name +
    /// its surface element. The output contains the
    /// `report_sources:` block — the contract that
    /// `app.adversarial_refresh_grounded` promotes.
    #[test]
    fn adversarial_refresh_for_extern_agent_renders_grounded_suggestions() {
        let dir = tempdir().expect("tempdir");
        let src = dir.path().join("main.cor");
        fs::write(
            &src,
            "@budget($1.00)\npub extern \"c\"\nagent ask(question: String) -> String:\n    return \"hello-\" + question\n",
        )
        .expect("write source");

        let descriptor_json = build_descriptor_json(&src).expect("descriptor");
        let descriptor = descriptor_from_json(&descriptor_json).expect("parse descriptor");
        let report = adversarial_refresh_from_descriptor(&descriptor);
        assert!(report.coverage_counts.agent_suggestions >= 2);
        for s in &report.suggestions {
            assert!(
                !s.sources.is_empty(),
                "suggestion {} has empty sources",
                s.suggested_fixture_name
            );
        }
        let rendered = render_adversarial_refresh(&report);
        assert!(rendered.contains("Corvid app adversarial-refresh report"));
        assert!(rendered.contains("coverage_counts:"));
        assert!(rendered.contains("report_sources:"));
        assert!(rendered.contains("ask_malformed_payload_refused"));
        assert!(rendered.contains("ask_unauthorised_caller_refused"));
    }

    /// Adversarial: a source that fails to compile must surface
    /// a typed error naming the file, NOT panic. Same posture
    /// as boot-summary.
    #[test]
    fn adversarial_refresh_for_unparseable_source_returns_typed_error_not_panic() {
        let dir = tempdir().expect("tempdir");
        let src = dir.path().join("broken.cor");
        fs::write(&src, "definitely not corvid").expect("write source");
        let err = run_adversarial_refresh(&src).expect_err("must reject unparseable source");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("broken.cor"),
            "error must name the offending file, got: {msg}"
        );
    }
}
