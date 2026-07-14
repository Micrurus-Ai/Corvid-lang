//! Trust-tier approval derivation.
//!
//! The compile-time approve gate keys on the `dangerous` keyword AND
//! on the trust tier a tool's effect row composes: an effect whose
//! `trust` dimension is `supervisor_required` or `human_required`
//! derives the same approve requirement `dangerous` declares. An
//! author who declares high-trust semantics but forgets the
//! `dangerous` marker still gets compile-time protection — the
//! silent-footgun shape (declared high trust, zero enforcement) is
//! exactly what the language exists to prevent.
//!
//! Scope: TOOL call sites. Prompts and agents with high-trust
//! effects interact with the `@trust` ceiling and runtime
//! escalation instead — an LLM render is not an action; a tool call
//! is.

use super::EffectRegistry;
use corvid_ast::DimensionValue;

/// The two trust tiers that derive an approve requirement at tool
/// call sites. `autonomous` (and the confidence-gated
/// `autonomous_if_confident(...)`, which typechecks as `autonomous`
/// and escalates at runtime) do not.
const APPROVAL_DERIVING_TIERS: &[&str] = &["supervisor_required", "human_required"];

/// If this effect row derives an approve requirement from its trust
/// tier, return the `(effect_name, tier)` pair that drives it — the
/// diagnostic must name both so the author can see exactly which
/// declaration created the obligation.
pub fn effect_row_trust_requires_approval(
    effect_row: &corvid_ast::EffectRow,
    registry: &EffectRegistry,
) -> Option<(String, String)> {
    effect_row.effects.iter().find_map(|eff| {
        let profile = registry.get(&eff.name.name)?;
        match profile.dimensions.get("trust") {
            Some(DimensionValue::Name(tier)) if APPROVAL_DERIVING_TIERS.contains(&tier.as_str()) => {
                Some((eff.name.name.clone(), tier.clone()))
            }
            _ => None,
        }
    })
}
