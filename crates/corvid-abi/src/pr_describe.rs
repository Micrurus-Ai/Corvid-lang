//! Per-app PR-description helper: diffs two ABI descriptors
//! and renders a typed `PrDescription` summarising what the
//! change set means for the app's claim surface.
//!
//! Third per-app assistive helper. Same deterministic
//! typed-classifier posture as boot-summary and
//! adversarial-refresh — pure transform over typed data, every
//! derived bullet paired with `sources: Vec<DiffSource>` naming
//! the descriptor fields that diverged. Replay-stable.
//!
//! The "generative" framing in the dev log describes the helper's
//! *purpose* (generates the body of a PR description an operator
//! can paste into the merge UI), not an LLM round-trip. When the
//! LLM-provider substrate later lands, the helper's typed
//! contract stays the same; only a richer narration could opt in.
//!
//! In-binary guarantee anchor:
//! `GUARANTEE_ID_APP_PR_DESCRIBE_GROUNDED` declares
//! `app.pr_describe_grounded` to the launch-readiness coverage
//! matrix. Positive corpus rows assert that every emitted bullet
//! carries non-empty sources back-referencing the descriptor
//! fields that diverged; adversarial rows assert the no-change
//! case produces an empty (but typed + grounded) description and
//! the renderer is byte-identical across two invocations.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::schema::{
    AbiAgent, AbiApprovalSite, AbiClaimGuarantee, AbiStore, AbiTool, AbiTypeDecl, CorvidAbi,
};

/// In-binary anchor for the pr-describe launch-readiness row.
pub const GUARANTEE_ID_APP_PR_DESCRIBE_GROUNDED: &str = "app.pr_describe_grounded";

/// Names a single descriptor field that diverged between the
/// base and head descriptors and contributed to a bullet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DiffSource {
    pub descriptor_field: String,
    pub note: String,
}

/// Severity of a PR section. The renderer orders Breaking first
/// so the reviewer reads the most consequential changes up top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrSeverity {
    Breaking,
    Additive,
    Informational,
}

impl PrSeverity {
    pub const fn slug(self) -> &'static str {
        match self {
            PrSeverity::Breaking => "breaking",
            PrSeverity::Additive => "additive",
            PrSeverity::Informational => "informational",
        }
    }
}

/// One bullet line in a PR section. Operator-facing text +
/// sources that named the descriptor diff the bullet came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrBullet {
    pub text: String,
    pub sources: Vec<DiffSource>,
}

/// One section of a PR description: a heading, a severity, and
/// one or more bullets. Empty-bullet sections are never emitted
/// — the renderer skips a section whose `bullets` is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrSection {
    pub heading: String,
    pub severity: PrSeverity,
    pub bullets: Vec<PrBullet>,
}

/// Coarse counts the reviewer sees up front. Mirrors
/// `BootSurfaceCounts` posture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrChangeCounts {
    pub breaking: usize,
    pub additive: usize,
    pub informational: usize,
}

/// The full typed description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrDescription {
    pub title: String,
    pub app_name: String,
    pub base_descriptor_sha256: String,
    pub head_descriptor_sha256: String,
    pub change_counts: PrChangeCounts,
    pub sections: Vec<PrSection>,
    pub sources: Vec<DiffSource>,
}

/// Pure typed diff over two descriptors. No I/O, no clock, no
/// randomness. Two invocations on the same `(base, head)`
/// produce byte-identical descriptions.
pub fn pr_describe_from_descriptors(base: &CorvidAbi, head: &CorvidAbi) -> PrDescription {
    let mut sections: Vec<PrSection> = Vec::new();
    let mut sources: Vec<DiffSource> = Vec::new();

    sources.push(DiffSource {
        descriptor_field: "head.source_path".to_string(),
        note: "app name derived from head source path basename".to_string(),
    });

    push_section_if_non_empty(&mut sections, version_section(base, head, &mut sources));
    push_section_if_non_empty(&mut sections, agents_section(base, head, &mut sources));
    push_section_if_non_empty(&mut sections, tools_section(base, head, &mut sources));
    push_section_if_non_empty(&mut sections, approvals_section(base, head, &mut sources));
    push_section_if_non_empty(&mut sections, types_section(base, head, &mut sources));
    push_section_if_non_empty(&mut sections, stores_section(base, head, &mut sources));
    push_section_if_non_empty(
        &mut sections,
        guarantees_section(base, head, &mut sources),
    );

    sections.sort_by(|a, b| a.severity.cmp(&b.severity).then(a.heading.cmp(&b.heading)));

    let change_counts = PrChangeCounts {
        breaking: sections
            .iter()
            .filter(|s| s.severity == PrSeverity::Breaking)
            .map(|s| s.bullets.len())
            .sum(),
        additive: sections
            .iter()
            .filter(|s| s.severity == PrSeverity::Additive)
            .map(|s| s.bullets.len())
            .sum(),
        informational: sections
            .iter()
            .filter(|s| s.severity == PrSeverity::Informational)
            .map(|s| s.bullets.len())
            .sum(),
    };

    PrDescription {
        title: derive_pr_title(&change_counts, head),
        app_name: derive_app_name(&head.source_path),
        base_descriptor_sha256: descriptor_sha256_hex(base),
        head_descriptor_sha256: descriptor_sha256_hex(head),
        change_counts,
        sections,
        sources,
    }
}

fn descriptor_sha256_hex(descriptor: &CorvidAbi) -> String {
    match crate::canonical_hash::hash_abi(descriptor) {
        Ok(bytes) => hex::encode(bytes),
        Err(_) => String::new(),
    }
}

fn derive_pr_title(counts: &PrChangeCounts, head: &CorvidAbi) -> String {
    let app = derive_app_name(&head.source_path);
    if counts.breaking == 0 && counts.additive == 0 && counts.informational == 0 {
        return format!("{}: no descriptor changes", app);
    }
    if counts.breaking > 0 {
        return format!(
            "{}: {} breaking, {} additive, {} informational change(s)",
            app, counts.breaking, counts.additive, counts.informational
        );
    }
    if counts.additive > 0 {
        return format!(
            "{}: {} additive, {} informational change(s)",
            app, counts.additive, counts.informational
        );
    }
    format!("{}: {} informational change(s)", app, counts.informational)
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

fn push_section_if_non_empty(sections: &mut Vec<PrSection>, section: PrSection) {
    if !section.bullets.is_empty() {
        sections.push(section);
    }
}

fn version_section(
    base: &CorvidAbi,
    head: &CorvidAbi,
    sources: &mut Vec<DiffSource>,
) -> PrSection {
    let mut bullets = Vec::new();
    if base.corvid_abi_version != head.corvid_abi_version {
        bullets.push(PrBullet {
            text: format!(
                "ABI schema version changed: {} → {} (breaking; readers below the new floor must upgrade)",
                base.corvid_abi_version, head.corvid_abi_version
            ),
            sources: vec![DiffSource {
                descriptor_field: "descriptor.corvid_abi_version".to_string(),
                note: "schema-version bumps are breaking by definition".to_string(),
            }],
        });
        sources.push(DiffSource {
            descriptor_field: "descriptor.corvid_abi_version".to_string(),
            note: "version-section bullet derives from the schema version delta".to_string(),
        });
    }
    if base.compiler_version != head.compiler_version {
        bullets.push(PrBullet {
            text: format!(
                "compiler_version: {} → {}",
                base.compiler_version, head.compiler_version
            ),
            sources: vec![DiffSource {
                descriptor_field: "descriptor.compiler_version".to_string(),
                note: "compiler bump is informational unless paired with an ABI version delta"
                    .to_string(),
            }],
        });
        sources.push(DiffSource {
            descriptor_field: "descriptor.compiler_version".to_string(),
            note: "version-section bullet derives from the compiler version delta".to_string(),
        });
    }
    let severity = if base.corvid_abi_version != head.corvid_abi_version {
        PrSeverity::Breaking
    } else {
        PrSeverity::Informational
    };
    PrSection {
        heading: "ABI / compiler versions".to_string(),
        severity,
        bullets,
    }
}

fn agents_section(base: &CorvidAbi, head: &CorvidAbi, sources: &mut Vec<DiffSource>) -> PrSection {
    let base_map: BTreeMap<&str, &AbiAgent> =
        base.agents.iter().map(|a| (a.name.as_str(), a)).collect();
    let head_map: BTreeMap<&str, &AbiAgent> =
        head.agents.iter().map(|a| (a.name.as_str(), a)).collect();
    let mut bullets = Vec::new();
    let mut breaking = false;

    let removed: Vec<&str> = base_map
        .keys()
        .filter(|k| !head_map.contains_key(*k))
        .copied()
        .collect();
    let added: Vec<&str> = head_map
        .keys()
        .filter(|k| !base_map.contains_key(*k))
        .copied()
        .collect();

    if !removed.is_empty() {
        breaking = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.agents (removed)".to_string(),
            note: "removed agents break callers that referenced them".to_string(),
        });
    }
    if !added.is_empty() {
        sources.push(DiffSource {
            descriptor_field: "descriptor.agents (added)".to_string(),
            note: "added agents are additive".to_string(),
        });
    }
    for name in &removed {
        bullets.push(PrBullet {
            text: format!("agent `{}` removed (breaking — callers must update)", name),
            sources: vec![DiffSource {
                descriptor_field: format!("base.agents[name={}]", name),
                note: "agent present in base, absent in head".to_string(),
            }],
        });
    }
    for name in &added {
        let agent = head_map[*name];
        let exposure = if agent.attributes.pub_extern_c {
            " (`pub extern \"c\"` — visible to hosts)"
        } else {
            ""
        };
        bullets.push(PrBullet {
            text: format!("agent `{}` added{}", name, exposure),
            sources: vec![DiffSource {
                descriptor_field: format!("head.agents[name={}]", name),
                note: "agent absent in base, present in head".to_string(),
            }],
        });
    }

    // Same-name agents whose pub_extern_c attribute changed.
    for (name, base_agent) in &base_map {
        if let Some(head_agent) = head_map.get(name) {
            if base_agent.attributes.pub_extern_c != head_agent.attributes.pub_extern_c {
                breaking = breaking || base_agent.attributes.pub_extern_c;
                let direction = if head_agent.attributes.pub_extern_c {
                    "newly exposed as `pub extern \"c\"`"
                } else {
                    "no longer `pub extern \"c\"` (breaking for hosts that called it)"
                };
                bullets.push(PrBullet {
                    text: format!("agent `{}` {}", name, direction),
                    sources: vec![DiffSource {
                        descriptor_field: format!(
                            "descriptor.agents[name={}].attributes.pub_extern_c",
                            name
                        ),
                        note: "host-visibility toggle".to_string(),
                    }],
                });
            }
            if base_agent.attributes.dangerous != head_agent.attributes.dangerous {
                let direction = if head_agent.attributes.dangerous {
                    "newly marked `dangerous`"
                } else {
                    "no longer marked `dangerous`"
                };
                bullets.push(PrBullet {
                    text: format!("agent `{}` {}", name, direction),
                    sources: vec![DiffSource {
                        descriptor_field: format!(
                            "descriptor.agents[name={}].attributes.dangerous",
                            name
                        ),
                        note: "dangerous-attribute toggle".to_string(),
                    }],
                });
            }
        }
    }

    PrSection {
        heading: "Agents".to_string(),
        severity: severity_for(breaking, !added.is_empty()),
        bullets,
    }
}

fn tools_section(base: &CorvidAbi, head: &CorvidAbi, sources: &mut Vec<DiffSource>) -> PrSection {
    let base_map: BTreeMap<&str, &AbiTool> =
        base.tools.iter().map(|t| (t.name.as_str(), t)).collect();
    let head_map: BTreeMap<&str, &AbiTool> =
        head.tools.iter().map(|t| (t.name.as_str(), t)).collect();
    let mut bullets = Vec::new();
    let mut breaking = false;
    let mut added_any = false;

    let removed: Vec<&str> = base_map
        .keys()
        .filter(|k| !head_map.contains_key(*k))
        .copied()
        .collect();
    let added: Vec<&str> = head_map
        .keys()
        .filter(|k| !base_map.contains_key(*k))
        .copied()
        .collect();

    if !removed.is_empty() {
        breaking = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.tools (removed)".to_string(),
            note: "removed tools break callers that referenced them".to_string(),
        });
    }
    if !added.is_empty() {
        added_any = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.tools (added)".to_string(),
            note: "added tools are additive".to_string(),
        });
    }
    for name in &removed {
        bullets.push(PrBullet {
            text: format!("tool `{}` removed", name),
            sources: vec![DiffSource {
                descriptor_field: format!("base.tools[name={}]", name),
                note: "tool present in base, absent in head".to_string(),
            }],
        });
    }
    for name in &added {
        let tool = head_map[*name];
        let dangerous_note = if tool.dangerous {
            " (`dangerous`)"
        } else {
            ""
        };
        bullets.push(PrBullet {
            text: format!("tool `{}` added{}", name, dangerous_note),
            sources: vec![DiffSource {
                descriptor_field: format!("head.tools[name={}]", name),
                note: "tool absent in base, present in head".to_string(),
            }],
        });
    }
    // Dangerous-flag toggles
    for (name, base_tool) in &base_map {
        if let Some(head_tool) = head_map.get(name) {
            if base_tool.dangerous != head_tool.dangerous {
                let direction = if head_tool.dangerous {
                    "newly marked `dangerous`"
                } else {
                    "no longer marked `dangerous`"
                };
                bullets.push(PrBullet {
                    text: format!("tool `{}` {}", name, direction),
                    sources: vec![DiffSource {
                        descriptor_field: format!("descriptor.tools[name={}].dangerous", name),
                        note: "dangerous-flag toggle on a tool".to_string(),
                    }],
                });
            }
        }
    }
    PrSection {
        heading: "Tools".to_string(),
        severity: severity_for(breaking, added_any),
        bullets,
    }
}

fn approvals_section(
    base: &CorvidAbi,
    head: &CorvidAbi,
    sources: &mut Vec<DiffSource>,
) -> PrSection {
    let base_map: BTreeMap<&str, &AbiApprovalSite> = base
        .approval_sites
        .iter()
        .map(|a| (a.label.as_str(), a))
        .collect();
    let head_map: BTreeMap<&str, &AbiApprovalSite> = head
        .approval_sites
        .iter()
        .map(|a| (a.label.as_str(), a))
        .collect();
    let mut bullets = Vec::new();
    let mut breaking = false;
    let mut added_any = false;

    let removed: Vec<&str> = base_map
        .keys()
        .filter(|k| !head_map.contains_key(*k))
        .copied()
        .collect();
    let added: Vec<&str> = head_map
        .keys()
        .filter(|k| !base_map.contains_key(*k))
        .copied()
        .collect();

    if !removed.is_empty() {
        breaking = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.approval_sites (removed)".to_string(),
            note: "removed approval gates break the policy contract".to_string(),
        });
    }
    if !added.is_empty() {
        added_any = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.approval_sites (added)".to_string(),
            note: "added approval gates are additive but require review".to_string(),
        });
    }
    for label in &removed {
        bullets.push(PrBullet {
            text: format!(
                "approval gate `{}` removed (BREAKING — the dangerous surface it protected is now unguarded)",
                label
            ),
            sources: vec![DiffSource {
                descriptor_field: format!("base.approval_sites[label={}]", label),
                note: "approval site present in base, absent in head".to_string(),
            }],
        });
    }
    for label in &added {
        let site = head_map[*label];
        bullets.push(PrBullet {
            text: format!(
                "approval gate `{}` added (tier: {}, dangerous_targets: {})",
                label,
                site.required_tier,
                site.dangerous_targets.len()
            ),
            sources: vec![DiffSource {
                descriptor_field: format!("head.approval_sites[label={}]", label),
                note: "approval site absent in base, present in head".to_string(),
            }],
        });
    }
    // Tier changes on same label
    for (label, base_site) in &base_map {
        if let Some(head_site) = head_map.get(label) {
            if base_site.required_tier != head_site.required_tier {
                let weakening = matches!(
                    (base_site.required_tier.as_str(), head_site.required_tier.as_str()),
                    ("human_required", _) | ("operator", "autonomous")
                );
                if weakening {
                    breaking = true;
                }
                bullets.push(PrBullet {
                    text: format!(
                        "approval gate `{}` required_tier: {} → {}{}",
                        label,
                        base_site.required_tier,
                        head_site.required_tier,
                        if weakening {
                            " (BREAKING — weaker tier loosens the policy)"
                        } else {
                            ""
                        }
                    ),
                    sources: vec![DiffSource {
                        descriptor_field: format!(
                            "descriptor.approval_sites[label={}].required_tier",
                            label
                        ),
                        note: "required_tier change on an approval site".to_string(),
                    }],
                });
            }
        }
    }
    PrSection {
        heading: "Approval gates".to_string(),
        severity: severity_for(breaking, added_any),
        bullets,
    }
}

fn types_section(base: &CorvidAbi, head: &CorvidAbi, sources: &mut Vec<DiffSource>) -> PrSection {
    let base_names: BTreeSet<&str> = base.types.iter().map(|t| t.name.as_str()).collect();
    let head_names: BTreeSet<&str> = head.types.iter().map(|t| t.name.as_str()).collect();
    let mut bullets = Vec::new();
    let mut breaking = false;
    let mut added_any = false;

    let removed: Vec<&&str> = base_names.iter().filter(|n| !head_names.contains(*n)).collect();
    let added: Vec<&&str> = head_names.iter().filter(|n| !base_names.contains(*n)).collect();

    if !removed.is_empty() {
        breaking = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.types (removed)".to_string(),
            note: "removed types break dependents".to_string(),
        });
    }
    if !added.is_empty() {
        added_any = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.types (added)".to_string(),
            note: "added types are additive".to_string(),
        });
    }
    for n in &removed {
        bullets.push(PrBullet {
            text: format!("type `{}` removed", n),
            sources: vec![DiffSource {
                descriptor_field: format!("base.types[name={}]", n),
                note: "type present in base, absent in head".to_string(),
            }],
        });
    }
    for n in &added {
        bullets.push(PrBullet {
            text: format!("type `{}` added", n),
            sources: vec![DiffSource {
                descriptor_field: format!("head.types[name={}]", n),
                note: "type absent in base, present in head".to_string(),
            }],
        });
    }
    // Field-count changes on same-name types
    let base_map: BTreeMap<&str, &AbiTypeDecl> =
        base.types.iter().map(|t| (t.name.as_str(), t)).collect();
    let head_map: BTreeMap<&str, &AbiTypeDecl> =
        head.types.iter().map(|t| (t.name.as_str(), t)).collect();
    for (name, base_type) in &base_map {
        if let Some(head_type) = head_map.get(name) {
            if base_type.fields.len() != head_type.fields.len() {
                let direction = if head_type.fields.len() < base_type.fields.len() {
                    breaking = true;
                    " (BREAKING — fields removed)"
                } else {
                    ""
                };
                bullets.push(PrBullet {
                    text: format!(
                        "type `{}` field count: {} → {}{}",
                        name,
                        base_type.fields.len(),
                        head_type.fields.len(),
                        direction
                    ),
                    sources: vec![DiffSource {
                        descriptor_field: format!("descriptor.types[name={}].fields", name),
                        note: "field-list length changed".to_string(),
                    }],
                });
            }
        }
    }
    PrSection {
        heading: "Types".to_string(),
        severity: severity_for(breaking, added_any),
        bullets,
    }
}

fn stores_section(
    base: &CorvidAbi,
    head: &CorvidAbi,
    sources: &mut Vec<DiffSource>,
) -> PrSection {
    let base_map: BTreeMap<&str, &AbiStore> =
        base.stores.iter().map(|s| (s.name.as_str(), s)).collect();
    let head_map: BTreeMap<&str, &AbiStore> =
        head.stores.iter().map(|s| (s.name.as_str(), s)).collect();
    let mut bullets = Vec::new();
    let mut breaking = false;
    let mut added_any = false;

    let removed: Vec<&str> = base_map
        .keys()
        .filter(|k| !head_map.contains_key(*k))
        .copied()
        .collect();
    let added: Vec<&str> = head_map
        .keys()
        .filter(|k| !base_map.contains_key(*k))
        .copied()
        .collect();
    if !removed.is_empty() {
        breaking = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.stores (removed)".to_string(),
            note: "removed stores lose persisted state".to_string(),
        });
    }
    if !added.is_empty() {
        added_any = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.stores (added)".to_string(),
            note: "added stores are additive".to_string(),
        });
    }
    for n in &removed {
        bullets.push(PrBullet {
            text: format!("store `{}` removed", n),
            sources: vec![DiffSource {
                descriptor_field: format!("base.stores[name={}]", n),
                note: "store present in base, absent in head".to_string(),
            }],
        });
    }
    for n in &added {
        bullets.push(PrBullet {
            text: format!("store `{}` added", n),
            sources: vec![DiffSource {
                descriptor_field: format!("head.stores[name={}]", n),
                note: "store absent in base, present in head".to_string(),
            }],
        });
    }
    PrSection {
        heading: "Stores".to_string(),
        severity: severity_for(breaking, added_any),
        bullets,
    }
}

fn guarantees_section(
    base: &CorvidAbi,
    head: &CorvidAbi,
    sources: &mut Vec<DiffSource>,
) -> PrSection {
    let base_ids: BTreeSet<&str> = base
        .claim_guarantees
        .iter()
        .map(|g| g.id.as_str())
        .collect();
    let head_ids: BTreeSet<&str> = head
        .claim_guarantees
        .iter()
        .map(|g| g.id.as_str())
        .collect();
    let head_map: BTreeMap<&str, &AbiClaimGuarantee> = head
        .claim_guarantees
        .iter()
        .map(|g| (g.id.as_str(), g))
        .collect();
    let mut bullets = Vec::new();
    let mut breaking = false;

    let removed: Vec<&&str> = base_ids.iter().filter(|n| !head_ids.contains(*n)).collect();
    let added: Vec<&&str> = head_ids.iter().filter(|n| !base_ids.contains(*n)).collect();
    if !removed.is_empty() {
        breaking = true;
        sources.push(DiffSource {
            descriptor_field: "descriptor.claim_guarantees (removed)".to_string(),
            note: "removed claim guarantees weaken the safety contract".to_string(),
        });
    }
    if !added.is_empty() {
        sources.push(DiffSource {
            descriptor_field: "descriptor.claim_guarantees (added)".to_string(),
            note: "added claim guarantees strengthen the safety contract".to_string(),
        });
    }
    for id in &removed {
        bullets.push(PrBullet {
            text: format!("claim guarantee `{}` removed (BREAKING for downstream verifiers)", id),
            sources: vec![DiffSource {
                descriptor_field: format!("base.claim_guarantees[id={}]", id),
                note: "guarantee present in base, absent in head".to_string(),
            }],
        });
    }
    for id in &added {
        let g = head_map[**id];
        bullets.push(PrBullet {
            text: format!(
                "claim guarantee `{}` added (kind: {}, class: {})",
                id, g.kind, g.class
            ),
            sources: vec![DiffSource {
                descriptor_field: format!("head.claim_guarantees[id={}]", id),
                note: "guarantee absent in base, present in head".to_string(),
            }],
        });
    }
    PrSection {
        heading: "Claim guarantees".to_string(),
        severity: severity_for(breaking, !added.is_empty()),
        bullets,
    }
}

fn severity_for(breaking: bool, additive: bool) -> PrSeverity {
    if breaking {
        PrSeverity::Breaking
    } else if additive {
        PrSeverity::Additive
    } else {
        PrSeverity::Informational
    }
}

/// Renders a [`PrDescription`] as the operator-facing
/// markdown-ish text block that `corvid app pr-describe`
/// prints to stdout. Replay-stable.
pub fn render_pr_description(description: &PrDescription) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", description.title));
    out.push_str(&format!("app: {}\n", description.app_name));
    out.push_str(&format!(
        "base_descriptor_sha256: {}\n",
        description.base_descriptor_sha256
    ));
    out.push_str(&format!(
        "head_descriptor_sha256: {}\n",
        description.head_descriptor_sha256
    ));
    out.push_str("change_counts:\n");
    out.push_str(&format!("  breaking: {}\n", description.change_counts.breaking));
    out.push_str(&format!("  additive: {}\n", description.change_counts.additive));
    out.push_str(&format!(
        "  informational: {}\n",
        description.change_counts.informational
    ));
    out.push_str("\n");
    if description.sections.is_empty() {
        out.push_str("_no descriptor-surface changes between base and head_\n");
    } else {
        for section in &description.sections {
            out.push_str(&format!(
                "## [{}] {}\n",
                section.severity.slug(),
                section.heading
            ));
            for bullet in &section.bullets {
                out.push_str(&format!("- {}\n", bullet.text));
            }
            out.push_str("\n");
        }
    }
    out.push_str("## sources\n");
    let unique: BTreeSet<&str> = description
        .sources
        .iter()
        .chain(
            description
                .sections
                .iter()
                .flat_map(|s| s.bullets.iter().flat_map(|b| b.sources.iter())),
        )
        .map(|s| s.descriptor_field.as_str())
        .collect();
    for field in unique {
        out.push_str(&format!("- {}\n", field));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        AbiApprovalContract, AbiAttributes, AbiEffects, AbiProvenanceContract, AbiSourceSpan,
        AbiToolContract, ScalarTypeName, TypeDescription,
    };

    fn empty_descriptor() -> CorvidAbi {
        CorvidAbi {
            corvid_abi_version: 1,
            compiler_version: "0.0.1".to_string(),
            source_path: "examples/backend/x/src/main.cor".to_string(),
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

    fn scalar_int() -> TypeDescription {
        TypeDescription::Scalar {
            scalar: ScalarTypeName::Int,
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
            effects: AbiEffects::default(),
            attributes: AbiAttributes {
                replayable: false,
                deterministic: false,
                dangerous,
                pub_extern_c,
            },
            budget: None,
            required_capability: None,
            dispatch: None,
            approval_contract: AbiApprovalContract {
                required: false,
                labels: Vec::new(),
            },
            provenance: AbiProvenanceContract {
                returns_grounded: false,
                grounded_param_deps: Vec::new(),
            },
        }
    }

    fn tool(name: &str, dangerous: bool) -> AbiTool {
        AbiTool {
            name: name.to_string(),
            symbol: format!("__corvid_tool_{name}"),
            params: Vec::new(),
            return_type: scalar_int(),
            effects: AbiEffects::default(),
            dangerous,
            contract: AbiToolContract::default(),
        }
    }

    fn approval(label: &str, tier: &str) -> AbiApprovalSite {
        AbiApprovalSite {
            label: label.to_string(),
            declared_at: crate::schema::AbiDeclaredAt {
                source_span: AbiSourceSpan { start: 0, end: 0 },
            },
            agent_context: "test".to_string(),
            predicate: None,
            dangerous_targets: Vec::new(),
            effects: AbiEffects::default(),
            required_tier: tier.to_string(),
        }
    }

    /// Positive: a base→head delta with one new agent and one
    /// removed approval produces typed sections whose bullets
    /// every carry non-empty sources back-referencing the
    /// descriptor field that diverged. Establishes the grounded
    /// contract `app.pr_describe_grounded` promotes.
    #[test]
    fn pr_describe_emits_bullets_grounded_to_descriptor_fields() {
        let mut base = empty_descriptor();
        base.agents.push(agent("ask", true, false));
        base.approval_sites.push(approval("ShareAnswerToChat", "operator"));

        let mut head = empty_descriptor();
        head.agents.push(agent("ask", true, false));
        head.agents.push(agent("triage", true, false));
        // ShareAnswerToChat removed in head — BREAKING.

        let description = pr_describe_from_descriptors(&base, &head);
        assert!(description.change_counts.breaking >= 1);
        assert!(description.change_counts.additive >= 1);
        for section in &description.sections {
            for bullet in &section.bullets {
                assert!(
                    !bullet.sources.is_empty(),
                    "bullet `{}` in section `{}` has empty sources",
                    bullet.text,
                    section.heading
                );
            }
        }
    }

    /// Adversarial: identical descriptors produce a typed but
    /// empty description — no sections, but the report-level
    /// `sources` array is still non-empty. The renderer is
    /// byte-identical across two invocations.
    #[test]
    fn no_change_case_produces_typed_grounded_description() {
        let base = empty_descriptor();
        let head = empty_descriptor();
        let description = pr_describe_from_descriptors(&base, &head);
        assert!(description.sections.is_empty());
        assert_eq!(description.change_counts.breaking, 0);
        assert_eq!(description.change_counts.additive, 0);
        assert_eq!(description.change_counts.informational, 0);
        assert!(
            !description.sources.is_empty(),
            "even a no-change description must carry report-level sources"
        );
        let a = render_pr_description(&description);
        let b = render_pr_description(&description);
        assert_eq!(a, b);
        assert!(a.contains("no descriptor-surface changes"));
    }

    /// Severity ordering: a breaking section must precede an
    /// additive section in the rendered output, so a reviewer
    /// reads the most consequential change first.
    #[test]
    fn breaking_section_precedes_additive_in_rendered_output() {
        let mut base = empty_descriptor();
        base.tools.push(tool("write_index", true));

        let mut head = empty_descriptor();
        // write_index removed (breaking), new safe tool added (additive).
        head.tools.push(tool("read_only_lookup", false));

        let description = pr_describe_from_descriptors(&base, &head);
        let rendered = render_pr_description(&description);
        let breaking_pos = rendered.find("[breaking] Tools");
        let additive_pos = rendered.find("[additive]");
        assert!(breaking_pos.is_some(), "expected breaking section");
        if let Some(additive_pos) = additive_pos {
            assert!(breaking_pos.unwrap() < additive_pos);
        }
    }

    /// Tier weakening on an approval site (operator → autonomous)
    /// must be flagged as BREAKING. This is the subtle case the
    /// helper exists to catch — silently relaxing an approval
    /// gate is exactly the kind of change a reviewer needs the
    /// PR description to surface.
    #[test]
    fn approval_tier_weakening_is_flagged_breaking() {
        let mut base = empty_descriptor();
        base.approval_sites.push(approval("ShareAnswer", "operator"));
        let mut head = empty_descriptor();
        head.approval_sites.push(approval("ShareAnswer", "autonomous"));
        let description = pr_describe_from_descriptors(&base, &head);
        assert!(description.change_counts.breaking >= 1);
        let rendered = render_pr_description(&description);
        assert!(
            rendered.contains("BREAKING") && rendered.contains("ShareAnswer"),
            "tier weakening must surface as BREAKING; got:\n{rendered}"
        );
    }

    /// The renderer is byte-identical across two invocations on
    /// the same description, even with a populated mixed
    /// breaking + additive + informational surface.
    #[test]
    fn render_pr_description_is_byte_identical_across_two_invocations() {
        let mut base = empty_descriptor();
        base.agents.push(agent("ask", true, false));
        base.tools.push(tool("safe", false));
        let mut head = empty_descriptor();
        head.agents.push(agent("ask", true, false));
        head.agents.push(agent("triage", true, false));
        head.tools.push(tool("safe", false));
        head.tools.push(tool("write_index", true));
        head.compiler_version = "0.0.2".to_string();
        let description = pr_describe_from_descriptors(&base, &head);
        let a = render_pr_description(&description);
        let b = render_pr_description(&description);
        assert_eq!(a, b);
    }
}
