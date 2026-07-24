//! IR node types.
//!
//! A flatter, normalized form of the typed AST. References are already
//! resolved to `DefId`/`LocalId`; every expression carries its `Type`.

use corvid_ast::{
    Backoff, BackpressurePolicy, BinaryOp, Effect, HttpMethod, RouteResponseKind, Span, UnaryOp,
};
use corvid_resolve::{DefId, LocalId};
use corvid_types::Type;

/// A full `.cor` file in IR form.
#[derive(Debug, Clone)]
pub struct IrFile {
    pub imports: Vec<IrImport>,
    pub types: Vec<IrType>,
    pub tools: Vec<IrTool>,
    pub prompts: Vec<IrPrompt>,
    pub agents: Vec<IrAgent>,
    pub evals: Vec<IrEval>,
    pub tests: Vec<IrTest>,
    pub fixtures: Vec<IrFixture>,
    pub mocks: Vec<IrMock>,
    /// `server` blocks lowered to IR (Phase 35V2-P42-E0). Each route
    /// carries its method/path/types and the lowered handler body, so
    /// the HTTP-serve layer can register routes and dispatch to the
    /// handler agent.
    pub servers: Vec<IrServer>,
    /// `model` declarations' runtime-relevant fields (slice 46a).
    pub models: Vec<IrModel>,
    /// `connector` blocks lowered to IR (slice 52g-3). Each operation
    /// also lowers to an `IrTool` in `tools` (so it is callable and
    /// typed exactly like a tool); the connector entry here carries the
    /// declarative HTTP dispatch metadata (base URL, credentials,
    /// method/path/body, status→error map, reliability) the runtime
    /// needs to build a `ConnectorRequest` when one of those tools is
    /// called.
    pub connectors: Vec<IrConnector>,
}

/// A `connector` block lowered to IR (slice 52g-3). Its operations
/// are lowered into `IrFile::tools` as ordinary callable tools; the
/// dispatch metadata that turns a tool call into an HTTP request lives
/// here, keyed back to the tool by `IrOperation::tool_id`.
#[derive(Debug, Clone)]
pub struct IrConnector {
    pub name: String,
    /// Absolute `http(s)://` base URL. Operation paths are appended.
    pub base_url: String,
    /// Credential material, always as `secret(...)` reference NAMES —
    /// never literal values (the parser rejects literals). Resolved to
    /// live secrets by the runtime at dispatch, never embedded in a
    /// trace.
    pub auth: Option<IrConnectorAuth>,
    /// Retry attempts (slice 52g-4 reliability). `None` = no retry.
    pub retry: Option<u64>,
    /// Token-bucket rate limit (slice 52g-4). `None` = unlimited.
    pub rate_limit: Option<IrRateLimit>,
    /// Circuit-breaker consecutive-failure threshold (slice 52g-4).
    pub circuit_breaker: Option<u64>,
    /// The execution modes this connector is allowed to run in (slice
    /// 52g-3b). Never empty (the checker rejects an undeclared set).
    /// The deployment selects exactly one at start.
    pub modes: Vec<corvid_ast::ConnectorMode>,
    pub operations: Vec<IrOperation>,
    pub span: Span,
}

/// Connector credentials — every field is a `secret(...)` reference
/// NAME, resolved to a live value only at dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrConnectorAuth {
    Bearer { secret: String },
    Header { name: String, secret: String },
    Basic { username_secret: String, password_secret: String },
}

/// A connector's token-bucket rate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrRateLimit {
    pub limit: u64,
    pub window_secs: u64,
}

/// One connector `operation` lowered to dispatch metadata. The
/// operation is ALSO an `IrTool` (looked up by `tool_id`); this record
/// says how to turn a call to that tool into a `ConnectorRequest`.
#[derive(Debug, Clone)]
pub struct IrOperation {
    pub name: String,
    /// DefId of the `IrTool` this operation lowered to.
    pub tool_id: DefId,
    pub method: HttpMethod,
    /// Path template appended to the connector base URL. `{name}`
    /// placeholders bind from the operation's params by name.
    pub path: String,
    /// The request body: which param supplies it and how it encodes.
    /// `None` = no body (e.g. a GET).
    pub body: Option<IrOperationBody>,
    /// `on STATUS -> Variant` map: an HTTP status becomes a typed error
    /// variant instead of a transport failure (slice 52g-4).
    pub error_map: Vec<IrStatusErrorMapping>,
    /// The `mock:` payload expression, lowered (slice 52g-3b). In
    /// `mock` mode the runtime evaluates this to produce the operation
    /// response. `None` when the connector's allowed modes exclude
    /// `mock` (the checker requires one whenever `mock` is allowed).
    pub mock: Option<IrExpr>,
    /// The exact temporal graph accepted by the checker.
    pub protocol: Option<corvid_ast::ProviderProtocolDecl>,
    pub span: Span,
}

/// Which param supplies an operation's request body, and its encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrOperationBody {
    pub param_name: String,
    pub encoding: corvid_ast::BodyEncoding,
}

/// `on STATUS -> Variant` — maps an HTTP status code to a typed error
/// variant name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrStatusErrorMapping {
    pub status: u16,
    pub variant: String,
}

/// A `server` block lowered to IR.
#[derive(Debug, Clone)]
pub struct IrServer {
    pub id: DefId,
    pub name: String,
    pub routes: Vec<IrRoute>,
    pub span: Span,
}

/// One `route METHOD "path" ... -> RESPONSE: BODY` entry.
#[derive(Debug, Clone)]
pub struct IrRoute {
    pub method: HttpMethod,
    pub path: String,
    pub path_params: Vec<IrRoutePathParam>,
    /// Resolved type of the `query` binding, when the route declares one.
    pub query_ty: Option<Type>,
    /// Resolved type of the `body` binding, when the route declares one.
    pub body_ty: Option<Type>,
    pub response_kind: RouteResponseKind,
    pub response_ty: Type,
    /// Names of the effects the route's handler declares (`uses ...`).
    pub effect_names: Vec<String>,
    /// The lowered handler body. The `path`/`query`/`body` bindings the
    /// resolver introduced are referenced by `IrExprKind::Local` inside
    /// this block.
    pub body: IrBlock,
    /// Name of the synthetic per-route handler agent (slice 52a) that
    /// `lower_file` appends to `ir.agents`. Its params reuse the
    /// route's `path`/`query`/`body`/`actor` `LocalId`s, so `corvid
    /// serve` executes any route body by invoking this agent by name
    /// through the ordinary agent machinery.
    pub handler_agent: String,
    /// The explicit source-declared boundary policy for a direct
    /// `Upload<Format>` body. The checker guarantees this is present
    /// and carries a maximum size whenever `upload_format` is present.
    pub upload_policy: Option<corvid_ast::UploadSpec>,
    /// Complete source-declared queue/reviewer policy for approval
    /// boundaries reachable from this route.
    pub approval_policy: Option<corvid_ast::ApprovalSpec>,
    /// The `Upload<Format>` format tag (`Csv`, `Pdf`, `Image`, …) when
    /// the route body is an upload (slice 52c-2). `corvid serve` uses
    /// it to enforce the accepted MIME set on the multipart part. The
    /// resolved inner `Type` loses the tag (the format name is not a
    /// declared type), so it is carried here from the AST.
    pub upload_format: Option<String>,
    /// The route's `requires` authorization policy (slice 52f), lowered
    /// from the AST so `corvid serve` can enforce it before the handler
    /// runs. `None` = a public route.
    pub policy: Option<IrRoutePolicy>,
    pub span: Span,
}

/// A route's `requires authenticated|role|permission` clause, lowered
/// into the IR (slice 52f). All listed roles AND all listed permissions
/// must be satisfied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrRoutePolicy {
    pub authenticated: bool,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
}

impl IrRoutePolicy {
    /// Whether the route requires authentication at all — an explicit
    /// `authenticated`, or any role/permission requirement (which implies
    /// it).
    pub fn requires_auth(&self) -> bool {
        self.authenticated || !self.roles.is_empty() || !self.permissions.is_empty()
    }
}

/// A typed `{name}` path parameter on a route.
#[derive(Debug, Clone)]
pub struct IrRoutePathParam {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// `import python "..." as alias`.
#[derive(Debug, Clone)]
pub struct IrImport {
    pub id: DefId,
    pub source: IrImportSource,
    pub module: String,
    pub content_hash: Option<IrImportContentHash>,
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IrImportContentHash {
    pub algorithm: String,
    pub hex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrImportSource {
    Python,
    Corvid,
    RemoteCorvid,
    PackageCorvid,
}

/// A user-declared struct.
#[derive(Debug, Clone)]
pub struct IrType {
    pub id: DefId,
    pub name: String,
    pub fields: Vec<IrField>,
    /// Sum-type variants (slice 45h); empty for record types. The
    /// declared field names/types are 45i pattern + exhaustiveness
    /// metadata; runtime payloads are positional.
    pub variants: Vec<IrEnumVariant>,
    pub span: Span,
}

/// One lowered sum-type variant (slice 45h).
#[derive(Debug, Clone)]
pub struct IrEnumVariant {
    pub name: String,
    pub fields: Vec<IrField>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IrField {
    pub name: String,
    pub ty: Type,
    /// Value refinement (slice 50j) — enforced at typed decode; the
    /// violation message feeds the structured-output repair loop.
    pub refinement: Option<corvid_ast::Refinement>,
    pub span: Span,
}

/// A tool declaration (no body — externally implemented).
#[derive(Debug, Clone)]
pub struct IrTool {
    /// Circuit breaker threshold (slice 50k).
    pub breaker: Option<u64>,
    pub id: DefId,
    pub name: String,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub effect: Effect,
    pub effect_names: Vec<String>,
    /// If any declared effect has `trust: autonomous_if_confident(T)`,
    /// this carries the confidence threshold. At runtime, the
    /// interpreter checks composed input confidence and activates the
    /// approval gate if confidence is below this threshold.
    pub confidence_gate: Option<f64>,
    /// Provenance Propagation (slice 7b): true when any of this
    /// tool's declared effects carries `data: grounded` (or is the
    /// built-in `retrieval` effect). Mirrors the typechecker's
    /// Design X return-type wrapping: if this is set, the runtime
    /// MUST wrap the tool's result in `Value::Grounded` so the
    /// value-level invariant matches the type-level invariant. The
    /// IR computes this once at lower time from the effect registry
    /// so the interpreter doesn't need to consult the registry.
    pub produces_grounded: bool,
    /// The tool's composed worst-case cost (the `cost` dimension of its
    /// effect row), pre-computed at lower time from the effect registry
    /// (slice 52d-1). Used by effect-aware `parallel` scheduling to
    /// compute a block's combined cost without consulting the registry
    /// at runtime. Mirrors `produces_grounded`.
    pub effect_cost: f64,
    /// True unless the tool's composed effect row is `reversible:
    /// false` (slice 52d-1). Composed via `LeastReversible` — one
    /// irreversible effect makes the tool irreversible. The
    /// cancellation×reversibility rule (52d-2) reads this at the
    /// tool-execution site: an arm that has called an irreversible tool
    /// is past a non-reversible boundary and must not be cancelled.
    pub effect_reversible: bool,
    pub span: Span,
}

/// One arm of a `parallel:` block (slice 46e).
#[derive(Debug, Clone)]
pub struct IrParallelArm {
    pub name: String,
    pub local_id: LocalId,
    pub call: IrExpr,
    pub span: Span,
}

/// One role block of a multi-message prompt (slice 46b).
#[derive(Debug, Clone, PartialEq)]
pub struct IrPromptMessage {
    pub role: String,
    pub template: String,
}

/// A `model` declaration's runtime-relevant fields (slice 46a).
/// Until 46a, model decls were a static checker-side catalog with
/// no lowering; sampling made them load-bearing at dispatch: the
/// VM resolves `prompt override > model field > adapter default`.
#[derive(Debug, Clone, PartialEq)]
pub struct IrModel {
    pub name: String,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<u64>,
    /// Declared context window (slice 46c). Drives deterministic
    /// oldest-first history truncation at dispatch.
    pub context_window: Option<u64>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IrPrompt {
    pub id: DefId,
    pub name: String,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub template: String,
    /// Multi-message role blocks (slice 46b). Empty = the classic
    /// single-template form.
    pub messages: Vec<IrPromptMessage>,
    /// Conversation history (slice 46c): index of the parameter
    /// typed `List<AiMessage>`. Its messages splice after the
    /// declaration's system blocks and before the current turn.
    pub history_param: Option<usize>,
    pub effect_names: Vec<String>,
    pub effect_cost: f64,
    pub effect_confidence: f64,
    /// Provenance Propagation (slice 7b): true when any of this
    /// prompt's declared effects carries `data: grounded`. The
    /// runtime wraps the prompt's result in `Value::Grounded` when
    /// set, matching the typechecker's Design X return-type
    /// wrapping. Computed once at lower time from the effect
    /// registry.
    pub produces_grounded: bool,
    /// Index of the parameter whose content must appear in the LLM response.
    /// Set when the prompt declares `cites <param> strictly`.
    pub cites_strictly_param: Option<usize>,
    /// Stream-only prompt modifiers preserved for the interpreter tier.
    pub min_confidence: Option<f64>,
    pub max_tokens: Option<u64>,
    /// Per-prompt sampling overrides (slice 46a): beat the model
    /// declaration's fields; `None` falls through.
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    /// Structured-output auto-repair attempts (slice 46h).
    pub repair_attempts: Option<u64>,
    /// Judged output guard (slice 50l): (criteria, min score).
    pub judged_guard: Option<(String, f64)>,
    pub backpressure: Option<BackpressurePolicy>,
    pub escalate_to: Option<String>,
    /// Runtime calibration flag. When true, prompt calls record
    /// confidence-vs-accuracy observations if the adapter supplies
    /// correctness metadata.
    pub calibrated: bool,
    /// Runtime prompt-response cache opt-in. Cache identity includes
    /// selected model, rendered prompt, JSON arguments, and output schema.
    pub cacheable: bool,
    /// Phase 20h: minimum model capability this prompt requires
    /// (`basic` | `standard` | `expert` | custom). The runtime
    /// uses this to pick the cheapest declared model whose own
    /// `capability` field satisfies the requirement. `None` means
    /// the prompt uses the default-capability model (first in the
    /// catalog, or the `default_model`-backed pipeline that shipped
    /// before the model substrate existed).
    pub capability_required: Option<String>,
    /// Required model output format (`strict_json`,
    /// `markdown_strict`, etc.). Runtime selection uses this as a
    /// hard eligibility filter.
    pub output_format_required: Option<String>,
    /// Phase 20h slice C: pattern-dispatched per-call model
    /// selection. Empty `arms` means the prompt uses the standard
    /// capability-based dispatch (slice B). Non-empty means the
    /// runtime evaluates each arm's guard in order and dispatches
    /// to the first match's model.
    pub route: Vec<IrRouteArm>,
    /// Phase 20h slice E: progressive refinement chain. Empty
    /// means the prompt doesn't use progressive dispatch. Non-empty
    /// means the runtime runs stages in order; each non-final
    /// stage's `threshold` is the minimum output confidence at
    /// which to accept the stage's result. If a stage's output is
    /// below its threshold, the runtime escalates to the next
    /// stage. The final stage has `threshold = None` and always
    /// runs as the terminal fallback.
    pub progressive: Vec<IrProgressiveStage>,
    /// Phase 20h slice I: A/B rollout. `None` means no rollout
    /// is configured. `Some(spec)` routes a fraction of calls to
    /// `spec.variant_def_id` and the rest to `spec.baseline_def_id`.
    /// Runtime chooses per-call (deterministic or random — that's
    /// Dev B's C-rt cohort decision).
    pub rollout: Option<IrRolloutSpec>,
    /// Phase 20h slice F: concurrent voting across multiple models.
    /// `None` means no ensemble. `Some(spec)` means the runtime
    /// dispatches to every model in `spec.models` concurrently and
    /// applies `spec.vote` to pick the winner.
    pub ensemble: Option<IrEnsembleSpec>,
    /// Phase 20h slice G: three-stage propose / challenge /
    /// adjudicate pipeline. Runtime dispatches sequentially —
    /// adjudicator's output is the prompt's result. Prior stages'
    /// outputs are available as reserved template variables.
    pub adversarial: Option<IrAdversarialSpec>,
    pub span: Span,
}

/// One arm of a prompt's `route:` clause at IR level.
#[derive(Debug, Clone)]
pub struct IrRouteArm {
    pub pattern: IrRoutePattern,
    /// DefId of the target `model` declaration.
    pub model_def_id: DefId,
    pub model_name: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum IrRoutePattern {
    Wildcard,
    Guard(IrExpr),
}

/// One stage of a prompt's `progressive:` chain at IR level.
/// `threshold = None` marks the terminal fallback (always runs).
#[derive(Debug, Clone)]
pub struct IrProgressiveStage {
    pub model_def_id: DefId,
    pub model_name: String,
    pub threshold: Option<f64>,
    pub span: Span,
}

/// Lowered A/B rollout spec.
#[derive(Debug, Clone)]
pub struct IrRolloutSpec {
    /// Percentage of calls routed to the variant (0.0 – 100.0).
    pub variant_percent: f64,
    pub variant_def_id: DefId,
    pub variant_name: String,
    pub baseline_def_id: DefId,
    pub baseline_name: String,
    pub span: Span,
}

/// Lowered ensemble voting spec.
#[derive(Debug, Clone)]
pub struct IrEnsembleSpec {
    /// Models to dispatch to concurrently. Runtime fires them via
    /// `tokio::join!` and applies the vote strategy to the results.
    pub models: Vec<IrEnsembleMember>,
    pub vote: IrVoteStrategy,
    pub weighting: Option<IrEnsembleWeighting>,
    pub disagreement_escalation: Option<IrEnsembleMember>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IrEnsembleMember {
    pub def_id: DefId,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrVoteStrategy {
    Majority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrEnsembleWeighting {
    AccuracyHistory,
}

/// Lowered adversarial pipeline. Runtime runs proposer →
/// challenger → adjudicator; the adjudicator's output is returned.
#[derive(Debug, Clone)]
pub struct IrAdversarialSpec {
    pub proposer_def_id: DefId,
    pub proposer_name: String,
    pub challenger_def_id: DefId,
    pub challenger_name: String,
    pub adjudicator_def_id: DefId,
    pub adjudicator_name: String,
    pub span: Span,
}

/// An agent declaration with a typed body.
#[derive(Debug, Clone)]
pub struct IrAgent {
    pub id: DefId,
    pub name: String,
    pub extern_abi: Option<IrExternAbi>,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub cost_budget: Option<f64>,
    pub wrapping_arithmetic: bool,
    /// True when the source carries `@replayable` or `@deterministic`
    /// (`@deterministic` implies `@replayable`). Lowered from
    /// `AgentAttribute::is_replayable(&attributes)`. Consumed by the
    /// durable-job executor (slice `35V2-P38-C-2`) to gate per-job
    /// trace emission, and by future replay-mode wiring (slice
    /// `35V2-P38-C-3` onward) to decide whether a job is safely
    /// replayable.
    pub is_replayable: bool,
    /// True when this entry lowered from a `fn` pure-function
    /// declaration (slice 45r): the checker proved the body
    /// effect-free, so every tier may treat calls as direct,
    /// trace-free invocations. `fn`s share the agent IR so no
    /// execution tier needs new machinery.
    #[allow(dead_code)]
    pub pure_fn: bool,
    /// `@retry(max_attempts: N, ...)` (slice 45q) — the agent-side
    /// default retry policy for durable jobs executing this agent.
    /// Enqueue-time values take precedence.
    pub retry_max_attempts: Option<u64>,
    /// Backoff from `@retry(..., backoff: ...)`: (exponential?, ms).
    pub retry_backoff_ms: Option<(bool, u64)>,
    /// `@idempotency(key: param)` (slice 45q) — the parameter whose
    /// value derives the durable job's idempotency key.
    pub idempotency_key_param: Option<String>,
    pub body: IrBlock,
    pub span: Span,
    /// Per-parameter ownership at the callee ABI.
    /// `None` = ownership analysis hasn't run on this agent (every
    /// parameter is treated as Owned, matching pre-17b semantics).
    /// `Some(v)` with `v.len() == params.len()` — each entry matches
    /// the parameter at the same index.
    ///
    /// Populated by `corvid-codegen-cl`'s ownership pass after IR
    /// lowering and before Cranelift codegen. The interpreter tier
    /// (`corvid-vm`) ignores this field — refcount there is via `Arc`
    /// and has no ABI distinction.
    pub borrow_sig: Option<Vec<ParamBorrow>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrExternAbi {
    C,
}

/// An eval declaration lowered into IR.
#[derive(Debug, Clone)]
pub struct IrEval {
    pub id: DefId,
    pub name: String,
    pub body: IrBlock,
    pub assertions: Vec<IrEvalAssert>,
    pub span: Span,
}

/// A test declaration lowered into IR. The runner lands in Phase 26 after the
/// compiler can already preserve test bodies and assertion metadata.
#[derive(Debug, Clone)]
pub struct IrTest {
    pub id: DefId,
    pub name: String,
    pub trace_fixture: Option<String>,
    pub body: IrBlock,
    pub assertions: Vec<IrEvalAssert>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IrFixture {
    pub id: DefId,
    pub name: String,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub body: IrBlock,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IrMock {
    pub target_id: DefId,
    pub target_name: String,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub body: IrBlock,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum IrEvalAssert {
    Value {
        expr: IrExpr,
        confidence: Option<f64>,
        runs: Option<u64>,
        span: Span,
    },
    Snapshot {
        expr: IrExpr,
        span: Span,
    },
    Called {
        def_id: DefId,
        name: String,
        span: Span,
    },
    Approved {
        label: String,
        span: Span,
    },
    Cost {
        op: BinaryOp,
        bound: f64,
        span: Span,
    },
    Similar {
        expr: IrExpr,
        expected: IrExpr,
        min: f64,
        span: Span,
    },
    Judged {
        expr: IrExpr,
        criteria: String,
        min: f64,
        span: Span,
    },
    Ordering {
        before_id: DefId,
        before_name: String,
        after_id: DefId,
        after_name: String,
        span: Span,
    },
}

/// Callee-side ABI for a refcounted parameter. Non-refcounted params
/// (Int, Float, Bool) have no RC ABI decision — this enum describes
/// them as `Owned` trivially (no retain/release either way).
///
/// Defined in corvid-ir rather than corvid-codegen-cl so the
/// interpreter crate can see it (and explicitly ignore it) without a
/// cross-crate cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamBorrow {
    /// Caller transfers a +1 on the argument; callee is responsible
    /// for eventual drop. Matches pre-17b behavior for all parameters.
    Owned,
    /// Caller does not transfer a +1; callee must NOT drop and must
    /// emit `Dup` locally before storing the value into a long-lived
    /// location or returning it. Saves one retain at the caller + one
    /// release at the callee when the body is read-only on the param.
    Borrowed,
}

#[derive(Debug, Clone)]
pub struct IrParam {
    pub name: String,
    pub local_id: LocalId,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IrBlock {
    pub stmts: Vec<IrStmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum IrStmt {
    /// `x = expr` — binds `local_id` to the value of `value`.
    Let {
        local_id: LocalId,
        name: String,
        ty: Type,
        value: IrExpr,
        span: Span,
    },

    /// `return expr?`
    Return {
        value: Option<IrExpr>,
        span: Span,
    },

    /// `yield expr`
    Yield {
        value: IrExpr,
        span: Span,
    },

    /// `if cond: then else else_`
    If {
        cond: IrExpr,
        then_block: IrBlock,
        else_block: Option<IrBlock>,
        span: Span,
    },

    /// `for var in iter: body`
    For {
        var_local: LocalId,
        var_name: String,
        iter: IrExpr,
        body: IrBlock,
        span: Span,
    },

    /// `parallel:` block (slice 46e): named arms run concurrently;
    /// arm trace buffers flush in ARM ORDER at the join; the error
    /// rule is arm-ordered. Interpreter-only in v1.
    Parallel {
        arms: Vec<IrParallelArm>,
        span: Span,
    },

    /// Destructuring binding (slice 45n): `Decision { refund, .. }
    /// = value`. The pattern is IRREFUTABLE (checker-enforced);
    /// the interpreter evaluates the value once and binds every
    /// pattern binding through the 45i pattern machinery.
    Destructure {
        pattern: IrPattern,
        value: IrExpr,
        span: Span,
    },

    /// `while cond: body` (slice 45k). The condition re-evaluates
    /// before every iteration.
    While {
        cond: IrExpr,
        body: IrBlock,
        span: Span,
    },

    /// `approve Label(args)` — authorizes matching dangerous tool calls.
    Approve {
        label: String,
        args: Vec<IrExpr>,
        span: Span,
    },

    /// Expression evaluated for side effects.
    Expr {
        expr: IrExpr,
        span: Span,
    },

    /// Place assignment (slice 45b): `x.field = v`, `xs[i] = v`, and
    /// compound `target op= value`. `local_id` is the ROOT local the
    /// path starts from; an empty `path` means the statement rebinds
    /// or compound-updates the local itself (`x += 1`). The compound
    /// operator lives here (not desugared) so index expressions in
    /// the path evaluate exactly once.
    Assign {
        local_id: LocalId,
        name: String,
        path: Vec<IrPathSeg>,
        op: Option<BinaryOp>,
        value: IrExpr,
        span: Span,
    },

    /// `break`, `continue`, `pass` — dedicated IR variants.
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Pass {
        span: Span,
    },

    /// Increment a refcounted local's refcount.
    /// Inserted by the ownership analysis pass at non-final uses of a
    /// binding. Codegen lowers this as a single `corvid_retain` call.
    /// The interpreter ignores it (Arc handles refcount implicitly).
    ///
    /// `Dup` on a non-refcounted local is a no-op — the analysis pass
    /// emits it only for refcounted types, but the codegen double-
    /// checks via the local's declared type before emitting.
    Dup {
        local_id: LocalId,
        span: Span,
    },

    /// Release a refcounted local's refcount.
    /// Inserted at final use (unless the use is a consume/move) or at
    /// scope exit for any still-owned bindings. Codegen lowers this as
    /// a single `corvid_release` call. The interpreter ignores it.
    Drop {
        local_id: LocalId,
        span: Span,
    },
}

/// One lambda parameter (slice 45j): the resolver-assigned local
/// slot the argument binds to when the closure is applied.
#[derive(Debug, Clone)]
pub struct IrLambdaParam {
    pub local_id: LocalId,
    pub name: String,
}

/// One arm of a lowered `match` (slice 45i).
#[derive(Debug, Clone)]
pub struct IrMatchArm {
    pub pattern: IrPattern,
    pub guard: Option<IrExpr>,
    pub body: IrExpr,
    pub span: Span,
}

/// A lowered `match` pattern (slice 45i). Bindings carry resolved
/// `LocalId`s; variant patterns carry the owning type's lowered
/// `DefId` + variant index; builtin patterns cover Option/Result.
#[derive(Debug, Clone)]
pub enum IrPattern {
    Wildcard,
    Literal(IrLiteral),
    Bind {
        local_id: LocalId,
        name: String,
    },
    At {
        local_id: LocalId,
        name: String,
        inner: Box<IrPattern>,
    },
    Variant {
        owner: DefId,
        variant_index: u32,
        variant_name: String,
        args: Vec<IrPattern>,
    },
    Some_(Box<IrPattern>),
    None_,
    Ok_(Box<IrPattern>),
    Err_(Box<IrPattern>),
    Record {
        /// (field name, subpattern). Shorthand fields lower to a
        /// `Bind` subpattern.
        fields: Vec<(String, IrPattern)>,
    },
}

/// One segment of a place-assignment path (slice 45b).
#[derive(Debug, Clone)]
pub enum IrPathSeg {
    /// `.field` on a struct value.
    Field(String),
    /// `[index]` on a list value; the index expression is lowered IR.
    Index(IrExpr),
}

#[derive(Debug, Clone)]
pub struct IrExpr {
    pub kind: IrExprKind,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum IrExprKind {
    /// A literal value.
    Literal(IrLiteral),

    /// Reference to a parameter or local binding.
    Local {
        local_id: LocalId,
        name: String,
    },

    /// Reference to a top-level declaration (imports only in v0.1).
    Decl {
        def_id: DefId,
        name: String,
    },

    /// `tool_or_agent_or_prompt(args)` — resolved to a specific declaration.
    Call {
        kind: IrCallKind,
        callee_name: String,
        args: Vec<IrExpr>,
    },

    /// A builtin method on a built-in receiver type (slice 45c).
    /// The kind comes from `corvid_types::BuiltinMethodKind` — the
    /// shared table that also drives the checker; the interpreter
    /// executes one arm per kind.
    BuiltinMethod {
        kind: corvid_types::BuiltinMethodKind,
        receiver: Box<IrExpr>,
        args: Vec<IrExpr>,
    },

    FieldAccess {
        target: Box<IrExpr>,
        field: String,
    },

    Index {
        target: Box<IrExpr>,
        index: Box<IrExpr>,
    },

    BinOp {
        op: BinaryOp,
        left: Box<IrExpr>,
        right: Box<IrExpr>,
    },

    /// Integer arithmetic under an enclosing `@wrapping` agent.
    /// Defaults remain checked; this node makes the opt-out explicit
    /// for every runtime/codegen tier.
    WrappingBinOp {
        op: BinaryOp,
        left: Box<IrExpr>,
        right: Box<IrExpr>,
    },

    UnOp {
        op: UnaryOp,
        operand: Box<IrExpr>,
    },

    /// Integer unary operations under an enclosing `@wrapping` agent.
    WrappingUnOp {
        op: UnaryOp,
        operand: Box<IrExpr>,
    },

    /// `match` expression (slice 45i).
    Match {
        scrutinee: Box<IrExpr>,
        arms: Vec<IrMatchArm>,
    },

    /// Named struct literal (slice 45n): `Decision { refund: true,
    /// amount, ..base }` lowered with fields in SOURCE order plus an
    /// optional spread. The interpreter builds the new cell from the
    /// spread's field values first (handle copies — a NEW cell whose
    /// fields share), then applies the named overrides.
    /// Interpreter-only in v1; compiled tiers degrade loudly (the
    /// positional constructor stays native).
    StructLiteral {
        def_id: DefId,
        type_name: String,
        fields: Vec<(String, IrExpr)>,
        spread: Option<Box<IrExpr>>,
    },

    /// Lambda expression (slice 45j). Evaluates to a closure value
    /// that snapshots the visible environment BY VALUE at creation
    /// (shared heap cells still share — the snapshot copies handles,
    /// not cells). Interpreter-only in v1; compiled tiers degrade
    /// loudly.
    Lambda {
        params: Vec<IrLambdaParam>,
        body: Box<IrExpr>,
    },

    /// Map literal (slice 45g): parallel key/value lowered exprs.
    MapLiteral {
        keys: Vec<IrExpr>,
        values: Vec<IrExpr>,
    },

    List {
        items: Vec<IrExpr>,
    },

    /// `grounded.unwrap_discarding_sources()` — consciously erase the
    /// provenance wrapper and keep the inner value.
    UnwrapGrounded {
        value: Box<IrExpr>,
    },

    WeakNew {
        strong: Box<IrExpr>,
    },
    WeakUpgrade {
        weak: Box<IrExpr>,
    },
    StreamSplitBy {
        stream: Box<IrExpr>,
        key: String,
    },
    StreamMerge {
        groups: Box<IrExpr>,
        policy: StreamMergePolicy,
    },
    StreamOrderedBy {
        stream: Box<IrExpr>,
        policy: StreamMergePolicy,
    },
    StreamResumeToken {
        stream: Box<IrExpr>,
    },
    ResumeStream {
        prompt_def_id: DefId,
        prompt_name: String,
        token: Box<IrExpr>,
    },
    ResultOk {
        inner: Box<IrExpr>,
    },
    ResultErr {
        inner: Box<IrExpr>,
    },
    OptionSome {
        inner: Box<IrExpr>,
    },
    OptionNone,
    Ask {
        prompt: Box<IrExpr>,
        target_ty: Type,
    },
    Choose {
        options: Box<IrExpr>,
    },
    TryPropagate {
        inner: Box<IrExpr>,
    },
    TryRetry {
        body: Box<IrExpr>,
        attempts: u64,
        backoff: Backoff,
        /// Per-attempt wall-clock bound (slice 50k).
        timeout_ms: Option<u64>,
    },
    /// `trusted(expr)` (slice 50i) — compile-time taint unwrap;
    /// runtime identity.
    TrustBoundary {
        inner: Box<IrExpr>,
    },

    /// `replay <trace>: when <pat> -> <body> else <body>` — the
    /// language-level replay primitive. Runtime semantics
    /// (21-inv-E-runtime): load the trace referenced by `trace`,
    /// walk its event stream, match each event against the arms in
    /// source order, and execute the first matching arm's body with
    /// captures bound. If no event in the trace matches any arm,
    /// execute `else_body`.
    ///
    /// Arms retain their `when` source order so runtime dispatch is
    /// unambiguous (first-match-wins). The `else_body` is separate
    /// rather than a trailing arm so codegen and the checker can
    /// both treat it as required — the grammar enforced that in
    /// 21-inv-E-1.
    Replay {
        trace: Box<IrExpr>,
        arms: Vec<IrReplayArm>,
        else_body: Box<IrExpr>,
    },

    /// `Page(items, next_cursor)` (slice 52c-2) — builds a cursor-
    /// paginated response envelope. The interpreter materialises a
    /// struct-shaped value `{ items, next_cursor, has_more }`, where
    /// `has_more` is derived from `next_cursor`'s presence, so a
    /// `Page<Item>` route serialises to the standard pagination
    /// envelope at the HTTP boundary.
    PageNew {
        items: Box<IrExpr>,
        next_cursor: Box<IrExpr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMergePolicy {
    Fifo,
    FairRoundRobin,
    Sorted,
}

impl StreamMergePolicy {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "fifo" => Some(Self::Fifo),
            "fair_round_robin" => Some(Self::FairRoundRobin),
            "sorted" => Some(Self::Sorted),
            _ => None,
        }
    }
}

/// One lowered arm of a replay block: pattern + optional
/// whole-event capture + body. Per-arg captures (tool-arg
/// identifier captures) live inside the pattern so the runtime
/// sees them alongside the literal / wildcard arg shapes.
#[derive(Debug, Clone)]
pub struct IrReplayArm {
    pub pattern: IrReplayPattern,
    /// `Some(local)` iff the arm had an `as <ident>` tail. The
    /// local's type is already in the type side-table from the
    /// checker slice (21-inv-E-3) and will be populated with the
    /// recorded event's payload value at runtime.
    pub capture: Option<IrReplayCapture>,
    pub body: Box<IrExpr>,
    pub span: Span,
}

/// A whole-event capture's runtime handle: the `LocalId` the
/// arm body reads from, plus the declared name for diagnostics.
#[derive(Debug, Clone)]
pub struct IrReplayCapture {
    pub local_id: LocalId,
    pub name: String,
    pub span: Span,
}

/// A lowered replay pattern. The string `prompt` / `tool` /
/// `label` fields are what the runtime matches against recorded
/// events' names — trace events carry strings, not DefIds.
#[derive(Debug, Clone)]
pub enum IrReplayPattern {
    Llm {
        prompt: String,
        span: Span,
    },
    Tool {
        tool: String,
        arg: IrReplayToolArgPattern,
        span: Span,
    },
    Approve {
        label: String,
        span: Span,
    },
}

impl IrReplayPattern {
    pub fn span(&self) -> Span {
        match self {
            Self::Llm { span, .. } | Self::Tool { span, .. } | Self::Approve { span, .. } => *span,
        }
    }
}

/// The three shapes a tool-arg pattern can take, one-to-one with
/// the AST forms. `Capture` carries the same `IrReplayCapture`
/// handle the whole-event capture uses, so runtime binding is
/// uniform.
#[derive(Debug, Clone)]
pub enum IrReplayToolArgPattern {
    Wildcard,
    StringLit(String),
    Capture(IrReplayCapture),
}

#[derive(Debug, Clone)]
pub enum IrLiteral {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Nothing,
}

/// What the call resolves to. Lets the codegen emit the right thing.
#[derive(Debug, Clone)]
pub enum IrCallKind {
    /// Tool call. Codegen dispatches through the runtime so effect +
    /// audit metadata can travel with the call.
    Tool {
        def_id: DefId,
        effect: Effect,
    },
    /// Prompt call. Codegen routes through the LLM runtime.
    Prompt {
        def_id: DefId,
    },
    /// Agent call — recursion or composition.
    Agent {
        def_id: DefId,
    },
    Fixture {
        def_id: DefId,
    },
    /// Struct constructor — `Order(id, amount)` builds an `Order`.
    /// Args are field values in declaration order. Codegen lowers as
    /// an allocation followed by per-field stores.
    /// Sum-variant construction (slice 45h): `Approved("alice")`.
    /// `def_id` is the OWNING sum type (lowered); the index selects
    /// the variant.
    EnumConstructor {
        def_id: DefId,
        variant_index: u32,
    },
    StructConstructor {
        def_id: DefId,
    },
    /// Call of a function-typed LOCAL (slice 45j): `f(1)` where `f`
    /// holds a closure value. The interpreter looks the local up in
    /// the environment and applies the closure.
    ClosureLocal {
        local_id: LocalId,
    },
    /// Something we couldn't resolve (graceful degradation).
    Unknown,
}
