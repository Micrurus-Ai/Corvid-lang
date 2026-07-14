//! `corvid add` — the capability surface: one verb for extending a
//! project with skills (effect-audited vendored packages), MCP
//! servers, and connectors. Slices 49a/49b ship the skill kind; MCP
//! and connector kinds ride 49c/49d. `corvid skill sign|update`
//! (publisher + maintenance verbs) live here too.

use anyhow::Result;
use corvid_driver::skills;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// Locate the project root: walk up from the current directory to
/// the nearest `corvid.toml`; without one, the current directory is
/// the root (matching how `corvid new` lays projects out).
fn project_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut cur = Some(cwd.as_path());
    while let Some(dir) = cur {
        if dir.join("corvid.toml").exists() {
            return dir.to_path_buf();
        }
        cur = dir.parent();
    }
    cwd
}

fn consent(question: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "refusing: consent required — re-run with `--yes` to accept the capability \
             label non-interactively"
        );
        return Ok(false);
    }
    print!("{question} [y/N] ");
    use std::io::Write as _;
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

pub(crate) fn cmd_add_skill(
    source: &str,
    yes: bool,
    publisher_key: Option<&Path>,
) -> Result<u8> {
    let root = project_root();
    let parsed = match skills::source::SkillSource::parse(source) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("invalid skill source: {err:#}");
            return Ok(1);
        }
    };
    let (plan, resolved) = match skills::plan_add_skill_from(&root, &parsed, publisher_key) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("refusing to add skill: {err:#}");
            return Ok(1);
        }
    };

    print!("{}", plan.rendered_label);
    match &plan.signature {
        skills::signing::SignatureStatus::Verified { key_id } => {
            println!("\n  signed — publisher key `{key_id}` verified; content hashes match.");
        }
        skills::signing::SignatureStatus::PresentUnverified => {
            println!(
                "\n  !! signature present but UNVERIFIED — pass `--publisher-key <path>` \
                 to check it."
            );
        }
        skills::signing::SignatureStatus::Unsigned => {}
    }
    println!(
        "\nwill vendor into `{}` — visible, git-diffable source; the label above is \
         re-verified on every `corvid check` / `corvid run`.",
        plan.destination.display()
    );

    if !consent("add this skill?", yes)? {
        println!("not added.");
        return Ok(1);
    }

    skills::vendor_skill(&plan, &resolved.skill_dir)?;
    println!(
        "added skill `{}` v{} (pinned to content hash {}).",
        plan.manifest.skill.name,
        plan.manifest.skill.version,
        &plan.content_hash[..12]
    );
    Ok(0)
}

/// `corvid skill sign <dir>` — publisher-side: sign the skill's
/// content manifest with an ed25519 key (`--key <path>` or the
/// CORVID_SIGNING_KEY env var).
pub(crate) fn cmd_skill_sign(dir: &Path, key: Option<&Path>) -> Result<u8> {
    let key_source = match key {
        Some(path) => corvid_abi::KeySource::Path(path.to_path_buf()),
        None => match std::env::var("CORVID_SIGNING_KEY") {
            Ok(value) => corvid_abi::KeySource::Env(value),
            Err(_) => {
                eprintln!(
                    "no signing key: pass `--key <path>` (64 hex chars or 32 raw bytes) or \
                     set CORVID_SIGNING_KEY"
                );
                return Ok(1);
            }
        },
    };
    match skills::signing::sign_skill(dir, &key_source) {
        Ok((key_id, verifying_hex)) => {
            println!(
                "signed `{}` — wrote skill.sig (key id `{key_id}`).\nyour VERIFYING key \
                 (distribute this; consumers pass it via `--publisher-key`):\n  {verifying_hex}",
                dir.display()
            );
            Ok(0)
        }
        Err(err) => {
            eprintln!("signing failed: {err:#}");
            Ok(1)
        }
    }
}

/// `corvid skill update <name>` — re-fetch the pinned source,
/// hash-diff, and re-consent on change.
pub(crate) fn cmd_skill_update(name: &str, yes: bool) -> Result<u8> {
    let root = project_root();
    match skills::pin::plan_update(&root, name) {
        Ok(skills::pin::UpdateOutcome::UpToDate) => {
            println!("skill `{name}` is up to date with its pinned source.");
            Ok(0)
        }
        Ok(skills::pin::UpdateOutcome::Changed {
            rendered_label,
            new_content_hash,
            staged,
        }) => {
            println!("upstream changed — the NEW label:");
            print!("{rendered_label}");
            if !consent("apply this update?", yes)? {
                println!("not updated.");
                return Ok(1);
            }
            skills::pin::apply_update(&root, name, &staged, &new_content_hash)?;
            println!(
                "updated `{name}` (re-pinned to content hash {}).",
                &new_content_hash[..12]
            );
            Ok(0)
        }
        Err(err) => {
            eprintln!("update failed: {err:#}");
            Ok(1)
        }
    }
}
