//! Operator-facing boot summary derived from an ABI descriptor.
//!
//! `BootSummary` is the assistive helper that an operator (or
//! orchestration layer) reads when booting a Corvid app. It is
//! computed entirely from the ABI descriptor — no LLM call, no
//! runtime probing, no network hop — and is replay-stable: two
//! invocations on the same descriptor produce byte-identical
//! output. Every field that the renderer surfaces is paired with
//! a `BootSource` entry naming the descriptor field that supplied
//! the value, mirroring the Grounded<T> sources posture used by
//! the drift narrator (`connector.drift_narration_grounded`).
//!
//! In-binary guarantee anchor:
//! `GUARANTEE_ID_APP_BOOT_SUMMARY_GROUNDED` — declares
//! `app.boot_summary_grounded` to the launch-readiness coverage
//! matrix. The positive corpus row asserts the summary's
//! `sources` array is non-empty and references the descriptor
//! fields actually consulted; the adversarial row asserts an
//! empty-surface descriptor produces a typed grounded summary
//! (counts == 0, sources still non-empty) rather than a panic
//! or a sourceless string.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::canonical_hash::hash_abi;
use crate::schema::{AbiAgent, AbiApprovalSite, AbiClaimGuarantee, AbiTool, CorvidAbi};

/// In-binary anchor for the boot-summary launch-readiness row.
/// The CLI's `corvid app boot-summary <source.cor>` command
/// delegates to [`boot_summary_from_descriptor`] and exposes the
/// Grounded<T>-shaped sources to operators. The coverage gate
/// refuses to promote `app.boot_summary_grounded` from declared
/// to runtime-checked unless a positive corpus row asserts a
/// non-empty sources array and an adversarial row asserts the
/// empty-surface case still produces a typed grounded summary.
pub const GUARANTEE_ID_APP_BOOT_SUMMARY_GROUNDED: &str = "app.boot_summary_grounded";

/// Names a single descriptor field that a [`BootSummary`] value
/// was derived from. Every populated boot-summary field is
/// paired with one or more sources — analogous to the typed
/// `sources: Vec<...>` array on `DriftNarration`. The contract:
/// no derived value is reported without an accompanying source
/// entry naming its provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BootSource {
    /// Dotted path naming the descriptor field consulted, e.g.
    /// `descriptor.agents`, `descriptor.tools[].dangerous`,
    /// `descriptor.claim_guarantees`.
    pub descriptor_field: String,
    /// One-line operator-facing justification of what the
    /// field contributed to the summary.
    pub note: String,
}

/// Per-surface population counts a boot operator wants to see
/// before flipping a deploy switch. Mirrors the descriptor's
/// own collection structure exactly so the operator can
/// cross-check counts against `corvid claim --explain` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootSurfaceCounts {
    pub agents: usize,
    pub prompts: usize,
    pub tools: usize,
    pub types: usize,
    pub stores: usize,
    pub approval_sites: usize,
}

/// Single enforced-guarantee row visible at boot time. The
/// runtime gate keyed by `id` is what actually fails the boot
/// if violated; the boot summary just surfaces the list of ids
/// the descriptor claims, so the operator can sanity-check that
/// every guarantee they expect for this app appears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootEnforcedGuarantee {
    pub id: String,
    pub kind: String,
    pub class: String,
    pub phase: String,
}

/// Summary of one approval site visible at boot time. Operators
/// inspect this list to confirm that every dangerous effect
/// surface the team expected an approval gate on actually has
/// one declared in the descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootApprovalGate {
    pub label: String,
    pub required_tier: String,
    pub dangerous_target_count: usize,
}

/// Names a single `pub extern "c"` agent that the cdylib will
/// expose to a linked host. The operator typically calls these
/// directly from the embedding process (e.g. via
/// `corvid_call_agent` JSON dispatch or the typed scalar ABI),
/// so the boot summary surfaces them up front as the visible
/// entrypoints of the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootFlagshipEntrypoint {
    pub agent_name: String,
    pub symbol: String,
    pub dangerous: bool,
}

/// Operator-facing summary of a Corvid app's surface, derived
/// from the ABI descriptor. Every field surfaces a structural
/// fact about the app that an operator wants to verify before
/// booting it. The accompanying `sources` array names every
/// descriptor field consulted in the derivation — the Grounded<T>
/// posture that `app.boot_summary_grounded` promotes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootSummary {
    pub app_name: String,
    pub source_path: String,
    pub compiler_version: String,
    pub corvid_abi_version: u32,
    pub descriptor_sha256: String,
    pub surface_counts: BootSurfaceCounts,
    pub flagship_entrypoints: Vec<BootFlagshipEntrypoint>,
    pub enforced_guarantees: Vec<BootEnforcedGuarantee>,
    pub approval_gates: Vec<BootApprovalGate>,
    pub dangerous_tool_count: usize,
    pub dangerous_agent_count: usize,
    pub stores_writeable: bool,
    pub sources: Vec<BootSource>,
}

/// Derives the operator-facing boot summary from a descriptor.
/// Pure function — no I/O, no clock, no randomness. The hash in
/// `descriptor_sha256` is the canonical descriptor hash that
/// `corvid claim --explain` reports, so an operator can confirm
/// at a glance that the boot summary describes the same binary
/// they are about to load.
pub fn boot_summary_from_descriptor(descriptor: &CorvidAbi) -> BootSummary {
    let mut sources = Vec::new();
    sources.push(BootSource {
        descriptor_field: "descriptor.source_path".to_string(),
        note: "app name derived from source path basename".to_string(),
    });
    sources.push(BootSource {
        descriptor_field: "descriptor.compiler_version".to_string(),
        note: "compiler version that produced this descriptor".to_string(),
    });
    sources.push(BootSource {
        descriptor_field: "descriptor.corvid_abi_version".to_string(),
        note: "ABI schema version the descriptor was emitted under".to_string(),
    });
    sources.push(BootSource {
        descriptor_field: "descriptor".to_string(),
        note: "descriptor_sha256 is the canonical hash of the full descriptor".to_string(),
    });

    let surface_counts = derive_surface_counts(descriptor, &mut sources);
    let flagship_entrypoints = derive_flagship_entrypoints(&descriptor.agents, &mut sources);
    let enforced_guarantees =
        derive_enforced_guarantees(&descriptor.claim_guarantees, &mut sources);
    let approval_gates = derive_approval_gates(&descriptor.approval_sites, &mut sources);
    let dangerous_tool_count = count_dangerous_tools(&descriptor.tools, &mut sources);
    let dangerous_agent_count = count_dangerous_agents(&descriptor.agents, &mut sources);
    let stores_writeable = derive_stores_writeable(descriptor, &mut sources);

    BootSummary {
        app_name: derive_app_name(&descriptor.source_path),
        source_path: descriptor.source_path.clone(),
        compiler_version: descriptor.compiler_version.clone(),
        corvid_abi_version: descriptor.corvid_abi_version,
        descriptor_sha256: descriptor_sha256_hex(descriptor),
        surface_counts,
        flagship_entrypoints,
        enforced_guarantees,
        approval_gates,
        dangerous_tool_count,
        dangerous_agent_count,
        stores_writeable,
        sources,
    }
}

fn descriptor_sha256_hex(descriptor: &CorvidAbi) -> String {
    match hash_abi(descriptor) {
        Ok(bytes) => hex::encode(bytes),
        Err(_) => String::new(),
    }
}

fn derive_app_name(source_path: &str) -> String {
    let trimmed = source_path.trim();
    if trimmed.is_empty() {
        return "<unnamed>".to_string();
    }
    let basename = trimmed
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(trimmed);
    let stem = basename.split('.').next().unwrap_or(basename);
    if stem.is_empty() {
        return "<unnamed>".to_string();
    }
    stem.to_string()
}

fn derive_surface_counts(descriptor: &CorvidAbi, sources: &mut Vec<BootSource>) -> BootSurfaceCounts {
    sources.push(BootSource {
        descriptor_field: "descriptor.agents".to_string(),
        note: "surface_counts.agents counts every declared agent".to_string(),
    });
    sources.push(BootSource {
        descriptor_field: "descriptor.prompts".to_string(),
        note: "surface_counts.prompts counts every declared prompt".to_string(),
    });
    sources.push(BootSource {
        descriptor_field: "descriptor.tools".to_string(),
        note: "surface_counts.tools counts every declared tool".to_string(),
    });
    sources.push(BootSource {
        descriptor_field: "descriptor.types".to_string(),
        note: "surface_counts.types counts every declared type".to_string(),
    });
    sources.push(BootSource {
        descriptor_field: "descriptor.stores".to_string(),
        note: "surface_counts.stores counts every declared store".to_string(),
    });
    sources.push(BootSource {
        descriptor_field: "descriptor.approval_sites".to_string(),
        note: "surface_counts.approval_sites counts every declared approval site".to_string(),
    });
    BootSurfaceCounts {
        agents: descriptor.agents.len(),
        prompts: descriptor.prompts.len(),
        tools: descriptor.tools.len(),
        types: descriptor.types.len(),
        stores: descriptor.stores.len(),
        approval_sites: descriptor.approval_sites.len(),
    }
}

fn derive_flagship_entrypoints(
    agents: &[AbiAgent],
    sources: &mut Vec<BootSource>,
) -> Vec<BootFlagshipEntrypoint> {
    let mut entrypoints: Vec<BootFlagshipEntrypoint> = agents
        .iter()
        .filter(|agent| agent.attributes.pub_extern_c)
        .map(|agent| BootFlagshipEntrypoint {
            agent_name: agent.name.clone(),
            symbol: agent.symbol.clone(),
            dangerous: agent.attributes.dangerous,
        })
        .collect();
    entrypoints.sort_by(|a, b| a.agent_name.cmp(&b.agent_name));
    if !entrypoints.is_empty() {
        sources.push(BootSource {
            descriptor_field: "descriptor.agents[].attributes.pub_extern_c".to_string(),
            note: "flagship_entrypoints lists every agent the cdylib exports as `pub extern \"c\"`"
                .to_string(),
        });
        sources.push(BootSource {
            descriptor_field: "descriptor.agents[].attributes.dangerous".to_string(),
            note: "flagship_entrypoints[].dangerous mirrors the dangerous attribute on the source agent"
                .to_string(),
        });
    }
    entrypoints
}

fn derive_enforced_guarantees(
    guarantees: &[AbiClaimGuarantee],
    sources: &mut Vec<BootSource>,
) -> Vec<BootEnforcedGuarantee> {
    if guarantees.is_empty() {
        return Vec::new();
    }
    sources.push(BootSource {
        descriptor_field: "descriptor.claim_guarantees".to_string(),
        note: "enforced_guarantees mirrors the descriptor's claim_guarantees vector".to_string(),
    });
    let mut rows: Vec<BootEnforcedGuarantee> = guarantees
        .iter()
        .map(|g| BootEnforcedGuarantee {
            id: g.id.clone(),
            kind: g.kind.clone(),
            class: g.class.clone(),
            phase: g.phase.clone(),
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

fn derive_approval_gates(
    sites: &[AbiApprovalSite],
    sources: &mut Vec<BootSource>,
) -> Vec<BootApprovalGate> {
    if sites.is_empty() {
        return Vec::new();
    }
    sources.push(BootSource {
        descriptor_field: "descriptor.approval_sites".to_string(),
        note: "approval_gates summarises every declared approval site by label + tier".to_string(),
    });
    let mut gates: Vec<BootApprovalGate> = sites
        .iter()
        .map(|site| BootApprovalGate {
            label: site.label.clone(),
            required_tier: site.required_tier.clone(),
            dangerous_target_count: site.dangerous_targets.len(),
        })
        .collect();
    gates.sort_by(|a, b| a.label.cmp(&b.label).then(a.required_tier.cmp(&b.required_tier)));
    gates
}

fn count_dangerous_tools(tools: &[AbiTool], sources: &mut Vec<BootSource>) -> usize {
    let count = tools.iter().filter(|t| t.dangerous).count();
    if count > 0 {
        sources.push(BootSource {
            descriptor_field: "descriptor.tools[].dangerous".to_string(),
            note: "dangerous_tool_count counts tools the descriptor marks dangerous".to_string(),
        });
    }
    count
}

fn count_dangerous_agents(agents: &[AbiAgent], sources: &mut Vec<BootSource>) -> usize {
    let count = agents
        .iter()
        .filter(|a| a.attributes.dangerous)
        .count();
    if count > 0 {
        sources.push(BootSource {
            descriptor_field: "descriptor.agents[].attributes.dangerous".to_string(),
            note: "dangerous_agent_count counts agents the descriptor marks dangerous".to_string(),
        });
    }
    count
}

fn derive_stores_writeable(descriptor: &CorvidAbi, sources: &mut Vec<BootSource>) -> bool {
    if descriptor.stores.is_empty() {
        return false;
    }
    let writeable = descriptor
        .stores
        .iter()
        .any(|s| !s.effects.write.is_empty() && s.effects.write != "none");
    sources.push(BootSource {
        descriptor_field: "descriptor.stores[].effects.write".to_string(),
        note: "stores_writeable is true iff any store declares a non-empty write effect".to_string(),
    });
    writeable
}

/// Renders a [`BootSummary`] as the operator-facing text block
/// that `corvid app boot-summary` prints to stdout. Replay-stable
/// — two calls with the same input return byte-identical output.
pub fn render_boot_summary(summary: &BootSummary) -> String {
    let mut out = String::new();
    out.push_str("Corvid app boot summary\n");
    out.push_str(&format!("app_name: {}\n", summary.app_name));
    out.push_str(&format!("source_path: {}\n", summary.source_path));
    out.push_str(&format!("compiler_version: {}\n", summary.compiler_version));
    out.push_str(&format!("corvid_abi_version: {}\n", summary.corvid_abi_version));
    out.push_str(&format!("descriptor_sha256: {}\n", summary.descriptor_sha256));
    out.push_str("surface:\n");
    out.push_str(&format!("  agents: {}\n", summary.surface_counts.agents));
    out.push_str(&format!("  prompts: {}\n", summary.surface_counts.prompts));
    out.push_str(&format!("  tools: {}\n", summary.surface_counts.tools));
    out.push_str(&format!("  types: {}\n", summary.surface_counts.types));
    out.push_str(&format!("  stores: {}\n", summary.surface_counts.stores));
    out.push_str(&format!(
        "  approval_sites: {}\n",
        summary.surface_counts.approval_sites
    ));
    out.push_str(&format!(
        "dangerous_tool_count: {}\n",
        summary.dangerous_tool_count
    ));
    out.push_str(&format!(
        "dangerous_agent_count: {}\n",
        summary.dangerous_agent_count
    ));
    out.push_str(&format!("stores_writeable: {}\n", summary.stores_writeable));
    out.push_str("flagship_entrypoints:\n");
    if summary.flagship_entrypoints.is_empty() {
        out.push_str("  (none — this app exposes no `pub extern \"c\"` agents)\n");
    } else {
        for ep in &summary.flagship_entrypoints {
            out.push_str(&format!(
                "  - {} (symbol: {}, dangerous: {})\n",
                ep.agent_name, ep.symbol, ep.dangerous
            ));
        }
    }
    out.push_str("approval_gates:\n");
    if summary.approval_gates.is_empty() {
        out.push_str("  (none declared)\n");
    } else {
        for gate in &summary.approval_gates {
            out.push_str(&format!(
                "  - {} (tier: {}, dangerous_targets: {})\n",
                gate.label, gate.required_tier, gate.dangerous_target_count
            ));
        }
    }
    out.push_str("enforced_guarantees:\n");
    if summary.enforced_guarantees.is_empty() {
        out.push_str("  (none declared)\n");
    } else {
        for g in &summary.enforced_guarantees {
            out.push_str(&format!(
                "  - {} (kind: {}, class: {}, phase: {})\n",
                g.id, g.kind, g.class, g.phase
            ));
        }
    }
    out.push_str("sources:\n");
    let unique: BTreeSet<&str> = summary
        .sources
        .iter()
        .map(|s| s.descriptor_field.as_str())
        .collect();
    for field in unique {
        out.push_str(&format!("  - {}\n", field));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        AbiApprovalContract, AbiAttributes, AbiClaimGuarantee, AbiEffects, AbiProvenanceContract,
        AbiSourceSpan, AbiStore, AbiStoreEffects, AbiTool, AbiToolContract, AbiTypeDecl,
        ScalarTypeName, TypeDescription,
    };

    fn empty_effects() -> AbiEffects {
        AbiEffects::default()
    }

    fn scalar_int() -> TypeDescription {
        TypeDescription::Scalar {
            scalar: ScalarTypeName::Int,
        }
    }

    fn empty_approval_contract() -> AbiApprovalContract {
        AbiApprovalContract {
            required: false,
            labels: Vec::new(),
        }
    }

    fn empty_provenance_contract() -> AbiProvenanceContract {
        AbiProvenanceContract {
            returns_grounded: false,
            grounded_param_deps: Vec::new(),
        }
    }

    fn empty_descriptor() -> CorvidAbi {
        CorvidAbi {
            corvid_abi_version: 1,
            compiler_version: "0.0.1".to_string(),
            source_path: "examples/backend/empty_app/src/main.cor".to_string(),
            generated_at: "1970-01-01T00:00:00Z".to_string(),
            agents: Vec::new(),
            prompts: Vec::new(),
            tools: Vec::new(),
            types: Vec::new(),
            stores: Vec::new(),
            approval_sites: Vec::new(),
            claim_guarantees: Vec::new(),
            extra: Default::default(),
        }
    }

    fn agent(name: &str, pub_extern_c: bool, dangerous: bool) -> AbiAgent {
        AbiAgent {
            name: name.to_string(),
            symbol: format!("__corvid_agent_{name}"),
            source_span: AbiSourceSpan { start: 0, end: 0 },
            source_line: 1,
            params: Vec::new(),
            return_type: scalar_int(),
            return_ownership: None,
            effects: empty_effects(),
            attributes: AbiAttributes {
                replayable: false,
                deterministic: false,
                dangerous,
                pub_extern_c,
            },
            budget: None,
            required_capability: None,
            dispatch: None,
            approval_contract: empty_approval_contract(),
            provenance: empty_provenance_contract(),
        }
    }

    fn tool(name: &str, dangerous: bool) -> AbiTool {
        AbiTool {
            name: name.to_string(),
            symbol: format!("__corvid_tool_{name}"),
            params: Vec::new(),
            return_type: scalar_int(),
            effects: empty_effects(),
            dangerous,
            contract: AbiToolContract::default(),
        }
    }

    fn type_decl(name: &str) -> AbiTypeDecl {
        AbiTypeDecl {
            name: name.to_string(),
            kind: "struct".to_string(),
            fields: Vec::new(),
        }
    }

    fn store_with_write(name: &str, write: &str) -> AbiStore {
        AbiStore {
            name: name.to_string(),
            kind: "kv".to_string(),
            fields: Vec::new(),
            policies: Vec::new(),
            accessors: Vec::new(),
            source_span: AbiSourceSpan { start: 0, end: 0 },
            effects: AbiStoreEffects {
                read: "store.read".to_string(),
                write: write.to_string(),
            },
        }
    }

    fn approval_site(label: &str, tier: &str, dangerous_targets: &[&str]) -> AbiApprovalSite {
        AbiApprovalSite {
            label: label.to_string(),
            declared_at: crate::schema::AbiDeclaredAt {
                source_span: AbiSourceSpan { start: 0, end: 0 },
            },
            agent_context: "test".to_string(),
            predicate: None,
            dangerous_targets: dangerous_targets.iter().map(|s| s.to_string()).collect(),
            effects: empty_effects(),
            required_tier: tier.to_string(),
        }
    }

    fn guarantee(id: &str) -> AbiClaimGuarantee {
        AbiClaimGuarantee {
            id: id.to_string(),
            kind: "RuntimeChecked".to_string(),
            class: "Safety".to_string(),
            phase: "P42".to_string(),
        }
    }

    /// Positive: a populated descriptor produces non-empty
    /// counts and a non-empty `sources` array that references
    /// every descriptor field actually consulted. This is the
    /// shape that `app.boot_summary_grounded` promotes.
    #[test]
    fn boot_summary_grounds_every_derived_field_to_a_descriptor_source() {
        let mut desc = empty_descriptor();
        desc.agents.push(agent("ask", true, false));
        desc.agents.push(agent("danger_agent", false, true));
        desc.tools.push(tool("safe", false));
        desc.tools.push(tool("write_db", true));
        desc.types.push(type_decl("Receipt"));
        desc.stores.push(store_with_write("audit", "store.write.audit"));
        desc.approval_sites
            .push(approval_site("write_receipt", "operator", &["receipt_id"]));
        desc.claim_guarantees.push(guarantee("approval.required"));

        let summary = boot_summary_from_descriptor(&desc);

        assert_eq!(summary.app_name, "main");
        assert_eq!(summary.surface_counts.agents, 2);
        assert_eq!(summary.surface_counts.tools, 2);
        assert_eq!(summary.dangerous_tool_count, 1);
        assert_eq!(summary.dangerous_agent_count, 1);
        assert!(summary.stores_writeable);
        assert_eq!(summary.flagship_entrypoints.len(), 1);
        assert_eq!(summary.flagship_entrypoints[0].agent_name, "ask");
        assert_eq!(summary.approval_gates.len(), 1);
        assert_eq!(summary.enforced_guarantees.len(), 1);
        assert!(!summary.descriptor_sha256.is_empty());

        let source_fields: BTreeSet<&str> = summary
            .sources
            .iter()
            .map(|s| s.descriptor_field.as_str())
            .collect();
        assert!(source_fields.contains("descriptor.source_path"));
        assert!(source_fields.contains("descriptor.compiler_version"));
        assert!(source_fields.contains("descriptor.corvid_abi_version"));
        assert!(source_fields.contains("descriptor.agents"));
        assert!(source_fields.contains("descriptor.tools"));
        assert!(source_fields.contains("descriptor.tools[].dangerous"));
        assert!(source_fields.contains("descriptor.agents[].attributes.dangerous"));
        assert!(source_fields.contains("descriptor.agents[].attributes.pub_extern_c"));
        assert!(source_fields.contains("descriptor.claim_guarantees"));
        assert!(source_fields.contains("descriptor.approval_sites"));
        assert!(source_fields.contains("descriptor.stores[].effects.write"));
    }

    /// Adversarial: the empty-surface descriptor still produces
    /// a typed grounded summary — counts are zero, dangerous
    /// counts are zero, but the `sources` array is still
    /// non-empty (the scalar fields are always consulted).
    /// This is what stops the helper from short-circuiting into
    /// a sourceless string when the input is degenerate.
    #[test]
    fn boot_summary_empty_surface_descriptor_returns_grounded_summary_not_sourceless() {
        let desc = empty_descriptor();
        let summary = boot_summary_from_descriptor(&desc);
        assert_eq!(summary.surface_counts.agents, 0);
        assert_eq!(summary.surface_counts.tools, 0);
        assert_eq!(summary.surface_counts.approval_sites, 0);
        assert_eq!(summary.flagship_entrypoints.len(), 0);
        assert_eq!(summary.enforced_guarantees.len(), 0);
        assert_eq!(summary.approval_gates.len(), 0);
        assert_eq!(summary.dangerous_tool_count, 0);
        assert_eq!(summary.dangerous_agent_count, 0);
        assert!(!summary.stores_writeable);
        assert!(
            !summary.sources.is_empty(),
            "empty descriptor must still produce non-empty sources for scalar fields"
        );
    }

    /// The renderer is replay-stable: two invocations on the
    /// same summary produce byte-identical output. This is what
    /// makes the helper safe to embed in CI gates that compare
    /// boot summaries across builds.
    #[test]
    fn render_boot_summary_is_byte_identical_across_two_invocations() {
        let mut desc = empty_descriptor();
        desc.agents.push(agent("ask", true, false));
        desc.tools.push(tool("safe", false));
        let summary = boot_summary_from_descriptor(&desc);
        let a = render_boot_summary(&summary);
        let b = render_boot_summary(&summary);
        assert_eq!(a, b);
        assert!(a.contains("app_name: main"));
        assert!(a.contains("flagship_entrypoints:"));
        assert!(a.contains("ask"));
    }

    /// `derive_app_name` handles both POSIX and Windows path
    /// separators, falling back to `<unnamed>` when the path
    /// is empty rather than producing an empty app name.
    #[test]
    fn derive_app_name_handles_posix_windows_and_empty_paths() {
        assert_eq!(derive_app_name("examples/backend/pka/src/main.cor"), "main");
        assert_eq!(
            derive_app_name("C:\\Users\\x\\examples\\pka\\src\\main.cor"),
            "main"
        );
        assert_eq!(derive_app_name(""), "<unnamed>");
        assert_eq!(derive_app_name("   "), "<unnamed>");
    }
}
