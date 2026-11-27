//! The `TraceEvent` taxonomy — every nondeterministic input or
//! dispatch decision the runtime records into a trace.
//!
//! The tag field is `kind`, and each variant has a flat struct
//! shape so the wire form reads naturally in JSONL:
//!
//! ```jsonl
//! {"kind":"schema_header","version":1,"writer":"corvid-vm","commit_sha":null,"ts_ms":0,"run_id":"r-1"}
//! {"kind":"run_started","ts_ms":1,"run_id":"r-1","agent":"demo","args":[]}
//! ```
//!
//! Every new event variant should be additive: old readers must
//! skip unknown variants rather than fail. Existing variants must
//! not change shape; evolving a variant means bumping
//! `SCHEMA_VERSION` and teaching readers how to upgrade old traces.

use serde::{Deserialize, Serialize};

/// Every event emitted by the runtime. Serialized one-per-line in
/// JSONL. A trace file typically starts with a `SchemaHeader`,
/// followed by exactly one `RunStarted`, then interleaved
/// `ToolCall` / `LlmCall` / `ApprovalResponse` / `SeedRead` /
/// `ClockRead` / dispatch events, and closes with a single
/// `RunCompleted`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TraceEvent {
    /// First event in every trace. Identifies the schema version
    /// the file was written against so readers can refuse
    /// incompatible traces or apply version-specific upgrade
    /// logic.
    SchemaHeader {
        version: u32,
        /// Identifier for the recording tier (`"corvid-vm"`,
        /// `"corvid-codegen-cl"`, etc.) — useful for cross-tier
        /// debugging when the runtime is ambiguous.
        writer: String,
        /// Git commit SHA the recording binary was built from, if
        /// known. `None` in tests and local dev builds without
        /// version injection.
        #[serde(default)]
        commit_sha: Option<String>,
        /// Path to the Corvid source file the recording ran against,
        /// relative to whatever anchor the recorder chose (typically
        /// the repo root or the CWD at record time). `None` for
        /// pre-schema-v2 traces and for run modes where no source
        /// file exists (REPL, ad-hoc bytecode). Present in v2+ traces
        /// so `corvid replay <trace>` can locate the source without a
        /// sidecar — the trace is self-describing.
        #[serde(default)]
        source_path: Option<String>,
        /// B06 — the IANA time zone database the recording binary was
        /// built with, such as `2025b`. Zone rules change between
        /// versions, so replay compares it with its own and warns at
        /// each zone-dependent operation when they differ. `None` for
        /// pre-schema-v3 traces, read as "unknown" rather than refused.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tzdb_version: Option<String>,
        ts_ms: u64,
        run_id: String,
    },
    RunStarted {
        ts_ms: u64,
        run_id: String,
        agent: String,
        #[serde(default)]
        args: Vec<serde_json::Value>,
    },
    RunCompleted {
        ts_ms: u64,
        run_id: String,
        ok: bool,
        #[serde(default)]
        result: Option<serde_json::Value>,
        #[serde(default)]
        error: Option<String>,
    },
    ToolCall {
        ts_ms: u64,
        run_id: String,
        tool: String,
        args: Vec<serde_json::Value>,
    },
    ToolResult {
        ts_ms: u64,
        run_id: String,
        tool: String,
        result: serde_json::Value,
    },
    LlmCall {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        model: Option<String>,
        #[serde(default)]
        model_version: Option<String>,
        #[serde(default)]
        rendered: Option<String>,
        #[serde(default)]
        args: Vec<serde_json::Value>,
        /// Sampling parameters the request carried (slice 46a) —
        /// `{"temperature": .., "top_p": .., "max_tokens": ..}`.
        /// Absent when every knob used the adapter default; absent
        /// in pre-46a traces (serde default keeps them readable).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sampling: Option<serde_json::Value>,
    },
    LlmResult {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        model_version: Option<String>,
        result: serde_json::Value,
        /// Streamed-response chunk boundaries (slice 46d): byte
        /// offsets into the final text where provider chunks ended.
        /// Replay re-chunks the recorded text at exactly these
        /// offsets, so a streamed run replays with identical chunk
        /// timing structure. Absent for non-streamed calls and in
        /// pre-46d traces.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chunk_boundaries: Option<Vec<u64>>,
    },
    /// Metadata for a cacheable prompt call. A cache hit is still recorded
    /// as a normal `LlmCall` / `LlmResult` pair so replay consumes the same
    /// semantic events whether the response was live or cached.
    PromptCache {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        model_version: Option<String>,
        fingerprint: String,
        hit: bool,
    },
    ApprovalRequest {
        ts_ms: u64,
        run_id: String,
        label: String,
        args: Vec<serde_json::Value>,
    },
    ApprovalDecision {
        ts_ms: u64,
        run_id: String,
        site: String,
        args: Vec<serde_json::Value>,
        accepted: bool,
        decider: String,
        #[serde(default)]
        rationale: Option<String>,
    },
    ApprovalResponse {
        ts_ms: u64,
        run_id: String,
        label: String,
        approved: bool,
    },
    ApprovalTokenIssued {
        ts_ms: u64,
        run_id: String,
        token_id: String,
        label: String,
        args: Vec<serde_json::Value>,
        scope: String,
        issued_at_ms: u64,
        expires_at_ms: u64,
    },
    ApprovalScopeViolation {
        ts_ms: u64,
        run_id: String,
        token_id: String,
        label: String,
        reason: String,
    },
    HumanInputRequest {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        expected_type: String,
    },
    HumanInputResponse {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        value: serde_json::Value,
    },
    HumanChoiceRequest {
        ts_ms: u64,
        run_id: String,
        options: Vec<serde_json::Value>,
    },
    HumanChoiceResponse {
        ts_ms: u64,
        run_id: String,
        selected_index: usize,
        selected_value: serde_json::Value,
    },
    HostEvent {
        ts_ms: u64,
        run_id: String,
        name: String,
        payload: serde_json::Value,
    },
    /// One HTTP request the runtime sent: one hop of one attempt. A
    /// retried call records each attempt, and a followed redirect each
    /// hop, so the trace shows every host a call reached. Evidence only:
    /// replay substitutes the enclosing tool call and never these.
    ///
    /// Nothing credential-shaped is recorded. Request and response
    /// headers appear by name only, and the body by its SHA-256 and size.
    HttpExchange {
        ts_ms: u64,
        run_id: String,
        /// The connector that sent the request, when one did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connector: Option<String>,
        /// The operation or tool the request belongs to.
        operation: String,
        /// 1 for the first attempt, 2 for the first retry, and so on.
        attempt: u32,
        /// 0 for the request as sent, 1 for the first redirect followed.
        hop: u32,
        method: String,
        url: String,
        request_headers: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_body_sha256: Option<String>,
        request_body_bytes: u64,
        /// The response status, absent when no response arrived.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
        response_headers: Vec<String>,
        response_body_bytes: u64,
        elapsed_ms: u64,
        /// `response`, `redirected`, `timeout`, `refused` or
        /// `transport_error`.
        outcome: String,
        /// Why the exchange did not complete, for every outcome but
        /// `response` and `redirected`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// An agent call began, written just before the first traced step
    /// it contains (an agent that records nothing leaves no events).
    /// Its steps, and agents it calls, nest under it until the matching
    /// `AgentFinished`. Evidence only: replay never substitutes it.
    AgentStarted {
        ts_ms: u64,
        run_id: String,
        agent: String,
    },
    /// The agent call opened by the latest unmatched `AgentStarted`
    /// ended: `ok` is false when it ended in an error.
    AgentFinished {
        ts_ms: u64,
        run_id: String,
        agent: String,
        ok: bool,
    },
    /// Where one recorded table operation spent its time in the
    /// database. It is written just before the operation's `ToolResult`,
    /// and `tool` names the operation (`table:<Table>.<op>`). Waiting
    /// covers a turn at the Postgres pool or at SQLite's writer queue; a
    /// growing wait with steady statement time means `connections:` is
    /// too small. Evidence only: replay never substitutes it, because
    /// timing is not part of a result.
    DbTiming {
        ts_ms: u64,
        run_id: String,
        tool: String,
        /// Microseconds spent waiting for a connection or a turn to write.
        wait_us: u64,
        /// Microseconds spent running the operation's statements.
        statement_us: u64,
        /// How many statements the operation ran.
        statements: u32,
    },
    /// A pseudo-random number read. Recorded per draw so replay
    /// can reproduce the exact sequence even when the seeded PRNG
    /// runs through different call paths.
    SeedRead {
        ts_ms: u64,
        run_id: String,
        /// Human-readable reason for the draw (`"rollout_cohort"`,
        /// `"retry_jitter"`, etc.). Useful for debugging a
        /// divergent replay — the kind names tell you which draw
        /// went missing.
        purpose: String,
        /// Raw PRNG output, u64. Consumers that need a different
        /// shape (bool, f64, range) re-derive from this via the
        /// same transformation used at record time.
        value: u64,
    },
    /// A read from a clock source. Every such read must be
    /// replayable for `@replayable` agents to compile.
    ClockRead {
        ts_ms: u64,
        run_id: String,
        /// `"wall"` (epoch-ms), `"monotonic"` (ns since process
        /// start), or `"system_start"` (epoch-ms of process boot).
        /// Named `source` not `kind` to avoid colliding with the
        /// enum's serde tag field.
        source: String,
        /// Raw clock value. Units depend on `source`; the
        /// consumer is responsible for matching units on replay.
        value: i64,
    },
    ModelSelected {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        model: String,
        #[serde(default)]
        model_version: Option<String>,
        #[serde(default)]
        capability_required: Option<String>,
        #[serde(default)]
        capability_picked: Option<String>,
        #[serde(default)]
        output_format_required: Option<String>,
        #[serde(default)]
        output_format_picked: Option<String>,
        cost_estimate: f64,
        #[serde(default)]
        arm_index: Option<usize>,
        #[serde(default)]
        stage_index: Option<usize>,
    },
    ProgressiveEscalation {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        from_stage: usize,
        to_stage: usize,
        confidence_observed: f64,
        threshold: f64,
    },
    ProgressiveExhausted {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        stages: Vec<String>,
    },
    StreamUpgrade {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        to_model: String,
        confidence_observed: f64,
        threshold: f64,
        partial: serde_json::Value,
    },
    AbVariantChosen {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        variant: String,
        baseline: String,
        rollout_pct: f64,
        chosen: String,
    },
    EnsembleVote {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        members: Vec<String>,
        results: Vec<String>,
        winner: String,
        agreement_rate: f64,
        strategy: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weights: Option<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        escalated_to: Option<String>,
    },
    AdversarialPipelineCompleted {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        contradiction: bool,
    },
    AdversarialContradiction {
        ts_ms: u64,
        run_id: String,
        prompt: String,
        proposed: String,
        challenge: String,
        verdict: serde_json::Value,
    },
    /// A provenance edge in the run's Grounded<T> dataflow graph.
    /// Emitted whenever the runtime constructs a Grounded<T> value
    /// from one or more upstream Grounded<T> inputs — lets
    /// `corvid trace dag` render the exact dataflow a posteriori
    /// without the renderer needing to understand runtime internals.
    ///
    /// Additive in schema v2 — old readers (v1 + v2-pre-provenance)
    /// skip unknown `kind` values, so no version bump required.
    ProvenanceEdge {
        ts_ms: u64,
        run_id: String,
        /// Stable identifier for the value this edge produces.
        /// Must be unique within a run. Recorder convention: a
        /// monotonic counter prefixed by the op kind
        /// (`"tool:17"`, `"llm:4"`, `"approve:2"`) so the same
        /// value has the same id across record + replay.
        node_id: String,
        /// Upstream `node_id`s whose values flowed into this one.
        /// Empty for root inputs (tool results with no
        /// Grounded<T> arguments, LLM calls with no grounded
        /// prompt parts).
        #[serde(default)]
        parents: Vec<String>,
        /// Operation that produced this value, in
        /// `<kind>:<name>` form. Examples: `"tool_call:get_order"`,
        /// `"llm:classify"`, `"approve:IssueRefund"`,
        /// `"literal:42"`.
        op: String,
        /// Optional human-readable label for the DAG renderer.
        /// `None` when the node_id is already self-describing.
        #[serde(default)]
        label: Option<String>,
    },
}

impl TraceEvent {
    /// The run this event belongs to. Every event carries one; readers ask
    /// here rather than matching each variant, so a new event kind is
    /// read everywhere once it is added to these two matches.
    pub fn run_id(&self) -> &str {
        match self {
            Self::SchemaHeader { run_id, .. }
            | Self::RunStarted { run_id, .. }
            | Self::RunCompleted { run_id, .. }
            | Self::ToolCall { run_id, .. }
            | Self::ToolResult { run_id, .. }
            | Self::LlmCall { run_id, .. }
            | Self::LlmResult { run_id, .. }
            | Self::PromptCache { run_id, .. }
            | Self::ApprovalRequest { run_id, .. }
            | Self::ApprovalDecision { run_id, .. }
            | Self::ApprovalResponse { run_id, .. }
            | Self::ApprovalTokenIssued { run_id, .. }
            | Self::ApprovalScopeViolation { run_id, .. }
            | Self::HumanInputRequest { run_id, .. }
            | Self::HumanInputResponse { run_id, .. }
            | Self::HumanChoiceRequest { run_id, .. }
            | Self::HumanChoiceResponse { run_id, .. }
            | Self::HostEvent { run_id, .. }
            | Self::HttpExchange { run_id, .. }
            | Self::DbTiming { run_id, .. }
            | Self::AgentStarted { run_id, .. }
            | Self::AgentFinished { run_id, .. }
            | Self::SeedRead { run_id, .. }
            | Self::ClockRead { run_id, .. }
            | Self::ModelSelected { run_id, .. }
            | Self::ProgressiveEscalation { run_id, .. }
            | Self::ProgressiveExhausted { run_id, .. }
            | Self::StreamUpgrade { run_id, .. }
            | Self::AbVariantChosen { run_id, .. }
            | Self::EnsembleVote { run_id, .. }
            | Self::AdversarialPipelineCompleted { run_id, .. }
            | Self::AdversarialContradiction { run_id, .. }
            | Self::ProvenanceEdge { run_id, .. } => run_id,
        }
    }

    /// When this event happened, in milliseconds since the Unix epoch.
    pub fn ts_ms(&self) -> u64 {
        match self {
            Self::SchemaHeader { ts_ms, .. }
            | Self::RunStarted { ts_ms, .. }
            | Self::RunCompleted { ts_ms, .. }
            | Self::ToolCall { ts_ms, .. }
            | Self::ToolResult { ts_ms, .. }
            | Self::LlmCall { ts_ms, .. }
            | Self::LlmResult { ts_ms, .. }
            | Self::PromptCache { ts_ms, .. }
            | Self::ApprovalRequest { ts_ms, .. }
            | Self::ApprovalDecision { ts_ms, .. }
            | Self::ApprovalResponse { ts_ms, .. }
            | Self::ApprovalTokenIssued { ts_ms, .. }
            | Self::ApprovalScopeViolation { ts_ms, .. }
            | Self::HumanInputRequest { ts_ms, .. }
            | Self::HumanInputResponse { ts_ms, .. }
            | Self::HumanChoiceRequest { ts_ms, .. }
            | Self::HumanChoiceResponse { ts_ms, .. }
            | Self::HostEvent { ts_ms, .. }
            | Self::HttpExchange { ts_ms, .. }
            | Self::DbTiming { ts_ms, .. }
            | Self::AgentStarted { ts_ms, .. }
            | Self::AgentFinished { ts_ms, .. }
            | Self::SeedRead { ts_ms, .. }
            | Self::ClockRead { ts_ms, .. }
            | Self::ModelSelected { ts_ms, .. }
            | Self::ProgressiveEscalation { ts_ms, .. }
            | Self::ProgressiveExhausted { ts_ms, .. }
            | Self::StreamUpgrade { ts_ms, .. }
            | Self::AbVariantChosen { ts_ms, .. }
            | Self::EnsembleVote { ts_ms, .. }
            | Self::AdversarialPipelineCompleted { ts_ms, .. }
            | Self::AdversarialContradiction { ts_ms, .. }
            | Self::ProvenanceEdge { ts_ms, .. } => *ts_ms,
        }
    }
}


#[cfg(test)]
mod accessor_tests {
    use super::TraceEvent;

    /// The two fields every event carries, read without matching its kind.
    #[test]
    fn every_event_reads_its_run_and_time() {
        let event = TraceEvent::SeedRead { ts_ms: 42, run_id: "run-7".into(), purpose: "p".into(), value: 1 };
        assert_eq!((event.run_id(), event.ts_ms()), ("run-7", 42));
    }
}
