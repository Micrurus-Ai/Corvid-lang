//! The `corvid-ai.json` metadata artifact (slice 51c).
//!
//! Where OpenAPI (51b) describes ordinary HTTP, this describes the
//! AI-native behavior a frontend must model: the typed STREAMING
//! EVENT PROTOCOL each public agent emits, whether an invocation can
//! raise an approval mid-flight, the grounding/source shape of the
//! output, the confidence floor and where a below-floor result
//! routes, and the cost/latency envelope. A `@corvid/client` (51l)
//! turns this into `for await (const event of agent.stream(...))`
//! with a fully-typed event union instead of hand-parsed SSE.
//!
//! It is a projection of the same [`crate::app_contract`] model
//! OpenAPI reads, so the two artifacts reference identical schemas.

use crate::app_contract::{
    ApplicationContract, Capabilities, ContractCallable, ContractParam, Pagination,
};
use serde::{Deserialize, Serialize};

/// The AI-native metadata document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorvidAiMetadata {
    pub contract_version: u32,
    pub source_path: String,
    pub generated_at: String,
    pub agents: Vec<AiCallable>,
    pub prompts: Vec<AiCallable>,
}

/// One agent/prompt's AI-native invocation shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiCallable {
    pub name: String,
    pub inputs: Vec<ContractParam>,
    pub output_type: String,
    /// The event kinds a client may observe over the invocation's
    /// lifetime, in the order they can appear.
    pub events: Vec<EventKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding: Option<Grounding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ConfidenceRouting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_class: Option<String>,
    /// An input carries untrusted content (slice 50i) — the client
    /// should not treat a response derived from it as trusted without
    /// the program's own sanitization boundary.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub tainted_input: bool,
    /// Pagination surface (slice 51f) — `Page<Item>` (cursor) or
    /// `Stream<Item>` (stream). A generic paginated hook drives
    /// "load more" / consume-to-end from this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
}

/// One event a streamed/invoked callable can emit. Serialized as a
/// lowercase tag matching the SSE `event:` field the runtime writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The invocation began; carries a run id for correlation/replay.
    Started,
    /// An incremental output chunk (streaming callables only).
    Chunk,
    /// A tool the agent orchestrates began / finished.
    ToolStarted,
    ToolCompleted,
    /// A dangerous/high-trust action needs approval before the
    /// invocation can continue; carries the approval contract.
    ApprovalRequired,
    /// Terminal success; carries the final value.
    Completed,
    /// Terminal failure; carries a typed error.
    Failed,
}

/// The grounding shape of a `Grounded<T>` output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grounding {
    /// The output carries a `sources` array of provenance entries.
    pub has_sources: bool,
    /// Each source carries a provider record id / citation key.
    pub source_shape: String,
}

/// How a below-floor confidence result is handled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceRouting {
    pub minimum: f64,
    /// What happens below the floor. `human_review` is the default
    /// escalation; the frontend renders a review state accordingly.
    pub below_minimum: String,
}

/// Project the application contract to its AI-native metadata.
pub fn emit_corvid_ai(contract: &ApplicationContract) -> CorvidAiMetadata {
    CorvidAiMetadata {
        contract_version: contract.contract_version,
        source_path: contract.source_path.clone(),
        generated_at: contract.generated_at.clone(),
        agents: contract
            .agents
            .iter()
            .map(|c| ai_callable(c, /* is_agent */ true))
            .collect(),
        prompts: contract
            .prompts
            .iter()
            .map(|c| ai_callable(c, /* is_agent */ false))
            .collect(),
    }
}

fn ai_callable(c: &ContractCallable, is_agent: bool) -> AiCallable {
    let caps = &c.capabilities;
    AiCallable {
        name: c.name.clone(),
        inputs: c.inputs.clone(),
        output_type: c.output_type.clone(),
        events: event_protocol(caps, is_agent),
        grounding: caps.grounded.then_some(Grounding {
            has_sources: true,
            source_shape: "provenance_entry".into(),
        }),
        confidence: caps.confidence_min.map(|minimum| ConfidenceRouting {
            minimum,
            below_minimum: "human_review".into(),
        }),
        max_cost_usd: caps.max_cost_usd,
        latency_class: caps.latency_class.clone(),
        tainted_input: caps.tainted_input,
        pagination: caps.pagination.clone(),
    }
}

/// The ordered event kinds a callable can emit. `started` opens and
/// `completed`/`failed` close every invocation; the middle events are
/// present only when the capability is.
fn event_protocol(caps: &Capabilities, is_agent: bool) -> Vec<EventKind> {
    let mut events = vec![EventKind::Started];
    if caps.streaming {
        events.push(EventKind::Chunk);
    }
    // Agents orchestrate tools; prompts do not.
    if is_agent {
        events.push(EventKind::ToolStarted);
        events.push(EventKind::ToolCompleted);
    }
    if caps.approvals_possible {
        events.push(EventKind::ApprovalRequired);
    }
    events.push(EventKind::Completed);
    events.push(EventKind::Failed);
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_contract::{ContractCallable, CONTRACT_VERSION};

    fn callable(name: &str, output: &str, caps: Capabilities) -> ContractCallable {
        ContractCallable {
            name: name.into(),
            inputs: vec![],
            output_type: output.into(),
            capabilities: caps,
        }
    }

    fn contract(agents: Vec<ContractCallable>) -> ApplicationContract {
        ApplicationContract {
            contract_version: CONTRACT_VERSION,
            compiler_version: "1.0.0".into(),
            generated_at: "now".into(),
            source_path: "app.cor".into(),
            types: vec![],
            routes: vec![],
            agents,
            prompts: vec![],
        }
    }

    #[test]
    fn streaming_agent_gets_chunk_events_and_terminals() {
        let caps = Capabilities {
            streaming: true,
            ..Default::default()
        };
        let meta = emit_corvid_ai(&contract(vec![callable("chat", "Stream<String>", caps)]));
        let chat = &meta.agents[0];
        assert!(chat.events.contains(&EventKind::Chunk));
        assert!(chat.events.contains(&EventKind::Started));
        assert!(chat.events.contains(&EventKind::Completed));
        assert!(chat.events.contains(&EventKind::ToolStarted));
    }

    #[test]
    fn approvals_and_grounding_and_confidence_surface() {
        let caps = Capabilities {
            grounded: true,
            approvals_possible: true,
            confidence_min: Some(0.85),
            ..Default::default()
        };
        let meta = emit_corvid_ai(&contract(vec![callable("refund", "Grounded<Receipt>", caps)]));
        let refund = &meta.agents[0];
        assert!(refund.events.contains(&EventKind::ApprovalRequired));
        assert!(refund.grounding.as_ref().unwrap().has_sources);
        assert_eq!(refund.confidence.as_ref().unwrap().minimum, 0.85);
        assert_eq!(
            refund.confidence.as_ref().unwrap().below_minimum,
            "human_review".to_string()
        );
    }

    #[test]
    fn non_streaming_agent_has_no_chunk_events() {
        let meta = emit_corvid_ai(&contract(vec![callable(
            "classify",
            "String",
            Capabilities::default(),
        )]));
        assert!(!meta.agents[0].events.contains(&EventKind::Chunk));
    }
}
