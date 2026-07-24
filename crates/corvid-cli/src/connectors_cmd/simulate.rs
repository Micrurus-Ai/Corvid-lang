//! `corvid connectors simulate` — counterfactual exploration of the
//! `async:` protocols declared in a source file.
//!
//! Compiles the file through the normal driver pipeline (so what is
//! explored is the graph the checker accepted, not a re-parse of the
//! text) and drives each protocol through the runtime's own transition
//! engine.

use anyhow::{Context, Result};
use corvid_runtime::protocol_simulate::{simulate_protocol, ProtocolSimulation};
use std::path::Path;

/// Explore every protocol in `file`, optionally narrowed to one
/// operation.
pub fn run_simulate(file: &Path, operation: Option<&str>) -> Result<Vec<ProtocolSimulation>> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("reading `{}`", file.display()))?;
    let ir = corvid_driver::compile_to_ir_with_config_at_path(&source, file, None)
        .map_err(|diagnostics| {
            anyhow::anyhow!(
                "{}",
                diagnostics
                    .iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })
        .with_context(|| format!("compiling `{}`", file.display()))?;

    let mut simulations = Vec::new();
    for connector in &ir.connectors {
        for op in &connector.operations {
            let Some(protocol) = &op.protocol else {
                continue;
            };
            if operation.is_some_and(|wanted| wanted != op.name) {
                continue;
            }
            simulations.push(simulate_protocol(
                protocol,
                &format!("{}.{}", connector.name, op.name),
            ));
        }
    }
    Ok(simulations)
}

/// Render the human report. Ordered so the consequences an author is
/// least likely to have considered come first.
pub fn render(simulations: &[ProtocolSimulation]) -> String {
    if simulations.is_empty() {
        return "no `async:` protocols declared in this file\n".to_string();
    }
    let mut out = String::new();
    for sim in simulations {
        out.push_str(&format!("protocol {}\n", sim.operation));
        out.push_str(&format!(
            "  deadline: {}s, at most {} observation(s) before `{}` is forced\n",
            sim.deadline_secs, sim.worst_case_polls, sim.deadline_target
        ));

        out.push_str("  outcomes the provider can produce:\n");
        for outcome in &sim.outcomes {
            let behaviour = if outcome.statuses.is_empty() {
                "<already terminal>".to_string()
            } else {
                outcome.statuses.join(" -> ")
            };
            out.push_str(&format!(
                "    {:<40} ends in `{}`\n",
                behaviour, outcome.final_state
            ));
        }

        if !sim.observations.is_empty() {
            out.push_str("  worth knowing:\n");
            for observation in &sim.observations {
                out.push_str(&format!(
                    "    [{}] {}\n",
                    observation.kind, observation.detail
                ));
            }
        }
        out.push('\n');
    }
    out
}

/// Every behaviour this command reports is LEGAL — the checker has
/// already refused the malformed protocols, including unreachable states
/// and terminals. So there is nothing here to fail on by default, and a
/// command that invented a failure would be lying about severity.
///
/// What differs between teams is which legal behaviours they are willing
/// to ship. `--deny non_terminating` is the useful one: it asserts this
/// protocol cannot be held open by a provider that never fails. That is a
/// property the checker cannot prove for you, because stalling is a
/// legitimate thing for a declaration to permit.
pub fn exit_code(simulations: &[ProtocolSimulation], deny: &[String]) -> u8 {
    if deny.is_empty() {
        return 0;
    }
    let denied = simulations.iter().any(|sim| {
        sim.observations
            .iter()
            .any(|observation| deny.iter().any(|kind| kind == observation.kind))
    });
    u8::from(denied)
}

/// The findings that tripped `--deny`, for the operator to read.
pub fn denied_findings<'a>(
    simulations: &'a [ProtocolSimulation],
    deny: &[String],
) -> Vec<(&'a str, &'a str)> {
    simulations
        .iter()
        .flat_map(|sim| {
            sim.observations
                .iter()
                .filter(|o| deny.iter().any(|kind| kind == o.kind))
                .map(|o| (sim.operation.as_str(), o.detail.as_str()))
        })
        .collect()
}

/// The JSON projection, for CI and tooling.
pub fn to_json(simulations: &[ProtocolSimulation]) -> serde_json::Value {
    serde_json::Value::Array(
        simulations
            .iter()
            .map(|sim| {
                serde_json::json!({
                    "operation": sim.operation,
                    "deadline_secs": sim.deadline_secs,
                    "deadline_target": sim.deadline_target,
                    "worst_case_polls": sim.worst_case_polls,
                    "outcomes": sim.outcomes.iter().map(|o| serde_json::json!({
                        "statuses": o.statuses,
                        "final_state": o.final_state,
                        "terminal": o.terminal,
                    })).collect::<Vec<_>>(),
                    "non_terminating": sim.stalls.iter().map(|s| serde_json::json!({
                        "status": s.status,
                        "state": s.state,
                    })).collect::<Vec<_>>(),
                    "unreachable_terminals": sim.unreachable_terminals,
                    "observations": sim.observations.iter().map(|o| serde_json::json!({
                        "kind": o.kind,
                        "detail": o.detail,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}
