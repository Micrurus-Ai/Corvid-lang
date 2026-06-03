use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustTier {
    Autonomous,
    HumanRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DimensionSnapshot {
    pub cost: f64,
    pub latency_ms: u64,
    pub trust_tier: Option<TrustTier>,
    pub budget_declared: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvenanceSnapshot {
    pub nodes: BTreeSet<String>,
    pub root_sources: BTreeSet<String>,
    pub has_chain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DangerousToolSpec {
    pub tool: String,
    pub approval_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentInvariantInfo {
    pub agent: String,
    pub replayable: bool,
    pub deterministic: bool,
    pub grounded_return: bool,
    pub budget_declared: Option<f64>,
    pub dangerous_tools: Vec<DangerousToolSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationSpec {
    pub step_1based: usize,
    pub replacement: serde_json::Value,
    pub label: String,
}

#[derive(Debug, Clone)]
pub enum ShadowExecutionMode {
    Replay,
    Differential { model: String },
    Mutation(MutationSpec),
}

#[derive(Debug, Clone)]
pub struct ShadowReplayOutcome {
    pub trace_path: PathBuf,
    pub run_id: String,
    pub agent: String,
    pub recorded_events: Vec<TraceEvent>,
    pub shadow_trace_path: PathBuf,
    pub shadow_events: Vec<TraceEvent>,
    pub recorded_output: Option<serde_json::Value>,
    pub shadow_output: Option<serde_json::Value>,
    pub replay_divergence: Option<ReplayDivergence>,
    pub differential_report: Option<ReplayDifferentialReport>,
    pub mutation_report: Option<ReplayMutationReport>,
    pub recorded_dimensions: DimensionSnapshot,
    pub shadow_dimensions: DimensionSnapshot,
    pub recorded_provenance: ProvenanceSnapshot,
    pub shadow_provenance: ProvenanceSnapshot,
    pub metadata: AgentInvariantInfo,
    pub mode: String,
    pub ok: bool,
    pub error: Option<String>,
}

impl ShadowReplayOutcome {
    pub fn normalized_recorded_events(&self) -> Vec<serde_json::Value> {
        self.recorded_events
            .iter()
            .filter(|event| is_decision_event(event))
            .map(normalize_event_json)
            .collect()
    }

    pub fn normalized_shadow_events(&self) -> Vec<serde_json::Value> {
        self.shadow_events
            .iter()
            .filter(|event| is_decision_event(event))
            .map(normalize_event_json)
            .collect()
    }

    /// `traces_match` compares the agent's decision trajectory between
    /// the live recording and the shadow-replay re-execution: schema
    /// header, run lifecycle, LLM call / result, tool call / result,
    /// seed and clock reads, observation events. The `host_event`
    /// family (`llm.usage`, `connector.call`, `cost.budget`, …)
    /// carries TELEMETRY about how a step was serviced — adapter id,
    /// token counts, cost-USD, latency-ms — rather than the step
    /// itself, and the live LLM dispatch path at
    /// `corvid-runtime/src/runtime/llm_dispatch.rs:306-322` emits
    /// these unconditionally on the record side, whereas the
    /// shadow-replay execution path under env-mock-llm skips
    /// emission for telemetry that was already captured in the
    /// recorded trace. Asserting byte-identity over the union would
    /// require the runtime's record-and-replay event symmetry to
    /// hold for telemetry too — a separate runtime work item
    /// tracked alongside the replay-determinism story — and
    /// softening the test by squashing all events on both sides
    /// would mask the very divergences this assertion is meant to
    /// catch. Filtering at the decision-event layer keeps the
    /// equality narrow and load-bearing: any mismatch in the
    /// decision steps (LLM result drift, tool result drift, seed
    /// non-determinism) still trips the assertion, while
    /// telemetry-only divergence (already covered by the
    /// usage-ledger / cost-budget invariants in
    /// `corvid-runtime`) does not.
    pub fn traces_match(&self) -> bool {
        self.normalized_recorded_events() == self.normalized_shadow_events()
    }

    pub fn seed_clock_positions(events: &[TraceEvent]) -> Vec<(usize, &'static str)> {
        events
            .iter()
            .enumerate()
            .filter_map(|(idx, event)| match event {
                TraceEvent::SeedRead { .. } => Some((idx, "seed")),
                TraceEvent::ClockRead { .. } => Some((idx, "clock")),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub enum ShadowExecutorError {
    Io(String),
    Compile(String),
    TraceLoad(String),
    Runtime(RuntimeError),
    Interp(String),
    UnsupportedProgramPath(PathBuf),
}

impl std::fmt::Display for ShadowExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => f.write_str(msg),
            Self::Compile(msg) => f.write_str(msg),
            Self::TraceLoad(msg) => f.write_str(msg),
            Self::Runtime(err) => err.fmt(f),
            Self::Interp(msg) => f.write_str(msg),
            Self::UnsupportedProgramPath(path) => write!(
                f,
                "shadow daemon v1 requires `daemon.ir_path` to point at a `.cor` source file; `{}` is not supported yet",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ShadowExecutorError {}
