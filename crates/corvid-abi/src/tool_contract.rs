use crate::schema::{AbiGeneratedApprovalContract, AbiToolContract, AbiToolDomainEffect};
use corvid_ast::{DimensionValue, ToolDecl};
use corvid_types::{EffectProfile, EffectRegistry};
use std::collections::BTreeSet;

pub fn emit_tool_contract(tool: &ToolDecl, registry: &EffectRegistry) -> AbiToolContract {
    let mut contract = AbiToolContract::default();
    let mut seen = BTreeSet::new();

    for effect_ref in &tool.effect_row.effects {
        let source_effect = effect_ref.name.name.as_str();
        let Some(profile) = registry.get(source_effect) else {
            continue;
        };
        collect_domain_effects(
            profile,
            source_effect,
            &tool
                .params
                .iter()
                .map(|param| param.name.name.as_str())
                .collect::<Vec<_>>(),
            &mut seen,
            &mut contract,
        );
        if contract.requires_approval.is_none() {
            contract.requires_approval = approval_requirement_from_profile(profile)
                .or_else(|| human_required_profile(profile).then(|| pascal_case(&tool.name.name)));
        }
    }

    if matches!(tool.effect, corvid_ast::Effect::Dangerous) {
        push_domain_effect(&mut contract, &mut seen, "irreversible", None, "dangerous");
        if contract.requires_approval.is_none() {
            contract.requires_approval = Some(pascal_case(&tool.name.name));
        }
    }

    let domain_effects = contract.domain_effects.clone();
    for effect in &domain_effects {
        match effect.kind.as_str() {
            "money" | "irreversible" => push_unique(&mut contract.ci_fail_on, effect.kind.clone()),
            "external" => push_unique(
                &mut contract.approval_card_hints,
                format!(
                    "external system: {}",
                    effect.target.as_deref().unwrap_or("unspecified")
                ),
            ),
            _ => {}
        }
    }
    if let Some(label) = contract.requires_approval.clone() {
        contract.generated_approval = Some(generated_approval_contract(
            tool,
            registry,
            &contract,
            &label,
        ));
        push_unique(
            &mut contract.approval_card_hints,
            format!("requires approval `{label}`"),
        );
    }
    contract
}

fn generated_approval_contract(
    tool: &ToolDecl,
    registry: &EffectRegistry,
    contract: &AbiToolContract,
    label: &str,
) -> AbiGeneratedApprovalContract {
    let profiles = tool
        .effect_row
        .effects
        .iter()
        .filter_map(|effect_ref| registry.get(effect_ref.name.name.as_str()))
        .collect::<Vec<_>>();
    // `target_resource` names the actual thing the approval is about
    // — the resource being modified, not the channel doing the
    // modification. When a tool's effect declares both `money: <field>`
    // and `external: <vendor>` (e.g. `charge_card` has
    // `money: amount`, `external: stripe`), the resource is the
    // amount and the vendor is incidental.
    //
    // The previous formulation just took the first
    // `domain_effects.target` it found. `domain_effects` is
    // populated by iterating `EffectProfile.dimensions`, which is a
    // `HashMap` (effects.rs:60). HashMap iteration uses
    // `RandomState`, so the order of the pushed effects — and thus
    // which target won here — flipped per process. CI hit a seed
    // where `external: stripe` came before `money: amount` and the
    // contract test failed; local runs and most other CI runs hit
    // the opposite seed and passed. The right fix is a deterministic
    // preference, not coin-flipping on iteration order.
    let pick_target = |kind: &str| {
        contract
            .domain_effects
            .iter()
            .filter(|effect| effect.kind == kind)
            .find_map(|effect| effect.target.clone())
    };
    let target_resource = pick_target("money")
        .or_else(|| pick_target("data"))
        .or_else(|| {
            // Any other kind that carries a target, as a last
            // resort. Deterministic by virtue of `Vec` iteration
            // order being insertion order — push_domain_effect
            // appends, and the calling order across effects is
            // determined by `tool.effect_row.effects` (source
            // order), so equal-priority kinds resolve stably.
            contract
                .domain_effects
                .iter()
                .filter(|effect| !matches!(effect.kind.as_str(), "external" | "irreversible"))
                .find_map(|effect| effect.target.clone())
        })
        .or_else(|| pick_target("external"))
        .or_else(|| tool.params.first().map(|param| param.name.name.clone()))
        .unwrap_or_else(|| tool.name.name.clone());
    let max_cost_usd = profiles
        .iter()
        .filter_map(|profile| profile.dimensions.get("cost"))
        .filter_map(cost_value)
        .sum::<f64>();
    let data_touched = profiles
        .iter()
        .filter_map(|profile| profile.dimensions.get("data"))
        .filter_map(dim_value_label)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    let irreversible = contract
        .domain_effects
        .iter()
        .any(|effect| effect.kind == "irreversible");
    let required_role = profiles
        .iter()
        .find_map(|profile| profile.dimensions.get("required_role"))
        .and_then(dim_value_label)
        .or_else(|| {
            profiles
                .iter()
                .find_map(|profile| profile.dimensions.get("required_approver_role"))
                .and_then(dim_value_label)
        })
        .unwrap_or_else(|| "Reviewer".to_string());
    let expiry_ms = profiles
        .iter()
        .find_map(|profile| profile.dimensions.get("expires_ms"))
        .and_then(number_value)
        .map(|value| value.max(0.0) as u64)
        .or_else(|| {
            profiles
                .iter()
                .find_map(|profile| profile.dimensions.get("expires_in_ms"))
                .and_then(number_value)
                .map(|value| value.max(0.0) as u64)
        });
    AbiGeneratedApprovalContract {
        id: label.to_string(),
        version: "v1".to_string(),
        expected_action: tool.name.name.clone(),
        target_resource,
        max_cost_usd,
        data_touched,
        irreversible,
        expiry_ms,
        required_role,
    }
}

fn collect_domain_effects(
    profile: &EffectProfile,
    source_effect: &str,
    param_names: &[&str],
    seen: &mut BTreeSet<String>,
    contract: &mut AbiToolContract,
) {
    let effect_name = profile.name.as_str();
    if effect_name == "money" || effect_name.starts_with("money_") {
        push_domain_effect(
            contract,
            seen,
            "money",
            money_target(profile).or_else(|| first_money_param(param_names)),
            source_effect,
        );
    }
    if effect_name == "external" || effect_name.starts_with("external_") {
        push_domain_effect(
            contract,
            seen,
            "external",
            external_target(profile).or_else(|| suffix_after(effect_name, "external_")),
            source_effect,
        );
    }
    if effect_name == "irreversible" || non_reversible_profile(profile) {
        push_domain_effect(contract, seen, "irreversible", None, source_effect);
    }

    for (name, value) in &profile.dimensions {
        match name.as_str() {
            "domain" if dim_name(value).as_deref() == Some("money") => push_domain_effect(
                contract,
                seen,
                "money",
                money_target(profile).or_else(|| first_money_param(param_names)),
                source_effect,
            ),
            "money" => push_domain_effect(
                contract,
                seen,
                "money",
                dim_value_label(value).or_else(|| first_money_param(param_names)),
                source_effect,
            ),
            "external" => push_domain_effect(
                contract,
                seen,
                "external",
                dim_value_label(value),
                source_effect,
            ),
            "irreversible" => push_domain_effect(
                contract,
                seen,
                "irreversible",
                dim_value_label(value),
                source_effect,
            ),
            _ => {}
        }
    }
}

fn push_domain_effect(
    contract: &mut AbiToolContract,
    seen: &mut BTreeSet<String>,
    kind: &str,
    target: Option<String>,
    source_effect: &str,
) {
    let key = format!("{kind}:{target:?}:{source_effect}");
    if seen.insert(key) {
        contract.domain_effects.push(AbiToolDomainEffect {
            kind: kind.to_string(),
            target,
            source_effect: source_effect.to_string(),
        });
    }
}

fn approval_requirement_from_profile(profile: &EffectProfile) -> Option<String> {
    profile
        .dimensions
        .get("requires_approval")
        .and_then(dim_value_label)
        .map(|label| label.replace('_', "-"))
}

fn human_required_profile(profile: &EffectProfile) -> bool {
    matches!(
        profile.dimensions.get("trust"),
        Some(DimensionValue::Name(tier)) if tier == "human_required"
    )
}

fn non_reversible_profile(profile: &EffectProfile) -> bool {
    matches!(
        profile
            .dimensions
            .get("reversible")
            .or_else(|| profile.dimensions.get("reversibility")),
        Some(DimensionValue::Bool(false))
    ) || matches!(
        profile
            .dimensions
            .get("reversible")
            .or_else(|| profile.dimensions.get("reversibility")),
        Some(DimensionValue::Name(value)) if value == "irreversible" || value == "non_reversible"
    )
}

fn money_target(profile: &EffectProfile) -> Option<String> {
    profile.dimensions.get("money").and_then(dim_value_label)
}

fn external_target(profile: &EffectProfile) -> Option<String> {
    profile.dimensions.get("external").and_then(dim_value_label)
}

fn first_money_param(param_names: &[&str]) -> Option<String> {
    param_names
        .iter()
        .find(|name| matches!(**name, "amount" | "price" | "total" | "usd" | "cents"))
        .map(|name| (*name).to_string())
}

fn suffix_after(value: &str, prefix: &str) -> Option<String> {
    value
        .strip_prefix(prefix)
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
}

fn dim_name(value: &DimensionValue) -> Option<String> {
    match value {
        DimensionValue::Name(value) => Some(value.clone()),
        _ => None,
    }
}

fn dim_value_label(value: &DimensionValue) -> Option<String> {
    match value {
        DimensionValue::Name(value) => Some(value.clone()),
        DimensionValue::Bool(value) => Some(value.to_string()),
        DimensionValue::Cost(value) | DimensionValue::Number(value) => Some(value.to_string()),
        DimensionValue::Streaming { backpressure } => Some(backpressure.label()),
        DimensionValue::ConfidenceGated { threshold, .. } => Some(threshold.to_string()),
    }
}

fn cost_value(value: &DimensionValue) -> Option<f64> {
    match value {
        DimensionValue::Cost(value) | DimensionValue::Number(value) => Some(*value),
        _ => None,
    }
}

fn number_value(value: &DimensionValue) -> Option<f64> {
    match value {
        DimensionValue::Number(value) | DimensionValue::Cost(value) => Some(*value),
        _ => None,
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}
