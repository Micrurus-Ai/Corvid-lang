//! Lower a typed AST into IR.
//!
//! Every AST construct maps to an IR construct. References are resolved
//! via the resolver's side-table; types come from the checker's side-table.

mod stream;

use crate::imports::{build_imported_def_ids, ImportedDefKey};
use crate::types::*;
use corvid_ast::{
    AgentAttribute, AgentDecl, BinaryOp, Block, ConnectorAuth, ConnectorDecl, Decl, Effect,
    EvalAssert, EvalDecl, Expr, ExtendMethodKind, ExternAbi, File, FixtureDecl, HttpRouteDecl,
    Ident, ImportDecl, ImportSource, Literal, MockDecl, OperationDecl, Param, PromptDecl, ReplayArm,
    ReplayPattern, ServerDecl, Span, Stmt, TestDecl, ToolArgPattern, ToolDecl, TypeDecl, TypeRef,
    UnaryOp,
};
use corvid_resolve::{
    resolver::MethodEntry, Binding, BuiltIn, DeclKind, DefId, LocalId, ModuleResolution, Resolved,
    ResolvedModule, SymbolTable,
};
use corvid_types::effects::{canonical_dimension_name, numeric_constraint_value, EffectRegistry};
use corvid_types::{Checked, ImportedCallKind, ImportedCallTarget, Type};
use std::collections::HashMap;
use std::path::PathBuf;

/// Entry point: produce an `IrFile` from parsed/resolved/checked sources.
pub fn lower(file: &File, resolved: &Resolved, checked: &Checked) -> IrFile {
    let imported_def_ids = HashMap::new();
    let mut l = Lowerer::new(file, resolved, checked, None, None, &imported_def_ids);
    l.lower_file(file)
}

/// Lower with cross-file module metadata. This is the file-backed
/// counterpart to [`lower`]: it preserves successful `alias.Type`
/// resolutions as `Type::ImportedStruct` in IR signatures instead
/// of degrading them to `Unknown` after typechecking.
pub fn lower_with_modules(
    file: &File,
    resolved: &Resolved,
    checked: &Checked,
    modules: &ModuleResolution,
    checked_modules: &HashMap<PathBuf, Checked>,
) -> IrFile {
    let imported_def_ids = build_imported_def_ids(resolved, modules);
    let mut l = Lowerer::new(file, resolved, checked, Some(modules), None, &imported_def_ids);
    let mut ir = l.lower_file(file);

    let mut loaded = modules.all_modules.values().collect::<Vec<_>>();
    loaded.sort_by(|a, b| a.path.cmp(&b.path));
    for module in loaded {
        let Some(module_checked) = checked_modules.get(&module.path) else {
            continue;
        };
        let mut module_lowerer = Lowerer::new(
            module.file.as_ref(),
            module.resolved.as_ref(),
            module_checked,
            Some(modules),
            Some(module),
            &imported_def_ids,
        );
        let module_ir = module_lowerer.lower_file(module.file.as_ref());
        ir.types.extend(module_ir.types);
        ir.tools.extend(module_ir.tools);
        ir.prompts.extend(module_ir.prompts);
        ir.agents.extend(module_ir.agents);
    }

    ir
}

/// The revision of a table row that has never been stored (B09).
fn zero_revision(span: Span) -> IrExpr {
    IrExpr { kind: IrExprKind::Literal(IrLiteral::Int(0)), ty: Type::Int, span }
}

/// High base for synthetic per-route handler-agent `DefId`s (slice
/// 52a). Real declarations use small sequential ids, so this never
/// collides; synthetic agents are invoked only by name.
const SYNTHETIC_ROUTE_AGENT_DEF_ID_BASE: u32 = 0x4000_0000;

/// The stable name of the synthetic handler agent for a route (slice
/// 52a): `__route__<METHOD>__<mangled-path>`. Deterministic so the
/// route and its handler agent agree without threading extra state.
pub fn synthetic_route_agent_name(method: corvid_ast::HttpMethod, path: &str) -> String {
    let mangled: String = path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("__route__{}__{}", method.as_str(), mangled)
}

/// The synthetic struct type of the `actor` bound in an authenticated
/// route body (slice 52a) — mirrors the checker's `actor_type()`:
/// id / tenant / display_name / roles / permissions.
fn actor_route_params_type() -> Type {
    Type::RouteParams(vec![
        ("id".to_string(), Type::String),
        ("tenant".to_string(), Type::String),
        ("display_name".to_string(), Type::String),
        ("roles".to_string(), Type::List(Box::new(Type::String))),
        ("permissions".to_string(), Type::List(Box::new(Type::String))),
    ])
}

struct Lowerer<'a> {
    /// The file being lowered, for declarations expressions refer to.
    file: &'a File,
    python_members: HashMap<(DefId, String), (DefId, String)>,
    symbols: &'a SymbolTable,
    bindings: &'a HashMap<Span, Binding>,
    types: &'a HashMap<Span, Type>,
    /// Per-receiver-type method side-table from the
    /// resolver. `lower_file` walks `Decl::Extend` blocks and looks
    /// up each method's allocated DefId here so the IR emits methods
    /// alongside free decls in the per-kind vectors.
    methods: &'a HashMap<DefId, HashMap<String, MethodEntry>>,
    variant_owners: &'a HashMap<DefId, (DefId, u32)>,
    /// Effect name → confidence gate threshold, populated from
    /// `EffectDecl`s with `trust: autonomous_if_confident(T)` dimension.
    /// Used during tool lowering to set `IrTool.confidence_gate`.
    confidence_gates: HashMap<String, f64>,
    effect_registry: EffectRegistry,
    module_resolution: Option<&'a ModuleResolution>,
    current_module: Option<&'a ResolvedModule>,
    imported_def_ids: &'a HashMap<ImportedDefKey, DefId>,
    imported_calls: &'a HashMap<Span, ImportedCallTarget>,
    /// Provenance Propagation D5 (slice 7b): every value-expression
    /// span the typechecker flagged (`Checked.grounded_coercion_sites`)
    /// as the site of a silent `Grounded<T> -> T` strip. `lower_expr`
    /// wraps the produced `IrExpr` in `IrExprKind::UnwrapGrounded` at
    /// each match, so `@grounded_pure` (slice 9) can fail a function
    /// whose body launders a grounded value through any slot-check.
    grounded_coercion_sites: &'a std::collections::HashSet<Span>,
    /// The checker's resolved type for every type reference
    /// (`Checked.type_refs`). Lowering never resolves a type reference
    /// itself; it reads this table and remaps ids across modules.
    type_refs: &'a HashMap<Span, Type>,
    wrapping_arithmetic: bool,
    /// Route-local side-table (slice 52a): per-route `path`/`query`/
    /// `body`/`actor` `LocalId`s, so `lower_server` can build a
    /// synthetic handler agent the runtime executes.
    route_locals: &'a HashMap<Span, corvid_resolve::RouteLocals>,
    /// Monotonic `DefId` allocator for synthetic per-route handler
    /// agents (slice 52a). Starts at a high base so it never collides
    /// with a real declaration's id; these agents are only ever
    /// invoked by name, so the id just needs to be unique in the
    /// runtime's `agents_by_id` map.
    next_synthetic_def_id: u32,
}

impl<'a> Lowerer<'a> {
    fn new(
        file: &'a File,
        resolved: &'a Resolved,
        checked: &'a Checked,
        module_resolution: Option<&'a ModuleResolution>,
        current_module: Option<&'a ResolvedModule>,
        imported_def_ids: &'a HashMap<ImportedDefKey, DefId>,
    ) -> Self {
        Self {
            file,
            python_members: HashMap::new(),
            symbols: &resolved.symbols,
            bindings: &resolved.bindings,
            types: &checked.types,
            methods: &resolved.methods,
            variant_owners: &resolved.variant_owners,
            confidence_gates: HashMap::new(),
            effect_registry: EffectRegistry::default(),
            module_resolution,
            current_module,
            imported_def_ids,
            imported_calls: &checked.imported_calls,
            grounded_coercion_sites: &checked.grounded_coercion_sites,
            type_refs: &checked.type_refs,
            wrapping_arithmetic: false,
            route_locals: &resolved.route_locals,
            next_synthetic_def_id: SYNTHETIC_ROUTE_AGENT_DEF_ID_BASE,
        }
    }

    fn remap_def_id(&self, def_id: DefId) -> DefId {
        let Some(module) = self.current_module else {
            return def_id;
        };
        self.imported_def_ids
            .get(&ImportedDefKey {
                module_path: module.path.to_string_lossy().into_owned(),
                def_id,
            })
            .copied()
            .unwrap_or(def_id)
    }

    fn remap_imported_target(&self, target: &ImportedCallTarget) -> DefId {
        self.imported_def_ids
            .get(&ImportedDefKey {
                module_path: target.module_path.clone(),
                def_id: target.def_id,
            })
            .copied()
            .unwrap_or(target.def_id)
    }

    /// Align a struct *type*'s `DefId` with the merged `ir.types` layout
    /// table (which, with imported-struct *construction*, is keyed by the
    /// cross-module-remapped id from `build_imported_def_ids`).
    ///
    /// Two cases:
    /// - `Type::Struct(id)` — a module agent's local struct carries the
    ///   module's own `DefId` in `checked.types`; `remap_def_id` maps it
    ///   to the appended (remapped) id. For the root file this is a no-op
    ///   (`current_module == None`), so root-file local structs are
    ///   unchanged.
    /// - `Type::ImportedStruct` — the root-file resolvers emit the
    ///   original per-module id; translate via `imported_def_ids`.
    ///
    /// Consumers that read the struct *name* are unaffected.
    fn remap_struct_type(&self, ty: Type) -> Type {
        match ty {
            Type::Struct(id) => Type::Struct(self.remap_def_id(id)),
            Type::ImportedStruct(mut imported) => {
                let Some(remapped) = self.imported_def_ids.get(&ImportedDefKey {
                    module_path: imported.module_path.clone(),
                    def_id: imported.def_id,
                }) else {
                    return Type::ImportedStruct(imported);
                };
                // Inside an imported module, a type from another module is
                // simply a record in the merged IR type table. The root
                // file keeps `ImportedStruct` so its signatures still name
                // the module the type came from.
                if self.current_module.is_some() {
                    return Type::Struct(*remapped);
                }
                imported.def_id = *remapped;
                Type::ImportedStruct(imported)
            }
            other => other,
        }
    }

    /// [`Self::remap_struct_type`] applied to every record id inside
    /// `ty`, for types the runtime walks (the target of a JSON decode).
    fn remap_struct_type_deep(&self, ty: Type) -> Type {
        let deep = |inner: Box<Type>| Box::new(self.remap_struct_type_deep(*inner));
        match ty {
            Type::Option(inner) => Type::Option(deep(inner)),
            Type::List(inner) => Type::List(deep(inner)),
            Type::Stream(inner) => Type::Stream(deep(inner)),
            Type::Tainted(inner) => Type::Tainted(deep(inner)),
            Type::Grounded(inner) => Type::Grounded(deep(inner)),
            Type::Partial(inner) => Type::Partial(deep(inner)),
            Type::ResumeToken(inner) => Type::ResumeToken(deep(inner)),
            Type::Upload(inner) => Type::Upload(deep(inner)),
            Type::Page(inner) => Type::Page(deep(inner)),
            Type::Weak(inner, effects) => Type::Weak(deep(inner), effects),
            Type::Map(key, item) => Type::Map(deep(key), deep(item)),
            Type::Result(ok, err) => Type::Result(deep(ok), deep(err)),
            Type::Function { params, ret, effect } => Type::Function {
                params: params.into_iter().map(|p| self.remap_struct_type_deep(p)).collect(),
                ret: deep(ret),
                effect,
            },
            Type::RouteParams(fields) => Type::RouteParams(
                fields.into_iter().map(|(name, ty)| (name, self.remap_struct_type_deep(ty))).collect(),
            ),
            other => self.remap_struct_type(other),
        }
    }

    /// Scan the file's effect declarations for `trust: autonomous_if_confident(T)`
    /// dimension values and populate the confidence_gates table.
    fn populate_confidence_gates(&mut self, file: &File) {
        for decl in &file.decls {
            let Decl::Effect(effect) = decl else { continue };
            for dim in &effect.dimensions {
                if dim.name.name == "trust" {
                    if let corvid_ast::DimensionValue::ConfidenceGated { threshold, .. } =
                        &dim.value
                    {
                        self.confidence_gates
                            .insert(effect.name.name.clone(), *threshold);
                    }
                }
            }
        }
    }

    fn lower_file(&mut self, file: &File) -> IrFile {
        self.populate_confidence_gates(file);
        self.populate_effect_registry(file);
        let mut imports = Vec::new();
        let mut types = Vec::new();
        let mut tools = Vec::new();
        let mut prompts = Vec::new();
        let mut agents = Vec::new();
        let mut evals = Vec::new();
        let mut tests = Vec::new();
        let mut fixtures = Vec::new();
        let mut mocks = Vec::new();
        let mut servers = Vec::new();
        let mut models = Vec::new();
        let mut connectors = Vec::new();

        for decl in &file.decls {
            let Decl::Import(import) = decl else { continue };
            if import.python_functions.is_empty() { continue; }
            let alias = import.alias.as_ref().map(|a| &a.name).unwrap_or(&import.module);
            let alias_id = self.symbols.lookup_def(alias).expect("resolved Python import");
            for function in &import.python_functions {
                let Some(Binding::Decl(id)) = self.bindings.get(&function.name.span) else {
                    unreachable!("resolved Python function signature")
                };
                let name = format!("python:{}.{}", import.module, function.name.name);
                let declaration = ToolDecl {
                    name: Ident::new(name.clone(), function.name.span),
                    params: function.params.clone(),
                    return_ty: function.return_ty.clone(),
                    return_ownership: None,
                    breaker: None,
                    effect: Effect::Safe,
                    effect_row: import.effect_row.clone(),
                    visibility: corvid_ast::Visibility::Private,
                    span: function.name.span,
                };
                let mut tool = self.lower_tool_with_id(&declaration, *id);
                tool.python_approval_required = corvid_types::effects::effect_row_trust_requires_approval(
                    &import.effect_row, &self.effect_registry,
                ).is_some();
                tool.python = Some((import.module.clone(), function.name.name.clone()));
                self.python_members.insert((alias_id, function.name.name.clone()), (tool.id, name));
                tools.push(tool);
            }
        }
        for decl in &file.decls {
            match decl {
                Decl::Import(i) => imports.push(self.lower_import(i)),
                Decl::Type(t) => types.push(self.lower_type(t)),
                Decl::Store(_) => {}
                // A table is a record type; its storage travels with each
                // operation (IrExprKind::TableOp).
                Decl::Table(t) => types.push(self.lower_type(&t.record)),
                Decl::Database(_) => {}
                Decl::Tool(t) => tools.push(self.lower_tool(t)),
                Decl::Prompt(p) => prompts.push(self.lower_prompt(p)),
                Decl::Agent(a) => agents.push(self.lower_agent(a)),
                Decl::Fn(f) => agents.push(self.lower_fn(f)),
                Decl::Eval(e) => evals.push(self.lower_eval(e)),
                Decl::Test(t) => tests.push(self.lower_test(t)),
                Decl::Fixture(f) => fixtures.push(self.lower_fixture(f)),
                Decl::Mock(m) => mocks.push(self.lower_mock(m)),
                Decl::Effect(_) => {}
                Decl::Model(m) => {
                    // Slice 46a: sampling fields made model decls
                    // load-bearing at dispatch. Other fields stay
                    // checker-side (capability routing reads the
                    // AST catalog directly).
                    let get = |key: &str| {
                        m.fields.iter().find_map(|f| {
                            if f.name.name != key {
                                return None;
                            }
                            match &f.value {
                                corvid_ast::DimensionValue::Number(n) => Some(*n),
                                _ => None,
                            }
                        })
                    };
                    models.push(IrModel {
                        name: m.name.name.clone(),
                        temperature: get("temperature"),
                        top_p: get("top_p"),
                        max_tokens: get("max_tokens").map(|n| n as u64),
                        context_window: get("context_window").map(|n| n as u64),
                        span: m.span,
                    });
                }
                Decl::Server(s) => {
                    let server = self.lower_server(s);
                    // Slice 52a: emit a synthetic handler agent per route
                    // so `corvid serve` executes the route body through
                    // the ordinary agent machinery.
                    for (ir_route, ast_route) in server.routes.iter().zip(s.routes.iter()) {
                        if let Some(handler) = self.build_route_handler_agent(ir_route, ast_route) {
                            agents.push(handler);
                        }
                    }
                    servers.push(server);
                }
                Decl::Identity(_) => {
                    // Slice 51g: an identity block is a static auth
                    // configuration surface (providers + session). It
                    // is read from the AST by the application contract
                    // and the auth-runtime wiring, not lowered into
                    // executable IR — the OAuth routes it implies land
                    // in slice 51h.
                }
                Decl::Schedule(_) => {
                    // Phase 38D2: schedules are static audit/runtime
                    // manifests. They do not lower into executable IR
                    // until the scheduler runner slice.
                }
                Decl::Connector(c) => {
                    // 52g-3: each `operation` lowers to a callable IrTool
                    // (so a call to it types + dispatches like any tool)
                    // plus an IrConnector dispatch record carrying the
                    // HTTP metadata the runtime turns into a
                    // `ConnectorRequest`.
                    let connector = self.lower_connector(c, &mut tools);
                    connectors.push(connector);
                }
                Decl::Extend(ext) => {
                    // Lower each method into the appropriate per-kind
                    // IR vector. Methods get
                    // their `DefId` from the resolver's method side
                    // table (NOT the by-name namespace, since two
                    // types can share method names like `total`).
                    let Some(type_def_id) = self.symbols.lookup_def(&ext.type_name.name) else {
                        continue;
                    };
                    let Some(method_table) = self.methods.get(&type_def_id) else {
                        continue;
                    };
                    for method in &ext.methods {
                        let Some(entry) = method_table.get(&method.name().name) else {
                            continue;
                        };
                        match &method.kind {
                            ExtendMethodKind::Tool(t) => {
                                tools.push(self.lower_tool_with_id(t, entry.def_id));
                            }
                            ExtendMethodKind::Prompt(p) => {
                                prompts.push(self.lower_prompt_with_id(p, entry.def_id));
                            }
                            ExtendMethodKind::Agent(a) => {
                                agents.push(self.lower_agent_with_id(a, entry.def_id));
                            }
                        }
                    }
                }
            }
        }

        IrFile {
            import_fingerprint: None,
            imports,
            types,
            tools,
            prompts,
            agents,
            evals,
            tests,
            fixtures,
            mocks,
            servers,
            models,
            connectors,
        }
    }

    /// Lower a `connector` block (slice 52g-3). Each `operation` is
    /// appended to `tools` as an ordinary callable `IrTool`, and the
    /// connector's HTTP dispatch metadata (base URL, credentials,
    /// per-operation method/path/body/error-map, reliability) is
    /// returned as an `IrConnector` keyed back to those tools by DefId.
    fn lower_connector(&self, c: &ConnectorDecl, tools: &mut Vec<IrTool>) -> IrConnector {
        let mut operations = Vec::with_capacity(c.operations.len());
        for op in &c.operations {
            let tool_id = self
                .symbols
                .lookup_def(&op.name.name)
                .expect("connector operation missing from symbol table");
            tools.push(self.lower_operation_as_tool(op, tool_id, c.circuit_breaker));
            operations.push(IrOperation {
                name: op.name.name.clone(),
                tool_id: self.remap_def_id(tool_id),
                method: op.method,
                path: op.path.clone(),
                body: op.body.as_ref().map(|b| IrOperationBody {
                    param_name: b.param.name.clone(),
                    encoding: b.encoding,
                }),
                query: op.query.as_ref().map(|q| q.name.clone()),
                url_param: op.url_param.as_ref().map(|u| u.name.clone()),
                response_text: op.response_text,
                on_failure: op.on_failure.as_ref().map(|v| v.name.clone()),
                headers: op
                    .headers
                    .iter()
                    .map(|h| crate::types::IrOperationHeader {
                        name: h.name.clone(),
                        value: match &h.value {
                            corvid_ast::OperationHeaderValue::Param(p) => {
                                crate::types::IrOperationHeaderValue::Param(p.name.clone())
                            }
                            corvid_ast::OperationHeaderValue::Literal(text) => {
                                crate::types::IrOperationHeaderValue::Literal(text.clone())
                            }
                        },
                    })
                    .collect(),
                error_map: op
                    .error_map
                    .iter()
                    .map(|m| IrStatusErrorMapping {
                        status: m.status,
                        variant: m.variant.name.clone(),
                    })
                    .collect(),
                timeout_secs: c.operation_timeout_secs(op),
                mock: op.mock.as_ref().map(|e| self.lower_expr(e)),
                protocol: op.protocol.clone(),
                span: op.span,
            });
        }
        IrConnector {
            name: c.name.name.clone(),
            base_url: c.base_url.clone(),
            auth: c.auth.as_ref().map(lower_connector_auth),
            retry: c.retry,
            rate_limit: c.rate_limit.map(|r| IrRateLimit {
                limit: r.limit,
                window_secs: r.window_secs,
            }),
            circuit_breaker: c.circuit_breaker,
            timeout_secs: c.timeout_secs,
            redirects: c.redirects,
            modes: c.modes.clone(),
            operations,
            span: c.span,
        }
    }

    /// Lower a connector `operation` into a callable `IrTool`. An
    /// operation IS a tool with a declarative HTTP body — same
    /// signature shape (params / effect / effect row / return), so its
    /// effect row composes with budgets / approval / replay / taint
    /// exactly like a hand-written tool's. It carries no circuit
    /// breaker of its own (connector-level reliability lands in 52g-4).
    fn lower_operation_as_tool(
        &self,
        op: &OperationDecl,
        id: DefId,
        circuit_breaker: Option<u64>,
    ) -> IrTool {
        let mut confidence_gate: Option<f64> = None;
        for effect_ref in &op.effect_row.effects {
            if let Some(&threshold) = self.confidence_gates.get(&effect_ref.name.name) {
                confidence_gate = match confidence_gate {
                    Some(current) => Some(current.max(threshold)),
                    None => Some(threshold),
                };
            }
        }
        let produces_grounded = corvid_types::effects::effect_row_is_grounded(
            &op.effect_row,
            &self.effect_registry,
        );
        let effect_names: Vec<String> = op
            .effect_row
            .effects
            .iter()
            .map(|e| e.name.name.clone())
            .collect();
        let effect_refs: Vec<&str> = effect_names.iter().map(|n| n.as_str()).collect();
        let profile = self.effect_registry.compose(&effect_refs);
        let effect_cost = numeric_profile_dimension(&profile, "cost");
        let effect_reversible = profile_is_reversible(&profile);
        IrTool {
            // The connector's `circuit_breaker: N` becomes the
            // operation-tool's breaker threshold (slice 52g-3c-5), so the
            // existing Tool-arm circuit-breaker machinery trips a
            // repeatedly-failing connector operation for free.
            python: None,
            python_approval_required: false,
            breaker: circuit_breaker,
            id: self.remap_def_id(id),
            name: op.name.name.clone(),
            params: self.lower_params(&op.params),
            return_ty: self.type_ref_to_type(&op.return_ty),
            effect: op.effect,
            effect_names,
            confidence_gate,
            produces_grounded,
            effect_cost,
            effect_reversible,
            span: op.span,
        }
    }

    fn lower_server(&self, s: &ServerDecl) -> IrServer {
        let id = self
            .symbols
            .lookup_def(&s.name.name)
            .expect("server missing from symbol table");
        IrServer {
            id: self.remap_def_id(id),
            name: s.name.name.clone(),
            routes: s.routes.iter().map(|r| self.lower_route(r)).collect(),
            span: s.span,
        }
    }

    fn lower_route(&self, r: &HttpRouteDecl) -> IrRoute {
        IrRoute {
            method: r.method,
            path: r.path.clone(),
            path_params: r
                .path_params
                .iter()
                .map(|p| IrRoutePathParam {
                    name: p.name.name.clone(),
                    ty: self.type_ref_to_type(&p.ty),
                    span: p.span,
                })
                .collect(),
            query_ty: r.query_ty.as_ref().map(|t| self.type_ref_to_type(t)),
            body_ty: r.body_ty.as_ref().map(|t| self.type_ref_to_type(t)),
            form_encoded: r.form_encoded,
            response_kind: r.response.kind,
            response_ty: self.type_ref_to_type(&r.response.ty),
            effect_names: r
                .effect_row
                .effects
                .iter()
                .map(|e| e.name.name.clone())
                .collect(),
            body: self.lower_block(&r.body),
            handler_agent: synthetic_route_agent_name(r.method, &r.path),
            upload_policy: r.upload.clone(),
            approval_policy: r.approval.clone(),
            upload_format: r.body_ty.as_ref().and_then(upload_format_tag),
            policy: r.policy.as_ref().map(|p| crate::types::IrRoutePolicy {
                authenticated: p.authenticated,
                roles: p.roles.clone(),
                permissions: p.permissions.clone(),
            }),
            span: r.span,
        }
    }

    /// Build the synthetic per-route handler agent (slice 52a): params
    /// = `path` / `query` / `body` / `actor` reusing the route's
    /// resolver `LocalId`s, body = the lowered route body. Returns
    /// `None` if the resolver recorded no locals for this route (should
    /// not happen for a well-formed route).
    fn build_route_handler_agent(
        &mut self,
        ir_route: &IrRoute,
        ast_route: &HttpRouteDecl,
    ) -> Option<IrAgent> {
        let locals = self.route_locals.get(&ast_route.span).copied()?;
        let mut params = Vec::new();
        // `path` — a synthetic struct of the declared path params.
        let path_fields: Vec<(String, Type)> = ir_route
            .path_params
            .iter()
            .map(|p| (p.name.clone(), p.ty.clone()))
            .collect();
        params.push(IrParam {
            name: "path".to_string(),
            local_id: locals.path,
            ty: Type::RouteParams(path_fields),
            span: ast_route.span,
        });
        if let (Some(query_local), Some(query_ty)) = (locals.query, ir_route.query_ty.clone()) {
            params.push(IrParam {
                name: "query".to_string(),
                local_id: query_local,
                ty: query_ty,
                span: ast_route.span,
            });
        }
        if let (Some(body_local), Some(body_ty)) = (locals.body, ir_route.body_ty.clone()) {
            params.push(IrParam {
                name: "body".to_string(),
                local_id: body_local,
                ty: body_ty,
                span: ast_route.span,
            });
        }
        if let Some(actor_local) = locals.actor {
            params.push(IrParam {
                name: "actor".to_string(),
                local_id: actor_local,
                ty: actor_route_params_type(),
                span: ast_route.span,
            });
        }

        let id = DefId(self.next_synthetic_def_id);
        self.next_synthetic_def_id += 1;
        Some(IrAgent {
            id,
            name: ir_route.handler_agent.clone(),
            extern_abi: None,
            params,
            return_ty: ir_route.response_ty.clone(),
            cost_budget: None,
            wrapping_arithmetic: self.wrapping_arithmetic,
            is_replayable: false,
            pure_fn: false,
            retry_max_attempts: None,
            retry_backoff_ms: None,
            idempotency_key_param: None,
            body: ir_route.body.clone(),
            span: ast_route.span,
            borrow_sig: None,
        })
    }

    fn lower_import(&self, i: &ImportDecl) -> IrImport {
        let alias_name = i.alias.as_ref().map(|a| a.name.clone());
        let binding_name = alias_name.clone().unwrap_or_else(|| i.module.clone());
        let id = self
            .symbols
            .lookup_def(&binding_name)
            .expect("import binding missing from symbol table");
        let source = match i.source {
            ImportSource::Python => IrImportSource::Python,
            ImportSource::Corvid => IrImportSource::Corvid,
            ImportSource::RemoteCorvid => IrImportSource::RemoteCorvid,
            ImportSource::PackageCorvid => IrImportSource::PackageCorvid,
        };
        IrImport {
            id,
            source,
            module: i.module.clone(),
            content_hash: i.content_hash.as_ref().map(|hash| IrImportContentHash {
                algorithm: hash.algorithm.clone(),
                hex: hash.hex.clone(),
            }),
            alias: alias_name,
            span: i.span,
        }
    }

    fn lower_eval(&self, e: &EvalDecl) -> IrEval {
        let id = self
            .symbols
            .lookup_def(&e.name.name)
            .expect("eval missing from symbol table");
        IrEval {
            id: self.remap_def_id(id),
            name: e.name.name.clone(),
            body: self.lower_block(&e.body),
            assertions: e
                .assertions
                .iter()
                .map(|assertion| self.lower_eval_assert(assertion))
                .collect(),
            span: e.span,
        }
    }

    fn lower_eval_assert(&self, assertion: &EvalAssert) -> IrEvalAssert {
        match assertion {
            EvalAssert::Value {
                expr,
                confidence,
                runs,
                span,
            } => IrEvalAssert::Value {
                expr: self.lower_expr(expr),
                confidence: *confidence,
                runs: *runs,
                span: *span,
            },
            EvalAssert::Snapshot { expr, span } => IrEvalAssert::Snapshot {
                expr: self.lower_expr(expr),
                span: *span,
            },
            EvalAssert::Called { tool, span } => {
                let def_id = match self.bindings.get(&tool.span) {
                    Some(Binding::Decl(def_id)) => *def_id,
                    _ => panic!("eval called assertion missing resolved callable"),
                };
                IrEvalAssert::Called {
                    def_id: self.remap_def_id(def_id),
                    name: tool.name.clone(),
                    span: *span,
                }
            }
            EvalAssert::Approved { label, span } => IrEvalAssert::Approved {
                label: label.name.clone(),
                span: *span,
            },
            EvalAssert::Similar {
                expr,
                expected,
                min,
                span,
            } => IrEvalAssert::Similar {
                expr: self.lower_expr(expr),
                expected: self.lower_expr(expected),
                min: *min,
                span: *span,
            },
            EvalAssert::Judged {
                expr,
                criteria,
                min,
                span,
            } => IrEvalAssert::Judged {
                expr: self.lower_expr(expr),
                criteria: criteria.clone(),
                min: *min,
                span: *span,
            },
            EvalAssert::Cost { op, bound, span } => IrEvalAssert::Cost {
                op: *op,
                bound: *bound,
                span: *span,
            },
            EvalAssert::Ordering {
                before,
                after,
                span,
            } => {
                let before_id = match self.bindings.get(&before.span) {
                    Some(Binding::Decl(def_id)) => *def_id,
                    _ => panic!("eval ordering assertion missing resolved `before` callable"),
                };
                let after_id = match self.bindings.get(&after.span) {
                    Some(Binding::Decl(def_id)) => *def_id,
                    _ => panic!("eval ordering assertion missing resolved `after` callable"),
                };
                IrEvalAssert::Ordering {
                    before_id: self.remap_def_id(before_id),
                    before_name: before.name.clone(),
                    after_id: self.remap_def_id(after_id),
                    after_name: after.name.clone(),
                    span: *span,
                }
            }
        }
    }

    fn lower_type(&self, t: &TypeDecl) -> IrType {
        let id = self
            .symbols
            .lookup_def(&t.name.name)
            .expect("type missing from symbol table");
        let fields = t
            .fields
            .iter()
            .map(|f| IrField {
                name: f.name.name.clone(),
                ty: self.type_ref_to_type(&f.ty),
                refinement: f.refinement,
                span: f.span,
            })
            .collect();
        let variants = t
            .variants
            .iter()
            .map(|v| IrEnumVariant {
                name: v.name.name.clone(),
                fields: v
                    .fields
                    .iter()
                    .map(|f| IrField {
                        name: f.name.name.clone(),
                        ty: self.type_ref_to_type(&f.ty),
                        refinement: f.refinement,
                        span: f.span,
                    })
                    .collect(),
                span: v.span,
            })
            .collect();
        IrType {
            id: self.remap_def_id(id),
            name: t.name.name.clone(),
            fields,
            variants,
            span: t.span,
        }
    }

    fn lower_tool(&self, t: &ToolDecl) -> IrTool {
        let id = self
            .symbols
            .lookup_def(&t.name.name)
            .expect("tool missing from symbol table");
        self.lower_tool_with_id(t, id)
    }

    /// Lower a tool decl whose DefId was allocated outside
    /// the by-name namespace (i.e. it's a method inside an `extend`
    /// block, looked up via the methods side-table rather than by
    /// name).
    fn lower_tool_with_id(&self, t: &ToolDecl, id: DefId) -> IrTool {
        // If any of the tool's declared effects has `autonomous_if_confident(T)`,
        // carry the strictest threshold as the confidence gate.
        let mut confidence_gate: Option<f64> = None;
        for effect_ref in &t.effect_row.effects {
            if let Some(&threshold) = self.confidence_gates.get(&effect_ref.name.name) {
                confidence_gate = match confidence_gate {
                    Some(current) => Some(current.max(threshold)),
                    None => Some(threshold),
                };
            }
        }

        let produces_grounded = corvid_types::effects::effect_row_is_grounded(
            &t.effect_row,
            &self.effect_registry,
        );
        let effect_names: Vec<String> = t
            .effect_row
            .effects
            .iter()
            .map(|e| e.name.name.clone())
            .collect();
        // Composed cost + reversibility, pre-computed from the registry
        // for effect-aware `parallel` scheduling (slice 52d-1).
        let effect_refs: Vec<&str> = effect_names.iter().map(|n| n.as_str()).collect();
        let profile = self.effect_registry.compose(&effect_refs);
        let effect_cost = numeric_profile_dimension(&profile, "cost");
        let effect_reversible = profile_is_reversible(&profile);
        IrTool {
            python: None,
            python_approval_required: false,
            breaker: t.breaker,
            id: self.remap_def_id(id),
            name: t.name.name.clone(),
            params: self.lower_params(&t.params),
            return_ty: self.type_ref_to_type(&t.return_ty),
            effect: t.effect,
            effect_names,
            confidence_gate,
            produces_grounded,
            effect_cost,
            effect_reversible,
            span: t.span,
        }
    }

    fn lower_prompt(&self, p: &PromptDecl) -> IrPrompt {
        let id = self
            .symbols
            .lookup_def(&p.name.name)
            .expect("prompt missing from symbol table");
        self.lower_prompt_with_id(p, id)
    }

    fn lower_prompt_with_id(&self, p: &PromptDecl, id: DefId) -> IrPrompt {
        let cites_strictly_param = p.cites_strictly.as_ref().and_then(|param_name| {
            p.params
                .iter()
                .position(|param| param.name.name == *param_name)
        });
        // Conversation history (46c): a param typed List<AiMessage>
        // is the history surface (syntactic recognition by type
        // name — works for the std/ai import and local decls alike;
        // the VM validates role/content at runtime). The CHECKER
        // enforces at-most-one and the no-interpolation rule.
        let lowered_params = self.lower_params(&p.params);
        let history_param = p.params.iter().position(param_is_history);
        let effect_names: Vec<String> = p
            .effect_row
            .effects
            .iter()
            .map(|e| e.name.name.clone())
            .collect();
        let effect_refs: Vec<&str> = effect_names.iter().map(|name| name.as_str()).collect();
        let profile = self.effect_registry.compose(&effect_refs);
        let route = p
            .route
            .as_ref()
            .map(|rt| self.lower_route_arms(&rt.arms))
            .unwrap_or_default();
        let progressive = p
            .progressive
            .as_ref()
            .map(|chain| self.lower_progressive_stages(&chain.stages))
            .unwrap_or_default();
        let rollout = p.rollout.as_ref().and_then(|spec| {
            let variant = self.remap_def_id(self.symbols.lookup_def(&spec.variant.name)?);
            let baseline = self.remap_def_id(self.symbols.lookup_def(&spec.baseline.name)?);
            Some(IrRolloutSpec {
                variant_percent: spec.variant_percent,
                variant_def_id: variant,
                variant_name: spec.variant.name.clone(),
                baseline_def_id: baseline,
                baseline_name: spec.baseline.name.clone(),
                span: spec.span,
            })
        });
        let ensemble = p.ensemble.as_ref().map(|spec| {
            let members = spec
                .models
                .iter()
                .filter_map(|model| {
                    let def_id = self.symbols.lookup_def(&model.name)?;
                    Some(IrEnsembleMember {
                        def_id: self.remap_def_id(def_id),
                        name: model.name.clone(),
                        span: model.span,
                    })
                })
                .collect();
            let vote = match spec.vote {
                corvid_ast::VoteStrategy::Majority => IrVoteStrategy::Majority,
            };
            let weighting = spec.weighting.map(|weighting| match weighting {
                corvid_ast::EnsembleWeighting::AccuracyHistory => {
                    IrEnsembleWeighting::AccuracyHistory
                }
            });
            let disagreement_escalation = spec.disagreement_escalation.as_ref().and_then(|model| {
                let def_id = self.symbols.lookup_def(&model.name)?;
                Some(IrEnsembleMember {
                    def_id: self.remap_def_id(def_id),
                    name: model.name.clone(),
                    span: model.span,
                })
            });
            IrEnsembleSpec {
                models: members,
                vote,
                weighting,
                disagreement_escalation,
                span: spec.span,
            }
        });
        let adversarial = p.adversarial.as_ref().and_then(|spec| {
            let proposer = self.remap_def_id(self.symbols.lookup_def(&spec.proposer.name)?);
            let challenger = self.remap_def_id(self.symbols.lookup_def(&spec.challenger.name)?);
            let adjudicator = self.remap_def_id(self.symbols.lookup_def(&spec.adjudicator.name)?);
            Some(IrAdversarialSpec {
                proposer_def_id: proposer,
                proposer_name: spec.proposer.name.clone(),
                challenger_def_id: challenger,
                challenger_name: spec.challenger.name.clone(),
                adjudicator_def_id: adjudicator,
                adjudicator_name: spec.adjudicator.name.clone(),
                span: spec.span,
            })
        });
        let produces_grounded = corvid_types::effects::effect_row_is_grounded(
            &p.effect_row,
            &self.effect_registry,
        );
        IrPrompt {
            id: self.remap_def_id(id),
            name: p.name.name.clone(),
            params: lowered_params,
            return_ty: self.type_ref_to_type(&p.return_ty),
            template: p.template.clone(),
            messages: p
                .messages
                .iter()
                .map(|m| IrPromptMessage {
                    role: m.role.clone(),
                    template: m.template.clone(),
                })
                .collect(),
            history_param,
            effect_names,
            effect_cost: numeric_profile_dimension(&profile, "cost"),
            effect_confidence: confidence_profile_dimension(&profile),
            produces_grounded,
            cites_strictly_param,
            min_confidence: p.stream.min_confidence,
            temperature: p.stream.temperature,
            top_p: p.stream.top_p,
            repair_attempts: p.stream.repair,
            judged_guard: p
                .stream
                .judged
                .as_ref()
                .map(|g| (g.criteria.clone(), g.min)),
            max_tokens: p.stream.max_tokens,
            backpressure: p.stream.backpressure.clone(),
            escalate_to: p
                .stream
                .escalate_to
                .as_ref()
                .map(|model| model.name.clone()),
            calibrated: p.calibrated,
            cacheable: p.cacheable,
            capability_required: p.capability_required.as_ref().map(|c| c.name.clone()),
            output_format_required: p.output_format_required.as_ref().map(|f| f.name.clone()),
            route,
            progressive,
            rollout,
            ensemble,
            adversarial,
            span: p.span,
        }
    }

    fn lower_progressive_stages(
        &self,
        stages: &[corvid_ast::ProgressiveStage],
    ) -> Vec<IrProgressiveStage> {
        let mut out = Vec::with_capacity(stages.len());
        for stage in stages {
            let Some(def_id) = self.symbols.lookup_def(&stage.model.name) else {
                continue;
            };
            out.push(IrProgressiveStage {
                model_def_id: self.remap_def_id(def_id),
                model_name: stage.model.name.clone(),
                threshold: stage.threshold,
                span: stage.span,
            });
        }
        out
    }

    fn lower_route_arms(&self, arms: &[corvid_ast::RouteArm]) -> Vec<IrRouteArm> {
        use corvid_ast::RoutePattern;
        let mut out = Vec::with_capacity(arms.len());
        for arm in arms {
            let pattern = match &arm.pattern {
                RoutePattern::Wildcard { .. } => IrRoutePattern::Wildcard,
                RoutePattern::Guard(expr) => IrRoutePattern::Guard(self.lower_expr(expr)),
            };
            // Arms whose model ident didn't resolve to a Decl::Model
            // were already flagged by the checker. At IR time we
            // best-effort resolve again; unresolved arms are skipped
            // so IR doesn't carry broken references.
            let Some(def_id) = self.symbols.lookup_def(&arm.model.name) else {
                continue;
            };
            out.push(IrRouteArm {
                pattern,
                model_def_id: self.remap_def_id(def_id),
                model_name: arm.model.name.clone(),
                span: arm.span,
            });
        }
        out
    }

    fn lower_agent(&mut self, a: &AgentDecl) -> IrAgent {
        let id = self
            .symbols
            .lookup_def(&a.name.name)
            .expect("agent missing from symbol table");
        self.lower_agent_with_id(a, id)
    }

    fn lower_agent_with_id(&mut self, a: &AgentDecl, id: DefId) -> IrAgent {
        let previous_wrapping = self.wrapping_arithmetic;
        self.wrapping_arithmetic = AgentAttribute::is_wrapping(&a.attributes);
        let body = self.lower_block(&a.body);
        self.wrapping_arithmetic = previous_wrapping;
        IrAgent {
            id: self.remap_def_id(id),
            name: a.name.name.clone(),
            extern_abi: a.extern_abi.map(|abi| match abi {
                ExternAbi::C => IrExternAbi::C,
            }),
            params: self.lower_params(&a.params),
            return_ty: self.type_ref_to_type(&a.return_ty),
            cost_budget: agent_cost_budget(a),
            wrapping_arithmetic: AgentAttribute::is_wrapping(&a.attributes),
            is_replayable: AgentAttribute::is_replayable(&a.attributes),
            pure_fn: false,
            retry_max_attempts: a.attributes.iter().find_map(|attr| match attr {
                AgentAttribute::Retry { max_attempts, .. } => Some(*max_attempts),
                _ => None,
            }),
            retry_backoff_ms: a.attributes.iter().find_map(|attr| match attr {
                AgentAttribute::Retry {
                    backoff: Some(corvid_ast::Backoff::Linear(ms)),
                    ..
                } => Some((false, *ms)),
                AgentAttribute::Retry {
                    backoff: Some(corvid_ast::Backoff::Exponential(ms)),
                    ..
                } => Some((true, *ms)),
                _ => None,
            }),
            idempotency_key_param: a.attributes.iter().find_map(|attr| match attr {
                AgentAttribute::Idempotency { key, .. } => Some(key.name.clone()),
                _ => None,
            }),
            body,
            span: a.span,
            // Populated by corvid-codegen-cl's ownership pass. `None`
            // at lowering time means "every parameter is
            // Owned" at codegen (matches pre-17b semantics).
            borrow_sig: None,
        }
    }

    /// `fn` pure function (slice 45r): shares the agent IR with
    /// `pure_fn: true`. No attributes, no effect row, no extern ABI
    /// — the checker proved the body effect-free.
    fn lower_fn(&mut self, f: &corvid_ast::FnDecl) -> IrAgent {
        let id = self
            .symbols
            .lookup_def(&f.name.name)
            .expect("fn missing from symbol table");
        IrAgent {
            id: self.remap_def_id(id),
            name: f.name.name.clone(),
            extern_abi: None,
            params: self.lower_params(&f.params),
            return_ty: self.type_ref_to_type(&f.return_ty),
            cost_budget: None,
            wrapping_arithmetic: false,
            is_replayable: false,
            pure_fn: true,
            retry_max_attempts: None,
            retry_backoff_ms: None,
            idempotency_key_param: None,
            body: self.lower_block(&f.body),
            span: f.span,
            borrow_sig: None,
        }
    }

    fn populate_effect_registry(&mut self, file: &File) {
        let decls: Vec<_> = file
            .decls
            .iter()
            .filter_map(|decl| match decl {
                Decl::Effect(effect) => Some(effect.clone()),
                _ => None,
            })
            .collect();
        self.effect_registry = EffectRegistry::from_decls(&decls);
    }

    fn lower_params(&self, ps: &[Param]) -> Vec<IrParam> {
        ps.iter()
            .map(|p| {
                let local_id = match self.bindings.get(&p.name.span) {
                    Some(Binding::Local(id)) => *id,
                    _ => LocalId(u32::MAX), // should not happen post-resolve
                };
                IrParam {
                    name: p.name.name.clone(),
                    local_id,
                    ty: self.type_ref_to_type(&p.ty),
                    span: p.span,
                }
            })
            .collect()
    }

    fn lower_block(&self, b: &Block) -> IrBlock {
        IrBlock {
            stmts: b.stmts.iter().map(|s| self.lower_stmt(s)).collect(),
            span: b.span,
        }
    }

    fn lower_stmt(&self, s: &Stmt) -> IrStmt {
        match s {
            Stmt::Let {
                name, value, span, ..
            } => {
                let local_id = match self.bindings.get(&name.span) {
                    Some(Binding::Local(id)) => *id,
                    _ => LocalId(u32::MAX),
                };
                let lowered_value = self.lower_expr(value);
                IrStmt::Let {
                    local_id,
                    name: name.name.clone(),
                    ty: lowered_value.ty.clone(),
                    value: lowered_value,
                    span: *span,
                }
            }
            Stmt::Return { value, span } => IrStmt::Return {
                value: value.as_ref().map(|e| self.lower_expr(e)),
                span: *span,
            },
            Stmt::Yield { value, span } => IrStmt::Yield {
                value: self.lower_expr(value),
                span: *span,
            },
            Stmt::If {
                cond,
                then_block,
                else_block,
                span,
            } => IrStmt::If {
                cond: self.lower_expr(cond),
                then_block: self.lower_block(then_block),
                else_block: else_block.as_ref().map(|b| self.lower_block(b)),
                span: *span,
            },
            Stmt::For {
                var,
                iter,
                body,
                span,
            } => {
                let var_local = match self.bindings.get(&var.span) {
                    Some(Binding::Local(id)) => *id,
                    _ => LocalId(u32::MAX),
                };
                IrStmt::For {
                    var_local,
                    var_name: var.name.clone(),
                    iter: self.lower_expr(iter),
                    body: self.lower_block(body),
                    span: *span,
                }
            }
            Stmt::Parallel { arms, span } => IrStmt::Parallel {
                arms: arms
                    .iter()
                    .map(|arm| IrParallelArm {
                        name: arm.name.name.clone(),
                        local_id: match self.bindings.get(&arm.name.span) {
                            Some(Binding::Local(id)) => *id,
                            _ => LocalId(u32::MAX),
                        },
                        call: self.lower_expr(&arm.call),
                        span: arm.span,
                    })
                    .collect(),
                span: *span,
            },
            Stmt::Destructure {
                pattern,
                value,
                span,
            } => IrStmt::Destructure {
                pattern: self.lower_pattern(pattern),
                value: self.lower_expr(value),
                span: *span,
            },
            Stmt::While { cond, body, span } => IrStmt::While {
                cond: self.lower_expr(cond),
                body: self.lower_block(body),
                span: *span,
            },
            Stmt::Atomic { body, span } => {
                // The checker proved every table here lives in one database.
                let tables = corvid_types::table_ops::atomic_tables(body, self.file, self.symbols, self.bindings);
                let database = tables
                    .first()
                    .and_then(|table| corvid_types::table_ops::table_database(self.file, &table.name().name))
                    .and_then(ir_database);
                IrStmt::Atomic { database, body: self.lower_block(body), span: *span }
            }
            Stmt::Break { span } => IrStmt::Break { span: *span },
            Stmt::Continue { span } => IrStmt::Continue { span: *span },
            Stmt::Pass { span } => IrStmt::Pass { span: *span },
            // Place assignment (45b): decompose the target into a root
            // local + a Field/Index path. The checker guarantees the
            // root is a local binding.
            Stmt::Assign {
                target,
                op,
                value,
                span,
            } => {
                let mut path_rev: Vec<IrPathSeg> = Vec::new();
                let mut root = target;
                loop {
                    match root {
                        Expr::FieldAccess { target, field, .. } => {
                            path_rev.push(IrPathSeg::Field(field.name.clone()));
                            root = target;
                        }
                        Expr::Index { target, index, .. } => {
                            path_rev.push(IrPathSeg::Index(self.lower_expr(index)));
                            root = target;
                        }
                        _ => break,
                    }
                }
                let (local_id, name) = match root {
                    Expr::Ident { name, .. } => (
                        match self.bindings.get(&name.span) {
                            Some(Binding::Local(id)) => *id,
                            _ => LocalId(u32::MAX),
                        },
                        name.name.clone(),
                    ),
                    // The checker rejected non-local roots; emit a
                    // sentinel so downstream stays total.
                    _ => (LocalId(u32::MAX), "<invalid>".to_string()),
                };
                path_rev.reverse();
                IrStmt::Assign {
                    local_id,
                    root_ty: self.remap_struct_type(self.types.get(&root.span()).cloned().unwrap_or(Type::Unknown)),
                    name,
                    path: path_rev,
                    op: *op,
                    value: self.lower_expr(value),
                    span: *span,
                }
            }
            Stmt::Approve { action, span } => {
                let (label, args) = self.extract_approve_action(action);
                IrStmt::Approve {
                    label,
                    args,
                    span: *span,
                }
            }
            Stmt::Expr { expr, span } => IrStmt::Expr {
                expr: self.lower_expr(expr),
                span: *span,
            },
        }
    }

    fn extract_approve_action(&self, action: &Expr) -> (String, Vec<IrExpr>) {
        if let Expr::Call { callee, args, .. } = action {
            if let Some((_, name)) = self.python_member(callee) {
                return (name.clone(), args.iter().map(|arg| self.lower_expr(arg)).collect());
            }
            if let Expr::Ident { name, .. } = &**callee {
                let lowered_args: Vec<IrExpr> = args.iter().map(|a| self.lower_expr(a)).collect();
                return (name.name.clone(), lowered_args);
            }
        }
        // Non-call or non-ident callee: synthesize a label.
        ("<unknown>".to_string(), Vec::new())
    }

    fn lower_expr(&self, e: &Expr) -> IrExpr {
        // Expression types come from the checker (`checked.types`),
        // which records imported structs under their original
        // per-module DefId. Remap to the cross-module id so every
        // imported-struct *type* in the IR — signatures (G0-1) and
        // expression types alike — keys the merged `ir.types` layout
        // table. Without this, native field access would index the
        // table with the wrong id.
        let ty = self.remap_struct_type(
            self.types.get(&e.span()).cloned().unwrap_or(Type::Unknown),
        );
        if matches!(ty, Type::Function { .. }) && matches!(e, Expr::Ident { .. } | Expr::FieldAccess { .. }) {
            if let Some(target) = self.imported_calls.get(&e.span()) {
                return IrExpr {
                    kind: IrExprKind::Decl { def_id: self.remap_imported_target(target), name: target.name.clone() },
                    ty,
                    span: e.span(),
                };
            }
        }
        let kind = match e {
            // B06 — a number literal the checker typed `Decimal`, read
            // from its source digits by the parser the checker used.
            Expr::Literal { value, .. } if matches!(ty, Type::Decimal) => {
                IrExprKind::Literal(IrLiteral::Decimal(
                    corvid_types::number_literal_text(value)
                        .and_then(|text| corvid_decimal::Decimal::parse(&text).ok())
                        // Unreachable for a program that checked: the
                        // checker reports a literal with no exact value.
                        .unwrap_or(corvid_decimal::Decimal::ZERO),
                ))
            }
            // B06 — string literals the checker typed `Currency` or
            // `Locale`, read with the parser the checker used.
            Expr::Literal { value: Literal::String(code), .. } if matches!(ty, Type::Currency) => {
                match corvid_currency::Currency::parse(code) {
                    Ok(currency) => IrExprKind::Literal(IrLiteral::Currency(currency)),
                    // Unreachable for a program that checked.
                    Err(_) => IrExprKind::Literal(IrLiteral::String(code.clone())),
                }
            }
            Expr::Literal { value: Literal::String(tag), .. } if matches!(ty, Type::Locale) => {
                match corvid_currency::Locale::parse(tag) {
                    Ok(locale) => IrExprKind::Literal(IrLiteral::Locale(locale)),
                    Err(_) => IrExprKind::Literal(IrLiteral::String(tag.clone())),
                }
            }
            // B06 — a string literal the checker typed `TimeZone`, looked
            // up with the parser the checker used.
            Expr::Literal { value: Literal::String(name), .. } if matches!(ty, Type::TimeZone) => {
                IrExprKind::Literal(IrLiteral::TimeZone(
                    // Unreachable for a program that checked: the checker
                    // reports an unknown zone name.
                    corvid_time::TimeZone::parse(name).unwrap_or(corvid_time::TimeZone::UTC),
                ))
            }
            Expr::Literal { value, .. } => IrExprKind::Literal(match value {
                Literal::Int(n) => IrLiteral::Int(*n),
                Literal::Float { value: f, .. } => IrLiteral::Float(*f),
                Literal::String(s) => IrLiteral::String(s.clone()),
                Literal::Bool(b) => IrLiteral::Bool(*b),
                Literal::Nothing => IrLiteral::Nothing,
            }),
            Expr::Ident { name, .. } => self.lower_ident(name),
            Expr::StructLiteral {
                name,
                fields,
                spread,
                ..
            } => {
                // The checker recorded the (alias-expanded) struct
                // type on this expression's span.
                let def_id = match self.types.get(&e.span()) {
                    Some(Type::Struct(id)) => self.remap_def_id(*id),
                    _ => DefId(u32::MAX),
                };
                IrExprKind::StructLiteral {
                    def_id,
                    type_name: name.name.clone(),
                    fields: fields
                        .iter()
                        .map(|f| {
                            let value = match &f.value {
                                Some(v) => self.lower_expr(v),
                                // Shorthand reads the local of the
                                // same name.
                                None => IrExpr {
                                    kind: match self.bindings.get(&f.name.span) {
                                        Some(Binding::Local(id)) => IrExprKind::Local {
                                            local_id: *id,
                                            name: f.name.name.clone(),
                                        },
                                        _ => IrExprKind::Local {
                                            local_id: LocalId(u32::MAX),
                                            name: f.name.name.clone(),
                                        },
                                    },
                                    ty: Type::Unknown,
                                    span: f.name.span,
                                },
                            };
                            (f.name.name.clone(), value)
                        })
                        .chain(
                            // A row built without `revision` and without
                            // a base has never been stored (B09).
                            (spread.is_none()
                                && self.table_named(name).is_some()
                                && !fields.iter().any(|f| f.name.name == corvid_ast::TableDecl::REVISION_FIELD))
                            .then(|| (corvid_ast::TableDecl::REVISION_FIELD.to_string(), zero_revision(name.span))),
                        )
                        .collect(),
                    spread: spread.as_ref().map(|s| Box::new(self.lower_expr(s))),
                }
            }
            Expr::Lambda { params, body, .. } => IrExprKind::Lambda {
                params: params
                    .iter()
                    .map(|p| IrLambdaParam {
                        local_id: match self.bindings.get(&p.name.span) {
                            Some(Binding::Local(id)) => *id,
                            _ => LocalId(u32::MAX),
                        },
                        name: p.name.name.clone(),
                    })
                    .collect(),
                body: Box::new(self.lower_expr(body)),
            },
            Expr::Call { callee, args, .. } => {
                if let Some(initial) = e.browser_state_initializer().filter(|value|
                    matches!(self.types.get(&value.span()), Some(Type::List(_)))) {
                    self.lower_expr(initial).kind
                } else {
                    self.lower_call(callee, args)
                }
            }
            Expr::FieldAccess { target, field, .. } => IrExprKind::FieldAccess {
                target: Box::new(self.lower_expr(target)),
                field: field.name.clone(),
            },
            Expr::Index { target, index, .. } => IrExprKind::Index {
                target: Box::new(self.lower_expr(target)),
                index: Box::new(self.lower_expr(index)),
            },
            Expr::BinOp {
                op,
                left,
                right,
                span,
            } => {
                let left = Box::new(self.lower_expr(left));
                let right = Box::new(self.lower_expr(right));
                if self.wrapping_arithmetic && is_wrapping_int_binop(*op, self.types.get(span)) {
                    IrExprKind::WrappingBinOp {
                        op: *op,
                        left,
                        right,
                    }
                } else {
                    IrExprKind::BinOp {
                        op: *op,
                        left,
                        right,
                    }
                }
            }
            Expr::UnOp { op, operand, span } => {
                // Unary `+` (45q) is numeric identity: the CHECKER
                // enforced Int/Float; lowering elides it entirely,
                // so no backend or interpreter arm exists for it.
                if matches!(op, UnaryOp::Pos) {
                    return self.lower_expr(operand);
                }
                let operand = Box::new(self.lower_expr(operand));
                if self.wrapping_arithmetic && is_wrapping_int_unop(*op, self.types.get(span)) {
                    IrExprKind::WrappingUnOp { op: *op, operand }
                } else {
                    IrExprKind::UnOp { op: *op, operand }
                }
            }
            Expr::Match {
                scrutinee, arms, ..
            } => IrExprKind::Match {
                scrutinee: Box::new(self.lower_expr(scrutinee)),
                arms: arms
                    .iter()
                    .map(|arm| IrMatchArm {
                        pattern: self.lower_pattern(&arm.pattern),
                        guard: arm.guard.as_ref().map(|g| self.lower_expr(g)),
                        body: self.lower_expr(&arm.body),
                        span: arm.span,
                    })
                    .collect(),
            },
            Expr::MapLiteral { entries, .. } => IrExprKind::MapLiteral {
                keys: entries.iter().map(|(k, _)| self.lower_expr(k)).collect(),
                values: entries.iter().map(|(_, v)| self.lower_expr(v)).collect(),
            },
            Expr::List { items, .. } => IrExprKind::List {
                items: items.iter().map(|i| self.lower_expr(i)).collect(),
            },
            Expr::TryPropagate { inner, .. } => IrExprKind::TryPropagate {
                inner: Box::new(self.lower_expr(inner)),
            },
            Expr::TrustBoundary { inner, .. } => IrExprKind::TrustBoundary {
                inner: Box::new(self.lower_expr(inner)),
            },
            Expr::TryRetry {
                body,
                attempts,
                backoff,
                timeout_ms,
                ..
            } => IrExprKind::TryRetry {
                body: Box::new(self.lower_expr(body)),
                attempts: *attempts,
                backoff: *backoff,
                timeout_ms: *timeout_ms,
            },
            Expr::Replay {
                trace,
                arms,
                else_body,
                ..
            } => IrExprKind::Replay {
                trace: Box::new(self.lower_expr(trace)),
                arms: arms.iter().map(|arm| self.lower_replay_arm(arm)).collect(),
                else_body: Box::new(self.lower_expr(else_body)),
            },
        };
        // A JSON decode reads its target type from `ty` at run time, so
        // every record id inside it must key the merged `ir.types` table,
        // not only the outermost one.
        let ty = match &kind {
            IrExprKind::BuiltinMethod {
                kind: corvid_types::BuiltinMethodKind::JsonDecode | corvid_types::BuiltinMethodKind::JsonDecodeStrict,
                ..
            } => self.remap_struct_type_deep(ty),
            _ => ty,
        };
        let lowered = IrExpr {
            kind,
            ty: ty.clone(),
            span: e.span(),
        };
        // Provenance Propagation D5 (slice 7b): if the typechecker
        // flagged this span as a silent `Grounded<T> -> T` coercion
        // site, wrap the lowered expression in `UnwrapGrounded` so
        // the discard is IR-visible. The inner `IrExpr` keeps its
        // `Grounded<T>` IR type; the wrapper's outer type is the
        // stripped inner. `@grounded_pure` (slice 9) walks for
        // `UnwrapGrounded` to fail the moat.
        if self.grounded_coercion_sites.contains(&e.span()) {
            let unwrapped_ty = match &ty {
                Type::Grounded(inner) => inner.as_ref().clone(),
                other => other.clone(),
            };
            IrExpr {
                kind: IrExprKind::UnwrapGrounded {
                    value: Box::new(lowered),
                },
                ty: unwrapped_ty,
                span: e.span(),
            }
        } else {
            lowered
        }
    }

    /// Lower one replay arm. The body is lowered in the same
    /// per-arm scope the resolver opened (21-inv-E-2b), so any
    /// capture the arm binds is already reachable via
    /// `self.bindings` keyed by the capture's span.
    fn lower_replay_arm(&self, arm: &ReplayArm) -> IrReplayArm {
        let pattern = self.lower_replay_pattern(&arm.pattern);
        let capture = arm.capture.as_ref().map(|ident| IrReplayCapture {
            local_id: self.lookup_local(ident.span, "replay `as` capture"),
            name: ident.name.clone(),
            span: ident.span,
        });
        let body = Box::new(self.lower_expr(&arm.body));
        IrReplayArm {
            pattern,
            capture,
            body,
            span: arm.span,
        }
    }

    fn lower_replay_pattern(&self, pattern: &ReplayPattern) -> IrReplayPattern {
        match pattern {
            ReplayPattern::Llm { prompt, span } => IrReplayPattern::Llm {
                prompt: prompt.clone(),
                span: *span,
            },
            ReplayPattern::Tool { tool, arg, span } => IrReplayPattern::Tool {
                tool: tool.clone(),
                arg: self.lower_replay_tool_arg(arg),
                span: *span,
            },
            ReplayPattern::Approve { label, span } => IrReplayPattern::Approve {
                label: label.clone(),
                span: *span,
            },
        }
    }

    fn lower_replay_tool_arg(&self, arg: &ToolArgPattern) -> IrReplayToolArgPattern {
        match arg {
            ToolArgPattern::Wildcard { .. } => IrReplayToolArgPattern::Wildcard,
            ToolArgPattern::StringLit { value, .. } => {
                IrReplayToolArgPattern::StringLit(value.clone())
            }
            ToolArgPattern::Capture { name, span } => {
                IrReplayToolArgPattern::Capture(IrReplayCapture {
                    local_id: self.lookup_local(*span, "replay tool-arg capture"),
                    name: name.name.clone(),
                    span: *span,
                })
            }
        }
    }

    /// Resolve a capture ident's span to its `LocalId`. The
    /// resolver (E-2b) is the source of truth: every capture span
    /// is registered as `Binding::Local(_)` before the checker and
    /// lowerer ever see it. A missing binding here signals a
    /// resolver bug, not a user error, so we fall back to
    /// `LocalId(u32::MAX)` (the same sentinel `lower_ident` uses
    /// for unresolved names) so codegen doesn't panic — the
    /// resolver's diagnostics will already have been surfaced.
    fn lookup_local(&self, span: Span, context: &str) -> LocalId {
        match self.bindings.get(&span) {
            Some(Binding::Local(local_id)) => *local_id,
            _ => {
                let _ = context; // reserved for future debug-assert
                LocalId(u32::MAX)
            }
        }
    }

    /// Lower one `match` pattern (slice 45i). The resolver already
    /// disambiguated bare names (variant vs binding) via the
    /// bindings table.
    fn lower_pattern(&self, pattern: &corvid_ast::Pattern) -> IrPattern {
        use corvid_ast::Pattern;
        match pattern {
            Pattern::Wildcard { .. } => IrPattern::Wildcard,
            Pattern::Literal { value, .. } => IrPattern::Literal(match value {
                corvid_ast::Literal::Int(v) => IrLiteral::Int(*v),
                corvid_ast::Literal::Float { value: v, .. } => IrLiteral::Float(*v),
                corvid_ast::Literal::String(s) => IrLiteral::String(s.clone()),
                corvid_ast::Literal::Bool(b) => IrLiteral::Bool(*b),
                corvid_ast::Literal::Nothing => IrLiteral::Nothing,
            }),
            Pattern::Name { name, .. } => match self.bindings.get(&name.span) {
                Some(Binding::Decl(def_id)) => {
                    let (owner, idx) = self
                        .variant_owners
                        .get(def_id)
                        .copied()
                        .unwrap_or((DefId(u32::MAX), 0));
                    IrPattern::Variant {
                        owner: self.remap_def_id(owner),
                        variant_index: idx,
                        variant_name: name.name.clone(),
                        args: Vec::new(),
                    }
                }
                Some(Binding::BuiltIn(_)) => IrPattern::None_,
                Some(Binding::Local(local_id)) => IrPattern::Bind {
                    local_id: *local_id,
                    name: name.name.clone(),
                },
                None => IrPattern::Wildcard,
            },
            Pattern::At { name, inner, .. } => {
                let local_id = match self.bindings.get(&name.span) {
                    Some(Binding::Local(id)) => *id,
                    _ => LocalId(u32::MAX),
                };
                IrPattern::At {
                    local_id,
                    name: name.name.clone(),
                    inner: Box::new(self.lower_pattern(inner)),
                }
            }
            Pattern::Variant { name, args, .. } => {
                let lowered_args: Vec<IrPattern> =
                    args.iter().map(|a| self.lower_pattern(a)).collect();
                match self.bindings.get(&name.span) {
                    Some(Binding::Decl(def_id)) => {
                        let (owner, idx) = self
                            .variant_owners
                            .get(def_id)
                            .copied()
                            .unwrap_or((DefId(u32::MAX), 0));
                        IrPattern::Variant {
                            owner: self.remap_def_id(owner),
                            variant_index: idx,
                            variant_name: name.name.clone(),
                            args: lowered_args,
                        }
                    }
                    Some(Binding::BuiltIn(b)) => {
                        let first = Box::new(
                            lowered_args.into_iter().next().unwrap_or(IrPattern::Wildcard),
                        );
                        match b {
                            BuiltIn::Some => IrPattern::Some_(first),
                            BuiltIn::Ok => IrPattern::Ok_(first),
                            BuiltIn::Err => IrPattern::Err_(first),
                            _ => IrPattern::Wildcard,
                        }
                    }
                    _ => IrPattern::Wildcard,
                }
            }
            Pattern::Record { fields, .. } => IrPattern::Record {
                fields: fields
                    .iter()
                    .map(|fp| {
                        let sub = match &fp.pattern {
                            Some(p) => self.lower_pattern(p),
                            None => match self.bindings.get(&fp.name.span) {
                                Some(Binding::Local(id)) => IrPattern::Bind {
                                    local_id: *id,
                                    name: fp.name.name.clone(),
                                },
                                _ => IrPattern::Wildcard,
                            },
                        };
                        (fp.name.name.clone(), sub)
                    })
                    .collect(),
            },
        }
    }

    fn lower_ident(&self, id: &Ident) -> IrExprKind {
        match self.bindings.get(&id.span) {
            Some(Binding::Local(local_id)) => IrExprKind::Local {
                local_id: *local_id,
                name: id.name.clone(),
            },
            // Bare unit variant (45h): `Pending` constructs the
            // zero-field variant value directly.
            Some(Binding::Decl(def_id))
                if self.symbols.get(*def_id).kind
                    == corvid_resolve::DeclKind::Variant =>
            {
                let (owner, idx) = self
                    .variant_owners
                    .get(def_id)
                    .copied()
                    .unwrap_or((DefId(u32::MAX), 0));
                IrExprKind::Call {
                    kind: IrCallKind::EnumConstructor {
                        def_id: self.remap_def_id(owner),
                        variant_index: idx,
                    },
                    callee_name: id.name.clone(),
                    args: Vec::new(),
                }
            }
            Some(Binding::Decl(def_id)) => IrExprKind::Decl {
                def_id: self.remap_def_id(*def_id),
                name: id.name.clone(),
            },
            Some(Binding::BuiltIn(BuiltIn::None)) => IrExprKind::OptionNone,
            Some(Binding::BuiltIn(_)) => IrExprKind::Local {
                local_id: LocalId(u32::MAX),
                name: id.name.clone(),
            },
            None => IrExprKind::Local {
                local_id: LocalId(u32::MAX),
                name: id.name.clone(),
            },
        }
    }

    fn python_member(&self, callee: &Expr) -> Option<&(DefId, String)> {
        let Expr::FieldAccess { target, field, .. } = callee else { return None };
        let Expr::Ident { name, .. } = target.as_ref() else { return None };
        let Binding::Decl(id) = self.bindings.get(&name.span)? else { return None };
        self.python_members.get(&(*id, field.name.clone()))
    }

    fn lower_call(&self, callee: &Expr, args: &[Expr]) -> IrExprKind {
        // `target.method(args)` rewrites to a regular call
        // with the receiver prepended. Method DefId comes from the
        // resolver's per-type method side-table; the caller's type
        // is read from the type checker's per-expression side table.
        if let Expr::Call { .. } = callee {
            // (no-op: shouldn't happen — Call's callee is an Expr,
            // never another Call directly. Keeps the match arm
            // catchall narrower below.)
        }
        if let Expr::FieldAccess { target, field, .. } = callee {
            if let Some((def_id, callee_name)) = self.python_member(callee) {
                return IrExprKind::Call {
                    kind: IrCallKind::Tool { def_id: *def_id, effect: Effect::Safe },
                    callee_name: callee_name.clone(),
                    args: args.iter().map(|arg| self.lower_expr(arg)).collect(),
                };
            }
            if let Some(rewrite) = self.try_table_query(callee, args) {
                return rewrite;
            }
            if let Some(rewrite) = self.try_table_call(callee, args) {
                return rewrite;
            }
            if let Some(rewrite) = self.try_builtin_type_method_call(target, field, args) {
                return rewrite;
            }
            if let Some(rewrite) = self.try_type_method_call(target, field, args) {
                return rewrite;
            }
            if let Some(rewrite) = self.try_imported_call(callee, field, args) {
                return rewrite;
            }
            if let Some(rewrite) = self.try_grounded_builtin_call(target, field, args) {
                return rewrite;
            }
            if let Some(rewrite) = stream::try_stream_builtin_call(self, target, field, args) {
                return rewrite;
            }
            if let Some(rewrite) = self.try_builtin_method_call(target, field, args) {
                return rewrite;
            }
            if let Some(rewrite) = self.try_method_call(target, field, args) {
                return rewrite;
            }
        }

        let (kind, callee_name) = match callee {
            Expr::Ident { name, .. } => match self.bindings.get(&name.span) {
                Some(Binding::BuiltIn(BuiltIn::Ok)) => {
                    let inner = args
                        .first()
                        .map(|arg| self.lower_expr(arg))
                        .unwrap_or_else(|| IrExpr {
                            kind: IrExprKind::OptionNone,
                            ty: Type::Unknown,
                            span: name.span,
                        });
                    return IrExprKind::ResultOk {
                        inner: Box::new(inner),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::Err)) => {
                    let inner = args
                        .first()
                        .map(|arg| self.lower_expr(arg))
                        .unwrap_or_else(|| IrExpr {
                            kind: IrExprKind::OptionNone,
                            ty: Type::Unknown,
                            span: name.span,
                        });
                    return IrExprKind::ResultErr {
                        inner: Box::new(inner),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::Some)) => {
                    let inner = args
                        .first()
                        .map(|arg| self.lower_expr(arg))
                        .unwrap_or_else(|| IrExpr {
                            kind: IrExprKind::OptionNone,
                            ty: Type::Unknown,
                            span: name.span,
                        });
                    return IrExprKind::OptionSome {
                        inner: Box::new(inner),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::None)) => return IrExprKind::OptionNone,
                Some(Binding::BuiltIn(BuiltIn::Page)) => {
                    // `Page(items, next_cursor)` (slice 52c-2).
                    let mut lowered = args.iter().map(|a| self.lower_expr(a));
                    let items = lowered.next().unwrap_or_else(|| IrExpr {
                        kind: IrExprKind::List { items: vec![] },
                        ty: Type::Unknown,
                        span: name.span,
                    });
                    let next_cursor = lowered.next().unwrap_or_else(|| IrExpr {
                        kind: IrExprKind::OptionNone,
                        ty: Type::Option(Box::new(Type::String)),
                        span: name.span,
                    });
                    return IrExprKind::PageNew {
                        items: Box::new(items),
                        next_cursor: Box::new(next_cursor),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::BrowserFieldKey | BuiltIn::BrowserRowKey)) if matches!(args.len(), 2 | 3) => {
                    return self.lower_expr(&args[1]).kind;
                }
                Some(Binding::BuiltIn(BuiltIn::BrowserFields)) if args.len() == 1 => {
                    return IrExprKind::BuiltinMethod {
                        kind: corvid_types::BuiltinMethodKind::BrowserFields,
                        receiver: Box::new(self.lower_expr(&args[0])),
                        args: Vec::new(),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::FormFor)) if args.len() == 3 => {
                    return IrExprKind::BuiltinMethod {
                        kind: corvid_types::BuiltinMethodKind::RouteForm,
                        receiver: Box::new(self.lower_expr(&args[0])),
                        args: args[1..].iter().map(|arg| self.lower_expr(arg)).collect(),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::Range)) => {
                    // range(start, end) rides the BuiltinMethod IR
                    // with `start` as the receiver.
                    let mut lowered = args.iter().map(|a| self.lower_expr(a));
                    let start = lowered.next().unwrap_or_else(|| IrExpr {
                        kind: IrExprKind::Literal(IrLiteral::Int(0)),
                        ty: Type::Int,
                        span: name.span,
                    });
                    let rest: Vec<IrExpr> = lowered.collect();
                    return IrExprKind::BuiltinMethod {
                        kind: corvid_types::BuiltinMethodKind::RangeIntList,
                        receiver: Box::new(start),
                        args: rest,
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::WeakNew)) => {
                    let strong =
                        args.first()
                            .map(|arg| self.lower_expr(arg))
                            .unwrap_or_else(|| IrExpr {
                                kind: IrExprKind::Literal(IrLiteral::Nothing),
                                ty: Type::Unknown,
                                span: name.span,
                            });
                    return IrExprKind::WeakNew {
                        strong: Box::new(strong),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::WeakUpgrade)) => {
                    let weak = args
                        .first()
                        .map(|arg| self.lower_expr(arg))
                        .unwrap_or_else(|| IrExpr {
                            kind: IrExprKind::Literal(IrLiteral::Nothing),
                            ty: Type::Unknown,
                            span: name.span,
                        });
                    return IrExprKind::WeakUpgrade {
                        weak: Box::new(weak),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::StreamMerge)) => {
                    return stream::lower_merge_call(self, name.span, args);
                }
                Some(Binding::BuiltIn(BuiltIn::StreamResumeToken)) => {
                    let stream =
                        args.first()
                            .map(|arg| self.lower_expr(arg))
                            .unwrap_or_else(|| IrExpr {
                                kind: IrExprKind::Literal(IrLiteral::Nothing),
                                ty: Type::Unknown,
                                span: name.span,
                            });
                    return IrExprKind::StreamResumeToken {
                        stream: Box::new(stream),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::Ask)) => {
                    let prompt =
                        args.first()
                            .map(|arg| self.lower_expr(arg))
                            .unwrap_or_else(|| IrExpr {
                                kind: IrExprKind::Literal(IrLiteral::String(String::new())),
                                ty: Type::String,
                                span: name.span,
                            });
                    return IrExprKind::Ask {
                        prompt: Box::new(prompt),
                        target_ty: args
                            .get(1)
                            .map(|arg| self.type_expr_to_type(arg))
                            .unwrap_or(Type::Unknown),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::Choose)) => {
                    let options =
                        args.first()
                            .map(|arg| self.lower_expr(arg))
                            .unwrap_or_else(|| IrExpr {
                                kind: IrExprKind::List { items: Vec::new() },
                                ty: Type::List(Box::new(Type::Unknown)),
                                span: name.span,
                            });
                    return IrExprKind::Choose {
                        options: Box::new(options),
                    };
                }
                Some(Binding::BuiltIn(BuiltIn::Resume)) => {
                    if let Some(Expr::Ident {
                        name: prompt_name, ..
                    }) = args.first()
                    {
                        if let Some(Binding::Decl(def_id)) = self.bindings.get(&prompt_name.span) {
                            if self.symbols.get(*def_id).kind == DeclKind::Prompt {
                                let token = args
                                    .get(1)
                                    .map(|arg| self.lower_expr(arg))
                                    .unwrap_or_else(|| IrExpr {
                                        kind: IrExprKind::Literal(IrLiteral::Nothing),
                                        ty: Type::Unknown,
                                        span: name.span,
                                    });
                                return IrExprKind::ResumeStream {
                                    prompt_def_id: self.remap_def_id(*def_id),
                                    prompt_name: prompt_name.name.clone(),
                                    token: Box::new(token),
                                };
                            }
                        }
                    }
                    (IrCallKind::Unknown, name.name.clone())
                }
                Some(Binding::Decl(def_id)) => {
                    let entry = self.symbols.get(*def_id);
                    if entry.kind == DeclKind::ImportedUse {
                        if let Some(rewrite) = self.try_imported_lifted_call(name, args) {
                            return rewrite;
                        }
                    }
                    let lowered_def_id = self.remap_def_id(*def_id);
                    let kind = match entry.kind {
                        DeclKind::Tool => {
                            // Effect is stored on the AST ToolDecl; we need
                            // it at call sites. We pass `Effect::Safe` as a
                            // stable default and let IrTool carry the truth.
                            // Codegen looks up the IrTool by def_id to route.
                            IrCallKind::Tool {
                                def_id: lowered_def_id,
                                effect: lookup_tool_effect(self.symbols, lowered_def_id),
                            }
                        }
                        DeclKind::Prompt => IrCallKind::Prompt {
                            def_id: lowered_def_id,
                        },
                        // A `fn` call (45r) lowers as an agent
                        // call: fns share the agent IR, so every
                        // tier's existing call path executes them.
                        DeclKind::Fn => IrCallKind::Agent {
                            def_id: lowered_def_id,
                        },
                        DeclKind::Agent => IrCallKind::Agent {
                            def_id: lowered_def_id,
                        },
                        DeclKind::Fixture => IrCallKind::Fixture {
                            def_id: lowered_def_id,
                        },
                        DeclKind::Type => IrCallKind::StructConstructor {
                            def_id: lowered_def_id,
                        },
                        DeclKind::Variant => {
                            let (owner, idx) = self
                                .variant_owners
                                .get(def_id)
                                .copied()
                                .unwrap_or((DefId(u32::MAX), 0));
                            IrCallKind::EnumConstructor {
                                def_id: self.remap_def_id(owner),
                                variant_index: idx,
                            }
                        }
                        _ => IrCallKind::Unknown,
                    };
                    (kind, name.name.clone())
                }
                Some(Binding::Local(local_id)) => (
                    IrCallKind::ClosureLocal {
                        local_id: *local_id,
                        signature: self.types.get(&name.span).cloned().unwrap_or(Type::Unknown),
                    },
                    name.name.clone(),
                ),
                _ => (IrCallKind::Unknown, name.name.clone()),
            },
            _ => (IrCallKind::Unknown, "<indirect>".to_string()),
        };
        let mut lowered: Vec<IrExpr> = args.iter().map(|a| self.lower_expr(a)).collect();
        // A table row built without its trailing `revision` has never
        // been stored, so its revision is 0 (B09).
        if let (IrCallKind::StructConstructor { .. }, Expr::Ident { name, .. }) = (&kind, callee) {
            if self.table_named(name).is_some_and(|t| args.len() + 1 == t.record.fields.len()) {
                lowered.push(zero_revision(name.span));
            }
        }
        IrExprKind::Call { kind, callee_name, args: lowered }
    }

    /// The table a type name refers to, if it names one.
    fn table_named(&self, name: &Ident) -> Option<&'a corvid_ast::TableDecl> {
        let Some(Binding::Decl(id)) = self.bindings.get(&name.span) else { return None };
        corvid_types::table_decl(self.file, self.symbols.get(*id).span)
    }

    /// Detect and lower a `target.method(args)` call. Returns
    /// `Some(IrExprKind::Call { ... })` with the receiver prepended
    /// when `target`'s type matches a registered method. Returns
    /// `None` when the call doesn't resolve to a method (caller
    /// falls back to the regular field-access-of-a-fn path, which
    /// produces `IrCallKind::Unknown` and lets later validation error).
    fn try_method_call(&self, target: &Expr, field: &Ident, args: &[Expr]) -> Option<IrExprKind> {
        // Receiver type lives on the type-checker's side-table.
        let recv_ty = self.types.get(&target.span())?;
        let recv_def_id = match recv_ty {
            Type::Struct(id) => *id,
            _ => return None,
        };
        let entry = self.methods.get(&recv_def_id)?.get(&field.name)?;
        let def_id = self.remap_def_id(entry.def_id);
        let kind = match entry.kind {
            corvid_resolve::resolver::MethodKind::Tool => IrCallKind::Tool {
                def_id,
                // Method-tool effects keep `Safe` as the conservative
                // default; the IR's `IrTool` carries the
                // declared effect once `define_tool` lowers it.
                effect: Effect::Safe,
            },
            corvid_resolve::resolver::MethodKind::Prompt => IrCallKind::Prompt { def_id },
            corvid_resolve::resolver::MethodKind::Agent => IrCallKind::Agent { def_id },
        };
        // Receiver becomes the first argument.
        let mut lowered_args: Vec<IrExpr> = Vec::with_capacity(args.len() + 1);
        lowered_args.push(self.lower_expr(target));
        lowered_args.extend(args.iter().map(|a| self.lower_expr(a)));
        Some(IrExprKind::Call {
            kind,
            callee_name: field.name.clone(),
            args: lowered_args,
        })
    }

    /// Builtin-method dispatch (slice 45c): re-derive the same
    /// lookup the checker performed from the receiver's checked
    /// type, and lower to `IrExprKind::BuiltinMethod` — the shared
    /// `corvid_types::builtin_method` table keeps the two in sync.
    fn try_builtin_method_call(
        &self,
        target: &Expr,
        field: &Ident,
        args: &[Expr],
    ) -> Option<IrExprKind> {
        let recv_ty = self.types.get(&target.span())?;
        let sig = corvid_types::builtin_method(recv_ty, &field.name)?;
        // B06 — `(a / b).round_<mode>(places)` on decimals is one
        // rounded division: the quotient never exists unrounded.
        if let (corvid_types::BuiltinMethodKind::DecimalRound(mode), Expr::BinOp { op: BinaryOp::Div, left, right, .. }) =
            (sig.kind, target)
        {
            return Some(IrExprKind::BuiltinMethod {
                kind: corvid_types::BuiltinMethodKind::DecimalDivideRound(mode),
                receiver: Box::new(self.lower_expr(left)),
                args: std::iter::once(self.lower_expr(right))
                    .chain(args.iter().map(|a| self.lower_expr(a)))
                    .collect(),
            });
        }
        Some(IrExprKind::BuiltinMethod {
            kind: corvid_types::refine_builtin_method_kind(sig.kind, args),
            receiver: Box::new(self.lower_expr(target)),
            args: args.iter().map(|a| self.lower_expr(a)).collect(),
        })
    }

    /// `Shelf.save(row)`: a table operation (B09), recognized by the
    /// same `table_call_target` the checker uses.
    fn try_table_call(&self, callee: &Expr, args: &[Expr]) -> Option<IrExprKind> {
        let (table, op) = corvid_types::table_call_target(callee, self.file, self.symbols, self.bindings)?;
        Some(IrExprKind::TableOp {
            op,
            table: self.ir_table(table)?,
            args: args.iter().map(|arg| self.lower_expr(arg)).collect(),
        })
    }

    /// A query chain (B09 P2), recognized and read by the same
    /// `table_ops` functions the checker uses.
    fn try_table_query(&self, callee: &Expr, args: &[Expr]) -> Option<IrExprKind> {
        let query = corvid_types::table_ops::table_query_call(callee, args, self.file, self.symbols, self.bindings)?;
        let filter = match query.filter {
            Some(Expr::Lambda { params, body, .. }) => {
                let row = &params.first()?.name.name;
                let condition = corvid_types::table_ops::query_condition(query.table, row, body).ok()?;
                Some(self.lower_table_filter(&condition))
            }
            Some(_) => return None,
            None => None,
        };
        let order = match query.order {
            Some(Expr::Lambda { params, body, .. }) => {
                Some(corvid_types::table_ops::query_column(query.table, &params.first()?.name.name, body)?)
            }
            Some(_) => return None,
            None => None,
        };
        Some(IrExprKind::TableQuery {
            table: self.ir_table(query.table)?,
            filter,
            order,
            descending: query.descending,
            take: query.take.map(|count| Box::new(self.lower_expr(count))),
        })
    }

    fn lower_table_filter(&self, condition: &corvid_types::table_ops::QueryCondition<'_>) -> crate::types::IrTableFilter {
        use corvid_types::table_ops::QueryCondition as C;
        use crate::types::IrTableFilter as F;
        match condition {
            C::Const(b) => F::Const(*b),
            C::Column(column) => F::Column(column.clone()),
            C::And(left, right) => F::And(Box::new(self.lower_table_filter(left)), Box::new(self.lower_table_filter(right))),
            C::Or(left, right) => F::Or(Box::new(self.lower_table_filter(left)), Box::new(self.lower_table_filter(right))),
            C::Not(inner) => F::Not(Box::new(self.lower_table_filter(inner))),
            C::Compare { column, op, value } => {
                F::Compare { column: column.clone(), op: *op, value: Box::new(self.lower_expr(value)) }
            }
            C::Text { column, test, value } => {
                F::Text { column: column.clone(), test: *test, value: Box::new(self.lower_expr(value)) }
            }
        }
    }

    /// What a table's operations need: its row type, key, columns and database.
    fn ir_table(&self, table: &corvid_ast::TableDecl) -> Option<crate::types::IrTable> {
        let def_id = self.remap_def_id(self.symbols.lookup_def(&table.name().name)?);
        let database = corvid_types::table_ops::table_database(self.file, &table.name().name)?;
        Some(crate::types::IrTable {
            def_id,
            name: table.name().name.clone(),
            keys: table.keys.iter().map(|key| key.name.clone()).collect(),
            columns: table.record.fields.iter().map(|f| f.name.name.clone()).collect(),
            unique: table.unique.iter().map(|rule| rule.iter().map(|c| c.name.clone()).collect()).collect(),
            database: ir_database(database)?,
        })
    }

    /// `Instant.from_epoch_milliseconds(n)`: a method of a compiler-known
    /// type name, found with the lookup the checker used. Its one
    /// argument becomes the receiver.
    fn try_builtin_type_method_call(&self, target: &Expr, field: &Ident, args: &[Expr]) -> Option<IrExprKind> {
        let receiver = corvid_types::builtin_type_receiver(target, self.bindings)?;
        let sig = corvid_types::builtin_type_method(&receiver, &field.name)?;
        let (first, rest) = args.split_first()?;
        Some(IrExprKind::BuiltinMethod {
            kind: sig.kind,
            receiver: Box::new(self.lower_expr(first)),
            args: rest.iter().map(|a| self.lower_expr(a)).collect(),
        })
    }

    /// `Order.from_json(text)`: a method called on a type. The text
    /// becomes the receiver, and the target type travels in the call's
    /// checked `Result<T, JsonError>` type.
    fn try_type_method_call(
        &self,
        target: &Expr,
        field: &Ident,
        args: &[Expr],
    ) -> Option<IrExprKind> {
        // The checker decided the receiver names a type with this same
        // function, so the two layers cannot disagree.
        corvid_types::type_receiver(target, self.bindings, self.symbols, self.module_resolution)?;
        if args.len() != 1 {
            return None;
        }
        let sig = corvid_types::type_method(&Type::Unknown, &field.name)?;
        Some(IrExprKind::BuiltinMethod {
            kind: sig.kind,
            receiver: Box::new(self.lower_expr(&args[0])),
            args: Vec::new(),
        })
    }

    fn try_grounded_builtin_call(
        &self,
        target: &Expr,
        field: &Ident,
        args: &[Expr],
    ) -> Option<IrExprKind> {
        if field.name != "unwrap_discarding_sources" || !args.is_empty() {
            return None;
        }
        match self.types.get(&target.span())? {
            Type::Grounded(_) => Some(IrExprKind::UnwrapGrounded {
                value: Box::new(self.lower_expr(target)),
            }),
            _ => None,
        }
    }

    fn lower_test(&self, t: &TestDecl) -> IrTest {
        let id = self
            .symbols
            .lookup_def(&t.name.name)
            .unwrap_or(DefId(u32::MAX));
        IrTest {
            id,
            name: t.name.name.clone(),
            trace_fixture: t.trace_fixture.clone(),
            body: self.lower_block(&t.body),
            assertions: t
                .assertions
                .iter()
                .map(|assertion| self.lower_eval_assert(assertion))
                .collect(),
            span: t.span,
        }
    }

    fn lower_fixture(&self, f: &FixtureDecl) -> IrFixture {
        let id = self
            .symbols
            .lookup_def(&f.name.name)
            .unwrap_or(DefId(u32::MAX));
        IrFixture {
            id,
            name: f.name.name.clone(),
            params: self.lower_params(&f.params),
            return_ty: self.type_ref_to_type(&f.return_ty),
            body: self.lower_block(&f.body),
            span: f.span,
        }
    }

    fn lower_mock(&self, m: &MockDecl) -> IrMock {
        let target_id = match self.bindings.get(&m.target.span) {
            Some(Binding::Decl(def_id)) => self.remap_def_id(*def_id),
            _ => DefId(u32::MAX),
        };
        IrMock {
            target_id,
            target_name: m.target.name.clone(),
            params: self.lower_params(&m.params),
            return_ty: self.type_ref_to_type(&m.return_ty),
            body: self.lower_block(&m.body),
            span: m.span,
        }
    }

    fn try_imported_call(&self, callee: &Expr, field: &Ident, args: &[Expr]) -> Option<IrExprKind> {
        let target = self.imported_calls.get(&callee.span())?;
        let def_id = self.remap_imported_target(target);
        let kind = match target.kind {
            ImportedCallKind::Type => IrCallKind::StructConstructor { def_id },
            ImportedCallKind::Tool => IrCallKind::Tool {
                def_id,
                effect: Effect::Safe,
            },
            ImportedCallKind::Prompt => IrCallKind::Prompt { def_id },
            ImportedCallKind::Agent => IrCallKind::Agent { def_id },
        };
        Some(IrExprKind::Call {
            kind,
            callee_name: field.name.clone(),
            args: args.iter().map(|arg| self.lower_expr(arg)).collect(),
        })
    }

    fn type_expr_to_type(&self, expr: &Expr) -> Type {
        let Expr::Ident { name, .. } = expr else {
            return Type::Unknown;
        };
        match self.bindings.get(&name.span) {
            Some(Binding::BuiltIn(BuiltIn::Int)) => Type::Int,
            Some(Binding::BuiltIn(BuiltIn::Float)) => Type::Float,
            Some(Binding::BuiltIn(BuiltIn::Decimal)) => Type::Decimal,
            Some(Binding::BuiltIn(BuiltIn::Duration)) => Type::Duration,
            Some(Binding::BuiltIn(BuiltIn::Instant)) => Type::Instant,
            Some(Binding::BuiltIn(BuiltIn::TimeZone)) => Type::TimeZone,
            Some(Binding::BuiltIn(BuiltIn::ZonedTime)) => Type::ZonedTime,
            Some(Binding::BuiltIn(BuiltIn::Currency)) => Type::Currency,
            Some(Binding::BuiltIn(BuiltIn::Locale)) => Type::Locale,
            Some(Binding::BuiltIn(BuiltIn::Bytes)) => Type::Bytes,
            Some(Binding::BuiltIn(BuiltIn::Secret)) => Type::Secret,
            Some(Binding::BuiltIn(BuiltIn::String)) => Type::String,
            Some(Binding::BuiltIn(BuiltIn::Bool)) => Type::Bool,
            Some(Binding::BuiltIn(BuiltIn::Nothing)) => Type::Nothing,
            Some(Binding::Decl(def_id)) => Type::Struct(self.remap_def_id(*def_id)),
            _ => Type::Unknown,
        }
    }

    fn try_imported_lifted_call(&self, name: &Ident, args: &[Expr]) -> Option<IrExprKind> {
        let target = self.imported_calls.get(&name.span)?;
        let def_id = self.remap_imported_target(target);
        let kind = match target.kind {
            ImportedCallKind::Type => IrCallKind::StructConstructor { def_id },
            ImportedCallKind::Tool => IrCallKind::Tool {
                def_id,
                effect: Effect::Safe,
            },
            ImportedCallKind::Prompt => IrCallKind::Prompt { def_id },
            ImportedCallKind::Agent => IrCallKind::Agent { def_id },
        };
        Some(IrExprKind::Call {
            kind,
            callee_name: target.name.clone(),
            args: args.iter().map(|arg| self.lower_expr(arg)).collect(),
        })
    }

    /// The type a type reference stands for, as the checker resolved
    /// it, with ids remapped into the merged IR. The checker records
    /// every reference lowering reads (`checker/signature_types.rs`),
    /// so a missing entry is a compiler bug, not a user error.
    fn type_ref_to_type(&self, tr: &TypeRef) -> Type {
        let Some(ty) = self.type_refs.get(&tr.span()) else {
            panic!(
                "internal compiler error: the type checker did not record the type reference at {:?}; \
                 add its position to `record_signature_types`",
                tr.span()
            );
        };
        self.remap_struct_type_deep(ty.clone())
    }
}
/// Extract the `Upload<Format>` format tag (`Csv`, `Pdf`, …) from a
/// route body type ref (slice 52c-2). Returns `None` for any other
/// body type. The resolved `Type::Upload` loses the tag because the
/// format name is not a declared type, so serve reads it from here.
fn upload_format_tag(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Generic { name, args, .. } if name.name == "Upload" && args.len() == 1 => {
            match &args[0] {
                TypeRef::Named { name, .. } => Some(name.name.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Lower a connector's `auth:` clause (slice 52g-3). Only the secret
/// reference NAMES are carried — never a literal value — so a trace can
/// name which secret was used without ever revealing it.
fn lower_connector_auth(auth: &ConnectorAuth) -> IrConnectorAuth {
    match auth {
        ConnectorAuth::Bearer(secret) => IrConnectorAuth::Bearer {
            secret: secret.name.clone(),
        },
        ConnectorAuth::Header { name, value } => IrConnectorAuth::Header {
            name: name.clone(),
            secret: value.name.clone(),
        },
        ConnectorAuth::Basic { username, password } => IrConnectorAuth::Basic {
            username_secret: username.name.clone(),
            password_secret: password.name.clone(),
        },
    }
}

fn numeric_profile_dimension(profile: &corvid_types::effects::ComposedProfile, dim: &str) -> f64 {
    match profile.dimensions.get(dim) {
        Some(corvid_ast::DimensionValue::Cost(value)) => *value,
        Some(corvid_ast::DimensionValue::Number(value)) => *value,
        _ => 0.0,
    }
}

fn confidence_profile_dimension(profile: &corvid_types::effects::ComposedProfile) -> f64 {
    match profile.dimensions.get("confidence") {
        Some(corvid_ast::DimensionValue::Number(value)) => *value,
        _ => 1.0,
    }
}

/// A composed effect profile is reversible unless it carries an explicit
/// `reversible: false` (slice 52d-1). Effects default to reversible; the
/// `LeastReversible` composition rule inserts `Bool(false)` the moment
/// any composed effect is irreversible.
fn profile_is_reversible(profile: &corvid_types::effects::ComposedProfile) -> bool {
    !matches!(
        profile.dimensions.get("reversible"),
        Some(corvid_ast::DimensionValue::Bool(false))
    )
}

fn agent_cost_budget(agent: &AgentDecl) -> Option<f64> {
    agent
        .constraints
        .iter()
        .filter(|constraint| canonical_dimension_name(&constraint.dimension.name) == "cost")
        .filter_map(numeric_constraint_value)
        .reduce(f64::min)
}

fn is_wrapping_int_binop(op: BinaryOp, ty: Option<&Type>) -> bool {
    // See through `Grounded<>`: a grounded `Int` operator under a
    // `@wrapping` agent must lower to the wrapping path, not the
    // overflow-trapping one — `Grounded<Int>` is operationally an
    // `Int` (Provenance Propagation, native value-correctness).
    matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
        && matches!(ty.map(Type::ungrounded), Some(Type::Int))
}

fn is_wrapping_int_unop(op: UnaryOp, ty: Option<&Type>) -> bool {
    matches!(op, UnaryOp::Neg) && matches!(ty.map(Type::ungrounded), Some(Type::Int))
}

/// Retrieve a tool's declared effect by its `DefId`.
///
/// Note: the `SymbolTable` only stores `DeclEntry`, not the full decl.
/// We don't have access to the AST here without plumbing it in, so we
/// conservatively return `Safe`. The IR also records effect on `IrTool`
/// itself, so codegen should prefer that. A refactor to flow effects
/// through the symbol table can happen when it becomes a hot path.
fn lookup_tool_effect(_symbols: &SymbolTable, _def_id: DefId) -> Effect {
    Effect::Safe
}


/// The 46c history-recognition rule, applied syntactically: a
/// parameter typed `List<AiMessage>`. Works across module
/// boundaries (the std/ai import) without def-id gymnastics; the
/// checker verifies the local shape and the VM validates values.
pub fn param_is_history(p: &Param) -> bool {
    matches!(&p.ty, corvid_ast::TypeRef::Generic { name, args, .. }
        if name.name == "List"
            && args.len() == 1
            && matches!(&args[0], corvid_ast::TypeRef::Named { name, .. } if name.name == "AiMessage"))
}

/// A `database` block as table operations and `atomic:` blocks run it.
fn ir_database(database: &corvid_ast::DatabaseDecl) -> Option<crate::types::IrDatabase> {
    Some(crate::types::IrDatabase {
        name: database.name.name.clone(),
        engine: database.engine.as_ref()?.name.clone(),
        location: database.location.clone()?,
        tls: database.tls.as_ref().map(|tls| tls.name.clone()),
    })
}
