//! The per-skill pin: `skill.lock` inside the vendored directory.
//!
//! Records where the skill came from and the content hash the user
//! consented to, so `corvid skill update` can re-fetch the SAME
//! source reproducibly and hash-diff before asking for consent
//! again. Lives with the vendored artifact (git-diffable, travels
//! with the project) rather than in the package-manager lockfile —
//! Corvid.lock stays package-only until the registry story.

use std::path::Path;

pub const SKILL_PIN_FILE: &str = "skill.lock";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillPin {
    /// The source string as given to `corvid add skill` (local path,
    /// `git:...`, `github:...`).
    pub source: String,
    /// Hex sha256 of the canonical content manifest at consent time.
    pub content_hash: String,
    /// Signing key id when the install verified a signature; empty
    /// for unsigned installs.
    #[serde(default)]
    pub signed_key_id: String,
}

pub fn write_pin(vendored_dir: &Path, pin: &SkillPin) -> anyhow::Result<()> {
    let toml = toml::to_string_pretty(pin)?;
    std::fs::write(vendored_dir.join(SKILL_PIN_FILE), toml)?;
    Ok(())
}

pub fn read_pin(vendored_dir: &Path) -> anyhow::Result<SkillPin> {
    let path = vendored_dir.join(SKILL_PIN_FILE);
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "no readable {SKILL_PIN_FILE} at `{}` — was this skill added by `corvid add \
             skill`? ({e})",
            path.display()
        )
    })?;
    Ok(toml::from_str(&raw)?)
}

/// The outcome of `corvid skill update <name>`.
#[derive(Debug)]
pub enum UpdateOutcome {
    /// Upstream content hash matches the pin — nothing to do.
    UpToDate,
    /// Upstream changed; carries the freshly rendered label and the
    /// staged checkout to vendor from after consent.
    Changed {
        rendered_label: String,
        new_content_hash: String,
        staged: super::source::ResolvedSource,
    },
}

/// Stage an update: re-fetch the pinned source, hash-diff, and (on
/// change) re-audit + re-verify the label so the caller can render
/// it for fresh consent. Never mutates the vendored skill itself.
pub fn plan_update(project_root: &Path, name: &str) -> anyhow::Result<UpdateOutcome> {
    let vendored = project_root.join("src").join("skills").join(name);
    let pin = read_pin(&vendored)?;
    let source = super::source::SkillSource::parse(&pin.source)?;
    let staged = super::source::resolve(&source)?;

    let manifest = super::load_manifest(&staged.skill_dir)?;
    if manifest.skill.name != name {
        anyhow::bail!(
            "the pinned source now serves skill `{}`, not `{name}` — refusing a \
             name-swapped update",
            manifest.skill.name
        );
    }
    let content = super::signing::content_manifest(&staged.skill_dir)?;
    let new_content_hash = super::signing::content_hash(&content)?;
    if new_content_hash == pin.content_hash {
        return Ok(UpdateOutcome::UpToDate);
    }
    let audit = super::compute_skill_audit(&staged.skill_dir)?;
    let violations = super::verify_label(&manifest, &audit);
    if !violations.is_empty() {
        anyhow::bail!(
            "the updated skill's label does not cover its source:\n  {}",
            violations.join("\n  ")
        );
    }
    let rendered_label = super::render_label(&manifest, &audit, false);
    Ok(UpdateOutcome::Changed {
        rendered_label,
        new_content_hash,
        staged,
    })
}

/// Apply a staged update after consent: replace the vendored dir and
/// re-pin.
pub fn apply_update(
    project_root: &Path,
    name: &str,
    staged: &super::source::ResolvedSource,
    new_content_hash: &str,
) -> anyhow::Result<()> {
    let vendored = project_root.join("src").join("skills").join(name);
    let pin = read_pin(&vendored)?;
    std::fs::remove_dir_all(&vendored)?;
    std::fs::create_dir_all(&vendored)?;
    super::copy_dir(&staged.skill_dir, &vendored)?;
    write_pin(
        &vendored,
        &SkillPin {
            source: pin.source,
            content_hash: new_content_hash.to_string(),
            signed_key_id: pin.signed_key_id,
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, version: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("skill.toml"),
            format!("[skill]\nname = \"pinned\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
        std::fs::write(dir.join("main.cor"), body).unwrap();
    }

    #[test]
    fn update_detects_upstream_change_and_up_to_date() {
        let tmp = tempfile::tempdir().unwrap();
        let upstream = tmp.path().join("upstream");
        write_skill(
            &upstream,
            "0.1.0",
            "public agent hello() -> String:\n    return \"hi\"\n",
        );

        // Install: vendor + pin (via the public add path).
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let plan = crate::skills::plan_add_skill(&project, &upstream).unwrap();
        crate::skills::vendor_skill(&plan, &upstream).unwrap();
        let content = crate::skills::signing::content_manifest(&upstream).unwrap();
        let hash = crate::skills::signing::content_hash(&content).unwrap();
        write_pin(
            &plan.destination,
            &SkillPin {
                source: upstream.display().to_string(),
                content_hash: hash,
                signed_key_id: String::new(),
            },
        )
        .unwrap();

        // Unchanged upstream → UpToDate.
        match plan_update(&project, "pinned").unwrap() {
            UpdateOutcome::UpToDate => {}
            other => panic!("expected UpToDate; got {other:?}"),
        }

        // Changed upstream → Changed with a fresh label; apply re-pins.
        write_skill(
            &upstream,
            "0.2.0",
            "public agent hello() -> String:\n    return \"hello again\"\n",
        );
        let (staged, new_hash) = match plan_update(&project, "pinned").unwrap() {
            UpdateOutcome::Changed {
                staged,
                new_content_hash,
                rendered_label,
            } => {
                assert!(rendered_label.contains("pinned v0.2.0"), "{rendered_label}");
                (staged, new_content_hash)
            }
            other => panic!("expected Changed; got {other:?}"),
        };
        apply_update(&project, "pinned", &staged, &new_hash).unwrap();
        let pin = read_pin(&project.join("src").join("skills").join("pinned")).unwrap();
        assert_eq!(pin.content_hash, new_hash);
        match plan_update(&project, "pinned").unwrap() {
            UpdateOutcome::UpToDate => {}
            other => panic!("expected UpToDate after apply; got {other:?}"),
        }
    }
}
