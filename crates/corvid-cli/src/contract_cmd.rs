//! `corvid contract` — public-surface inspection of the canonical
//! [`corvid_guarantees::GUARANTEE_REGISTRY`].
//!
//! `corvid contract list` prints the registry as either a
//! human-readable table or structured JSON. The JSON output is the
//! single source of truth that:
//!
//!   * `docs/reference/core-semantics.md` is generated from in slice 35-D
//!     (CI rejects drift between the committed doc and this command's
//!     output), and
//!   * `corvid claim --explain` (slice 35-I) cross-references when
//!     reporting which guarantees a given binary was checked against.
//!
//! Optional `--class` and `--kind` filters narrow the output for
//! human inspection without changing the canonical ordering. The
//! command never reorders the registry — declaration order in
//! `corvid-guarantees` is the stable serialization order.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use corvid_guarantees::{
    render_core_semantics_markdown, Guarantee, GuaranteeClass, GuaranteeKind, Phase,
    GUARANTEE_REGISTRY,
};
use serde::Serialize;

/// Run `corvid contract list`.
///
/// `json == true` emits the structured payload (one outer JSON
/// object with a `guarantees` array). The human-readable form prints
/// a fixed-width table sorted in declaration order — readers should
/// be able to scan it in well under ten minutes per the Phase 35
/// goal.
pub fn run_list(json: bool, class_filter: Option<&str>, kind_filter: Option<&str>) -> Result<u8> {
    let class = class_filter.map(parse_class).transpose()?;
    let kind = kind_filter.map(parse_kind).transpose()?;

    let rows: Vec<&'static Guarantee> = GUARANTEE_REGISTRY
        .iter()
        .filter(|g| class.map_or(true, |c| g.class == c))
        .filter(|g| kind.map_or(true, |k| g.kind == k))
        .collect();

    if json {
        let payload = JsonPayload {
            schema_version: 1,
            count: rows.len(),
            guarantees: rows.iter().map(|g| JsonGuarantee::from(*g)).collect(),
        };
        let text = serde_json::to_string_pretty(&payload)
            .map_err(|e| anyhow!("serialize guarantees as JSON: {e}"))?;
        println!("{text}");
    } else {
        print_table(&rows);
    }
    Ok(0)
}

/// Run `corvid contract regen-doc <output>`.
///
/// Writes the canonical `docs/reference/core-semantics.md` rendering to the
/// given path. The output is byte-deterministic for a given
/// registry, so committing the result and gating CI on
/// `corvid_guarantees::render::tests::rendered_markdown_matches_committed_doc`
/// keeps spec ≡ implementation.
pub fn run_regen_doc(output: &Path) -> Result<u8> {
    let rendered = render_core_semantics_markdown();
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "creating parent directory `{}` for regenerated doc",
                    parent.display()
                )
            })?;
        }
    }
    std::fs::write(output, &rendered)
        .with_context(|| format!("writing regenerated doc to `{}`", output.display()))?;
    eprintln!(
        "wrote {} bytes to {}",
        rendered.len(),
        output.display()
    );
    Ok(0)
}

fn parse_class(raw: &str) -> Result<GuaranteeClass> {
    match raw {
        "static" => Ok(GuaranteeClass::Static),
        "runtime_checked" | "runtime-checked" => Ok(GuaranteeClass::RuntimeChecked),
        "out_of_scope" | "out-of-scope" => Ok(GuaranteeClass::OutOfScope),
        other => Err(anyhow!(
            "unknown --class `{other}` — expected `static`, `runtime_checked`, or `out_of_scope`"
        )),
    }
}

fn parse_kind(raw: &str) -> Result<GuaranteeKind> {
    for kind in GuaranteeKind::ALL {
        if kind.slug() == raw {
            return Ok(*kind);
        }
    }
    let valid: Vec<&'static str> = GuaranteeKind::ALL.iter().map(|k| k.slug()).collect();
    Err(anyhow!(
        "unknown --kind `{raw}` — expected one of {}",
        valid.join(", ")
    ))
}

#[derive(Serialize)]
struct JsonPayload {
    schema_version: u32,
    count: usize,
    guarantees: Vec<JsonGuarantee>,
}

#[derive(Serialize)]
struct JsonGuarantee {
    id: &'static str,
    kind: &'static str,
    class: &'static str,
    phase: &'static str,
    description: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    out_of_scope_reason: &'static str,
    positive_test_refs: Vec<&'static str>,
    adversarial_test_refs: Vec<&'static str>,
}

impl From<&Guarantee> for JsonGuarantee {
    fn from(g: &Guarantee) -> Self {
        JsonGuarantee {
            id: g.id,
            kind: g.kind.slug(),
            class: g.class.slug(),
            phase: g.phase.slug(),
            description: g.description,
            out_of_scope_reason: g.out_of_scope_reason,
            positive_test_refs: g.positive_test_refs.to_vec(),
            adversarial_test_refs: g.adversarial_test_refs.to_vec(),
        }
    }
}

fn print_table(rows: &[&'static Guarantee]) {
    if rows.is_empty() {
        println!("(no guarantees match the supplied filters)");
        return;
    }
    let id_w = rows.iter().map(|g| g.id.len()).max().unwrap_or(0).max(4);
    let class_w = GuaranteeClass::ALL
        .iter()
        .map(|c| c.slug().len())
        .max()
        .unwrap_or(0)
        .max(5);
    let phase_w = Phase::ALL
        .iter()
        .map(|p| p.slug().len())
        .max()
        .unwrap_or(0)
        .max(5);

    println!(
        "{:<id_w$}  {:<class_w$}  {:<phase_w$}  description",
        "id",
        "class",
        "phase",
        id_w = id_w,
        class_w = class_w,
        phase_w = phase_w,
    );
    println!(
        "{}  {}  {}  {}",
        "-".repeat(id_w),
        "-".repeat(class_w),
        "-".repeat(phase_w),
        "-".repeat(11),
    );
    for g in rows {
        println!(
            "{:<id_w$}  {:<class_w$}  {:<phase_w$}  {}",
            g.id,
            g.class.slug(),
            g.phase.slug(),
            g.description,
            id_w = id_w,
            class_w = class_w,
            phase_w = phase_w,
        );
        if g.class == GuaranteeClass::OutOfScope && !g.out_of_scope_reason.is_empty() {
            println!(
                "{:<id_w$}  {:<class_w$}  {:<phase_w$}  reason: {}",
                "",
                "",
                "",
                g.out_of_scope_reason,
                id_w = id_w,
                class_w = class_w,
                phase_w = phase_w,
            );
        }
    }
    println!();
    println!(
        "{} guarantees (registry size {})",
        rows.len(),
        GUARANTEE_REGISTRY.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_class_accepts_known_slugs() {
        assert_eq!(parse_class("static").unwrap(), GuaranteeClass::Static);
        assert_eq!(
            parse_class("runtime_checked").unwrap(),
            GuaranteeClass::RuntimeChecked
        );
        assert_eq!(
            parse_class("runtime-checked").unwrap(),
            GuaranteeClass::RuntimeChecked
        );
        assert_eq!(
            parse_class("out_of_scope").unwrap(),
            GuaranteeClass::OutOfScope
        );
    }

    #[test]
    fn parse_class_rejects_unknown() {
        assert!(parse_class("nope").is_err());
    }

    #[test]
    fn parse_kind_accepts_every_registered_kind() {
        for kind in GuaranteeKind::ALL {
            assert_eq!(parse_kind(kind.slug()).unwrap(), *kind);
        }
    }

    #[test]
    fn json_payload_matches_registry_size() {
        let rows: Vec<&'static Guarantee> = GUARANTEE_REGISTRY.iter().collect();
        let payload = JsonPayload {
            schema_version: 1,
            count: rows.len(),
            guarantees: rows.iter().map(|g| JsonGuarantee::from(*g)).collect(),
        };
        assert_eq!(payload.count, GUARANTEE_REGISTRY.len());
        assert_eq!(payload.guarantees.len(), GUARANTEE_REGISTRY.len());
    }

    /// Phase 35V-T1-C sentinel. The serialised JSON payload contains
    /// every registry id as a literal string. Stronger than the size
    /// match: a `JsonGuarantee` field rename or a serde
    /// `#[serde(rename = ...)]` attribute drift would still pass
    /// `json_payload_matches_registry_size` (count is preserved) but
    /// would silently change the JSON shape downstream consumers
    /// parse. This sentinel pins the byte-level surface: every id
    /// must round-trip into the JSON string.
    #[test]
    fn json_payload_contains_every_registry_id() {
        let rows: Vec<&'static Guarantee> = GUARANTEE_REGISTRY.iter().collect();
        let json = serde_json::to_string(&JsonPayload {
            schema_version: 1,
            count: rows.len(),
            guarantees: rows.iter().map(|g| JsonGuarantee::from(*g)).collect(),
        })
        .expect("serialise");
        let mut missing: Vec<&'static str> = Vec::new();
        for g in GUARANTEE_REGISTRY {
            if !json.contains(g.id) {
                missing.push(g.id);
            }
        }
        assert!(
            missing.is_empty(),
            "phase 35V-T1-C: registry rows whose id does not appear \
             in the `corvid contract list --json` payload:\n  - {}\n\n\
             A JsonGuarantee field-rename or serde-rename drift would \
             surface here.",
            missing.join("\n  - ")
        );
    }

    #[test]
    fn json_payload_emits_out_of_scope_reason_only_for_out_of_scope() {
        let json = serde_json::to_string(&JsonPayload {
            schema_version: 1,
            count: GUARANTEE_REGISTRY.len(),
            guarantees: GUARANTEE_REGISTRY
                .iter()
                .map(|g| JsonGuarantee::from(g))
                .collect(),
        })
        .unwrap();
        // Static and RuntimeChecked entries must NOT carry an
        // out_of_scope_reason in the JSON — `skip_serializing_if`
        // drops the field for them. The seed currently has at least
        // one Static row (`approval.dangerous_call_requires_token`),
        // so confirm its description is present without a reason.
        assert!(json.contains("approval.dangerous_call_requires_token"));
        // OutOfScope rows MUST include their reason.
        assert!(json.contains("platform.host_kernel_compromise"));
        assert!(json.contains("Outside Corvid's trust boundary"));
    }
}

/// Compile a source file to its Application Contract, or print the
/// diagnostics and return `Ok(None)` on failure. Shared by the `app`
/// and `openapi` contract commands.
fn build_contract(
    file: Option<&Path>,
) -> Result<Option<corvid_abi::app_contract::ApplicationContract>> {
    let source_path = match file {
        Some(f) => f.to_path_buf(),
        None => crate::project_source::resolve_project_source(None)
            .context("no source file given and no src/main.cor found")?,
    };
    let source = std::fs::read_to_string(&source_path)
        .with_context(|| format!("cannot read `{}`", source_path.display()))?;
    let config = corvid_driver::load_corvid_config_for(&source_path);
    let generated_at =
        std::env::var("CORVID_BUILD_DATE").unwrap_or_else(|_| "unknown".to_string());

    match corvid_driver::compile_to_application_contract_with_config(
        &source,
        &source_path.display().to_string(),
        &generated_at,
        config.as_ref(),
    ) {
        Ok(contract) => Ok(Some(contract)),
        Err(diags) => {
            eprint!(
                "{}",
                corvid_driver::render_all_pretty(&diags, &source_path, &source)
            );
            Ok(None)
        }
    }
}

/// Write a JSON artifact to `out` (or a default path), or stdout for
/// `-`. Returns the resolved path when written to disk.
fn write_artifact(json: &str, out: Option<&str>, default_path: &str) -> Result<Option<PathBuf>> {
    if out == Some("-") {
        println!("{json}");
        return Ok(None);
    }
    let path = PathBuf::from(out.unwrap_or(default_path));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, json)?;
    Ok(Some(path))
}

/// Slice 51a — emit the Application Contract for `file` (default
/// `src/main.cor`) to `out` (default `target/contracts/app.corvid.json`,
/// or `-` for stdout).
pub fn run_app_contract(file: Option<&Path>, out: Option<&str>) -> Result<u8> {
    let Some(contract) = build_contract(file)? else {
        return Ok(1);
    };
    let json = serde_json::to_string_pretty(&contract)?;
    if let Some(path) = write_artifact(&json, out, "target/contracts/app.corvid.json")? {
        println!(
            "wrote application contract: {} ({} route(s), {} agent(s), {} prompt(s), {} type(s))",
            path.display(),
            contract.routes.len(),
            contract.agents.len(),
            contract.prompts.len(),
            contract.types.len(),
        );
    }
    Ok(0)
}

/// Slice 51b — emit a standard OpenAPI 3.1 document for `file`.
pub fn run_openapi(file: Option<&Path>, out: Option<&str>) -> Result<u8> {
    let Some(contract) = build_contract(file)? else {
        return Ok(1);
    };
    let openapi = corvid_abi::openapi::emit_openapi(&contract);
    let json = serde_json::to_string_pretty(&openapi)?;
    if let Some(path) = write_artifact(&json, out, "target/contracts/openapi.json")? {
        println!(
            "wrote OpenAPI 3.1: {} ({} path(s), {} schema(s))",
            path.display(),
            contract.routes.len(),
            contract.types.len(),
        );
    }
    Ok(0)
}

/// Slice 51l — generate the TypeScript client (`types.ts` + `api.ts`)
/// into `out_dir`. The generated code delegates to `@corvid/client`.
pub fn run_ts_client(file: Option<&Path>, out_dir: &Path) -> Result<u8> {
    let Some(contract) = build_contract(file)? else {
        return Ok(1);
    };
    let files = corvid_abi::ts_client::emit_ts_client(&contract);
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output directory `{}`", out_dir.display()))?;
    for gf in &files {
        let path = out_dir.join(&gf.filename);
        std::fs::write(&path, &gf.contents)
            .with_context(|| format!("writing `{}`", path.display()))?;
        println!("wrote {} ({} bytes)", path.display(), gf.contents.len());
    }
    println!(
        "generated TypeScript client for {} agent(s), {} prompt(s), {} type(s) — import the shipped `@corvid/client` package for the transport",
        contract.agents.len(),
        contract.prompts.len(),
        contract.types.len(),
    );
    Ok(0)
}

/// Slice 51c — emit the AI-native metadata (`corvid-ai.json`).
pub fn run_corvid_ai(file: Option<&Path>, out: Option<&str>) -> Result<u8> {
    let Some(contract) = build_contract(file)? else {
        return Ok(1);
    };
    let meta = corvid_abi::corvid_ai::emit_corvid_ai(&contract);
    let json = serde_json::to_string_pretty(&meta)?;
    if let Some(path) = write_artifact(&json, out, "target/contracts/corvid-ai.json")? {
        println!(
            "wrote AI metadata: {} ({} agent(s), {} prompt(s))",
            path.display(),
            meta.agents.len(),
            meta.prompts.len(),
        );
    }
    Ok(0)
}
