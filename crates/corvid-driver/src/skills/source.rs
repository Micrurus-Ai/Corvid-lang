//! Skill source resolution: local paths, `git:` URLs, and
//! `github:org/repo` shorthand — registry-free distribution with
//! shallow fetches. A fetched source lands in a temp checkout the
//! caller audits and vendors from; the ORIGINAL source string plus
//! the content hash go into the skill's pin so `corvid skill update`
//! can re-fetch reproducibly.

use std::path::PathBuf;

/// A parsed skill source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSource {
    /// A local directory containing skill.toml.
    Local(PathBuf),
    /// A git repository URL, optional `#ref`, optional `//subdir`.
    Git {
        url: String,
        reference: Option<String>,
        subdir: Option<String>,
    },
}

impl SkillSource {
    /// Parse the CLI source string:
    /// - `github:org/repo[/sub/dir][@ref]` — GitHub shorthand
    /// - `git:<url>[#ref][//subdir]` — any git URL
    /// - anything else — a local path
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        if let Some(spec) = raw.strip_prefix("github:") {
            let (path_part, reference) = match spec.split_once('@') {
                Some((p, r)) => (p, Some(r.to_string())),
                None => (spec, None),
            };
            let mut segments = path_part.splitn(3, '/');
            let org = segments.next().unwrap_or_default();
            let repo = segments.next().unwrap_or_default();
            if org.is_empty() || repo.is_empty() {
                anyhow::bail!(
                    "github source must be `github:org/repo[/subdir][@ref]`; got `{raw}`"
                );
            }
            let subdir = segments.next().map(str::to_string);
            return Ok(Self::Git {
                url: format!("https://github.com/{org}/{repo}.git"),
                reference,
                subdir,
            });
        }
        if let Some(spec) = raw.strip_prefix("git:") {
            // The `//` subdir separator must not collide with the
            // URL scheme's own `://` — search after the scheme.
            let scheme_end = spec.find("://").map(|i| i + 3).unwrap_or(0);
            let (url_and_ref, subdir) = match spec[scheme_end..].find("//") {
                Some(i) => {
                    let split = scheme_end + i;
                    (&spec[..split], Some(spec[split + 2..].to_string()))
                }
                None => (spec, None),
            };
            let (url, reference) = match url_and_ref.rsplit_once('#') {
                Some((u, r)) => (u.to_string(), Some(r.to_string())),
                None => (url_and_ref.to_string(), None),
            };
            if url.is_empty() {
                anyhow::bail!("git source must be `git:<url>[#ref][//subdir]`; got `{raw}`");
            }
            return Ok(Self::Git {
                url,
                reference,
                subdir,
            });
        }
        Ok(Self::Local(PathBuf::from(raw)))
    }
}

/// A resolved checkout: the directory holding the skill source. The
/// optional tempdir keeps a git checkout alive until vendoring
/// completes.
pub struct ResolvedSource {
    pub skill_dir: PathBuf,
    _checkout: Option<tempfile::TempDir>,
}

impl std::fmt::Debug for ResolvedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedSource")
            .field("skill_dir", &self.skill_dir)
            .finish()
    }
}

/// Resolve a source to a local directory, shallow-cloning git
/// sources.
pub fn resolve(source: &SkillSource) -> anyhow::Result<ResolvedSource> {
    match source {
        SkillSource::Local(path) => Ok(ResolvedSource {
            skill_dir: path.clone(),
            _checkout: None,
        }),
        SkillSource::Git {
            url,
            reference,
            subdir,
        } => {
            let checkout = tempfile::tempdir()?;
            let mut cmd = std::process::Command::new("git");
            cmd.arg("clone").arg("--depth").arg("1");
            if let Some(reference) = reference {
                cmd.arg("--branch").arg(reference);
            }
            cmd.arg(url).arg(checkout.path());
            let output = cmd
                .output()
                .map_err(|e| anyhow::anyhow!("cannot run git (is it installed?): {e}"))?;
            if !output.status.success() {
                anyhow::bail!(
                    "git clone of `{url}`{} failed:\n{}",
                    reference
                        .as_deref()
                        .map(|r| format!(" at `{r}`"))
                        .unwrap_or_default(),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let skill_dir = match subdir {
                Some(sub) => checkout.path().join(sub),
                None => checkout.path().to_path_buf(),
            };
            if !skill_dir.join("skill.toml").exists() {
                anyhow::bail!(
                    "no skill.toml at `{}` in the fetched repository — for a skill in a \
                     subdirectory use `github:org/repo/<subdir>` or `git:<url>//<subdir>`",
                    skill_dir.display()
                );
            }
            Ok(ResolvedSource {
                skill_dir,
                _checkout: Some(checkout),
            })
        }
    }
}

/// Render the source back to the string form recorded in the pin.
pub fn source_string(source: &SkillSource) -> String {
    match source {
        SkillSource::Local(path) => path.display().to_string(),
        SkillSource::Git {
            url,
            reference,
            subdir,
        } => {
            let mut out = format!("git:{url}");
            if let Some(r) = reference {
                out.push('#');
                out.push_str(r);
            }
            if let Some(s) = subdir {
                out.push_str("//");
                out.push_str(s);
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_shorthand_with_subdir_and_ref() {
        let source = SkillSource::parse("github:corvid-lang/skills/summarize@v1.2").unwrap();
        assert_eq!(
            source,
            SkillSource::Git {
                url: "https://github.com/corvid-lang/skills.git".into(),
                reference: Some("v1.2".into()),
                subdir: Some("summarize".into()),
            }
        );
    }

    #[test]
    fn parses_git_url_with_ref_and_subdir() {
        let source =
            SkillSource::parse("git:https://example.com/r.git#main//skills/x").unwrap();
        assert_eq!(
            source,
            SkillSource::Git {
                url: "https://example.com/r.git".into(),
                reference: Some("main".into()),
                subdir: Some("skills/x".into()),
            }
        );
    }

    #[test]
    fn bare_path_is_local() {
        assert_eq!(
            SkillSource::parse("../my-skill").unwrap(),
            SkillSource::Local(PathBuf::from("../my-skill"))
        );
    }

    /// End-to-end against a REAL local git repo (file:// clone) —
    /// shallow fetch, subdir resolution, skill.toml discovery.
    #[test]
    fn resolves_skill_from_local_git_repository() {
        let upstream = tempfile::tempdir().unwrap();
        let skill_dir = upstream.path().join("skills").join("greeter");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            "[skill]\nname = \"greeter\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("main.cor"),
            "public agent hello() -> String:\n    return \"hi\"\n",
        )
        .unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(upstream.path())
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git runs");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "skill"]);

        let url = format!(
            "file:///{}",
            upstream.path().display().to_string().replace('\\', "/")
        );
        let source = SkillSource::parse(&format!("git:{url}//skills/greeter")).unwrap();
        let resolved = resolve(&source).expect("clone resolves");
        assert!(resolved.skill_dir.join("skill.toml").exists());
    }
}
