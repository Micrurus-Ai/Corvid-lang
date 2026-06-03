use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use corvid_abi::{load_signing_key, sign_envelope, KeySource};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-
/// coverage sentinel. Names the registry id whose runtime
/// enforcement lives in `run_release_notes` below — the
/// release-notes generator pairs every grouped line with a git
/// SHA so every claim back-references commit history.
#[allow(dead_code)]
pub const GUARANTEE_ID_RELEASE_NOTES_GROUNDED: &str = "release.notes_grounded";

#[derive(Serialize)]
struct ReleaseManifest<'a> {
    schema: &'a str,
    channel: &'a str,
    version: &'a str,
    stability: &'a str,
    binary: String,
    binary_sha256: String,
    checksum_file: &'a str,
    changelog: &'a str,
    policy: &'a str,
}

pub fn run_release(channel: &str, version: Option<&str>, out: &Path) -> Result<()> {
    let normalized = normalize_channel(channel)?;
    let version = version
        .map(str::to_string)
        .unwrap_or_else(|| default_version(normalized));
    validate_version(normalized, &version)?;

    fs::create_dir_all(out)
        .with_context(|| format!("create release output dir `{}`", out.display()))?;

    let binary_path = copy_current_binary(normalized, &version, out)?;
    let binary_bytes = fs::read(&binary_path)
        .with_context(|| format!("read release binary `{}`", binary_path.display()))?;
    let binary_sha256 = hex::encode(Sha256::digest(&binary_bytes));

    let changelog_name = "CHANGELOG.md";
    let changelog = render_changelog(normalized, &version);
    fs::write(out.join(changelog_name), &changelog).context("write release changelog")?;

    let binary_name = file_name(&binary_path)?;
    fs::write(out.join("install.sh"), render_install_sh(&binary_name))
        .context("write unix install script")?;
    fs::write(out.join("install.ps1"), render_install_ps1(&binary_name))
        .context("write powershell install script")?;
    fs::write(out.join("REPRODUCIBLE.md"), render_reproducible(normalized, &version))
        .context("write reproducible build notes")?;
    fs::write(out.join("DEMO.md"), render_demo_script()).context("write demo script")?;
    fs::write(out.join("INCIDENT_CONTACTS.md"), render_incident_contacts())
        .context("write incident contacts")?;
    fs::write(out.join("ROLLBACK.md"), render_rollback()).context("write rollback plan")?;
    let checksums = format!("{binary_sha256}  {binary_name}\n");
    fs::write(out.join("SHA256SUMS.txt"), checksums).context("write release checksums")?;

    let manifest = ReleaseManifest {
        schema: "corvid.release.manifest.v1",
        channel: normalized,
        version: &version,
        stability: stability_for(normalized),
        binary: binary_name,
        binary_sha256,
        checksum_file: "SHA256SUMS.txt",
        changelog: changelog_name,
        policy: "docs/release-policy.md",
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("serialize release manifest")?;
    fs::write(out.join("release-manifest.json"), &manifest_json)
        .context("write release manifest")?;

    let attestation = sign_release_manifest(&manifest_json)?;
    fs::write(out.join("release-attestation.dsse.json"), attestation)
        .context("write release attestation")?;

    println!("release channel: {normalized}");
    println!("version: {version}");
    println!("binary: {}", binary_path.display());
    println!("checksums: {}", out.join("SHA256SUMS.txt").display());
    println!("install: {}", out.join("install.sh").display());
    println!("manifest: {}", out.join("release-manifest.json").display());
    println!(
        "attestation: {}",
        out.join("release-attestation.dsse.json").display()
    );
    Ok(())
}

fn normalize_channel(channel: &str) -> Result<&'static str> {
    match channel {
        "nightly" => Ok("nightly"),
        "beta" => Ok("beta"),
        "stable" => Ok("stable"),
        other => bail!("unknown release channel `{other}`; expected nightly, beta, or stable"),
    }
}

fn default_version(channel: &str) -> String {
    match channel {
        "nightly" => "0.0.0-nightly.local".to_string(),
        "beta" => "1.0.0-beta.1".to_string(),
        "stable" => env!("CARGO_PKG_VERSION").to_string(),
        _ => unreachable!("channel normalized before default version"),
    }
}

fn validate_version(channel: &str, version: &str) -> Result<()> {
    match channel {
        "nightly" if version.contains("-nightly.") => Ok(()),
        "beta" if version.contains("-beta.") => Ok(()),
        "stable" if !version.contains('-') && version.split('.').count() == 3 => Ok(()),
        "nightly" => bail!("nightly versions must contain `-nightly.`"),
        "beta" => bail!("beta versions must contain `-beta.`"),
        "stable" => bail!("stable versions must be plain MAJOR.MINOR.PATCH"),
        _ => unreachable!("channel normalized before version validation"),
    }
}

fn copy_current_binary(channel: &str, version: &str, out: &Path) -> Result<PathBuf> {
    let current = std::env::current_exe().context("locate current corvid binary")?;
    let target = out.join(binary_file_name_for(channel, version));
    fs::copy(&current, &target).with_context(|| {
        format!(
            "copy release binary `{}` to `{}`",
            current.display(),
            target.display()
        )
    })?;
    Ok(target)
}

fn binary_file_name_for(channel: &str, version: &str) -> String {
    let base = format!(
        "corvid-{channel}-{version}-{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let ext = std::env::consts::EXE_EXTENSION;
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .with_context(|| format!("release path `{}` has no file name", path.display()))
}

fn render_changelog(channel: &str, version: &str) -> String {
    format!(
        r#"# Corvid {version}

Channel: {channel}

## Required Verification

- Verify `SHA256SUMS.txt` against the release binary.
- Verify `release-attestation.dsse.json` with the release public key.
- Run `corvid claim audit` before promoting this release.
- Run `corvid upgrade --check` for applications moving from the previous channel baseline.

## Compatibility

This release follows `docs/release-policy.md`. Any breaking change must appear in the upgrade report and migration guide.
"#
    )
}

fn render_install_sh(binary_name: &str) -> String {
    format!(
        r#"#!/usr/bin/env sh
set -eu
prefix="${{CORVID_INSTALL_PREFIX:-/usr/local/bin}}"
mkdir -p "$prefix"
cp "./{binary_name}" "$prefix/corvid"
chmod 0755 "$prefix/corvid"
"#
    )
}

fn render_install_ps1(binary_name: &str) -> String {
    format!(
        r#"$ErrorActionPreference = "Stop"
$prefix = if ($env:CORVID_INSTALL_PREFIX) {{ $env:CORVID_INSTALL_PREFIX }} else {{ "$env:LOCALAPPDATA\Corvid\bin" }}
New-Item -ItemType Directory -Force -Path $prefix | Out-Null
Copy-Item ".\{binary_name}" (Join-Path $prefix "corvid.exe") -Force
"#
    )
}

fn render_reproducible(channel: &str, version: &str) -> String {
    format!(
        r#"# Reproducible Build Notes

Channel: {channel}
Version: {version}

Required verification:

```bash
cargo build -p corvid-cli --release
sha256sum target/release/corvid
sha256sum -c SHA256SUMS.txt
corvid claim audit --json
```

A stable release needs an independent rebuild report before public launch. If the rebuilt hash differs, attach compiler version, host OS, linker, target triple, and environment diff to the release issue.
"#
    )
}

fn render_demo_script() -> &'static str {
    r#"# Launch Demo Script

1. `corvid check examples/backend/personal_executive_agent/src/main.cor`
2. `corvid audit examples/backend/personal_executive_agent/src/main.cor --json`
3. `corvid deploy package examples/backend/personal_executive_agent --out target/pea-package`
4. `corvid release build beta 1.0.0-beta.1 --out target/release/beta`
5. `corvid claim audit --json`

The demo must show approval-gated external writes, deploy artifacts, release artifacts, and claim-audit alignment.
"#
}

fn render_incident_contacts() -> &'static str {
    r#"# Incident Contacts

- Release owner: record in `release-manifest.json`
- Security owner: record in the release issue
- Claim-audit owner: record in the release issue
- Rollback owner: record in the release issue

Do not publish stable artifacts without named owners.
"#
}

fn render_rollback() -> &'static str {
    r#"# Rollback Plan

1. Stop promoting the affected channel.
2. Preserve binaries, checksums, SBOM, and attestations for investigation.
3. Publish rollback note with affected versions and workaround.
4. Cut patched nightly or beta from a fixed commit.
5. Re-run release, upgrade, deploy, and claim-audit checks.
"#
}

fn stability_for(channel: &str) -> &'static str {
    match channel {
        "nightly" => "nightly-no-compatibility-promise",
        "beta" => "beta-train-compatible-with-migration-notes",
        "stable" => "semver-stable",
        _ => unreachable!("channel normalized before stability lookup"),
    }
}

/// Render structured release notes between two git refs.
/// Walks `git log <from>..<to>` over the current repository,
/// groups commits by conventional-commit prefix, and writes
/// the result to `out_path` (or stdout when `None`).
///
/// Output shape (markdown):
/// ```text
/// # Release notes: <from>..<to>
///
/// ## Features
/// - <subject> (<sha>)
///
/// ## Fixes
/// - <subject> (<sha>)
///
/// ## (etc. — one section per non-empty category)
///
/// ## Other
/// - <subject> (<sha>)   # commits whose prefix didn't match
/// ```
///
/// Every line ends with the short SHA, so every claim in the
/// notes traces back to a commit. The slice is deterministic —
/// `git log` is the source of truth, no LLM, no synthesised
/// summary.
pub fn run_release_notes(from: &str, to: &str, out_path: Option<&Path>) -> Result<()> {
    validate_git_ref(from, "from")?;
    validate_git_ref(to, "to")?;
    let commits = git_log_between(from, to)?;
    let categorised = categorise_commits(&commits);
    let markdown = render_release_notes_markdown(from, to, &categorised);
    if let Some(path) = out_path {
        fs::write(path, &markdown)
            .with_context(|| format!("write release notes to `{}`", path.display()))?;
    } else {
        print!("{markdown}");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitLine {
    sha: String,
    subject: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CategorisedCommits {
    features: Vec<CommitLine>,
    fixes: Vec<CommitLine>,
    perf: Vec<CommitLine>,
    refactors: Vec<CommitLine>,
    docs: Vec<CommitLine>,
    tests: Vec<CommitLine>,
    chores: Vec<CommitLine>,
    other: Vec<CommitLine>,
}

impl CategorisedCommits {
    fn total(&self) -> usize {
        self.features.len()
            + self.fixes.len()
            + self.perf.len()
            + self.refactors.len()
            + self.docs.len()
            + self.tests.len()
            + self.chores.len()
            + self.other.len()
    }
}

fn validate_git_ref(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("release notes `{field}` ref must not be empty");
    }
    // Reject characters that could turn a ref into a shell-meta
    // surprise even though we use Command::args (which avoids
    // shell expansion). Fail closed on operator typos like
    // `--from=v1.0` (we want the value to be a ref, not a
    // flag).
    if value.starts_with('-') {
        bail!(
            "release notes `{field}` ref `{value}` looks like a flag; \
             refusing to pass it to git as a positional"
        );
    }
    Ok(())
}

fn git_log_between(from: &str, to: &str) -> Result<Vec<CommitLine>> {
    let range = format!("{from}..{to}");
    let output = Command::new("git")
        .args([
            "log",
            "--no-merges",
            "--pretty=format:%h\t%s",
            range.as_str(),
        ])
        .output()
        .with_context(|| {
            format!(
                "run `git log --pretty=... {range}` (is git installed + in PATH?)"
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`git log {range}` failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_git_log_output(&stdout))
}

fn parse_git_log_output(raw: &str) -> Vec<CommitLine> {
    let mut commits = Vec::new();
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some((sha, subject)) = line.split_once('\t') {
            let sha = sha.trim();
            let subject = subject.trim();
            if !sha.is_empty() && !subject.is_empty() {
                commits.push(CommitLine {
                    sha: sha.to_string(),
                    subject: subject.to_string(),
                });
            }
        }
    }
    commits
}

fn categorise_commits(commits: &[CommitLine]) -> CategorisedCommits {
    let mut out = CategorisedCommits::default();
    for commit in commits {
        let prefix = conventional_commit_prefix(&commit.subject);
        match prefix {
            Some("feat") => out.features.push(commit.clone()),
            Some("fix") => out.fixes.push(commit.clone()),
            Some("perf") => out.perf.push(commit.clone()),
            Some("refactor") => out.refactors.push(commit.clone()),
            Some("docs") => out.docs.push(commit.clone()),
            Some("test") => out.tests.push(commit.clone()),
            Some("chore") | Some("build") | Some("ci") | Some("style") => {
                out.chores.push(commit.clone())
            }
            _ => out.other.push(commit.clone()),
        }
    }
    out
}

/// Extract the conventional-commit type from a subject like
/// `feat(scope): description` or `fix: description`. Returns
/// `None` for subjects that don't match the
/// `<type>(<scope>)?: <desc>` shape.
fn conventional_commit_prefix(subject: &str) -> Option<&str> {
    let colon = subject.find(':')?;
    let head = &subject[..colon];
    let prefix = if let Some(paren) = head.find('(') {
        &head[..paren]
    } else {
        head
    };
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c == '-')
    {
        Some(trimmed)
    } else {
        None
    }
}

fn render_release_notes_markdown(
    from: &str,
    to: &str,
    grouped: &CategorisedCommits,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Release notes: `{from}`..`{to}`\n\n"));
    if grouped.total() == 0 {
        out.push_str(&format!(
            "No changes between `{from}` and `{to}`.\n"
        ));
        return out;
    }
    render_section(&mut out, "Features", &grouped.features);
    render_section(&mut out, "Fixes", &grouped.fixes);
    render_section(&mut out, "Performance", &grouped.perf);
    render_section(&mut out, "Refactors", &grouped.refactors);
    render_section(&mut out, "Documentation", &grouped.docs);
    render_section(&mut out, "Tests", &grouped.tests);
    render_section(&mut out, "Build, CI, Chores", &grouped.chores);
    render_section(&mut out, "Other", &grouped.other);
    out
}

fn render_section(out: &mut String, title: &str, commits: &[CommitLine]) {
    if commits.is_empty() {
        return;
    }
    out.push_str(&format!("## {title}\n\n"));
    for commit in commits {
        out.push_str(&format!("- {} ({})\n", commit.subject, commit.sha));
    }
    out.push('\n');
}

fn sign_release_manifest(manifest_json: &str) -> Result<String> {
    let signing_key = std::env::var("CORVID_RELEASE_SIGNING_KEY")
        .context("CORVID_RELEASE_SIGNING_KEY is required for release attestation")?;
    let key = load_signing_key(&KeySource::Env(signing_key))
        .map_err(|err| anyhow::anyhow!("load release signing key: {err}"))?;
    let envelope = sign_envelope(
        manifest_json.as_bytes(),
        "application/vnd.corvid.release.manifest.v1+json",
        &key,
        "release-channel",
    );
    serde_json::to_string_pretty(&envelope).context("serialize release attestation")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 43V: `corvid release` accepts a known channel + a version
    /// that matches the channel's pattern. This is the positive
    /// half of the `release.signed_artifact` contract — the
    /// validate-version path is the gatekeeper that prevents a
    /// nightly-shaped version from being shipped as stable.
    #[test]
    fn release_validate_version_accepts_each_channel_shape() {
        // Each channel accepts its own pattern.
        assert!(validate_version("nightly", "0.0.0-nightly.20260518").is_ok());
        assert!(validate_version("beta", "1.0.0-beta.3").is_ok());
        assert!(validate_version("stable", "1.0.0").is_ok());
        // normalize_channel accepts the three documented channels.
        assert!(normalize_channel("nightly").is_ok());
        assert!(normalize_channel("beta").is_ok());
        assert!(normalize_channel("stable").is_ok());
    }

    /// 43V: a channel/version mismatch is refused. Catches the
    /// failure mode where a stable version (1.0.0) is published
    /// to the nightly channel, or a nightly-suffixed version
    /// ships as stable.
    #[test]
    fn release_validate_version_refuses_channel_version_mismatch() {
        // Stable can't take a -nightly. or -beta. suffix.
        assert!(validate_version("stable", "1.0.0-nightly.20260518").is_err());
        assert!(validate_version("stable", "1.0.0-beta.3").is_err());
        // Nightly must have -nightly. — plain stable is refused.
        assert!(validate_version("nightly", "1.0.0").is_err());
        assert!(validate_version("nightly", "1.0.0-beta.3").is_err());
        // Beta must have -beta. — neither stable nor nightly is
        // accepted as beta.
        assert!(validate_version("beta", "1.0.0").is_err());
        assert!(validate_version("beta", "1.0.0-nightly.20260518").is_err());
        // Unknown channel refused at normalization.
        assert!(normalize_channel("preview").is_err());
        assert!(normalize_channel("rc").is_err());
        assert!(normalize_channel("").is_err());
    }

    /// Slice 35V2-P43-T-LR (positive, parsing): the
    /// `git log --pretty=format:%h\t%s` output parses into
    /// CommitLine pairs that preserve sha + subject exactly.
    /// Empty lines and malformed lines are dropped, never
    /// promoted to a stub entry.
    #[test]
    fn release_notes_parse_git_log_output_drops_malformed_lines() {
        let raw = "abc1234\tfeat(approvals): explain helper\n\
                   def5678\tfix(jobs): retry-wait clamp\n\
                   \n\
                   malformed-without-tab\n\
                   \t\n\
                   ff00aa1\t\n\
                   \tonly-subject\n\
                   012abcd\tdocs(guides): rewrite auth.md\n";
        let commits = parse_git_log_output(raw);
        assert_eq!(commits.len(), 3);
        assert_eq!(commits[0].sha, "abc1234");
        assert_eq!(
            commits[0].subject,
            "feat(approvals): explain helper"
        );
        assert_eq!(commits[1].sha, "def5678");
        assert_eq!(commits[2].sha, "012abcd");
    }

    /// Slice 35V2-P43-T-LR (positive, categorisation): each
    /// recognised conventional-commit prefix routes to the
    /// matching section. Unrecognised subjects go to "Other"
    /// rather than being silently dropped.
    #[test]
    fn release_notes_categorise_commits_routes_each_prefix() {
        let commits = vec![
            CommitLine {
                sha: "a".into(),
                subject: "feat: ship explain helper".into(),
            },
            CommitLine {
                sha: "b".into(),
                subject: "feat(scope): scoped feature".into(),
            },
            CommitLine {
                sha: "c".into(),
                subject: "fix(jobs): retry-wait clamp".into(),
            },
            CommitLine {
                sha: "d".into(),
                subject: "perf: hot loop".into(),
            },
            CommitLine {
                sha: "e".into(),
                subject: "refactor(parser): split file".into(),
            },
            CommitLine {
                sha: "f".into(),
                subject: "docs: rewrite guide".into(),
            },
            CommitLine {
                sha: "g".into(),
                subject: "test(runtime): add adversarial".into(),
            },
            CommitLine {
                sha: "h".into(),
                subject: "chore: bump dep".into(),
            },
            CommitLine {
                sha: "i".into(),
                subject: "ci(build): tighten matrix".into(),
            },
            CommitLine {
                sha: "j".into(),
                subject: "build: switch linker".into(),
            },
            CommitLine {
                sha: "k".into(),
                subject: "style: rustfmt".into(),
            },
            CommitLine {
                sha: "l".into(),
                subject: "Just a free-form commit".into(),
            },
        ];
        let grouped = categorise_commits(&commits);
        assert_eq!(grouped.features.len(), 2);
        assert_eq!(grouped.fixes.len(), 1);
        assert_eq!(grouped.perf.len(), 1);
        assert_eq!(grouped.refactors.len(), 1);
        assert_eq!(grouped.docs.len(), 1);
        assert_eq!(grouped.tests.len(), 1);
        // chore + ci + build + style all collapse to Chores.
        assert_eq!(grouped.chores.len(), 4);
        // Unrecognised subject falls through to Other.
        assert_eq!(grouped.other.len(), 1);
        assert_eq!(grouped.total(), commits.len());
    }

    /// Slice 35V2-P43-T-LR (adversarial, free-form subjects):
    /// commits that don't match the conventional-commit shape
    /// fall through to "Other" rather than being misrouted as
    /// feats or fixes via fuzzy matching. Drift would mean a
    /// docs change appearing in Features.
    #[test]
    fn release_notes_unrecognised_prefix_falls_through_to_other() {
        // `Feat:` with capital F doesn't match conventional shape (lowercase).
        assert_eq!(conventional_commit_prefix("Feat: thing"), None);
        // No colon at all.
        assert_eq!(conventional_commit_prefix("feat ship a thing"), None);
        // Empty prefix.
        assert_eq!(conventional_commit_prefix(": thing"), None);
        // Recognised lowercase prefix.
        assert_eq!(conventional_commit_prefix("feat: thing"), Some("feat"));
        assert_eq!(
            conventional_commit_prefix("feat(scope): thing"),
            Some("feat")
        );
        // Prefix with hyphen is allowed (some teams use
        // `pre-release:` etc.) — categoriser routes anything
        // not in the known set to Other.
        assert_eq!(
            conventional_commit_prefix("pre-release: thing"),
            Some("pre-release")
        );
        let unknown = categorise_commits(&[CommitLine {
            sha: "x".into(),
            subject: "pre-release: thing".into(),
        }]);
        assert_eq!(unknown.other.len(), 1);
        assert_eq!(unknown.features.len(), 0);
    }

    /// Slice 35V2-P43-T-LR (positive, markdown output): the
    /// renderer emits one section per non-empty category, in a
    /// stable order, and every line carries the short SHA so
    /// the notes back-reference the commit history (the
    /// Grounded<T>-flavoured property: every claim traces back
    /// to a SHA).
    #[test]
    fn release_notes_markdown_renders_sections_with_grounded_shas() {
        let grouped = CategorisedCommits {
            features: vec![CommitLine {
                sha: "a1b2c3d".into(),
                subject: "feat: ship X".into(),
            }],
            fixes: vec![CommitLine {
                sha: "e4f5g6h".into(),
                subject: "fix: clamp Y".into(),
            }],
            ..CategorisedCommits::default()
        };
        let md = render_release_notes_markdown("v1.0.0", "v1.0.1", &grouped);
        assert!(md.starts_with("# Release notes: `v1.0.0`..`v1.0.1`"));
        assert!(md.contains("## Features"));
        assert!(md.contains("- feat: ship X (a1b2c3d)"));
        assert!(md.contains("## Fixes"));
        assert!(md.contains("- fix: clamp Y (e4f5g6h)"));
        // No empty sections — Performance / Refactors / etc.
        // are absent because their commit lists are empty.
        assert!(!md.contains("## Performance"));
        assert!(!md.contains("## Refactors"));
        assert!(!md.contains("## Documentation"));
        // Features come before Fixes (stable section order).
        let feat_pos = md.find("## Features").unwrap();
        let fix_pos = md.find("## Fixes").unwrap();
        assert!(feat_pos < fix_pos);
    }

    /// Slice 35V2-P43-T-LR (adversarial, empty range): a
    /// from..to pair with no commits between produces a "No
    /// changes" stub, not a partially-rendered section header.
    /// Catches the silent-empty-output failure mode where a
    /// release issue would say "## Features" with no entries.
    #[test]
    fn release_notes_empty_range_renders_no_changes_stub() {
        let empty = CategorisedCommits::default();
        let md = render_release_notes_markdown("v1.0.0", "v1.0.0", &empty);
        assert!(md.contains("No changes between `v1.0.0` and `v1.0.0`"));
        // No section headers at all.
        assert!(!md.contains("## Features"));
        assert!(!md.contains("## Fixes"));
        assert!(!md.contains("## Other"));
    }

    /// Slice 35V2-P43-T-LR (adversarial, ref validation): a
    /// from/to value that looks like a flag (`--from=v1.0`) is
    /// refused before reaching `git log`. Catches the typo
    /// where an operator omits the `=` and the value is parsed
    /// as a positional starting with `-`.
    #[test]
    fn release_notes_ref_validation_refuses_empty_or_flag_shapes() {
        assert!(validate_git_ref("v1.0.0", "from").is_ok());
        assert!(validate_git_ref("HEAD", "to").is_ok());
        assert!(validate_git_ref("main", "to").is_ok());
        // Empty refused.
        assert!(validate_git_ref("", "from")
            .unwrap_err()
            .to_string()
            .contains("must not be empty"));
        assert!(validate_git_ref("   ", "from")
            .unwrap_err()
            .to_string()
            .contains("must not be empty"));
        // Flag-shaped refused.
        let err = validate_git_ref("--all", "from").unwrap_err().to_string();
        assert!(err.contains("looks like a flag"));
    }

    /// 43V: `sign_release_manifest` produces a DSSE envelope
    /// whose payload type names the v1 release manifest schema
    /// + whose payload decodes to JSON that round-trips through
    /// serde. This is the structural sanity check the
    /// `release.signed_artifact` row promises — the
    /// payload-type drift mode the audit was specifically
    /// guarding against.
    #[test]
    fn sign_release_manifest_emits_v1_payload_type() {
        std::env::set_var(
            "CORVID_RELEASE_SIGNING_KEY",
            "0".repeat(64),
        );
        let manifest_json = r#"{"schema":"corvid.release.manifest.v1","channel":"nightly"}"#;
        let envelope_json = sign_release_manifest(manifest_json).expect("sign manifest");
        let parsed: serde_json::Value =
            serde_json::from_str(&envelope_json).expect("envelope JSON");
        assert_eq!(
            parsed["payloadType"],
            "application/vnd.corvid.release.manifest.v1+json",
            "payload type must name the v1 schema; mismatch means the \
             release attestation's downstream consumers cannot rely \
             on the documented payload shape"
        );
        // DSSE envelope base64s the payload; decode + round-trip.
        use base64::Engine as _;
        let payload_b64 = parsed["payload"]
            .as_str()
            .expect("DSSE payload base64 field");
        let payload_bytes = base64::engine::general_purpose::STANDARD
            .decode(payload_b64)
            .expect("decode DSSE payload");
        let payload: serde_json::Value =
            serde_json::from_slice(&payload_bytes).expect("payload JSON");
        assert_eq!(payload["schema"], "corvid.release.manifest.v1");
        assert_eq!(payload["channel"], "nightly");
        std::env::remove_var("CORVID_RELEASE_SIGNING_KEY");
    }
}
