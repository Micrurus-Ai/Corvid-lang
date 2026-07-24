//! Counterfactual exploration of a declared provider protocol.
//!
//! The checker already proves a protocol is well-formed: total transition
//! tables, reachable states, non-zero bounds. Well-formed is not the same
//! as *understood*. The question this answers is the one an author cannot
//! easily hold in their head:
//!
//! > What could the provider do to me?
//!
//! A provider is not adversarial by design, but it is not under your
//! control either. It can stall forever, fail on the first observation,
//! flap between two states, or finish immediately. Each of those is a
//! legal walk through a declaration the checker happily accepted, and each
//! has a different consequence for cost, latency, and the terminal state
//! your program has to handle.
//!
//! Every walk here is driven through [`crate::protocol::observe`] — the
//! same transition function the durable dispatcher uses. Re-implementing
//! the walk would mean simulating a protocol subtly different from the one
//! that actually runs, which is worse than not simulating at all. For the
//! same reason the worst-case observation count comes from
//! [`corvid_ast::ProviderProtocolDecl::worst_case_poll_count`], the
//! definition the budget analysis charges against.

use crate::protocol::{is_terminal, observe, ProtocolIntentState};
use corvid_ast::ProviderProtocolDecl;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One counterfactual: a provider behaviour and where it leaves you.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolOutcome {
    /// The statuses the provider reports, in order.
    pub statuses: Vec<String>,
    /// The state the intent ends in.
    pub final_state: String,
    /// Whether that state is declared terminal.
    pub terminal: bool,
}

/// A provider behaviour that never terminates on its own. These are the
/// ones worth knowing about: the provider holds the intent until the
/// declared deadline fires, and the deadline target is what the program
/// actually has to handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolStall {
    /// The status the provider repeats.
    pub status: String,
    /// The state the intent sits in while it repeats.
    pub state: String,
}

/// Something true of this declaration that its author probably did not
/// state out loud. Not errors — the checker has already rejected the
/// malformed ones — but consequences worth seeing before deploying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolObservation {
    pub kind: &'static str,
    pub detail: String,
}

/// The result of exploring one declared protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolSimulation {
    pub operation: String,
    /// The shortest provider behaviour reaching each terminal state.
    pub outcomes: Vec<ProtocolOutcome>,
    /// Terminal states no provider behaviour can reach. The checker
    /// already proves reachability and refuses these at compile time, so
    /// in practice this stays empty — it is a cross-check that the two
    /// layers agree, not a finding an author normally sees.
    pub unreachable_terminals: Vec<String>,
    /// Behaviours that never terminate on their own.
    pub stalls: Vec<ProtocolStall>,
    /// The most observations the deadline permits.
    pub worst_case_polls: u64,
    /// The declared deadline, and the state it forces.
    pub deadline_secs: u64,
    pub deadline_target: String,
    pub observations: Vec<ProtocolObservation>,
}

/// Explore every distinct provider behaviour this declaration admits.
///
/// The state space is the declared states, which the checker bounds, so
/// this is a breadth-first walk rather than an unbounded search: the
/// shortest status sequence reaching each state is enough to describe
/// what the provider can do, and BFS finds all of them in one pass.
pub fn simulate_protocol(protocol: &ProviderProtocolDecl, operation: &str) -> ProtocolSimulation {
    // BFS from the initial state, driving the REAL transition function.
    let mut shortest: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();
    let initial = protocol.initial.name.clone();
    shortest.insert(initial.clone(), Vec::new());
    queue.push_back((initial.clone(), Vec::new()));

    let mut stalls = Vec::new();
    while let Some((state, path)) = queue.pop_front() {
        if is_terminal(protocol, &state) {
            continue;
        }
        for status in &protocol.statuses {
            let Some(next) = step(protocol, operation, &state, &status.name) else {
                continue;
            };
            // A status that leaves the state unchanged is a stall lever:
            // the provider can repeat it indefinitely and the intent never
            // advances. Worth naming explicitly, because the declaration
            // reads as progress ("on processing -> processing") while the
            // behaviour is "hold me here until the deadline".
            if next == state && !is_terminal(protocol, &next) {
                stalls.push(ProtocolStall {
                    status: status.name.clone(),
                    state: state.clone(),
                });
                continue;
            }
            if shortest.contains_key(&next) {
                continue;
            }
            let mut next_path = path.clone();
            next_path.push(status.name.clone());
            shortest.insert(next.clone(), next_path.clone());
            queue.push_back((next, next_path));
        }
    }

    let mut outcomes: Vec<ProtocolOutcome> = shortest
        .iter()
        .filter(|(state, _)| is_terminal(protocol, state))
        .map(|(state, statuses)| ProtocolOutcome {
            statuses: statuses.clone(),
            final_state: state.clone(),
            terminal: true,
        })
        .collect();
    outcomes.sort_by(|a, b| {
        a.statuses
            .len()
            .cmp(&b.statuses.len())
            .then_with(|| a.final_state.cmp(&b.final_state))
    });

    let reached: BTreeSet<&str> = shortest.keys().map(|s| s.as_str()).collect();
    let unreachable_terminals: Vec<String> = protocol
        .terminal
        .iter()
        .filter(|t| !reached.contains(t.name.as_str()))
        .map(|t| t.name.clone())
        .collect();

    stalls.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| a.status.cmp(&b.status)));
    stalls.dedup();

    let worst_case_polls = protocol.worst_case_poll_count();
    let observations = observe_properties(protocol, &outcomes, &stalls, worst_case_polls);

    ProtocolSimulation {
        operation: operation.to_string(),
        outcomes,
        unreachable_terminals,
        stalls,
        worst_case_polls,
        deadline_secs: protocol.deadline_secs,
        deadline_target: protocol.deadline_target.name.clone(),
        observations,
    }
}

/// Apply one observed status through the real transition engine.
/// `None` when the engine refuses it, which the checker's totality proof
/// should make unreachable — but the simulator asks the engine rather
/// than assuming, so a gap between them shows up here instead of in
/// production.
fn step(
    protocol: &ProviderProtocolDecl,
    operation: &str,
    state: &str,
    status: &str,
) -> Option<String> {
    let mut intent = ProtocolIntentState::new(protocol);
    intent.state = state.to_string();
    let response = serde_json::json!({ "status": status });
    observe(protocol, operation, &mut intent, &response)
        .ok()
        .map(|_| intent.state)
}

fn observe_properties(
    protocol: &ProviderProtocolDecl,
    outcomes: &[ProtocolOutcome],
    stalls: &[ProtocolStall],
    worst_case_polls: u64,
) -> Vec<ProtocolObservation> {
    let mut found = Vec::new();

    // A provider that can finish on its very first answer means the code
    // after the call must handle a terminal outcome with no intermediate
    // state ever observed.
    for outcome in outcomes.iter().filter(|o| o.statuses.len() == 1) {
        found.push(ProtocolObservation {
            kind: "immediate_terminal",
            detail: format!(
                "reporting `{}` once ends the protocol immediately in `{}` — the intent never \
                 passes through an intermediate state",
                outcome.statuses[0], outcome.final_state
            ),
        });
    }

    // The stall is the most consequential provider behaviour, because it
    // is the one that costs money and latency without failing.
    for stall in stalls {
        found.push(ProtocolObservation {
            kind: "non_terminating",
            detail: format!(
                "reporting `{}` forever holds the intent in `{}`; it never terminates on its own, \
                 so after {}s ({} observations) the declared deadline forces `{}`",
                stall.status,
                stall.state,
                protocol.deadline_secs,
                worst_case_polls,
                protocol.deadline_target.name
            ),
        });
    }

    // A terminal the provider can never drive you to is dead weight in
    // every match the program writes against this protocol.
    let reachable: BTreeSet<&str> = outcomes.iter().map(|o| o.final_state.as_str()).collect();
    for terminal in &protocol.terminal {
        if !reachable.contains(terminal.name.as_str()) {
            found.push(ProtocolObservation {
                kind: "unreachable_terminal",
                detail: format!(
                    "no provider behaviour reaches terminal state `{}` — code matching on it is \
                     unreachable",
                    terminal.name
                ),
            });
        }
    }

    // The deadline target is the outcome nobody writes a test for, and it
    // is reached by the provider doing nothing wrong at all.
    if !stalls.is_empty() {
        found.push(ProtocolObservation {
            kind: "deadline_reachable",
            detail: format!(
                "`{}` is reachable without the provider ever failing — a slow provider is enough",
                protocol.deadline_target.name
            ),
        });
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::{
        HttpMethod, Ident, ProtocolIdempotency, ProtocolPoll, ProtocolPollInterval,
        ProtocolStateDecl, ProtocolTransition, Span,
    };

    fn id(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::new(0, 0),
        }
    }

    fn transitions(pairs: &[(&str, &str)]) -> Vec<ProtocolTransition> {
        pairs
            .iter()
            .map(|(s, t)| ProtocolTransition {
                status: id(s),
                target: id(t),
                span: Span::new(0, 0),
            })
            .collect()
    }

    /// queued -> processing -> completed, with `failed` always available.
    fn protocol() -> ProviderProtocolDecl {
        ProviderProtocolDecl {
            statuses: vec![id("queued"), id("processing"), id("completed"), id("failed")],
            initial: id("queued"),
            terminal: vec![id("completed"), id("failed")],
            deadline_secs: 600,
            deadline_target: id("failed"),
            idempotency: ProtocolIdempotency {
                strategy: corvid_ast::ProtocolIdempotencyStrategy::Intent,
                transport: corvid_ast::ProtocolIdempotencyTransport::Header(
                    "Idempotency-Key".to_string(),
                ),
            },
            poll: ProtocolPoll {
                method: HttpMethod::Get,
                path: "/jobs/{id}".to_string(),
                span: Span::new(0, 0),
            },
            cancel: None,
            interval: ProtocolPollInterval::FixedSeconds(30),
            on_protocol_change: corvid_ast::ProtocolChangePolicy::Refuse,
            states: vec![
                ProtocolStateDecl {
                    name: id("queued"),
                    transitions: transitions(&[
                        ("queued", "queued"),
                        ("processing", "processing"),
                        ("completed", "completed"),
                        ("failed", "failed"),
                    ]),
                    span: Span::new(0, 0),
                },
                ProtocolStateDecl {
                    name: id("processing"),
                    transitions: transitions(&[
                        ("queued", "processing"),
                        ("processing", "processing"),
                        ("completed", "completed"),
                        ("failed", "failed"),
                    ]),
                    span: Span::new(0, 0),
                },
            ],
            span: Span::new(0, 0),
        }
    }

    #[test]
    fn every_terminal_gets_a_shortest_provider_behaviour() {
        let sim = simulate_protocol(&protocol(), "submit");
        let completed = sim
            .outcomes
            .iter()
            .find(|o| o.final_state == "completed")
            .expect("completed is reachable");
        assert_eq!(
            completed.statuses,
            vec!["completed"],
            "the provider can finish in one observation"
        );
        assert!(sim.outcomes.iter().any(|o| o.final_state == "failed"));
        assert!(sim.unreachable_terminals.is_empty());
    }

    /// The finding that matters most: a well-formed protocol the checker
    /// accepts can still be held open indefinitely by a provider doing
    /// nothing wrong.
    #[test]
    fn a_self_looping_status_is_reported_as_non_terminating() {
        let sim = simulate_protocol(&protocol(), "submit");
        assert!(
            sim.stalls
                .iter()
                .any(|s| s.status == "processing" && s.state == "processing"),
            "reporting `processing` forever must be reported as a stall; got {:?}",
            sim.stalls
        );
        let stall = sim
            .observations
            .iter()
            .find(|o| o.kind == "non_terminating")
            .expect("a non-terminating behaviour must be surfaced");
        assert!(
            stall.detail.contains("deadline forces `failed`"),
            "the stall must name the outcome the program actually has to handle; got: {}",
            stall.detail
        );
    }

    /// The worst case reported must be the worst case CHARGED, or the
    /// simulator is lying about the budget.
    #[test]
    fn the_worst_case_poll_count_matches_the_budget_bound() {
        let p = protocol();
        let sim = simulate_protocol(&p, "submit");
        assert_eq!(sim.worst_case_polls, p.worst_case_poll_count());
        assert_eq!(sim.worst_case_polls, 20, "600s deadline / 30s cadence");
    }

    #[test]
    fn an_immediate_terminal_is_called_out() {
        let sim = simulate_protocol(&protocol(), "submit");
        assert!(
            sim.observations
                .iter()
                .any(|o| o.kind == "immediate_terminal"),
            "a provider that can finish on the first answer must be surfaced"
        );
    }

    /// A terminal nothing can reach makes every `match` arm against it
    /// dead code, and the author cannot see that by reading the table.
    #[test]
    fn a_terminal_no_behaviour_reaches_is_reported_unreachable() {
        let mut p = protocol();
        p.terminal.push(id("abandoned"));
        let sim = simulate_protocol(&p, "submit");
        assert_eq!(sim.unreachable_terminals, vec!["abandoned".to_string()]);
        assert!(sim
            .observations
            .iter()
            .any(|o| o.kind == "unreachable_terminal" && o.detail.contains("abandoned")));
    }
}
