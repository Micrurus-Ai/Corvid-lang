//! The type checker and effect checker.
//!
//! Walks a parsed, resolved `File` and:
//!   * assigns a `Type` to every expression (side table, keyed by span)
//!   * validates call arities and parameter/return compatibility
//!   * enforces the approve-before-dangerous invariant
//!
//! See `ARCHITECTURE.md` §6 and `FEATURES.md` v0.1.

use crate::errors::{TypeError, TypeErrorKind, TypeWarning, TypeWarningKind};
use crate::types::Type;
use corvid_ast::{
    AgentDecl, Decl, Effect, ExtendMethodKind, File, FixtureDecl, ModelDecl, Param, PromptDecl,
    Span, ToolDecl, TypeDecl, WeakEffect, WeakEffectRow,
};
use corvid_resolve::{
    resolver::MethodEntry, Binding, DefId, LocalId, ReplayPatternBinding, Resolved, SymbolTable,
};
use std::collections::{HashMap, HashSet};

fn file_top_span(file: &File) -> Span {
    file.span
}

/// Output of the type checker.
#[derive(Debug, Clone)]
pub struct Checked {
    /// Type assigned to each expression, keyed by the expression's span.
    pub types: HashMap<Span, Type>,
    /// Type assigned to each local binding visible in the checked file.
    pub local_types: HashMap<LocalId, Type>,
    /// All errors found. Reporting continues past each error.
    pub errors: Vec<TypeError>,
    /// Non-fatal diagnostics.
    pub warnings: Vec<TypeWarning>,
    /// Qualified calls that resolved across a `.cor` import boundary,
    /// keyed by the `alias.member` callee expression span. IR lowering
    /// consumes this to emit a direct call to the imported declaration
    /// instead of treating the field access as an indirect value.
    pub imported_calls: HashMap<Span, ImportedCallTarget>,
    /// Spans of value expressions where the legacy `Grounded<T> -> T`
    /// assignability rule (`types.rs:153`) silently coerced a grounded
    /// value into a non-grounded slot — return / let-with-annotation /
    /// yield / call-arg / struct-field-init / control-flow condition.
    /// Provenance Propagation D5 (slice 7): IR lowering reads this set
    /// and inserts an `IrExprKind::UnwrapGrounded` node at each
    /// recorded span, so every silent provenance drop becomes
    /// IR-visible — which is what `@grounded_pure` (slice 9) forbids.
    /// Sound enumeration is the load-bearing property: a missed site
    /// is a silent moat hole.
    pub grounded_coercion_sites: HashSet<Span>,
    /// The type each type reference in this file resolved to, keyed by
    /// the reference's span. Aliases are expanded, including aliases
    /// declared in other modules. IR lowering reads its signature and
    /// field types from here instead of resolving type references a
    /// second time, so the two layers cannot disagree about a type.
    pub type_refs: HashMap<Span, Type>,
    /// The spans of the type references in [`Self::type_refs`] whose
    /// type holds a `Secret`, directly, as a type argument or in a field
    /// of a record it names. The application contract leaves these
    /// positions out, so no client, SDK or frontend is offered a field
    /// a secret would fill.
    pub secret_type_refs: HashSet<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedCallTarget {
    pub module_path: String,
    pub def_id: DefId,
    pub name: String,
    pub kind: ImportedCallKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportedCallKind {
    Type,
    Tool,
    Prompt,
    Agent,
}

pub fn typecheck(file: &File, resolved: &Resolved) -> Checked {
    typecheck_with_config(file, resolved, None)
}

/// Typecheck `file` with an explicit cross-file module resolution.
/// Callers that have already loaded + resolved imported `.cor`
/// files via [`corvid_driver::build_module_resolution`] pass the
/// resulting [`ModuleResolution`] here so qualified type
/// references (`alias.TypeName`) can consult it. Callers without
/// imports should use the plain [`typecheck`] or
/// [`typecheck_with_config`] and get the same behavior as before.
pub fn typecheck_with_modules(
    file: &File,
    resolved: &Resolved,
    modules: &corvid_resolve::ModuleResolution,
) -> Checked {
    typecheck_with_everything(file, resolved, None, Some(modules))
}

/// Typecheck `file` with both an explicit `corvid.toml` configuration
/// and cross-file module resolution. Production file-backed compiles
/// use this path so custom dimensions and `import "./path" as alias`
/// semantics compose rather than one disabling the other.
pub fn typecheck_with_config_and_modules(
    file: &File,
    resolved: &Resolved,
    config: Option<&crate::config::CorvidConfig>,
    modules: &corvid_resolve::ModuleResolution,
) -> Checked {
    typecheck_with_everything(file, resolved, config, Some(modules))
}

/// Typecheck `file`, consuming an optional `corvid.toml` configuration.
/// Custom dimensions declared under `[effect-system.dimensions.*]`
/// are merged into the `EffectRegistry` alongside the built-ins.
/// A malformed `corvid.toml` entry surfaces as an
/// `InvalidCustomDimension` type error at the file's top span.
pub fn typecheck_with_config(
    file: &File,
    resolved: &Resolved,
    config: Option<&crate::config::CorvidConfig>,
) -> Checked {
    typecheck_with_everything(file, resolved, config, None)
}

fn typecheck_with_everything(
    file: &File,
    resolved: &Resolved,
    config: Option<&crate::config::CorvidConfig>,
    modules: Option<&corvid_resolve::ModuleResolution>,
) -> Checked {
    // Self-trial round 4 gap (Gap A — schedule decls silently dropped):
    // emit a warning for every `schedule` declaration so reviewers
    // know the cron won't fire on v1.0. The IR lowering at
    // `crates/corvid-ir/src/lower.rs:231` silently drops the decl
    // ("until the scheduler runner slice"); without this warning a
    // reviewer writing `schedule "0 9 * * *" zone "America/New_York"
    // -> summarize_yesterday()` gets `ok: src/main.cor — no errors`
    // and confidently expects the cron to fire. The hint points
    // them at the external-cron workaround until the runner ships.
    let mut early_warnings: Vec<crate::errors::TypeWarning> = Vec::new();
    for decl in &file.decls {
        if let Decl::Schedule(sched) = decl {
            early_warnings.push(crate::errors::TypeWarning::new(
                crate::errors::TypeWarningKind::ScheduleNotExecutable {
                    agent: sched.target.name.clone(),
                    cron: sched.cron.clone(),
                },
                sched.span,
            ));
        }
    }

    // Build the effect registry up front. Slice 2b of Provenance
    // Propagation (Design X, D1 part A) needs it *during* the main
    // check pass so `check_*_call` can wrap a `data: grounded`
    // callee's return type to `Grounded<T>`. The post-passes below
    // (analyze_effects, compute_worst_case_cost, check_grounded_returns)
    // reuse the same registry.
    let registry = effect_registry_for(file, modules, config);
    let effect_decls: Vec<&corvid_ast::EffectDecl> = file
        .decls
        .iter()
        .filter_map(|d| if let Decl::Effect(e) = d { Some(e) } else { None })
        .collect();

    let mut c = Checker::new(file, resolved, modules, &registry);
    c.warnings.extend(early_warnings);
    c.validate_import_use_items(file);
    c.validate_python_import_effects(file);
    c.check_file(file);
    c.errors
        .extend(crate::approval_reachability::check_approval_reachability(
            file, resolved,
        ));

    for effect in &effect_decls {
        c.check_effect_decl_confidence(effect);
    }

    // Validate config-declared dimensions up-front so malformed entries
    // become surfaceable diagnostics instead of being swallowed by the
    // registry builder. The registry itself still silently skips
    // invalid entries — this is the user-facing channel.
    if let Some(cfg) = config {
        if let Err(err) = cfg.into_dimension_schemas() {
            let (dimension, message) = match &err {
                crate::config::DimensionConfigError::ParseError { message, .. } => {
                    (String::new(), message.clone())
                }
                crate::config::DimensionConfigError::UnknownComposition { dimension, .. }
                | crate::config::DimensionConfigError::UnknownType { dimension, .. }
                | crate::config::DimensionConfigError::BadDefault { dimension, .. }
                | crate::config::DimensionConfigError::CollidesWithBuiltin { dimension } => {
                    (dimension.clone(), err.to_string())
                }
            };
            let span = file_top_span(file);
            c.errors.push(TypeError::new(
                TypeErrorKind::InvalidCustomDimension { dimension, message },
                span,
            ));
        }
    }

    // Dimensional effect analysis: analyze agents against the
    // registry built above, and report non-cost constraint violations.
    if !effect_decls.is_empty()
        || file
            .decls
            .iter()
            .any(|d| matches!(d, Decl::Agent(a) if !a.constraints.is_empty()))
    {
        let summaries = crate::effects::analyze_effects(file, resolved, &registry);
        for summary in &summaries {
            for violation in &summary.violations {
                if matches!(
                    violation.dimension.as_str(),
                    "cost" | "tokens" | "latency_ms"
                ) {
                    continue;
                }
                // Slice 33Q3: dispatch by dimension so trust-lattice
                // violations anchor to the dedicated
                // `trust.constraint_enforcement` guarantee instead of
                // sharing `effect_row.body_completeness` with every
                // other non-cost dimension. This is what makes
                // `@trust(...)` signable: the registry row's Static +
                // TypeCheck classification requires a tagged
                // diagnostic site (enforced by
                // `every_typecheck_phase_static_guarantee_uses_with_guarantee_constructor`
                // in `corvid-guarantees/src/lib.rs`), and this is
                // that site.
                let guarantee_id = if violation.dimension.as_str() == "trust" {
                    "trust.constraint_enforcement"
                } else {
                    "effect_row.body_completeness"
                };
                c.errors.push(TypeError::with_guarantee(
                    TypeErrorKind::EffectConstraintViolation {
                        agent: summary.agent_name.clone(),
                        dimension: violation.dimension.clone(),
                        message: violation.to_string(),
                    },
                    violation.span,
                    guarantee_id,
                ));
            }
        }
    }

    for decl in &file.decls {
        let Decl::Agent(agent) = decl else { continue };
        let budget_constraints: Vec<_> = agent
            .constraints
            .iter()
            .filter(|constraint| {
                matches!(
                    crate::effects::canonical_dimension_name(&constraint.dimension.name).as_str(),
                    "cost" | "tokens" | "latency_ms"
                )
            })
            .cloned()
            .collect();
        if budget_constraints.is_empty() {
            continue;
        }

        if let Some(estimate) =
            crate::effects::compute_worst_case_cost_with_types(file, resolved, &registry, &agent.name.name, &c.types)
        {
            for warning in estimate.warnings.clone() {
                let crate::effects::CostWarningKind::UnboundedLoop { agent, message } =
                    warning.kind;
                c.warnings.push(TypeWarning::new(
                    TypeWarningKind::UnboundedCostAnalysis { agent, message },
                    warning.span,
                ));
            }
            if !estimate.bounded {
                // A budget promises the worst case is at most its limit,
                // which a worst case with no bound cannot be shown to be.
                let reason = estimate
                    .warnings
                    .iter()
                    .map(|warning| {
                        let crate::effects::CostWarningKind::UnboundedLoop { message, .. } = &warning.kind;
                        message.clone()
                    })
                    .next()
                    .unwrap_or_else(|| "a call it makes recurses".to_string());
                c.errors.push(TypeError::with_guarantee(
                    TypeErrorKind::BudgetUnbounded { agent: agent.name.name.clone(), reason },
                    budget_constraints[0].span,
                    "budget.compile_time_ceiling",
                ));
                continue;
            }

            for constraint in &budget_constraints {
                let dim = crate::effects::canonical_dimension_name(&constraint.dimension.name);
                let actual = estimate.dimensions.get(&dim).copied().unwrap_or(0.0);
                let Some(limit) = crate::effects::numeric_constraint_value(constraint) else {
                    continue;
                };
                if actual > limit {
                    let path = crate::effects::cost_path_for_dimension(&estimate.tree, &dim);
                    let path_text = if path.is_empty() {
                        "path attribution unavailable".to_string()
                    } else {
                        path.join(" → ")
                    };
                    let message = format!(
                        "{}: {} > {} budget (path: {})",
                        dim,
                        crate::effects::format_numeric_dimension(&dim, actual),
                        crate::effects::format_numeric_dimension(&dim, limit),
                        path_text,
                    );
                    c.errors.push(TypeError::with_guarantee(
                        TypeErrorKind::EffectConstraintViolation {
                            agent: agent.name.name.clone(),
                            dimension: dim,
                            message,
                        },
                        constraint.span,
                        "budget.compile_time_ceiling",
                    ));
                }
            }
        }
    }

    // Provenance verification: check that agents returning Grounded<T>
    // actually have a provenance path from a data: grounded source.
    {
        let provenance_violations =
            crate::effects::check_grounded_returns(file, resolved, &registry);
        for violation in provenance_violations {
            c.errors.push(TypeError::with_guarantee(
                TypeErrorKind::UngroundedReturn {
                    agent: violation.agent_name,
                    message: violation.message,
                },
                violation.span,
                "grounded.provenance_required",
            ));
        }
    }

    c.record_signature_types(file);
    c.check_secret_boundaries(file);
    c.check_action_paths(file);
    c.check_actor_sources(file);
    c.check_action_reserved_names(file);
    c.check_response_boundaries(file);

    Checked {
        types: c.types,
        local_types: c.local_types,
        errors: c.errors,
        warnings: c.warnings,
        imported_calls: c.imported_calls,
        grounded_coercion_sites: c.grounded_coercion_sites,
        type_refs: c.type_refs,
        secret_type_refs: c.secret_type_refs,
    }
}

struct Checker<'a> {
    file: &'a File,
    symbols: &'a SymbolTable,
    bindings: &'a HashMap<Span, Binding>,
    types: HashMap<Span, Type>,
    errors: Vec<TypeError>,
    warnings: Vec<TypeWarning>,
    imported_calls: HashMap<Span, ImportedCallTarget>,
    /// Provenance Propagation D5 (slice 7): every value-expression
    /// span where the legacy `Grounded<T> -> T` rule fired during
    /// slot-checking. Populated by `record_if_grounded_coercion` from
    /// the slot-check sites (return / let / yield / call-arg /
    /// struct-field-init / control-flow condition). Surfaces on
    /// `Checked` so IR lowering can insert a visible `UnwrapGrounded`
    /// at each recorded span.
    grounded_coercion_sites: HashSet<Span>,
    /// B06 — the span of the `Decimal` `/` being checked as the
    /// receiver of a `round_<mode>` call, the one place such a division
    /// is accepted. See `checker/decimal.rs`.
    rounded_division: Option<Span>,
    /// Every root-file type reference resolved so far, by span. See
    /// [`Checked::type_refs`].
    type_refs: HashMap<Span, Type>,
    /// [`Checked::secret_type_refs`], filled by `check_secret_boundaries`.
    secret_type_refs: HashSet<Span>,

    /// Indexed declarations for O(1) lookup by DefId. Methods from
    /// `extend` blocks get inserted here too — a method `extend Order: agent
    /// total(o: Order) -> Int` indexes into `agents_by_id` under the
    /// method's allocated DefId, alongside file-level free agents.
    tools_by_id: HashMap<DefId, &'a ToolDecl>,
    /// Connector operations, indexed by DefId (slice 52g-3). An
    /// `operation` is a tool with a declarative body, so a call to one
    /// is typed exactly like a tool call — its signature (params /
    /// effect / effect row / return) lives here.
    operations_by_id: HashMap<DefId, &'a corvid_ast::OperationDecl>,
    prompts_by_id: HashMap<DefId, &'a PromptDecl>,
    agents_by_id: HashMap<DefId, &'a AgentDecl>,
    fns_by_id: HashMap<DefId, &'a corvid_ast::FnDecl>,
    fixtures_by_id: HashMap<DefId, &'a FixtureDecl>,
    types_by_id: HashMap<DefId, &'a TypeDecl>,
    models_by_id: HashMap<DefId, &'a ModelDecl>,

    /// Per-receiver-type method side-table from the
    /// resolver. Method calls (`x.foo(args)`) look up `x`'s declared
    /// type then this map to find the method's `DefId`, after which
    /// dispatch reuses the existing tool / prompt / agent call paths.
    methods: &'a HashMap<DefId, HashMap<String, MethodEntry>>,
    /// Sum-type side-table (45h): variant DefId -> (owner, index).
    variant_owners: &'a HashMap<DefId, (DefId, u32)>,

    /// Replay-pattern side-table from the resolver. Gives the
    /// `DefId` (for prompt/tool resolutions) or the `Approve`
    /// marker for approval-label patterns, keyed by the pattern's
    /// own span. The checker uses it to compute capture types
    /// without re-resolving string literals.
    replay_pattern_bindings: &'a HashMap<Span, ReplayPatternBinding>,

    /// Cross-file module resolution populated by
    /// `corvid_driver::build_module_resolution`. When `None`, the
    /// checker falls back to single-file semantics and any
    /// `TypeRef::Qualified` yields a `CorvidImportNotYetResolved`
    /// error. When `Some`, qualified references to unknown aliases /
    /// private members / unknown members surface typed errors, and
    /// successful public type exports resolve to `Type::ImportedStruct`.
    module_resolution: Option<&'a corvid_resolve::ModuleResolution>,

    /// The effect registry, built before the main check pass so
    /// `check_*_call` can apply the Provenance Propagation Design X
    /// rule (D1 part A): a call to a prompt / tool / agent whose
    /// effect row carries `data: grounded` has its return type
    /// wrapped to `Grounded<T>`.
    registry: &'a crate::effects::EffectRegistry,

    /// Type of each local binding, populated as we enter scopes.
    local_types: HashMap<LocalId, Type>,
    browser_models: HashMap<LocalId, Option<Type>>,
    browser_field_types: HashMap<Span, Type>,

    /// Declared return type of the currently-checked function-like.
    current_return: Option<Type>,
    in_agent_body: bool,
    /// Nesting depth of `for`/`while` loops around the statement
    /// being checked — `break`/`continue` require depth > 0
    /// (slice 45k).
    loop_depth: usize,
    /// Alias-expansion depth guard (slice 45n): `type A = B` chains
    /// expand through `named_type_to_type`; a cycle would recurse
    /// forever without this.
    alias_depth: usize,
    in_test_body: bool,
    saw_stream_output: bool,

    /// Approvals visible at the current point. Represented as a flat
    /// stack that is truncated back to its parent's length when a block
    /// is exited. This gives block-local effect scoping for free.
    approvals: Vec<Approval>,

    /// Every approve token seen anywhere in the current agent's body,
    /// including ones currently out of lexical scope. Used by the
    /// dangerous-call check to discriminate between two registry
    /// rows: `approval.dangerous_call_requires_token` (no approve at
    /// all) vs `approval.token_lexical_only` (an approve with the
    /// right label+arity exists somewhere in this agent but is out
    /// of lexical scope at the call site). Reset at every agent
    /// boundary via `check_agent`'s prev-swap pattern.
    approvals_seen_in_agent: Vec<Approval>,

    /// Monotonic per-effect epochs used to prove `Weak::upgrade(...)`
    /// stays ahead of invalidating effects.
    effect_frontier: EffectFrontier,

    /// Last-refresh snapshot per weak local.
    weak_refresh: HashMap<LocalId, EffectFrontier>,

    /// Whether the file declares any `identity` block (slice 51h). A
    /// route `requires` policy is only legal when it does.
    has_identity: bool,

    /// Role names declared in the identity block's `roles:` (slice 52f).
    /// A `requires role("...")` clause must reference one of these.
    identity_roles: std::collections::HashSet<String>,

    /// The union of all permissions declared across the identity block's
    /// roles (slice 52f). A `requires permission("...")` clause must
    /// reference one of these.
    identity_permissions: std::collections::HashSet<String>,
}

#[derive(Debug, Clone)]
struct Approval {
    /// The user-written label (e.g. `IssueRefund`).
    label: String,
    /// Number of arguments in the approve.
    arity: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct EffectFrontier {
    tool_call: u64,
    llm: u64,
    approve: u64,
    human: u64,
}

impl EffectFrontier {
    fn bumped(mut self, effect: WeakEffect) -> Self {
        match effect {
            WeakEffect::ToolCall => self.tool_call += 1,
            WeakEffect::Llm => self.llm += 1,
            WeakEffect::Approve => self.approve += 1,
            WeakEffect::Human => self.human += 1,
        }
        self
    }

    fn merge_max(self, other: Self) -> Self {
        Self {
            tool_call: self.tool_call.max(other.tool_call),
            llm: self.llm.max(other.llm),
            approve: self.approve.max(other.approve),
            human: self.human.max(other.human),
        }
    }

    fn meet_min(self, other: Self) -> Self {
        Self {
            tool_call: self.tool_call.min(other.tool_call),
            llm: self.llm.min(other.llm),
            approve: self.approve.min(other.approve),
            human: self.human.min(other.human),
        }
    }

    fn invalidating_effects_since(
        &self,
        refreshed_at: &EffectFrontier,
        row: WeakEffectRow,
    ) -> Vec<String> {
        let mut effects = Vec::new();
        if row.tool_call && self.tool_call != refreshed_at.tool_call {
            effects.push("tool_call".into());
        }
        if row.llm && self.llm != refreshed_at.llm {
            effects.push("llm".into());
        }
        if row.approve && self.approve != refreshed_at.approve {
            effects.push("approve".into());
        }
        if row.human && self.human != refreshed_at.human {
            effects.push("human".into());
        }
        effects
    }
}

impl<'a> Checker<'a> {
    /// Slice 50j — refinement forms are type-directed: `between`
    /// refines Int fields, `len_between` refines String fields, and
    /// bounds must be ordered. A mismatched form is a decl error
    /// here, never a decode surprise later.
    /// B01: every field's JSON key is non-empty and names one field.
    /// Keys come from `corvid_ast::json_key`, the rule lowering and the
    /// contract use, so what is checked here is what is sent.
    fn check_json_keys(&mut self, t: &corvid_ast::TypeDecl) {
        let mut seen: HashMap<String, &str> = HashMap::new();
        for field in &t.fields {
            let key = corvid_ast::json_key(&field.name.name, field.json_key.as_deref(), t.json_case);
            let message = if key.is_empty() {
                Some(format!("field `{}` has an empty JSON key", field.name.name))
            } else {
                seen.insert(key.clone(), &field.name.name).map(|other| {
                    format!(
                        "fields `{other}` and `{}` both have the JSON key `{key}`",
                        field.name.name
                    )
                })
            };
            if let Some(message) = message {
                self.errors.push(TypeError::new(
                    TypeErrorKind::JsonKeyInvalid { type_name: t.name.name.clone(), message },
                    field.span,
                ));
            }
        }
    }

    /// Each parameter refinement fits its parameter's type, by the rule
    /// a field's does.
    fn check_param_refinements(&mut self, callable: &str, params: &[corvid_ast::Param]) {
        for param in params {
            let Some(refinement) = &param.refinement else { continue };
            let type_name = match &param.ty {
                corvid_ast::TypeRef::Named { name, .. } => name.name.as_str(),
                _ => "",
            };
            if let Some(message) = refinement_problem(refinement, type_name, "parameter") {
                self.errors.push(TypeError::new(
                    TypeErrorKind::ParamRefinementInvalid {
                        callable: callable.to_string(),
                        param: param.name.name.clone(),
                        message,
                    },
                    param.span,
                ));
            }
        }
    }

    fn check_field_refinements(&mut self, t: &corvid_ast::TypeDecl) {
        // A variant's payload fields take refinements as record fields do.
        for field in t.fields.iter().chain(t.variants.iter().flat_map(|v| v.fields.iter())) {
            let Some(refinement) = &field.refinement else {
                continue;
            };
            let field_type_name = match &field.ty {
                corvid_ast::TypeRef::Named { name, .. } => name.name.as_str(),
                _ => "",
            };
            let message = refinement_problem(refinement, field_type_name, "field");
            if let Some(message) = message {
                self.errors.push(TypeError::new(
                    TypeErrorKind::RefinementInvalid {
                        type_name: t.name.name.clone(),
                        field: field.name.name.clone(),
                        message,
                    },
                    field.span,
                ));
            }
        }
    }

    fn new(
        file: &'a File,
        resolved: &'a Resolved,
        module_resolution: Option<&'a corvid_resolve::ModuleResolution>,
        registry: &'a crate::effects::EffectRegistry,
    ) -> Self {
        let mut tools = HashMap::new();
        let mut operations = HashMap::new();
        let mut prompts = HashMap::new();
        let mut agents = HashMap::new();
        let mut fns = HashMap::new();
        let mut fixtures = HashMap::new();
        let mut types = HashMap::new();
        let mut models = HashMap::new();

        for decl in &file.decls {
            match decl {
                Decl::Tool(t) => {
                    if let Some(id) = resolved.symbols.lookup_def(&t.name.name) {
                        tools.insert(id, t);
                    }
                }
                Decl::Prompt(p) => {
                    if let Some(id) = resolved.symbols.lookup_def(&p.name.name) {
                        prompts.insert(id, p);
                    }
                }
                Decl::Agent(a) => {
                    if let Some(id) = resolved.symbols.lookup_def(&a.name.name) {
                        agents.insert(id, a);
                    }
                }
                Decl::Fn(f) => {
                    if let Some(id) = resolved.symbols.lookup_def(&f.name.name) {
                        fns.insert(id, f);
                    }
                }
                Decl::Fixture(f) => {
                    if let Some(id) = resolved.symbols.lookup_def(&f.name.name) {
                        fixtures.insert(id, f);
                    }
                }
                Decl::Eval(_) | Decl::Test(_) | Decl::Mock(_) | Decl::Schedule(_) => {}
                Decl::Type(t) => {
                    if let Some(id) = resolved.symbols.lookup_def(&t.name.name) {
                        types.insert(id, t);
                    }
                }
                Decl::Table(t) => {
                    if let Some(id) = resolved.symbols.lookup_def(&t.record.name.name) {
                        types.insert(id, &t.record);
                    }
                }
                Decl::Database(_) => {}
                Decl::Traces(_) => {}
                Decl::Store(_) => {}
                Decl::Import(_) => {}
                Decl::Effect(_) => {}
                Decl::Model(m) => {
                    if let Some(id) = resolved.symbols.lookup_def(&m.name.name) {
                        models.insert(id, m);
                    }
                }
                Decl::Server(_) => {}
                Decl::Identity(_) => {}
                Decl::Connector(c) => {
                    // 52g-3: each operation is a callable tool; index its
                    // signature by DefId so a call to it types identically
                    // to a tool call.
                    for op in &c.operations {
                        if let Some(id) = resolved.symbols.lookup_def(&op.name.name) {
                            operations.insert(id, op);
                        }
                    }
                }
                Decl::Extend(ext) => {
                    // Index method decls by their allocated DefIds
                    // (from the resolver's
                    // method side-table) into the same per-kind
                    // tables free decls use, so call-resolution can
                    // dispatch uniformly.
                    let Some(type_def_id) = resolved.symbols.lookup_def(&ext.type_name.name) else {
                        continue;
                    };
                    let Some(method_table) = resolved.methods.get(&type_def_id) else {
                        continue;
                    };
                    for method in &ext.methods {
                        let name = method.name().name.as_str();
                        let Some(entry) = method_table.get(name) else {
                            continue;
                        };
                        match &method.kind {
                            ExtendMethodKind::Tool(t) => {
                                tools.insert(entry.def_id, t);
                            }
                            ExtendMethodKind::Prompt(p) => {
                                prompts.insert(entry.def_id, p);
                            }
                            ExtendMethodKind::Agent(a) => {
                                agents.insert(entry.def_id, a);
                            }
                        }
                    }
                }
            }
        }

        Self {
            symbols: &resolved.symbols,
            bindings: &resolved.bindings,
            types: HashMap::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            imported_calls: HashMap::new(),
            grounded_coercion_sites: HashSet::new(),
            rounded_division: None,
            type_refs: HashMap::new(),
            secret_type_refs: HashSet::new(),
            tools_by_id: tools,
            file,
            operations_by_id: operations,
            prompts_by_id: prompts,
            agents_by_id: agents,
            fns_by_id: fns,
            fixtures_by_id: fixtures,
            types_by_id: types,
            models_by_id: models,
            methods: &resolved.methods,
            variant_owners: &resolved.variant_owners,
            replay_pattern_bindings: &resolved.replay_pattern_bindings,
            module_resolution,
            registry,
            local_types: HashMap::new(),
            browser_models: HashMap::new(),
            browser_field_types: HashMap::new(),
            current_return: None,
            in_agent_body: false,
            loop_depth: 0,
            alias_depth: 0,
            in_test_body: false,
            saw_stream_output: false,
            approvals: Vec::new(),
            approvals_seen_in_agent: Vec::new(),
            effect_frontier: EffectFrontier::default(),
            weak_refresh: HashMap::new(),
            has_identity: file
                .decls
                .iter()
                .any(|d| matches!(d, Decl::Identity(_))),
            identity_roles: file
                .decls
                .iter()
                .filter_map(|d| match d {
                    Decl::Identity(id) => Some(id),
                    _ => None,
                })
                .flat_map(|id| id.roles.iter().map(|r| r.name.clone()))
                .collect(),
            identity_permissions: file
                .decls
                .iter()
                .filter_map(|d| match d {
                    Decl::Identity(id) => Some(id),
                    _ => None,
                })
                .flat_map(|id| id.roles.iter().flat_map(|r| r.permissions.iter().cloned()))
                .collect(),
        }
    }

    // ------------------------------------------------------------
    // File-level traversal.
    // ------------------------------------------------------------

    /// A type the file declares or imports under a built-in's name: a core
    /// scalar's name is refused, and any other built-in it hides is named
    /// in a warning (see [`Type::builtin_named`]).
    fn check_builtin_type_names(&mut self, file: &File) {
        let mut names: Vec<(&corvid_ast::Ident, bool)> = Vec::new();
        for decl in &file.decls {
            match decl {
                Decl::Type(ty) => names.push((&ty.name, true)),
                Decl::Table(table) => names.push((table.name(), true)),
                Decl::Import(import) => {
                    for item in &import.use_items {
                        names.push((item.alias.as_ref().unwrap_or(&item.name), false));
                    }
                }
                _ => {}
            }
        }
        for (ident, declared_here) in names {
            let name = ident.name.as_str();
            if Type::compiler_known(name).is_none() {
                continue;
            }
            if Type::is_reserved_name(name) {
                self.errors.push(TypeError::new(TypeErrorKind::ReservedTypeName { name: name.to_string() }, ident.span));
                continue;
            }
            // An imported `use` hides the built-in only when it is a type.
            let hides = declared_here || self.named_type_to_type(name, ident.span) != Type::compiler_known(name).unwrap_or(Type::Unknown);
            if hides {
                self.warnings.push(TypeWarning::new(TypeWarningKind::BuiltinTypeHidden { name: name.to_string() }, ident.span));
            }
        }
    }

    fn check_file(&mut self, file: &File) {
        self.check_builtin_type_names(file);
        for decl in &file.decls {
            match decl {
                Decl::Agent(a) => self.check_param_refinements(&a.name.name, &a.params),
                Decl::Fn(f) => self.check_param_refinements(&f.name.name, &f.params),
                Decl::Prompt(p) => self.check_param_refinements(&p.name.name, &p.params),
                Decl::Tool(t) => self.check_param_refinements(&t.name.name, &t.params),
                Decl::Extend(ext) => {
                    for method in &ext.methods {
                        match &method.kind {
                            ExtendMethodKind::Agent(a) => self.check_param_refinements(&a.name.name, &a.params),
                            ExtendMethodKind::Prompt(p) => self.check_param_refinements(&p.name.name, &p.params),
                            ExtendMethodKind::Tool(t) => self.check_param_refinements(&t.name.name, &t.params),
                        }
                    }
                }
                _ => {}
            }
        }
        for decl in &file.decls {
            match decl {
                Decl::Agent(a) => self.check_agent(a),
                Decl::Fn(f) => self.check_fn(f),
                Decl::Eval(e) => self.check_eval(e),
                Decl::Test(t) => self.check_test(t),
                Decl::Fixture(f) => self.check_fixture(f),
                Decl::Mock(m) => self.check_mock(m),
                Decl::Prompt(p) => self.check_prompt(p),
                Decl::Server(s) => self.check_server(s),
                // Model sampling fields (slice 46a): validate
                // ranges at the declaration so a typo'd
                // `temperature: 20` fails compile, not dispatch.
                Decl::Model(m) => {
                    for f in &m.fields {
                        let corvid_ast::DimensionValue::Number(n) = &f.value else {
                            continue;
                        };
                        let bad = match f.name.name.as_str() {
                            "temperature" => !(0.0..=2.0).contains(n),
                            "top_p" => !(0.0..=1.0).contains(n),
                            "max_tokens" => *n < 1.0 || n.fract() != 0.0,
                            _ => false,
                        };
                        if bad {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::ModelFieldInvalid {
                                    model: m.name.name.clone(),
                                    field: f.name.name.clone(),
                                    message: format!("value {n} is out of range"),
                                },
                                f.span,
                            ));
                        }
                    }
                }
                Decl::Type(t) => {
                    self.check_field_refinements(t);
                    self.check_json_keys(t);
                }
                Decl::Table(t) => {
                    self.check_table(t);
                    self.check_json_keys(&t.record);
                }
                Decl::Database(d) => self.check_database(d),
                // Checked for the whole file below, as one declaration.
                Decl::Traces(_) => {}
                Decl::Identity(i) => self.check_identity(i),
                Decl::Connector(c) => self.check_connector(c),
                Decl::Tool(t) => self.check_retired_json_decoder_tool(t),
                Decl::Store(_)
                | Decl::Import(_)
                | Decl::Effect(_)
                | Decl::Schedule(_) => {}
                Decl::Extend(ext) => {
                    // Typecheck agent method bodies the same way free
                    // agents are checked.
                    // Tool methods have no body. Prompt methods
                    // have a template (not a code block) — its
                    // typecheck is the same as a free prompt's.
                    for method in &ext.methods {
                        match &method.kind {
                            ExtendMethodKind::Agent(a) => self.check_agent(a),
                            ExtendMethodKind::Prompt(p) => self.check_prompt(p),
                            ExtendMethodKind::Tool(_) => {}
                        }
                    }
                }
            }
        }
        self.check_traces(file);
    }

    fn has_known_approval_label(&self, label: &str) -> bool {
        self.tools_by_id.values().any(|tool| {
            matches!(tool.effect, Effect::Dangerous) && pascal_case(&tool.name.name) == label
        })
    }

    /// Provenance Propagation D5 (slice 7): record `span` if `from`
    /// is a `Grounded<...>` type being silently coerced into a
    /// non-grounded `to` slot. Call at every slot-check site that
    /// invokes `is_assignable_to` (return / let / yield / call-arg /
    /// struct-field-init), plus control-flow conditions that strip
    /// grounded via `.ungrounded()`. The recorded spans drive IR
    /// lowering to emit visible `UnwrapGrounded` nodes; missing a
    /// site is a silent moat hole, so the enumeration is the
    /// load-bearing property here, not the helper's wording.
    fn record_if_grounded_coercion(&mut self, from: &Type, to: &Type, span: Span) {
        if matches!(from, Type::Grounded(_)) && !matches!(to, Type::Grounded(_)) {
            self.grounded_coercion_sites.insert(span);
        }
    }

    fn bind_params(&mut self, params: &[Param]) {
        for p in params {
            if let Some(Binding::Local(local_id)) = self.bindings.get(&p.name.span) {
                let ty = self.type_ref_to_type(&p.ty);
                if matches!(ty, Type::Weak(_, _)) {
                    self.weak_refresh.insert(*local_id, self.effect_frontier);
                } else {
                    self.weak_refresh.remove(local_id);
                }
                self.local_types.insert(*local_id, ty);
            }
        }
    }

    // ------------------------------------------------------------
    // Blocks and statements.
    // ------------------------------------------------------------

    // ------------------------------------------------------------
    // Expressions.
    // ------------------------------------------------------------
}

mod actions;
mod call;
mod case;
mod decl;
mod decl_eval;
mod traces;
mod decl_extern_c;
mod decimal;
mod bytes;
mod crypto_arguments;
mod secrets;
mod response_boundaries;
mod currency;
mod time;
mod form;
mod live;
mod decl_grounded_pure;
mod decl_replayability;
mod effect_decl;
mod lambda_check;
mod fn_check;
mod struct_literal_check;
mod match_check;
mod match_coverage;
mod expr;
mod import_call;
mod json_methods;
mod regex_methods;
mod request_text;
mod signature_types;
mod tables;
mod ops;
mod prompt;
mod stmt;
mod stream;
mod types;

pub use case::approve_label_for;
pub(crate) use case::{pascal_case, snake_case};

fn is_weakable_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::String | Type::Struct(_) | Type::ImportedStruct(_) | Type::List(_)
    )
}

/// Why `refinement` cannot refine a `type_name` slot, if it cannot: the
/// one rule fields and parameters share. `what` names the slot.
fn refinement_problem(refinement: &corvid_ast::Refinement, type_name: &str, what: &str) -> Option<String> {
    match *refinement {
        corvid_ast::Refinement::Between { .. } if type_name != "Int" => Some(format!(
            "refinement `{}` requires an Int {what}, but the {what} is `{type_name}`",
            refinement.describe()
        )),
        corvid_ast::Refinement::LenBetween { .. } if type_name != "String" => Some(format!(
            "refinement `{}` requires a String {what}, but the {what} is `{type_name}`",
            refinement.describe()
        )),
        corvid_ast::Refinement::Between { min, max } if min > max => {
            Some(format!("refinement `{}` has min > max", refinement.describe()))
        }
        corvid_ast::Refinement::LenBetween { min, max } if min > max => {
            Some(format!("refinement `{}` has min > max", refinement.describe()))
        }
        _ => None,
    }
}

/// The effect registry a program's checks compose costs with: its own
/// effect declarations, and the public effects of the modules it imports.
/// Local declarations win on a name collision. The checker's `@budget`
/// analysis and `corvid graph` both use it, so they agree on every cost.
pub fn effect_registry_for(
    file: &File,
    modules: Option<&corvid_resolve::ModuleResolution>,
    config: Option<&crate::config::CorvidConfig>,
) -> crate::effects::EffectRegistry {
    let effect_decls: Vec<&corvid_ast::EffectDecl> = file
        .decls
        .iter()
        .filter_map(|d| {
            if let Decl::Effect(e) = d {
                Some(e)
            } else {
                None
            }
        })
        .collect();
    let mut owned_decls: Vec<corvid_ast::EffectDecl> =
        effect_decls.iter().cloned().cloned().collect();
    // Slice 45o: imported PUBLIC effects join the registry so
    // `import "./std/json" use json_egress_read` + `uses
    // json_egress_read` composes exactly like a locally-declared
    // effect. Local declarations win on name collisions (they are
    // pushed after, and `from_decls` is last-wins), matching the
    // shadowing story everywhere else.
    if let Some(modules) = modules {
        let mut imported: Vec<corvid_ast::EffectDecl> = Vec::new();
        for module in modules.root_imports.values() {
            for decl in &module.file.decls {
                if let Decl::Effect(e) = decl {
                    if !matches!(e.visibility, corvid_ast::Visibility::Private) {
                        imported.push(e.clone());
                    }
                }
            }
        }
        imported.extend(owned_decls);
        owned_decls = imported;
    }
    crate::effects::EffectRegistry::from_decls_with_config(&owned_decls, config)
}
