//! Top-level declarations — what appears at the root of a `.cor` file.

use crate::effect::{BackpressurePolicy, Effect, EffectConstraint, EffectDecl, EffectRow};
use crate::expr::{BinaryOp, Expr};
use crate::span::{Ident, Span};
use crate::stmt::Block;
use crate::ty::{Field, OwnershipAnnotation, Param, TypeRef};
use serde::{Deserialize, Serialize};

/// A full `.cor` source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub decls: Vec<Decl>,
    pub span: Span,
}

/// Any top-level declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Decl {
    Import(ImportDecl),
    Type(TypeDecl),
    Store(StoreDecl),
    Tool(ToolDecl),
    Prompt(PromptDecl),
    Agent(AgentDecl),
    /// `fn name(params) -> Ty:` — pure function (slice 45r). The
    /// fourth callable kind: statically EFFECT-FREE (the checker
    /// rejects tool/prompt/agent calls and effectful builtins in
    /// the body), synchronous in semantics, callable from
    /// `@deterministic` contexts and everywhere agents can call.
    Fn(FnDecl),
    Eval(EvalDecl),
    Test(TestDecl),
    Fixture(FixtureDecl),
    Mock(MockDecl),
    /// `extend T:` block attaching methods to a user type.
    Extend(ExtendDecl),
    /// `effect Name:` dimensional effect declaration.
    Effect(EffectDecl),
    /// `model Name:` typed-model-substrate declaration (Phase 20h).
    /// A catalog entry for an LLM the project can dispatch to.
    Model(ModelDecl),
    /// `server Name:` backend route declaration.
    Server(ServerDecl),
    /// `schedule "cron" zone "Area/City" -> job(args)` durable cron trigger.
    Schedule(ScheduleDecl),
}

impl Decl {
    pub fn span(&self) -> Span {
        match self {
            Decl::Import(d) => d.span,
            Decl::Type(d) => d.span,
            Decl::Store(d) => d.span,
            Decl::Tool(d) => d.span,
            Decl::Prompt(d) => d.span,
            Decl::Agent(d) => d.span,
            Decl::Fn(d) => d.span,
            Decl::Eval(d) => d.span,
            Decl::Test(d) => d.span,
            Decl::Fixture(d) => d.span,
            Decl::Mock(d) => d.span,
            Decl::Extend(d) => d.span,
            Decl::Effect(d) => d.span,
            Decl::Model(d) => d.span,
            Decl::Server(d) => d.span,
            Decl::Schedule(d) => d.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleDecl {
    pub cron: String,
    pub zone: String,
    pub target: Ident,
    pub args: Vec<Expr>,
    #[serde(default)]
    pub effect_row: EffectRow,
    pub span: Span,
}

/// A backend server surface:
///
/// ```text
/// server refund_api:
///     route GET "/orders/{id}" -> json Order:
///         return get_order(path.id)
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerDecl {
    pub name: Ident,
    pub routes: Vec<HttpRouteDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpRouteDecl {
    pub method: HttpMethod,
    pub path: String,
    pub path_params: Vec<RoutePathParam>,
    #[serde(default)]
    pub query_ty: Option<TypeRef>,
    #[serde(default)]
    pub body_ty: Option<TypeRef>,
    pub response: RouteResponse,
    #[serde(default)]
    pub effect_row: EffectRow,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutePathParam {
    pub name: Ident,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteResponse {
    pub kind: RouteResponseKind,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteResponseKind {
    Json,
}

/// Visibility modifier on a method declared inside an `extend` block.
/// Defaults to `Private` (file-scoped). `Public` is callable anywhere
/// the type is visible, and `PublicPackage` is reserved for a future
/// package-level visibility boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    /// Default. Only visible inside the declaring file.
    #[default]
    Private,
    /// `public` — visible wherever the declaring file is imported.
    Public,
    /// `public(package)` — visible within the declaring package once
    /// package-level visibility is wired up.
    PublicPackage,
}

impl Visibility {
    pub fn is_callable_from_outside_file(&self) -> bool {
        !matches!(self, Visibility::Private)
    }
}

/// `extend T:` block. Attaches methods to an existing
/// user-declared type. The inner decls can be any of tool / prompt /
/// agent — the receiver is the first parameter of each, whose type
/// must match the extended type. The block's visibility modifiers
/// travel with each inner decl via the parallel `visibilities` vec
/// (kept parallel rather than embedded so the existing `ToolDecl` /
/// `PromptDecl` / `AgentDecl` structs don't need new fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtendDecl {
    /// Name of the type being extended.
    pub type_name: Ident,
    /// Methods declared in the block. Each entry is an ordinary
    /// tool / prompt / agent decl (reusing existing AST structures);
    /// the surrounding `ExtendDecl` is the only thing that marks
    /// them as methods rather than free-standing declarations.
    pub methods: Vec<ExtendMethod>,
    pub span: Span,
}

/// One method inside an `extend` block. The kind-specific decl lives
/// alongside its visibility modifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtendMethod {
    pub visibility: Visibility,
    pub kind: ExtendMethodKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExtendMethodKind {
    Tool(ToolDecl),
    Prompt(PromptDecl),
    Agent(AgentDecl),
}

impl ExtendMethod {
    pub fn name(&self) -> &Ident {
        match &self.kind {
            ExtendMethodKind::Tool(d) => &d.name,
            ExtendMethodKind::Prompt(d) => &d.name,
            ExtendMethodKind::Agent(d) => &d.name,
        }
    }

    pub fn span(&self) -> Span {
        match &self.kind {
            ExtendMethodKind::Tool(d) => d.span,
            ExtendMethodKind::Prompt(d) => d.span,
            ExtendMethodKind::Agent(d) => d.span,
        }
    }

    pub fn params(&self) -> &[Param] {
        match &self.kind {
            ExtendMethodKind::Tool(d) => &d.params,
            ExtendMethodKind::Prompt(d) => &d.params,
            ExtendMethodKind::Agent(d) => &d.params,
        }
    }

    pub fn return_ty(&self) -> &TypeRef {
        match &self.kind {
            ExtendMethodKind::Tool(d) => &d.return_ty,
            ExtendMethodKind::Prompt(d) => &d.return_ty,
            ExtendMethodKind::Agent(d) => &d.return_ty,
        }
    }
}

/// Which external ecosystem an import pulls from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportSource {
    /// `import python "anthropic" as anthropic` — Python module via FFI.
    Python,
    /// `import "./path" as alias` — another `.cor` source file, relative
    /// to the importing file. Resolver builds a module graph, detects
    /// cycles, and makes the imported file's `pub` declarations visible
    /// through qualified access (`alias.Name`). The string in the
    /// [`ImportDecl::module`] field is the relative path *without* the
    /// `.cor` extension (extension is implicit).
    Corvid,
    /// `import "https://example.com/policy.cor" hash:sha256:... as p`
    /// — remote Corvid source fetched over HTTP(S). A content hash is
    /// mandatory; unhashed remote code is not a valid import boundary.
    RemoteCorvid,
    /// `import "corvid://@scope/name/v1.2" as p` — package import
    /// resolved through `Corvid.lock`. The source does not carry an
    /// inline hash; the lockfile supplies the immutable URL + digest.
    PackageCorvid,
    // JavaScript, C, MCP — added in later versions.
}

/// An import statement:
///
/// ```text
/// import python "anthropic" as anthropic    # external Python module
/// import "./default_policy" as p            # local Corvid file
/// import "./policy" use Review, Receipt as ReviewReceipt
/// ```
///
/// `module` holds either the external module identifier (Python imports)
/// or the relative filesystem path (Corvid imports). The distinction is
/// carried by [`source`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportDecl {
    pub source: ImportSource,
    pub module: String,
    #[serde(default)]
    pub content_hash: Option<ImportContentHash>,
    #[serde(default)]
    pub required_attributes: Vec<AgentAttribute>,
    #[serde(default)]
    pub required_constraints: Vec<EffectConstraint>,
    #[serde(default)]
    pub effect_row: EffectRow,
    pub alias: Option<Ident>,
    #[serde(default)]
    pub use_items: Vec<ImportUseItem>,
    pub span: Span,
}

/// Content pin attached to a Corvid import:
///
/// ```text
/// import "./policy" hash:sha256:abc123... as policy
/// ```
///
/// The parser currently accepts only `sha256`; the string field keeps
/// the AST forward-compatible with future hash algorithms without
/// forcing a new enum migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportContentHash {
    pub algorithm: String,
    pub hex: String,
    pub span: Span,
}

/// One explicitly lifted public symbol from a Corvid import:
///
/// ```text
/// import "./policy" use Review, Receipt as ReviewReceipt
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportUseItem {
    pub name: Ident,
    pub alias: Option<Ident>,
    pub span: Span,
}

/// A user-defined struct-like type:
///
/// ```text
/// type Ticket:
///     order_id: String
///     user_id: String
/// ```
///
/// v0.1 supports struct-like types only. Enum/union types arrive in v0.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDecl {
    pub name: Ident,
    pub fields: Vec<Field>,
    /// Sum-type variants (slice 45h): `| Pending` /
    /// `| Approved(approver: String)`. A type declaration is a
    /// record (fields, no variants) XOR a sum (variants, no
    /// fields) — the parser rejects mixing. Variant payload fields
    /// are stored positionally at runtime; the declared names are
    /// metadata for diagnostics and 45i pattern destructuring.
    #[serde(default)]
    pub variants: Vec<SumVariant>,
    /// Type alias (slice 45n): `type CustomerId = String`. An alias
    /// is TRANSPARENT — the checker expands it to the target type
    /// everywhere, so `CustomerId` and `String` are the same type
    /// (no newtype semantics). A declaration is a record XOR a sum
    /// XOR an alias.
    #[serde(default)]
    pub alias: Option<crate::ty::TypeRef>,
    /// Module-level visibility. Defaults to [`Visibility::Private`]
    /// (file-scoped). Marked `public` to be visible to importers
    /// once cross-file `.cor` imports land in `lang-cor-imports-basic`.
    /// Existing single-file programs behave identically regardless of
    /// the field's value.
    #[serde(default)]
    pub visibility: Visibility,
    pub span: Span,
}

/// One variant of a sum type (slice 45h).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SumVariant {
    pub name: Ident,
    /// Payload fields: `(name, type)` pairs; empty for unit
    /// variants like `| Pending`.
    pub fields: Vec<Field>,
    pub span: Span,
}

/// A typed state declaration.
///
/// `session Name:` declares per-conversation state; `memory Name:`
/// declares durable state. Both use the same field grammar as `type`
/// declarations so later runtime accessors can expose typed `get` /
/// `set` APIs without re-parsing schema metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreDecl {
    pub kind: StoreKind,
    pub name: Ident,
    pub fields: Vec<Field>,
    #[serde(default)]
    pub policies: Vec<StorePolicy>,
    #[serde(default)]
    pub visibility: Visibility,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorePolicy {
    pub name: Ident,
    pub value: crate::effect::DimensionValue,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    Session,
    Memory,
}

impl StoreKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Memory => "memory",
        }
    }

    pub fn read_effect(&self) -> &'static str {
        match self {
            Self::Session => "reads_session",
            Self::Memory => "reads_memory",
        }
    }

    pub fn write_effect(&self) -> &'static str {
        match self {
            Self::Session => "writes_session",
            Self::Memory => "writes_memory",
        }
    }
}

/// A tool declaration:
///
/// ```text
/// tool get_order(id: String) -> Order
/// tool issue_refund(id: String, amount: Float) -> Receipt dangerous
/// ```
///
/// Tools have no body — they are externally implemented and registered
/// with the runtime. The `dangerous` keyword is optional; when absent the
/// effect is `Effect::Safe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_ty: TypeRef,
    #[serde(default)]
    pub return_ownership: Option<OwnershipAnnotation>,
    /// Circuit breaker (slice 50k): `breaker N` — after N
    /// CONSECUTIVE failures of this tool within a run, further
    /// calls short-circuit to an error naming the breaker until a
    /// success resets the count. Run-scoped by design: wall-clock
    /// cooldowns would break replay determinism.
    #[serde(default)]
    pub breaker: Option<u64>,
    pub effect: Effect,
    /// Dimensional effect row: `uses transfer_money, audit_log`.
    #[serde(default)]
    pub effect_row: EffectRow,
    /// Module-level visibility. Defaults to [`Visibility::Private`]
    /// (file-scoped). Marked `public` to be visible to importers.
    #[serde(default)]
    pub visibility: Visibility,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PromptStreamSettings {
    #[serde(default)]
    pub min_confidence: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    /// Sampling override (slice 46a): `with temperature 0.2`.
    /// Beats the model declaration's `temperature:` field.
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Sampling override (slice 46a): `with top_p 0.9`.
    #[serde(default)]
    pub top_p: Option<f64>,
    /// Structured-output auto-repair (slice 46h): `with repair N`.
    /// When the response fails typed decode, re-ask with the
    /// schema-violation feedback appended, up to N extra attempts.
    /// Each attempt is a fully traced LLM call, so replay
    /// reproduces the exact repair sequence; failed attempts still
    /// cost and the accumulated cost lands on the final result.
    #[serde(default)]
    pub repair: Option<u64>,
    #[serde(default)]
    pub backpressure: Option<BackpressurePolicy>,
    #[serde(default)]
    pub escalate_to: Option<Ident>,
}

/// A prompt declaration:
///
/// ```text
/// prompt classify(t: Ticket) -> Category:
///     "Classify this ticket into one category."
/// ```
///
/// The body is a string template the compiler turns into an LLM call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// One role block of a multi-message prompt (slice 46b):
/// `system: "You are terse."`. Roles are `system` / `user` /
/// `assistant`, validated at parse. Templates interpolate
/// `{param}` exactly like the single-template form.
pub struct PromptMessage {
    pub role: String,
    pub template: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_ty: TypeRef,
    #[serde(default)]
    pub return_ownership: Option<OwnershipAnnotation>,
    pub template: String,
    /// Multi-message role blocks (slice 46b). Empty means the
    /// single-`template` form; non-empty means `template` holds the
    /// role-labeled concatenation for display and the blocks carry
    /// the real structure.
    #[serde(default)]
    pub messages: Vec<PromptMessage>,
    /// Dimensional effect row: `uses llm_call, reads_context`.
    #[serde(default)]
    pub effect_row: EffectRow,
    /// `cites <param> strictly` — runtime verification that the LLM
    /// response references content from the named parameter.
    #[serde(default)]
    pub cites_strictly: Option<String>,
    /// Stream-only prompt modifiers such as `with min_confidence 0.80`.
    #[serde(default)]
    pub stream: PromptStreamSettings,
    /// `calibrated` — runtime records confidence-vs-accuracy samples
    /// when an adapter/eval supplies correctness observations.
    #[serde(default)]
    pub calibrated: bool,
    /// `cacheable: true` declares that the prompt is a pure function of
    /// its selected model, rendered input, arguments, and output schema.
    #[serde(default)]
    pub cacheable: bool,
    /// `requires: <capability>` — minimum model capability this prompt
    /// needs to execute. Composed via Max through the call graph. The
    /// runtime uses this to pick the cheapest model whose `capability`
    /// field satisfies the requirement. See Phase 20h slice B.
    #[serde(default)]
    pub capability_required: Option<Ident>,
    /// `output_format: strict_json` declares the required response
    /// shape for model routing. Named dispatch targets must advertise
    /// the same `output_format`; capability dispatch filters runtime
    /// catalog models by this value.
    #[serde(default)]
    pub output_format_required: Option<Ident>,
    /// `route:` clause — pattern-dispatched per-call model selection.
    /// Each arm pairs a guard expression (or the `_` wildcard) with
    /// a `model` reference. At runtime, arms are evaluated top-to-
    /// bottom and the first match's model executes the template.
    /// See Phase 20h slice C.
    #[serde(default)]
    pub route: Option<RouteTable>,
    /// `progressive:` clause — sequential dispatch with confidence
    /// escalation. Try the first model; if its output confidence is
    /// below the declared threshold, escalate to the next model; and
    /// so on. The final stage always runs (no threshold). Mutually
    /// exclusive with `route:`. See Phase 20h slice E.
    #[serde(default)]
    pub progressive: Option<ProgressiveChain>,
    /// `rollout N% <variant>, else <baseline>` — probabilistic
    /// A/B dispatch. A fraction of calls go to the variant model;
    /// the rest go to the baseline. Mutually exclusive with
    /// `route:` and `progressive:`. See Phase 20h slice I.
    #[serde(default)]
    pub rollout: Option<RolloutSpec>,
    /// `ensemble [m1, m2, m3] vote majority` — concurrent dispatch
    /// to every listed model; deterministic vote picks the winner.
    /// Mutually exclusive with `route:`, `progressive:`, and
    /// `rollout`. See Phase 20h slice F.
    #[serde(default)]
    pub ensemble: Option<EnsembleSpec>,
    /// `adversarial:` block — a three-stage propose / challenge /
    /// adjudicate pipeline. Each stage runs sequentially against a
    /// different model; the adjudicator's output is returned.
    /// Mutually exclusive with every other dispatch clause.
    /// See Phase 20h slice G.
    #[serde(default)]
    pub adversarial: Option<AdversarialSpec>,
    /// Module-level visibility. Defaults to [`Visibility::Private`]
    /// (file-scoped). Marked `public` to be visible to importers.
    #[serde(default)]
    pub visibility: Visibility,
    pub span: Span,
}

/// A three-stage adversarial validation pipeline.
///
/// At runtime the proposer produces a candidate, the challenger
/// inspects it for flaws, and the adjudicator returns the final
/// verdict given both prior outputs. Each stage dispatches to its
/// own model so the adjudicator is structurally distinct from the
/// proposer — the type system enforces three positional stages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdversarialSpec {
    pub proposer: Ident,
    pub challenger: Ident,
    pub adjudicator: Ident,
    pub span: Span,
}

/// `ensemble` clause — concurrent voting dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnsembleSpec {
    /// Models to dispatch to concurrently. Must have ≥ 2 entries;
    /// ties are broken deterministically by the vote strategy.
    pub models: Vec<Ident>,
    /// Vote strategy. Currently only `Majority` is supported — see
    /// `VoteStrategy` for future extensions.
    pub vote: VoteStrategy,
    /// Optional vote weighting policy. `accuracy_history` weights each
    /// member by observed calibration accuracy for this prompt/model pair.
    pub weighting: Option<EnsembleWeighting>,
    /// Optional disagreement fallback. If ensemble members disagree,
    /// dispatch the same prompt to this model and return its answer.
    pub disagreement_escalation: Option<Ident>,
    pub span: Span,
}

/// Vote strategy for an ensemble. Reserved for future extension
/// (weighted, plurality, unanimity) — slice F ships only Majority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteStrategy {
    Majority,
}

impl VoteStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Majority => "majority",
        }
    }
}

/// `rollout N% <variant>, else <baseline>` — probabilistic A/B
/// variant dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnsembleWeighting {
    AccuracyHistory,
}

impl EnsembleWeighting {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccuracyHistory => "accuracy_history",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutSpec {
    /// Percentage of calls routed to the variant. Stored as the
    /// literal percentage (0.0 to 100.0) the user wrote, so error
    /// messages can surface the original number unchanged.
    pub variant_percent: f64,
    pub variant: Ident,
    pub baseline: Ident,
    pub span: Span,
}

/// A `progressive:` clause body — a linear chain of
/// (model, optional threshold) stages. The final stage has
/// `threshold: None` and acts as the terminal fallback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressiveChain {
    pub stages: Vec<ProgressiveStage>,
    pub span: Span,
}

/// One stage in a progressive chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressiveStage {
    pub model: Ident,
    /// `below N` — escalate to the next stage when output confidence
    /// is strictly less than this value. `None` on the last stage,
    /// which is always run as the terminal fallback.
    pub threshold: Option<f64>,
    pub span: Span,
}

/// A `route:` clause body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteTable {
    pub arms: Vec<RouteArm>,
    pub span: Span,
}

/// One arm inside a `route:` clause. `pattern -> model`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteArm {
    pub pattern: RoutePattern,
    pub model: Ident,
    pub span: Span,
}

/// What an arm matches against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RoutePattern {
    /// `_` — catches anything not matched by an earlier arm.
    Wildcard { span: Span },
    /// A boolean-valued expression evaluated against the prompt's
    /// inputs. The arm fires when the expression is `true` at the
    /// call site.
    Guard(Expr),
}

/// An agent declaration:
///
/// ```text
/// agent refund_bot(ticket: Ticket) -> Decision:
///     ...
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A pure function declaration (slice 45r): `fn add(a: Int,
/// b: Int) -> Int:`. No effect row (the body is checked
/// effect-free), no annotations, no extern ABI. Lowered into the
/// agent IR with `pure_fn: true`, so every execution tier runs it
/// without new machinery.
pub struct FnDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_ty: TypeRef,
    pub body: Block,
    #[serde(default)]
    pub visibility: Visibility,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDecl {
    pub name: Ident,
    #[serde(default)]
    pub extern_abi: Option<ExternAbi>,
    pub params: Vec<Param>,
    pub return_ty: TypeRef,
    #[serde(default)]
    pub return_ownership: Option<OwnershipAnnotation>,
    pub body: Block,
    /// Declared effect row: `uses search_knowledge, transfer_money`.
    /// If empty, the typechecker infers the effect row from the body.
    #[serde(default)]
    pub effect_row: EffectRow,
    /// Constraints: `@budget($1.00)`, `@trust(autonomous)`, etc.
    ///
    /// Dimensional effect constraints that participate in cost
    /// analysis and compose through the call graph. Distinct from
    /// `attributes`, which carry compile-time guarantees that are
    /// not dimensional (e.g., `@replayable`).
    #[serde(default)]
    pub constraints: Vec<EffectConstraint>,
    /// Non-dimensional compile-time attributes on this agent.
    /// `@replayable` is the first; `@deterministic` ships in
    /// Phase 21 slice F. Attributes are invariants the compiler
    /// checks but that do not compose through the call graph
    /// the way effect constraints do.
    #[serde(default)]
    pub attributes: Vec<AgentAttribute>,
    /// Module-level visibility. Defaults to [`Visibility::Private`]
    /// (file-scoped). Marked `public` to be visible to importers.
    /// `pub extern "c"` agents are implicitly public regardless of
    /// any preceding `public` keyword — FFI export requires external
    /// visibility by definition.
    #[serde(default)]
    pub visibility: Visibility,
    pub span: Span,
}

/// ABI marker on an exported agent declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternAbi {
    C,
}

impl ExternAbi {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::C => "c",
        }
    }
}

/// Compile-time attribute on an agent declaration. Distinct from
/// `EffectConstraint` because attributes do not name dimensions
/// or carry numeric bounds — they are pure declarative markers
/// that the type checker consumes to enforce guarantees like
/// replayability, pure determinism, or explicit wrapping arithmetic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentAttribute {
    /// `@replayable` — compile-time guarantee that every
    /// nondeterministic input in the agent's body is captured
    /// in the recorded trace, so a `corvid replay` reproduces
    /// the agent byte-identically. See Phase 21 slice
    /// `21-inv-A` and `docs/phases/phase-21-determinism-sources.md`.
    Replayable { span: Span },
    /// `@deterministic` — strictly stronger than `@replayable`.
    /// Given the same inputs, always produces the same outputs,
    /// trace or no trace. Forbids every LLM / tool / approve
    /// call and every catalog-registered nondeterministic
    /// builtin, plus calls to agents not themselves marked
    /// `@deterministic`. The agent is a pure function over
    /// its parameters. See Phase 21 slice `21-inv-F`.
    Deterministic { span: Span },
    /// `@wrapping` — opt out of integer overflow traps inside this
    /// agent. Integer add/sub/mul/neg wrap as i64 two's-complement;
    /// division and modulo by zero still trap.
    Wrapping { span: Span },
    /// `@grounded_pure` — Provenance Propagation moat (D6).
    /// Compile-time guarantee that the agent's body launders no
    /// `Grounded<T>` value into a non-grounded slot. The proof
    /// obligation (slice 9): no `IrExprKind::UnwrapGrounded` is
    /// reachable in the agent's IR body. The discard nodes are
    /// inserted by slice 7 at every silent `Grounded<T> -> T`
    /// coercion site, so a passing `@grounded_pure` agent is one
    /// whose every grounded value either reaches a grounded slot
    /// or is explicitly stripped via `.unwrap_discarding_sources()`.
    /// The attribute composes through the call graph the same way
    /// `@deterministic` does — a `@grounded_pure` agent may only
    /// call other `@grounded_pure` (or laundering-free built-in)
    /// agents.
    GroundedPure { span: Span },
    /// `@retry(max_attempts: 3)` or `@retry(max_attempts: 3,
    /// backoff: exponential 250)` (slice 45q). Declares the durable
    /// job runner's retry policy for jobs that execute this agent.
    /// Enqueue-time flag values take precedence; the annotation is
    /// the agent-side default.
    Retry {
        max_attempts: u64,
        backoff: Option<crate::expr::Backoff>,
        span: Span,
    },
    /// `@idempotency(key: order_id)` (slice 45q). Names the agent
    /// PARAMETER whose value derives the durable job's idempotency
    /// key. The checker verifies the name matches a declared
    /// parameter of String or Int type.
    Idempotency { key: Ident, span: Span },
}

impl AgentAttribute {
    /// Span of the `@name` annotation, used for diagnostics.
    pub fn span(&self) -> Span {
        match self {
            Self::Replayable { span } => *span,
            Self::Deterministic { span } => *span,
            Self::Wrapping { span } => *span,
            Self::GroundedPure { span } => *span,
            Self::Retry { span, .. } => *span,
            Self::Idempotency { span, .. } => *span,
        }
    }

    /// Stable name used in diagnostics and parser lookup.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Replayable { .. } => "replayable",
            Self::Deterministic { .. } => "deterministic",
            Self::Wrapping { .. } => "wrapping",
            Self::GroundedPure { .. } => "grounded_pure",
            Self::Retry { .. } => "retry",
            Self::Idempotency { .. } => "idempotency",
        }
    }

    /// `@deterministic` implies `@replayable`. An agent marked
    /// only `@deterministic` still satisfies every replayability
    /// invariant. Callers checking one attribute or the other
    /// use these helpers rather than pattern-matching directly.
    pub fn is_replayable(attrs: &[AgentAttribute]) -> bool {
        attrs
            .iter()
            .any(|a| matches!(a, Self::Replayable { .. } | Self::Deterministic { .. }))
    }

    pub fn is_deterministic(attrs: &[AgentAttribute]) -> bool {
        attrs
            .iter()
            .any(|a| matches!(a, Self::Deterministic { .. }))
    }

    pub fn is_wrapping(attrs: &[AgentAttribute]) -> bool {
        attrs.iter().any(|a| matches!(a, Self::Wrapping { .. }))
    }

    /// True when any of `attrs` carries `@grounded_pure`. Slice 9
    /// uses this to gate the IR reachability check.
    pub fn is_grounded_pure(attrs: &[AgentAttribute]) -> bool {
        attrs
            .iter()
            .any(|a| matches!(a, Self::GroundedPure { .. }))
    }
}

/// An eval declaration. The body executes setup code and the trailing
/// assertions validate either values or the execution trace shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalDecl {
    pub name: Ident,
    pub body: Block,
    pub assertions: Vec<EvalAssert>,
    pub span: Span,
}

/// A `test` declaration. Tests are deterministic developer checks over
/// ordinary setup code plus assertions. They reuse eval assertion syntax so
/// value checks and trace/process checks share one assertion model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestDecl {
    pub name: Ident,
    #[serde(default)]
    pub trace_fixture: Option<String>,
    pub body: Block,
    pub assertions: Vec<EvalAssert>,
    pub span: Span,
}

/// A reusable test data factory. Fixtures are callable from `test` and `mock`
/// bodies, but they are not production agents and are not exposed through
/// normal package metadata as executable app entry points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_ty: TypeRef,
    pub body: Block,
    pub span: Span,
}

/// A test-only override for an external tool. Mocks must match the target
/// tool's signature exactly, so tests cannot accidentally weaken the tool
/// contract they are standing in for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MockDecl {
    pub target: Ident,
    pub params: Vec<Param>,
    pub return_ty: TypeRef,
    pub body: Block,
    #[serde(default)]
    pub effect_row: EffectRow,
    pub span: Span,
}

/// An assertion inside an `eval` block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EvalAssert {
    /// `assert <expr>` or `assert <expr> with confidence P over N runs`
    Value {
        expr: Expr,
        confidence: Option<f64>,
        runs: Option<u64>,
        span: Span,
    },
    /// `assert_snapshot <expr>`
    Snapshot { expr: Expr, span: Span },
    /// `assert called <tool>`
    Called { tool: Ident, span: Span },
    /// `assert approved <label>`
    Approved { label: Ident, span: Span },
    /// `assert cost < $0.50`
    Cost {
        op: BinaryOp,
        bound: f64,
        span: Span,
    },
    /// `assert called <A> before <B>`
    Ordering {
        before: Ident,
        after: Ident,
        span: Span,
    },
    /// `assert similar <expr>, <expr> min <float>` (slice 46h) —
    /// deterministic word-set similarity between the two rendered
    /// values must reach `min` (0..=1). No LLM cost.
    Similar {
        expr: Expr,
        expected: Expr,
        min: f64,
        span: Span,
    },
    /// `assert judged <expr>, "criteria" min <float>` (slice 46h)
    /// — an LLM judge scores the value against the criteria; the
    /// score must reach `min` (0..=1). The judge call flows
    /// through the normal LLM path (traced, cost-accounted, so
    /// eval `--max-spend` sees it).
    Judged {
        expr: Expr,
        criteria: String,
        min: f64,
        span: Span,
    },
}

/// `model Name:` declaration — a catalog entry for an LLM.
///
/// Each model carries a map of property name → value describing
/// cost, capability, latency, jurisdiction, privacy tier, specialty,
/// and so on. The set of valid property names is *not* hardcoded:
/// any property that corresponds to a declared dimension (built-in
/// or custom via `corvid.toml`) is accepted. This mirrors Phase 20g
/// invention #6 — the effect system is user-extensible, and the
/// model catalog extends alongside it without compiler changes.
///
/// Example:
///
/// ```text
/// model haiku:
///     cost_per_token_in: $0.00000025
///     cost_per_token_out: $0.00000125
///     capability: basic
///     latency: fast
///     max_context: 200000
///     jurisdiction: us_hosted
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDecl {
    pub name: Ident,
    pub fields: Vec<ModelField>,
    /// Module-level visibility (slice 45o): `public model x:`
    /// makes the model importable via `use`.
    #[serde(default)]
    pub visibility: Visibility,
    pub span: Span,
}

/// One property on a `model` block — a name and its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelField {
    pub name: Ident,
    pub value: crate::effect::DimensionValue,
    pub span: Span,
}
