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

/// `corvid add mcp <name>` — write the config entry, discover the
/// server's tools, generate the typed module.
pub(crate) fn cmd_add_mcp(
    name: &str,
    cmd: &[String],
    url: Option<&str>,
    trusted: bool,
) -> Result<u8> {
    if cmd.is_empty() && url.is_none() {
        eprintln!("an MCP server needs a transport: `--cmd <command>...` (stdio) or `--url <url>` (http)");
        return Ok(1);
    }
    if !cmd.is_empty() && url.is_some() {
        eprintln!("pass either `--cmd` or `--url`, not both");
        return Ok(1);
    }
    let root = project_root();
    let toml_path = root.join("corvid.toml");
    let mut config_text = std::fs::read_to_string(&toml_path).unwrap_or_default();
    let section_header = format!("[mcp.servers.{name}]");
    if config_text.contains(&section_header) {
        eprintln!(
            "`{section_header}` already exists in `{}` — use `corvid mcp regen {name}` to \
             refresh its typed module",
            toml_path.display()
        );
        return Ok(1);
    }

    // Generate FIRST (discovery can fail; the config write should
    // only land for a server we actually reached).
    let servers = std::collections::HashMap::from([(
        name.to_string(),
        corvid_runtime::mcp::McpServerConfig {
            command: cmd.to_vec(),
            url: url.map(str::to_string),
            trusted,
        },
    )]);
    let tool_count = generate_mcp_module_file(&root, name, servers)?;

    if !config_text.is_empty() && !config_text.ends_with('\n') {
        config_text.push('\n');
    }
    config_text.push('\n');
    config_text.push_str(&section_header);
    config_text.push('\n');
    if !cmd.is_empty() {
        let rendered = cmd
            .iter()
            .map(|part| format!("\"{part}\""))
            .collect::<Vec<_>>()
            .join(", ");
        config_text.push_str(&format!("command = [{rendered}]\n"));
    }
    if let Some(url) = url {
        config_text.push_str(&format!("url = \"{url}\"\n"));
    }
    if trusted {
        config_text.push_str("trust = \"autonomous\"\n");
    } else {
        config_text.push_str("# untrusted by default: every mcp_call is approval-gated\n");
    }
    std::fs::write(&toml_path, config_text)?;

    println!(
        "added MCP server `{name}` ({} tool(s) discovered) — typed module at \
         `src/mcp/{name}.cor`, config in `{}`.{}",
        tool_count,
        toml_path.display(),
        if trusted {
            ""
        } else {
            " Untrusted: calls require approval; add `trust = \"autonomous\"` after review."
        }
    );
    Ok(0)
}

/// `corvid mcp regen <name>` — refresh the typed module from the
/// configured server.
pub(crate) fn cmd_mcp_regen(name: &str) -> Result<u8> {
    let root = project_root();
    let main_anchor = root.join("src").join("main.cor");
    let servers = corvid_driver::load_mcp_servers(&main_anchor);
    if !servers.contains_key(name) {
        eprintln!(
            "no `[mcp.servers.{name}]` in corvid.toml — add it first with `corvid add mcp {name}`"
        );
        return Ok(1);
    }
    let tool_count = generate_mcp_module_file(&root, name, servers)?;
    println!("regenerated `src/mcp/{name}.cor` ({tool_count} tool(s)).");
    Ok(0)
}

/// Discover tools + write the generated module. Returns the tool
/// count.
fn generate_mcp_module_file(
    root: &Path,
    name: &str,
    servers: std::collections::HashMap<String, corvid_runtime::mcp::McpServerConfig>,
) -> Result<usize> {
    let mcp = corvid_runtime::mcp::McpRuntime::new(servers);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let listings = runtime
        .block_on(mcp.list_tools(name))
        .map_err(|e| anyhow::anyhow!("tool discovery failed: {e}"))?;
    let tools: Vec<corvid_driver::mcp_codegen::McpToolDescriptor> = listings
        .iter()
        .filter_map(corvid_driver::mcp_codegen::McpToolDescriptor::from_listing)
        .collect();
    let module = corvid_driver::mcp_codegen::generate_mcp_module(name, &tools);
    let mcp_dir = root.join("src").join("mcp");
    std::fs::create_dir_all(&mcp_dir)?;
    std::fs::write(mcp_dir.join(format!("{name}.cor")), module)?;
    Ok(tools.len())
}
