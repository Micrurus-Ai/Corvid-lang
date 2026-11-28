use super::super::compose::canonical_dimension_name;
use super::*;

pub fn render_cost_tree(
    tree: &CostTreeNode,
    budget_constraints: Option<&[EffectConstraint]>,
) -> String {
    let mut lines = Vec::new();
    render_cost_tree_lines(tree, "", true, &mut lines);

    if let Some(constraints) = budget_constraints {
        if !constraints.is_empty() {
            lines.push(String::new());
            for constraint in constraints {
                let dim = canonical_dimension_name(&constraint.dimension.name);
                let used = tree.costs.get(&dim).copied().unwrap_or(0.0);
                if let Some(limit) = numeric_constraint_value(constraint) {
                    let pct = if limit > 0.0 {
                        (used / limit) * 100.0
                    } else {
                        0.0
                    };
                    let status = if used <= limit { "✓" } else { "✗" };
                    lines.push(format!(
                        "{:8} budget: {:<10} used: {:<10} ({:.1}%) {status}",
                        dim,
                        format_numeric_dimension(&dim, limit),
                        format_numeric_dimension(&dim, used),
                        pct,
                    ));
                }
            }
        }
    }

    lines.join("\n")
}

fn render_cost_tree_lines(
    node: &CostTreeNode,
    prefix: &str,
    is_last: bool,
    lines: &mut Vec<String>,
) {
    let branch = if prefix.is_empty() {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };
    lines.push(format!(
        "{prefix}{branch}{:<28} total: {:<10} tokens: {:<10} latency: {}",
        node.name,
        format_numeric_dimension("cost", node.costs.get("cost").copied().unwrap_or(0.0)),
        format_numeric_dimension("tokens", node.costs.get("tokens").copied().unwrap_or(0.0)),
        format_numeric_dimension(
            "latency_ms",
            node.costs.get("latency_ms").copied().unwrap_or(0.0)
        ),
    ));

    let next_prefix = if prefix.is_empty() {
        String::new()
    } else if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}│   ")
    };

    for (index, child) in node.children.iter().enumerate() {
        render_cost_tree_lines(child, &next_prefix, index + 1 == node.children.len(), lines);
    }
}

pub fn format_numeric_dimension(dimension: &str, value: f64) -> String {
    match dimension {
        "cost" => format!("${}", at_least_three_decimals(value)),
        "tokens" => format!("{:.0}", value),
        "latency_ms" => format!("{:.1}s", value / 1000.0),
        _ => at_least_three_decimals(value),
    }
}

/// `value` with three decimals, or up to six when fewer would hide part of
/// it: a $0.0002 step must not read as `$0.000`, which looks free, nor a
/// $0.0125 one as `$0.013`.
fn at_least_three_decimals(value: f64) -> String {
    let mut text = format!("{value:.6}");
    while text.ends_with('0') && text.len() - text.find('.').map_or(text.len(), |dot| dot + 1) > 3 {
        text.pop();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::format_numeric_dimension;

    #[test]
    fn a_small_cost_is_never_shown_as_zero() {
        assert_eq!(format_numeric_dimension("cost", 0.0002), "$0.0002");
        assert_eq!(format_numeric_dimension("cost", 0.0125), "$0.0125");
        assert_eq!(format_numeric_dimension("cost", 0.03), "$0.030");
        assert_eq!(format_numeric_dimension("cost", 0.1 + 0.2), "$0.300");
        assert_eq!(format_numeric_dimension("cost", 0.0), "$0.000");
        assert_eq!(format_numeric_dimension("cost", 12.5), "$12.500");
        assert_eq!(format_numeric_dimension("trust", 0.00025), "0.00025");
    }
}
