//! `corvid add` — the capability surface: one verb for extending a
//! project with skills (effect-audited vendored packages), MCP
//! servers, and connectors. Slice 49a ships the skill kind; MCP and
//! connector kinds ride 49c/49d.

use anyhow::Result;
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

pub(crate) fn cmd_add_skill(source: &Path, yes: bool) -> Result<u8> {
    let root = project_root();
    let plan = match corvid_driver::skills::plan_add_skill(&root, source) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("refusing to add skill: {err:#}");
            return Ok(1);
        }
    };

    print!("{}", plan.rendered_label);
    println!(
        "\nwill vendor into `{}` — visible, git-diffable source; the label above is \
         re-verified on every `corvid check` / `corvid run`.",
        plan.destination.display()
    );

    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!(
                "refusing: consent required — re-run with `--yes` to accept the capability \
                 label non-interactively"
            );
            return Ok(1);
        }
        print!("add this skill? [y/N] ");
        use std::io::Write as _;
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("not added.");
            return Ok(1);
        }
    }

    corvid_driver::skills::vendor_skill(&plan, source)?;
    println!(
        "added skill `{}` v{}.",
        plan.manifest.skill.name, plan.manifest.skill.version
    );
    Ok(0)
}
