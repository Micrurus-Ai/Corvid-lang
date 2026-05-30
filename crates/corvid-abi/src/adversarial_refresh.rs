//! Per-app adversarial-fixture suggestion helper.
//!
//! `AdversarialRefreshReport` is the second per-app assistive
//! helper. Given a Corvid app's ABI descriptor, it walks every
//! surface element and emits a typed `AdversarialSuggestion` for
//! the canonical adversarial test that surface element should
//! have — cross-tenant variant, missing-budget edge,
//! approval-bypass attempt, replay-without-token, write-without-
//! approval, role-bypass, expired-approval-reuse,
//! batch-data-class-drift. Every suggestion carries
//! `sources: Vec<RefreshSource>` back-referencing the descriptor
//! element it was derived from (the Grounded<T> posture the
//! drift narrator established).
//!
//! Deterministic + LLM-free, exactly like the drift narrator and
//! the boot summary. "Adversarial" here describes the *purpose*
//! of each suggested fixture (the test it should anchor), not an
//! adversarial LLM round-trip. When the LLM-provider substrate
//! later lands as its own phase, the helper's typed contract
//! stays unchanged — only a richer prose rationale could opt in.
//!
//! In-binary guarantee anchor:
//! `GUARANTEE_ID_APP_ADVERSARIAL_REFRESH_GROUNDED` declares
//! `app.adversarial_refresh_grounded` to the launch-readiness
//! coverage matrix. The positive corpus rows assert that every
//! suggestion carries a non-empty sources array referencing the
//! descriptor field it was derived from; adversarial rows assert
//! the empty-surface descriptor produces an empty report (not a
//! sourceless string) and the renderer is byte-identical across
//! two invocations.

use serde::{Deserialize, Serialize};

use crate::schema::{
    AbiAgent, AbiApprovalSite, AbiStore, AbiTool, CorvidAbi,
};

/// In-binary anchor for the adversarial-refresh launch-readiness
/// row. Re-exported through `corvid-cli` so the runtime, CLI,
/// and coverage gate share one source of truth.
pub const GUARANTEE_ID_APP_ADVERSARIAL_REFRESH_GROUNDED: &str =
    "app.adversarial_refresh_grounded";

/// Names a single descriptor field that an
/// [`AdversarialSuggestion`] was derived from. Mirrors
/// [`crate::boot_summary::BootSource`] — every suggestion is
/// paired with one or more sources naming its provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RefreshSource {
    pub descriptor_field: String,
    pub note: String,
}

/// What canonical adversarial property a suggested fixture
/// should test. The categories are the recurring threat shapes
/// the app reference corpora already cover by hand; the
/// adversarial-refresh helper turns that hand-rolled coverage
/// into a per-surface checklist so no surface element ships
/// without its named adversarial counterpart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreatCategory {
    /// An action whose authorisation does not name a tenant is
    /// refused with a typed cross-tenant error rather than
    /// silently leaking across tenants.
    CrossTenant,
    /// An attempt to invoke a budgeted surface element without
    /// the budget header is refused before the inner body runs.
    MissingBudget,
    /// A `dangerous` site invoked without an in-scope approval
    /// token is refused before any side effect.
    ApprovalBypass,
    /// A `pub extern "c"` agent invoked from an unauthorised
    /// caller (no auth token / wrong role) is refused.
    UnauthorisedCaller,
    /// A `@replayable` operation that would otherwise duplicate
    /// is refused without a fresh replay token.
    ReplayWithoutToken,
    /// A writeable store accessed without an in-scope approval
    /// is refused before the write lands.
    WriteWithoutApproval,
    /// A role-scoped action invoked by a role that does not
    /// satisfy the contract's `required_role` is refused.
    RoleBypass,
    /// An approval token whose expiry has passed is refused as
    /// stale rather than honoured.
    ExpiredApprovalReuse,
    /// A batch invocation that spans more than one `data_class`
    /// without an explicit pin is refused outright.
    DataClassDrift,
    /// A malformed JSON payload to a `pub extern "c"` agent is
    /// refused with a typed parse error, not a panic.
    MalformedPayload,
}

impl ThreatCategory {
    pub const fn slug(self) -> &'static str {
        match self {
            ThreatCategory::CrossTenant => "cross_tenant",
            ThreatCategory::MissingBudget => "missing_budget",
            ThreatCategory::ApprovalBypass => "approval_bypass",
            ThreatCategory::UnauthorisedCaller => "unauthorised_caller",
            ThreatCategory::ReplayWithoutToken => "replay_without_token",
            ThreatCategory::WriteWithoutApproval => "write_without_approval",
            ThreatCategory::RoleBypass => "role_bypass",
            ThreatCategory::ExpiredApprovalReuse => "expired_approval_reuse",
            ThreatCategory::DataClassDrift => "data_class_drift",
            ThreatCategory::MalformedPayload => "malformed_payload",
        }
    }
}

/// What kind of surface element a suggestion attaches to. The
/// distinction matters for the operator: a `Tool` suggestion
/// covers a tool's call site, an `Agent` suggestion covers a
/// flagship entrypoint, an `ApprovalSite` covers an approval
/// contract, a `Store` covers a writeable store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    ApprovalSite,
    Tool,
    Agent,
    Store,
}

impl SurfaceKind {
    pub const fn slug(self) -> &'static str {
        match self {
            SurfaceKind::ApprovalSite => "approval_site",
            SurfaceKind::Tool => "tool",
            SurfaceKind::Agent => "agent",
            SurfaceKind::Store => "store",
        }
    }
}

/// One adversarial-fixture suggestion: which surface element to
/// test, which threat category to assert against, what to name
/// the fixture (snake_case, suitable for a `#[test] fn`), and a
/// one-line operator-facing rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdversarialSuggestion {
    pub surface_kind: SurfaceKind,
    pub surface_name: String,
    pub threat: ThreatCategory,
    /// Suggested snake_case fixture name —
    /// `<surface_name>_<threat_slug>_refused`. Stable across
    /// runs so the operator can grep for the name.
    pub suggested_fixture_name: String,
    /// One-line operator-facing rationale: what the fixture
    /// must assert, in plain English.
    pub rationale: String,
    pub sources: Vec<RefreshSource>,
}

/// Per-category populated counts the operator wants up front so
/// they can sanity-check the coverage shape before walking the
/// detail list. Mirrors `BootSurfaceCounts` posture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdversarialCoverageCounts {
    pub approval_site_suggestions: usize,
    pub tool_suggestions: usize,
    pub agent_suggestions: usize,
    pub store_suggestions: usize,
}

/// The full report. The operator reads `coverage_counts` to
/// sanity-check shape, then walks `suggestions` for detail. The
/// trailing `sources` array names every descriptor field the
/// walker consulted at the report level (per-suggestion sources
/// live on each `AdversarialSuggestion`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdversarialRefreshReport {
    pub app_name: String,
    pub source_path: String,
    pub coverage_counts: AdversarialCoverageCounts,
    pub suggestions: Vec<AdversarialSuggestion>,
    pub sources: Vec<RefreshSource>,
}

/// Pure transform over a descriptor. No I/O, no clock, no
/// randomness. Two invocations on the same descriptor produce
/// byte-identical reports.
pub fn adversarial_refresh_from_descriptor(
    descriptor: &CorvidAbi,
) -> AdversarialRefreshReport {
    let mut sources = Vec::new();
    let mut suggestions: Vec<AdversarialSuggestion> = Vec::new();

    sources.push(RefreshSource {
        descriptor_field: "descriptor.source_path".to_string(),
        note: "app name derived from source path basename".to_string(),
    });

    for site in &descriptor.approval_sites {
        suggestions.extend(suggestions_for_approval_site(site));
    }
    if !descriptor.approval_sites.is_empty() {
        sources.push(RefreshSource {
            descriptor_field: "descriptor.approval_sites".to_string(),
            note: "approval_site suggestions walk every declared approval contract"
                .to_string(),
        });
    }

    for tool in &descriptor.tools {
        suggestions.extend(suggestions_for_tool(tool));
    }
    if descriptor.tools.iter().any(|t| t.dangerous) {
        sources.push(RefreshSource {
            descriptor_field: "descriptor.tools[].dangerous".to_string(),
            note: "tool suggestions target every tool the descriptor marks dangerous"
                .to_string(),
        });
    }

    for agent in &descriptor.agents {
        suggestions.extend(suggestions_for_agent(agent));
    }
    if descriptor
        .agents
        .iter()
        .any(|a| a.attributes.pub_extern_c)
    {
        sources.push(RefreshSource {
            descriptor_field: "descriptor.agents[].attributes.pub_extern_c".to_string(),
            note: "agent suggestions target every `pub extern \"c\"` agent the cdylib exposes"
                .to_string(),
        });
    }

    for store in &descriptor.stores {
        suggestions.extend(suggestions_for_store(store));
    }
    if descriptor.stores.iter().any(|s| !s.effects.write.is_empty() && s.effects.write != "none") {
        sources.push(RefreshSource {
            descriptor_field: "descriptor.stores[].effects.write".to_string(),
            note: "store suggestions target every writeable store".to_string(),
        });
    }

    let coverage_counts = AdversarialCoverageCounts {
        approval_site_suggestions: suggestions
            .iter()
            .filter(|s| s.surface_kind == SurfaceKind::ApprovalSite)
            .count(),
        tool_suggestions: suggestions
            .iter()
            .filter(|s| s.surface_kind == SurfaceKind::Tool)
            .count(),
        agent_suggestions: suggestions
            .iter()
            .filter(|s| s.surface_kind == SurfaceKind::Agent)
            .count(),
        store_suggestions: suggestions
            .iter()
            .filter(|s| s.surface_kind == SurfaceKind::Store)
            .count(),
    };

    suggestions.sort_by(|a, b| {
        a.surface_kind
            .slug()
            .cmp(b.surface_kind.slug())
            .then(a.surface_name.cmp(&b.surface_name))
            .then(a.threat.slug().cmp(b.threat.slug()))
    });

    AdversarialRefreshReport {
        app_name: derive_app_name(&descriptor.source_path),
        source_path: descriptor.source_path.clone(),
        coverage_counts,
        suggestions,
        sources,
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

fn snake_case(input: &str) -> String {
    let mut out = String::new();
    let mut prev_lower_or_digit = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && prev_lower_or_digit && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        } else {
            if !out.ends_with('_') {
                out.push('_');
            }
            prev_lower_or_digit = false;
        }
    }
    out.trim_matches('_').to_string()
}

fn suggestions_for_approval_site(site: &AbiApprovalSite) -> Vec<AdversarialSuggestion> {
    let surface_name = site.label.clone();
    let sname = snake_case(&surface_name);
    let base_sources = vec![
        RefreshSource {
            descriptor_field: format!("descriptor.approval_sites[label={}]", site.label),
            note: "approval-site suggestion derives from the site's label + required_tier"
                .to_string(),
        },
        RefreshSource {
            descriptor_field: format!(
                "descriptor.approval_sites[label={}].required_tier",
                site.label
            ),
            note: format!(
                "required_tier `{}` informs the role-bypass suggestion",
                site.required_tier
            ),
        },
    ];
    let mut out = Vec::new();
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::ApprovalSite,
        surface_name: surface_name.clone(),
        threat: ThreatCategory::CrossTenant,
        suggested_fixture_name: format!("{}_cross_tenant_refused", sname),
        rationale: format!(
            "invoking `{}` without an in-scope tenant binding must refuse \
             with a typed cross-tenant error, not honour the call",
            site.label
        ),
        sources: base_sources.clone(),
    });
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::ApprovalSite,
        surface_name: surface_name.clone(),
        threat: ThreatCategory::RoleBypass,
        suggested_fixture_name: format!("{}_role_bypass_refused", sname),
        rationale: format!(
            "invoking `{}` with a role that does not satisfy `{}` \
             must refuse before any side effect",
            site.label, site.required_tier
        ),
        sources: base_sources.clone(),
    });
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::ApprovalSite,
        surface_name: surface_name.clone(),
        threat: ThreatCategory::ExpiredApprovalReuse,
        suggested_fixture_name: format!("{}_expired_approval_refused", sname),
        rationale: format!(
            "presenting a stale (post-expiry) approval token for `{}` \
             must refuse as expired, not honour",
            site.label
        ),
        sources: base_sources.clone(),
    });
    if !site.dangerous_targets.is_empty() {
        out.push(AdversarialSuggestion {
            surface_kind: SurfaceKind::ApprovalSite,
            surface_name: surface_name.clone(),
            threat: ThreatCategory::DataClassDrift,
            suggested_fixture_name: format!("{}_batch_data_class_drift_refused", sname),
            rationale: format!(
                "a batch over `{}` whose ids span >1 data_class without a pin \
                 must refuse outright, since `{}` declares {} dangerous_target(s)",
                site.label,
                site.label,
                site.dangerous_targets.len()
            ),
            sources: base_sources,
        });
    }
    out
}

fn suggestions_for_tool(tool: &AbiTool) -> Vec<AdversarialSuggestion> {
    if !tool.dangerous {
        return Vec::new();
    }
    let sname = snake_case(&tool.name);
    let base_sources = vec![
        RefreshSource {
            descriptor_field: format!("descriptor.tools[name={}]", tool.name),
            note: "tool suggestion derives from a dangerous tool's name + symbol".to_string(),
        },
        RefreshSource {
            descriptor_field: format!("descriptor.tools[name={}].dangerous", tool.name),
            note: "only `dangerous: true` tools get adversarial suggestions; \
                   safe tools are already covered by their type signature"
                .to_string(),
        },
    ];
    let mut out = Vec::new();
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::Tool,
        surface_name: tool.name.clone(),
        threat: ThreatCategory::CrossTenant,
        suggested_fixture_name: format!("{}_cross_tenant_refused", sname),
        rationale: format!(
            "invoking dangerous tool `{}` from outside the calling tenant's scope \
             must refuse with a typed cross-tenant error",
            tool.name
        ),
        sources: base_sources.clone(),
    });
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::Tool,
        surface_name: tool.name.clone(),
        threat: ThreatCategory::ApprovalBypass,
        suggested_fixture_name: format!("{}_approval_bypass_refused", sname),
        rationale: format!(
            "invoking dangerous tool `{}` without an in-scope approval token \
             must refuse before any side effect",
            tool.name
        ),
        sources: base_sources.clone(),
    });
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::Tool,
        surface_name: tool.name.clone(),
        threat: ThreatCategory::MissingBudget,
        suggested_fixture_name: format!("{}_missing_budget_refused", sname),
        rationale: format!(
            "invoking budgeted tool `{}` without a budget header must refuse \
             before the inner body runs",
            tool.name
        ),
        sources: base_sources,
    });
    out
}

fn suggestions_for_agent(agent: &AbiAgent) -> Vec<AdversarialSuggestion> {
    if !agent.attributes.pub_extern_c {
        return Vec::new();
    }
    let sname = snake_case(&agent.name);
    let base_sources = vec![
        RefreshSource {
            descriptor_field: format!("descriptor.agents[name={}]", agent.name),
            note: "agent suggestion derives from a `pub extern \"c\"` entrypoint".to_string(),
        },
        RefreshSource {
            descriptor_field: format!(
                "descriptor.agents[name={}].attributes.pub_extern_c",
                agent.name
            ),
            note: "only `pub extern \"c\"` agents get adversarial suggestions; \
                   internal agents are unreachable from a host"
                .to_string(),
        },
    ];
    let mut out = Vec::new();
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::Agent,
        surface_name: agent.name.clone(),
        threat: ThreatCategory::MalformedPayload,
        suggested_fixture_name: format!("{}_malformed_payload_refused", sname),
        rationale: format!(
            "calling `pub extern \"c\"` agent `{}` with a malformed JSON \
             payload must refuse with a typed parse error, not panic",
            agent.name
        ),
        sources: base_sources.clone(),
    });
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::Agent,
        surface_name: agent.name.clone(),
        threat: ThreatCategory::UnauthorisedCaller,
        suggested_fixture_name: format!("{}_unauthorised_caller_refused", sname),
        rationale: format!(
            "calling agent `{}` with no auth token or a wrong-role token \
             must refuse before any side effect",
            agent.name
        ),
        sources: base_sources.clone(),
    });
    if agent.attributes.replayable {
        out.push(AdversarialSuggestion {
            surface_kind: SurfaceKind::Agent,
            surface_name: agent.name.clone(),
            threat: ThreatCategory::ReplayWithoutToken,
            suggested_fixture_name: format!("{}_replay_without_token_refused", sname),
            rationale: format!(
                "re-invoking `@replayable` agent `{}` without a fresh replay \
                 token must refuse the duplicate rather than re-execute",
                agent.name
            ),
            sources: base_sources,
        });
    }
    out
}

fn suggestions_for_store(store: &AbiStore) -> Vec<AdversarialSuggestion> {
    if store.effects.write.is_empty() || store.effects.write == "none" {
        return Vec::new();
    }
    let sname = snake_case(&store.name);
    let base_sources = vec![
        RefreshSource {
            descriptor_field: format!("descriptor.stores[name={}]", store.name),
            note: "store suggestion derives from a writeable store".to_string(),
        },
        RefreshSource {
            descriptor_field: format!(
                "descriptor.stores[name={}].effects.write",
                store.name
            ),
            note: format!(
                "write effect `{}` informs the write-without-approval and \
                 cross-tenant-write suggestions",
                store.effects.write
            ),
        },
    ];
    let mut out = Vec::new();
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::Store,
        surface_name: store.name.clone(),
        threat: ThreatCategory::CrossTenant,
        suggested_fixture_name: format!("{}_cross_tenant_write_refused", sname),
        rationale: format!(
            "writing to `{}` from outside the calling tenant's scope must \
             refuse with a typed cross-tenant error before the row lands",
            store.name
        ),
        sources: base_sources.clone(),
    });
    out.push(AdversarialSuggestion {
        surface_kind: SurfaceKind::Store,
        surface_name: store.name.clone(),
        threat: ThreatCategory::WriteWithoutApproval,
        suggested_fixture_name: format!("{}_write_without_approval_refused", sname),
        rationale: format!(
            "writing to `{}` without an in-scope approval must refuse \
             before the row lands",
            store.name
        ),
        sources: base_sources,
    });
    out
}

/// Renders an [`AdversarialRefreshReport`] as the operator-
/// facing text block. Replay-stable.
pub fn render_adversarial_refresh(report: &AdversarialRefreshReport) -> String {
    let mut out = String::new();
    out.push_str("Corvid app adversarial-refresh report\n");
    out.push_str(&format!("app_name: {}\n", report.app_name));
    out.push_str(&format!("source_path: {}\n", report.source_path));
    out.push_str("coverage_counts:\n");
    out.push_str(&format!(
        "  approval_sites: {}\n",
        report.coverage_counts.approval_site_suggestions
    ));
    out.push_str(&format!(
        "  tools: {}\n",
        report.coverage_counts.tool_suggestions
    ));
    out.push_str(&format!(
        "  agents: {}\n",
        report.coverage_counts.agent_suggestions
    ));
    out.push_str(&format!(
        "  stores: {}\n",
        report.coverage_counts.store_suggestions
    ));
    out.push_str("suggestions:\n");
    if report.suggestions.is_empty() {
        out.push_str("  (none — descriptor has no surface elements that warrant adversarial coverage)\n");
    } else {
        for s in &report.suggestions {
            out.push_str(&format!(
                "  - [{}/{}] {} ({})\n",
                s.surface_kind.slug(),
                s.threat.slug(),
                s.suggested_fixture_name,
                s.surface_name
            ));
            out.push_str(&format!("      rationale: {}\n", s.rationale));
        }
    }
    out.push_str("report_sources:\n");
    if report.sources.is_empty() {
        out.push_str("  (none — descriptor had no surface to consult)\n");
    } else {
        for s in &report.sources {
            out.push_str(&format!("  - {}\n", s.descriptor_field));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        AbiApprovalContract, AbiAttributes, AbiEffects, AbiProvenanceContract, AbiSourceSpan,
        AbiStoreEffects, AbiToolContract, ScalarTypeName, TypeDescription,
    };

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

    fn scalar_int() -> TypeDescription {
        TypeDescription::Scalar {
            scalar: ScalarTypeName::Int,
        }
    }

    fn agent(name: &str, pub_extern_c: bool, replayable: bool) -> AbiAgent {
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
                replayable,
                deterministic: false,
                dangerous: false,
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

    fn dangerous_tool(name: &str) -> AbiTool {
        AbiTool {
            name: name.to_string(),
            symbol: format!("__corvid_tool_{name}"),
            params: Vec::new(),
            return_type: scalar_int(),
            effects: AbiEffects::default(),
            dangerous: true,
            contract: AbiToolContract::default(),
        }
    }

    fn writeable_store(name: &str) -> AbiStore {
        AbiStore {
            name: name.to_string(),
            kind: "kv".to_string(),
            fields: Vec::new(),
            policies: Vec::new(),
            accessors: Vec::new(),
            source_span: AbiSourceSpan { start: 0, end: 0 },
            effects: AbiStoreEffects {
                read: "store.read".to_string(),
                write: format!("store.write.{name}"),
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
            effects: AbiEffects::default(),
            required_tier: tier.to_string(),
        }
    }

    /// Positive: every emitted suggestion carries a non-empty
    /// `sources` array naming the descriptor field it came
    /// from. This is the contract that
    /// `app.adversarial_refresh_grounded` promotes.
    #[test]
    fn every_suggestion_carries_non_empty_sources() {
        let mut desc = empty_descriptor();
        desc.approval_sites.push(approval_site(
            "ShareAnswerToChat",
            "operator",
            &["chat_channel_id"],
        ));
        desc.tools.push(dangerous_tool("write_index"));
        desc.agents.push(agent("ask", true, false));
        desc.stores.push(writeable_store("audit"));

        let report = adversarial_refresh_from_descriptor(&desc);
        assert!(
            !report.suggestions.is_empty(),
            "non-empty surface must produce suggestions"
        );
        for s in &report.suggestions {
            assert!(
                !s.sources.is_empty(),
                "suggestion {} ({}) has empty sources",
                s.suggested_fixture_name,
                s.surface_name
            );
        }
        assert!(report.coverage_counts.approval_site_suggestions >= 4);
        assert!(report.coverage_counts.tool_suggestions >= 3);
        assert!(report.coverage_counts.agent_suggestions >= 2);
        assert!(report.coverage_counts.store_suggestions >= 2);
    }

    /// Adversarial: an empty-surface descriptor must produce an
    /// empty report with empty coverage_counts — NOT a panic,
    /// not a sourceless string. The single scalar source
    /// (`descriptor.source_path`) is always present.
    #[test]
    fn empty_surface_descriptor_produces_empty_report_not_sourceless() {
        let desc = empty_descriptor();
        let report = adversarial_refresh_from_descriptor(&desc);
        assert!(report.suggestions.is_empty());
        assert_eq!(report.coverage_counts.approval_site_suggestions, 0);
        assert_eq!(report.coverage_counts.tool_suggestions, 0);
        assert_eq!(report.coverage_counts.agent_suggestions, 0);
        assert_eq!(report.coverage_counts.store_suggestions, 0);
        assert!(
            !report.sources.is_empty(),
            "empty surface must still produce report-level sources (source_path)"
        );
    }

    /// The renderer is replay-stable: two invocations on the
    /// same report produce byte-identical output. Required for
    /// CI gates that diff reports across builds.
    #[test]
    fn render_adversarial_refresh_is_byte_identical_across_two_invocations() {
        let mut desc = empty_descriptor();
        desc.approval_sites.push(approval_site(
            "ExportTenantCorpus",
            "operator",
            &["tenant_id"],
        ));
        desc.tools.push(dangerous_tool("export_rows"));
        desc.agents.push(agent("ask", true, false));

        let report = adversarial_refresh_from_descriptor(&desc);
        let a = render_adversarial_refresh(&report);
        let b = render_adversarial_refresh(&report);
        assert_eq!(a, b);
        assert!(a.contains("Corvid app adversarial-refresh report"));
        assert!(a.contains("export_rows_approval_bypass_refused"));
        assert!(a.contains("export_tenant_corpus_cross_tenant_refused"));
        assert!(a.contains("ask_malformed_payload_refused"));
    }

    /// Replayable agents get the `replay_without_token`
    /// suggestion; non-replayable agents do not. This is the
    /// branch that mirrors `@replayable`'s replay-quarantine
    /// contract on the adversarial side.
    #[test]
    fn replayable_agents_get_replay_without_token_suggestion_non_replayable_do_not() {
        let mut desc = empty_descriptor();
        desc.agents.push(agent("ask", true, false));
        desc.agents.push(agent("mock_chunk", true, true));
        let report = adversarial_refresh_from_descriptor(&desc);
        let replay_suggestions: Vec<_> = report
            .suggestions
            .iter()
            .filter(|s| s.threat == ThreatCategory::ReplayWithoutToken)
            .collect();
        assert_eq!(replay_suggestions.len(), 1);
        assert_eq!(replay_suggestions[0].surface_name, "mock_chunk");
    }

    /// Adversarial: non-dangerous tools must NOT produce
    /// suggestions. Their type signature already constrains
    /// their adversarial surface; injecting adversarial
    /// fixtures for safe tools would fail closed and noise the
    /// operator.
    #[test]
    fn non_dangerous_tools_get_no_suggestions() {
        let mut desc = empty_descriptor();
        desc.tools.push(AbiTool {
            name: "safe_lookup".to_string(),
            symbol: "__corvid_tool_safe_lookup".to_string(),
            params: Vec::new(),
            return_type: scalar_int(),
            effects: AbiEffects::default(),
            dangerous: false,
            contract: AbiToolContract::default(),
        });
        let report = adversarial_refresh_from_descriptor(&desc);
        assert_eq!(report.coverage_counts.tool_suggestions, 0);
    }

    /// Adversarial: read-only stores (write effect empty or
    /// `"none"`) must NOT produce write-targeted suggestions.
    #[test]
    fn read_only_stores_get_no_write_suggestions() {
        let mut desc = empty_descriptor();
        desc.stores.push(AbiStore {
            name: "read_only_cache".to_string(),
            kind: "kv".to_string(),
            fields: Vec::new(),
            policies: Vec::new(),
            accessors: Vec::new(),
            source_span: AbiSourceSpan { start: 0, end: 0 },
            effects: AbiStoreEffects {
                read: "store.read".to_string(),
                write: "none".to_string(),
            },
        });
        let report = adversarial_refresh_from_descriptor(&desc);
        assert_eq!(report.coverage_counts.store_suggestions, 0);
    }
}
