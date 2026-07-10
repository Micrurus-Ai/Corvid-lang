//! WebAssembly code generator.
//!
//! Phase 23 starts with a deliberately honest deployment surface:
//! scalar, runtime-free agents compile to a standalone `.wasm` module
//! plus JS and TypeScript companions. AI-native host imports for LLMs,
//! tools, approvals, replay recording, and provenance are follow-up
//! slices because they need a real browser/edge host-capability ABI.

use crate::string_pool::StringPool;
use corvid_ast::{BinaryOp, UnaryOp};
use corvid_ir::{
    IrAgent, IrBlock, IrCallKind, IrExpr, IrExprKind, IrFile, IrLiteral, IrPathSeg, IrPrompt,
    IrStmt, IrTool,
};
use corvid_resolve::{DefId, LocalId};
use corvid_types::Type;
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, ImportSection, Instruction, MemorySection, Module, TypeSection,
    ValType,
};

mod allocator;
mod companions;
mod error;
mod string_pool;

pub use companions::WasmArtifacts;
pub use error::WasmCodegenError;

const HOST_MODULE: &str = "corvid:host";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostImportKind {
    Prompt,
    Tool,
    Approval,
}

impl HostImportKind {
    pub(crate) fn namespace(self) -> &'static str {
        match self {
            HostImportKind::Prompt => "prompts",
            HostImportKind::Tool => "tools",
            HostImportKind::Approval => "approvals",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            HostImportKind::Prompt => "prompt",
            HostImportKind::Tool => "tool",
            HostImportKind::Approval => "approval",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WasmHostImport {
    pub kind: HostImportKind,
    pub source_name: String,
    pub import_name: String,
    pub params: Vec<(String, Type)>,
    pub return_ty: Type,
}

struct HostImportPlan {
    imports: Vec<WasmHostImport>,
    tool_indices: HashMap<DefId, u32>,
    prompt_indices: HashMap<DefId, u32>,
    approval_indices: HashMap<String, u32>,
}

pub fn emit_wasm_artifacts(
    ir: &IrFile,
    module_name: &str,
) -> Result<WasmArtifacts, WasmCodegenError> {
    let scalar_agents = ir
        .agents
        .iter()
        .map(validate_agent)
        .collect::<Result<Vec<_>, _>>()?;
    let host_plan = collect_host_imports(ir, &scalar_agents)?;

    // Pre-codegen pass: walk every agent body and intern each
    // `IrLiteral::String` into the compile-time string pool. The
    // pool's bytes get emitted into a `DataSection` at offset
    // `allocator::HEAP_BASE`; the allocator's `$heap_top` global is
    // initialised to `HEAP_BASE + pool.total_bytes()` so user heap
    // allocations begin immediately past the literal pool.
    let mut string_pool = StringPool::new();
    for agent in &scalar_agents {
        intern_string_literals(&agent.body, &mut string_pool);
    }
    let heap_top_init = allocator::HEAP_BASE + string_pool.total_bytes() as i32;

    let mut types = TypeSection::new();
    let mut imports = ImportSection::new();
    let mut funcs = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    let mut mem = MemorySection::new();
    let mut globals = GlobalSection::new();
    let mut data = DataSection::new();

    for host_import in &host_plan.imports {
        let params = host_import
            .params
            .iter()
            .map(|(_, ty)| wasm_val_type(ty))
            .collect::<Result<Vec<_>, _>>()?;
        let results = wasm_result_types(&host_import.return_ty)?;
        let type_index = types.len();
        types.ty().function(params, results);
        imports.import(
            HOST_MODULE,
            &host_import.import_name,
            wasm_encoder::EntityType::Function(type_index),
        );
    }

    // Emit the allocator immediately after host imports. Its
    // function indices occupy slots `host_imports.len() ..
    // host_imports.len() + alloc_indices.func_count`. Agent
    // function indices shift accordingly. The allocator also owns
    // the module's single linear memory and the two global slots
    // (`$heap_top`, `$free_head`); no other code is allowed to
    // populate `mem` / `globals` after this point.
    let alloc_indices = allocator::emit_allocator(
        &mut types,
        &mut funcs,
        &mut code,
        &mut exports,
        &mut mem,
        &mut globals,
        host_plan.imports.len() as u32,
        heap_top_init,
    );

    // Emit the literal-pool data segment when the pool is non-empty.
    // Active segment at memory[0], offset = `HEAP_BASE`.
    if string_pool.total_bytes() > 0 {
        data.active(
            0,
            &ConstExpr::i32_const(allocator::HEAP_BASE),
            string_pool.bytes().iter().copied(),
        );
    }
    let agent_index_base = host_plan.imports.len() as u32 + alloc_indices.func_count;

    let mut agent_indices = HashMap::new();
    for (idx, agent) in scalar_agents.iter().enumerate() {
        agent_indices.insert(agent.id, agent_index_base + idx as u32);
    }

    for agent in &scalar_agents {
        // Each param contributes one or more `ValType`s — `Int`,
        // `Float`, `Bool` are single-slot; `String` expands to a
        // `(ptr, len)` `i32` pair. Flatten across all params to get
        // the function's param-list.
        let mut params: Vec<ValType> = Vec::with_capacity(agent.params.len());
        for param in &agent.params {
            params.extend(wasm_param_value_types(&param.ty)?);
        }
        let results = wasm_result_types(&agent.return_ty)?;
        let type_index = types.len();
        types.ty().function(params, results);
        funcs.function(type_index);
    }

    for (idx, agent) in scalar_agents.iter().enumerate() {
        exports.export(
            &agent.name,
            ExportKind::Func,
            agent_index_base + idx as u32,
        );
        let function = compile_agent(agent, &agent_indices, &host_plan, &string_pool)?;
        code.function(&function);
    }

    // WASM core module section ordering is fixed:
    //   type → import → function → memory → global → export → code → data
    // (table, start, element sections omitted in v1).
    let mut module = Module::new();
    module.section(&types);
    if !host_plan.imports.is_empty() {
        module.section(&imports);
    }
    module.section(&funcs);
    module.section(&mem);
    module.section(&globals);
    module.section(&exports);
    module.section(&code);
    if string_pool.total_bytes() > 0 {
        module.section(&data);
    }

    companions::build_artifacts(
        module_name,
        &scalar_agents,
        &host_plan.imports,
        module.finish(),
    )
}

fn validate_agent(agent: &IrAgent) -> Result<&IrAgent, WasmCodegenError> {
    if agent.extern_abi.is_some() {
        // Slice 33Q17d: point readers at the doc page that owns the
        // cdylib-only contract. Pre-33Q17d the message named the
        // restriction but left the reader to guess where the contract
        // is documented; the doc page explains the JSON-wire struct
        // boundary, the scalar set, and why the boundary lives in
        // cdylib and not wasm.
        return Err(WasmCodegenError::unsupported(format!(
            "wasm target does not lower `pub extern \"c\"` agent `{}`. \
             The `pub extern \"c\"` boundary is cdylib-only — wasm exports \
             normal Corvid agents. See `docs/reference/exported-abi.md` \
             for the boundary contract; drop the `pub extern \"c\"` modifier \
             to make this agent browser/edge-callable.",
            agent.name
        )));
    }
    for param in &agent.params {
        wasm_param_value_types(&param.ty).map_err(|_| {
            WasmCodegenError::unsupported(format!(
                "wasm target supports Int, Float, Bool, Nothing, and String agent parameters; agent `{}` parameter `{}` has `{}`",
                agent.name,
                param.name,
                param.ty.display_name()
            ))
        })?;
    }
    wasm_result_types(&agent.return_ty).map_err(|_| {
        WasmCodegenError::unsupported(format!(
            "wasm target supports Int, Float, Bool, Nothing, and String agent returns; agent `{}` returns `{}`",
            agent.name,
            agent.return_ty.display_name()
        ))
    })?;
    Ok(agent)
}

fn collect_host_imports(
    ir: &IrFile,
    agents: &[&IrAgent],
) -> Result<HostImportPlan, WasmCodegenError> {
    let tools = ir
        .tools
        .iter()
        .map(|tool| (tool.id, tool))
        .collect::<HashMap<_, _>>();
    let prompts = ir
        .prompts
        .iter()
        .map(|prompt| (prompt.id, prompt))
        .collect::<HashMap<_, _>>();
    let mut plan = HostImportPlan {
        imports: Vec::new(),
        tool_indices: HashMap::new(),
        prompt_indices: HashMap::new(),
        approval_indices: HashMap::new(),
    };
    for agent in agents {
        collect_block_imports(&agent.body, &tools, &prompts, &mut plan, &agent.name)?;
    }
    Ok(plan)
}

/// Pre-codegen walk: visit every `IrLiteral::String` inside `block`
/// and intern it into `pool`. Mirrors `collect_block_imports`'s
/// recursive visitor shape; only difference is the leaf action
/// (intern vs. add to host-plan).
fn intern_string_literals(block: &IrBlock, pool: &mut StringPool) {
    for stmt in &block.stmts {
        match stmt {
            IrStmt::Let { value, .. } | IrStmt::Expr { expr: value, .. } => {
                intern_expr_literals(value, pool);
            }
            IrStmt::Assign { path, value, .. } => {
                for seg in path {
                    if let IrPathSeg::Index(idx) = seg {
                        intern_expr_literals(idx, pool);
                    }
                }
                intern_expr_literals(value, pool);
            }
            IrStmt::Return { value, .. } => {
                if let Some(value) = value {
                    intern_expr_literals(value, pool);
                }
            }
            IrStmt::Yield { value, .. } => {
                intern_expr_literals(value, pool);
            }
            IrStmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                intern_expr_literals(cond, pool);
                intern_string_literals(then_block, pool);
                if let Some(else_block) = else_block {
                    intern_string_literals(else_block, pool);
                }
            }
            IrStmt::For { iter, body, .. } => {
                intern_expr_literals(iter, pool);
                intern_string_literals(body, pool);
            }
            IrStmt::Approve { args, .. } => {
                for arg in args {
                    intern_expr_literals(arg, pool);
                }
            }
            IrStmt::Break { .. }
            | IrStmt::Continue { .. }
            | IrStmt::Pass { .. }
            | IrStmt::Dup { .. }
            | IrStmt::Drop { .. } => {}
        }
    }
}

fn intern_expr_literals(expr: &IrExpr, pool: &mut StringPool) {
    match &expr.kind {
        IrExprKind::Literal(IrLiteral::String(value)) => {
            pool.intern(value);
        }
        IrExprKind::Literal(_) | IrExprKind::Local { .. } => {}
        IrExprKind::Call { args, .. } => {
            for arg in args {
                intern_expr_literals(arg, pool);
            }
        }
        IrExprKind::BinOp { left, right, .. }
        | IrExprKind::WrappingBinOp { left, right, .. } => {
            intern_expr_literals(left, pool);
            intern_expr_literals(right, pool);
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            intern_expr_literals(operand, pool);
        }
        // The wasm target rejects field access / cast / partial /
        // grounded / template / route / structconstructor / weak /
        // result / option / stream surfaces in the same `emit_expr`
        // pass that reaches them; if we don't reach them in `emit`
        // we don't need to intern through them either. Treat the
        // catch-all as "no nested string literals to discover."
        _ => {}
    }
}

fn collect_block_imports(
    block: &IrBlock,
    tools: &HashMap<DefId, &IrTool>,
    prompts: &HashMap<DefId, &IrPrompt>,
    plan: &mut HostImportPlan,
    agent_name: &str,
) -> Result<(), WasmCodegenError> {
    for stmt in &block.stmts {
        match stmt {
            IrStmt::Let { value, .. } | IrStmt::Expr { expr: value, .. } => {
                collect_expr_imports(value, tools, prompts, plan, agent_name)?;
            }
            IrStmt::Assign { path, value, .. } => {
                for seg in path {
                    if let IrPathSeg::Index(idx) = seg {
                        collect_expr_imports(idx, tools, prompts, plan, agent_name)?;
                    }
                }
                collect_expr_imports(value, tools, prompts, plan, agent_name)?;
            }
            IrStmt::Return { value, .. } => {
                if let Some(value) = value {
                    collect_expr_imports(value, tools, prompts, plan, agent_name)?;
                }
            }
            IrStmt::Yield { value, .. } => {
                collect_expr_imports(value, tools, prompts, plan, agent_name)?;
            }
            IrStmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                collect_expr_imports(cond, tools, prompts, plan, agent_name)?;
                collect_block_imports(then_block, tools, prompts, plan, agent_name)?;
                if let Some(else_block) = else_block {
                    collect_block_imports(else_block, tools, prompts, plan, agent_name)?;
                }
            }
            IrStmt::For { .. } => {
                return Err(WasmCodegenError::unsupported(format!(
                    "wasm target does not yet lower loops in agent `{agent_name}`"
                )));
            }
            IrStmt::Approve { label, args, .. } => {
                for arg in args {
                    collect_expr_imports(arg, tools, prompts, plan, agent_name)?;
                    wasm_val_type(&arg.ty).map_err(|_| {
                        WasmCodegenError::unsupported(format!(
                            "wasm approval `{label}` in agent `{agent_name}` has unsupported argument type `{}`",
                            arg.ty.display_name()
                        ))
                    })?;
                }
                add_approval_import(plan, label, args)?;
            }
            IrStmt::Break { .. } | IrStmt::Continue { .. } | IrStmt::Pass { .. } => {}
            IrStmt::Dup { .. } | IrStmt::Drop { .. } => {}
        }
    }
    Ok(())
}

fn collect_expr_imports(
    expr: &IrExpr,
    tools: &HashMap<DefId, &IrTool>,
    prompts: &HashMap<DefId, &IrPrompt>,
    plan: &mut HostImportPlan,
    agent_name: &str,
) -> Result<(), WasmCodegenError> {
    match &expr.kind {
        IrExprKind::MapLiteral { keys, values } => {
            for e in keys.iter().chain(values) {
                collect_expr_imports(e, tools, prompts, plan, agent_name)?;
            }
            Ok(())
        }
        IrExprKind::BuiltinMethod { receiver, args, .. } => {
            collect_expr_imports(receiver, tools, prompts, plan, agent_name)?;
            for arg in args {
                collect_expr_imports(arg, tools, prompts, plan, agent_name)?;
            }
            Ok(())
        }
        IrExprKind::Call {
            kind,
            args,
            callee_name,
        } => {
            for arg in args {
                collect_expr_imports(arg, tools, prompts, plan, agent_name)?;
            }
            match kind {
                IrCallKind::Agent { .. } => Ok(()),
                IrCallKind::Fixture { .. } => Err(WasmCodegenError::unsupported(format!(
                    "wasm target cannot lower test fixture call `{callee_name}`"
                ))),
                IrCallKind::Tool { def_id, .. } => {
                    let tool = tools.get(def_id).ok_or_else(|| {
                        WasmCodegenError::unsupported(format!(
                            "wasm target could not resolve tool import `{callee_name}`"
                        ))
                    })?;
                    add_tool_import(plan, tool)
                }
                IrCallKind::Prompt { def_id } => {
                    let prompt = prompts.get(def_id).ok_or_else(|| {
                        WasmCodegenError::unsupported(format!(
                            "wasm target could not resolve prompt import `{callee_name}`"
                        ))
                    })?;
                    add_prompt_import(plan, prompt)
                }
                IrCallKind::StructConstructor { .. } | IrCallKind::Unknown => Err(
                    WasmCodegenError::unsupported(format!(
                        "wasm target currently supports scalar runtime-free agents; call `{callee_name}` in agent `{agent_name}` is not scalar"
                    )),
                ),
            }
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            collect_expr_imports(left, tools, prompts, plan, agent_name)?;
            collect_expr_imports(right, tools, prompts, plan, agent_name)
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            collect_expr_imports(operand, tools, prompts, plan, agent_name)
        }
        IrExprKind::FieldAccess { target, .. }
        | IrExprKind::Index { target, .. }
        | IrExprKind::UnwrapGrounded { value: target }
        | IrExprKind::WeakNew { strong: target }
        | IrExprKind::WeakUpgrade { weak: target }
        | IrExprKind::StreamSplitBy { stream: target, .. }
        | IrExprKind::StreamMerge { groups: target, .. }
        | IrExprKind::StreamOrderedBy { stream: target, .. }
        | IrExprKind::StreamResumeToken { stream: target }
        | IrExprKind::ResumeStream { token: target, .. }
        | IrExprKind::ResultOk { inner: target }
        | IrExprKind::ResultErr { inner: target }
        | IrExprKind::OptionSome { inner: target }
        | IrExprKind::Ask { prompt: target, .. }
        | IrExprKind::Choose { options: target }
        | IrExprKind::TryPropagate { inner: target } => {
            collect_expr_imports(target, tools, prompts, plan, agent_name)
        }
        IrExprKind::TryRetry { body, .. } => {
            collect_expr_imports(body, tools, prompts, plan, agent_name)
        }
        IrExprKind::List { items } => {
            for item in items {
                collect_expr_imports(item, tools, prompts, plan, agent_name)?;
            }
            Ok(())
        }
        IrExprKind::Replay {
            trace,
            arms,
            else_body,
        } => {
            collect_expr_imports(trace, tools, prompts, plan, agent_name)?;
            for arm in arms {
                collect_expr_imports(&arm.body, tools, prompts, plan, agent_name)?;
            }
            collect_expr_imports(else_body, tools, prompts, plan, agent_name)
        }
        IrExprKind::Literal(_)
        | IrExprKind::Local { .. }
        | IrExprKind::Decl { .. }
        | IrExprKind::OptionNone => Ok(()),
    }
}

fn add_tool_import(plan: &mut HostImportPlan, tool: &IrTool) -> Result<(), WasmCodegenError> {
    if plan.tool_indices.contains_key(&tool.id) {
        return Ok(());
    }
    let import = WasmHostImport {
        kind: HostImportKind::Tool,
        source_name: tool.name.clone(),
        import_name: format!("tool.{}", tool.name),
        params: tool
            .params
            .iter()
            .map(|param| validate_import_param(&tool.name, &param.name, &param.ty))
            .collect::<Result<Vec<_>, _>>()?,
        return_ty: validate_import_return(&tool.name, &tool.return_ty)?,
    };
    let index = plan.imports.len() as u32;
    plan.tool_indices.insert(tool.id, index);
    plan.imports.push(import);
    Ok(())
}

fn add_prompt_import(plan: &mut HostImportPlan, prompt: &IrPrompt) -> Result<(), WasmCodegenError> {
    if plan.prompt_indices.contains_key(&prompt.id) {
        return Ok(());
    }
    let import = WasmHostImport {
        kind: HostImportKind::Prompt,
        source_name: prompt.name.clone(),
        import_name: format!("prompt.{}", prompt.name),
        params: prompt
            .params
            .iter()
            .map(|param| validate_import_param(&prompt.name, &param.name, &param.ty))
            .collect::<Result<Vec<_>, _>>()?,
        return_ty: validate_import_return(&prompt.name, &prompt.return_ty)?,
    };
    let index = plan.imports.len() as u32;
    plan.prompt_indices.insert(prompt.id, index);
    plan.imports.push(import);
    Ok(())
}

fn add_approval_import(
    plan: &mut HostImportPlan,
    label: &str,
    args: &[IrExpr],
) -> Result<(), WasmCodegenError> {
    if plan.approval_indices.contains_key(label) {
        return Ok(());
    }
    let import = WasmHostImport {
        kind: HostImportKind::Approval,
        source_name: label.to_string(),
        import_name: format!("approve.{label}"),
        params: args
            .iter()
            .enumerate()
            .map(|(idx, arg)| validate_import_param(label, &format!("arg{}", idx + 1), &arg.ty))
            .collect::<Result<Vec<_>, _>>()?,
        return_ty: Type::Bool,
    };
    let index = plan.imports.len() as u32;
    plan.approval_indices.insert(label.to_string(), index);
    plan.imports.push(import);
    Ok(())
}

fn validate_import_param(
    owner: &str,
    name: &str,
    ty: &Type,
) -> Result<(String, Type), WasmCodegenError> {
    wasm_val_type(ty).map_err(|_| {
        WasmCodegenError::unsupported(format!(
            "wasm host import `{owner}` parameter `{name}` has unsupported type `{}`",
            ty.display_name()
        ))
    })?;
    Ok((name.to_string(), ty.clone()))
}

fn validate_import_return(owner: &str, ty: &Type) -> Result<Type, WasmCodegenError> {
    wasm_result_types(ty).map_err(|_| {
        WasmCodegenError::unsupported(format!(
            "wasm host import `{owner}` returns unsupported type `{}`",
            ty.display_name()
        ))
    })?;
    Ok(ty.clone())
}

fn compile_agent(
    agent: &IrAgent,
    agent_indices: &HashMap<DefId, u32>,
    host_plan: &HostImportPlan,
    string_pool: &StringPool,
) -> Result<Function, WasmCodegenError> {
    let mut locals = LocalLayout::from_agent(agent)?;
    collect_block_locals(&agent.body, &mut locals)?;
    let local_groups = locals.local_groups();
    let mut function = Function::new(local_groups);

    emit_block(
        &agent.body,
        &mut function,
        &locals,
        agent_indices,
        host_plan,
        string_pool,
    )?;
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    Ok(function)
}

fn collect_block_locals(block: &IrBlock, locals: &mut LocalLayout) -> Result<(), WasmCodegenError> {
    for stmt in &block.stmts {
        match stmt {
            IrStmt::Let {
                local_id, ty, name, ..
            } => locals.add_local(*local_id, name, ty)?,
            IrStmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_block_locals(then_block, locals)?;
                if let Some(else_block) = else_block {
                    collect_block_locals(else_block, locals)?;
                }
            }
            IrStmt::For { .. } => {
                return Err(WasmCodegenError::unsupported(
                    "wasm target does not yet lower loop locals",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Per-`LocalId` mapping into WASM's flat local index space. `String`
/// locals occupy two consecutive WASM slots — one for the `(ptr,
/// len)` pair's `ptr` and one for `len`. All other types take one
/// slot. Storing the start-index plus a count is enough; consumers
/// either need the single index (for scalar locals) or both indices
/// (for the String pair).
#[derive(Debug, Clone, Copy)]
struct LocalSlot {
    /// First WASM local index occupied by the slot.
    start: u32,
    /// Number of WASM locals (1 for scalar, 2 for String).
    count: u32,
}

struct LocalLayout {
    map: HashMap<LocalId, LocalSlot>,
    locals: Vec<(String, ValType)>,
}

impl LocalLayout {
    fn from_agent(agent: &IrAgent) -> Result<Self, WasmCodegenError> {
        let mut layout = Self {
            map: HashMap::new(),
            locals: Vec::new(),
        };
        let mut next_index: u32 = 0;
        for param in &agent.params {
            let slot_types = wasm_param_value_types(&param.ty)?;
            let count = slot_types.len() as u32;
            layout.map.insert(
                param.local_id,
                LocalSlot {
                    start: next_index,
                    count,
                },
            );
            next_index += count;
        }
        Ok(layout)
    }

    fn add_local(
        &mut self,
        local_id: LocalId,
        name: &str,
        ty: &Type,
    ) -> Result<(), WasmCodegenError> {
        if self.map.contains_key(&local_id) {
            return Ok(());
        }
        let slot_types = wasm_param_value_types(ty).map_err(|_| {
            WasmCodegenError::unsupported(format!(
                "wasm target local `{name}` has unsupported type `{}`",
                ty.display_name()
            ))
        })?;
        let count = slot_types.len() as u32;
        // Total occupied slots so far = sum of counts; equivalently
        // the next free index is the highest-`start + count` across
        // entries. We track this by walking the map; the param-count
        // path of `from_agent` initialised entries densely so this
        // works.
        let next_index = self
            .map
            .values()
            .map(|slot| slot.start + slot.count)
            .max()
            .unwrap_or(0)
            .max(self.locals.len() as u32 + count_param_slots(&self.locals));
        self.map.insert(
            local_id,
            LocalSlot {
                start: next_index,
                count,
            },
        );
        for (offset, ty) in slot_types.into_iter().enumerate() {
            let suffix = if count == 1 {
                String::new()
            } else if offset == 0 {
                "_ptr".to_string()
            } else {
                "_len".to_string()
            };
            self.locals.push((format!("{name}{suffix}"), ty));
        }
        Ok(())
    }

    fn local_groups(&self) -> Vec<(u32, ValType)> {
        self.locals.iter().map(|(_, ty)| (1, *ty)).collect()
    }

    /// Look up the slot for a local. Returns the `LocalSlot` so the
    /// caller can decide whether to emit a single `LocalGet`/`LocalSet`
    /// or a pair (for `String`).
    fn slot(&self, local_id: LocalId, name: &str) -> Result<LocalSlot, WasmCodegenError> {
        self.map.get(&local_id).copied().ok_or_else(|| {
            WasmCodegenError::unsupported(format!("wasm target could not resolve local `{name}`"))
        })
    }
}

/// Sum of slot counts across already-emitted locals. Used so
/// `add_local`'s next-index computation stays consistent when a
/// caller has added no locals yet (`max` of an empty iter is 0,
/// but the param-side may already have consumed `N` slots).
fn count_param_slots(_locals: &[(String, ValType)]) -> u32 {
    // The locals vector tracks ONLY post-parameter locals (params
    // are encoded in the function type, not in the locals section).
    // The next-index computation in `add_local` must include the
    // parameter slot count, but since `LocalLayout::map` already
    // records every param's slot, the `map.values().max()` walk
    // captures it. This helper exists to keep the math explicit
    // even though the value is currently 0; when struct returns
    // arrive in 20n-C and may add hidden parameter slots, this
    // is the place to centralise the offset.
    0
}

fn emit_block(
    block: &IrBlock,
    function: &mut Function,
    locals: &LocalLayout,
    agent_indices: &HashMap<DefId, u32>,
    host_plan: &HostImportPlan,
    string_pool: &StringPool,
) -> Result<(), WasmCodegenError> {
    for stmt in &block.stmts {
        match stmt {
            IrStmt::Let {
                local_id,
                name,
                value,
                ..
            } => {
                emit_expr(value, function, locals, agent_indices, host_plan, string_pool)?;
                let slot = locals.slot(*local_id, name)?;
                if slot.count == 1 {
                    function.instruction(&Instruction::LocalSet(slot.start));
                } else {
                    // Multi-slot local (currently `String` only).
                    // The expression pushed `(ptr, len)` in that
                    // order, so `len` is on top of the WASM stack;
                    // store it first into the second slot, then
                    // `ptr` into the first.
                    for offset in (0..slot.count).rev() {
                        function.instruction(&Instruction::LocalSet(slot.start + offset));
                    }
                }
            }
            IrStmt::Assign { span, .. } => {
                return Err(WasmCodegenError::unsupported(format!(
                    "place assignment (x.field = v / xs[i] = v / compound ops) is                      interpreter-only in 45b (at {span:?})"
                )));
            }
            IrStmt::Return { value, .. } => {
                if let Some(value) = value {
                    emit_expr(value, function, locals, agent_indices, host_plan, string_pool)?;
                }
                function.instruction(&Instruction::Return);
            }
            IrStmt::Expr { expr, .. } => {
                emit_expr(expr, function, locals, agent_indices, host_plan, string_pool)?;
                if !matches!(expr.ty, Type::Nothing) {
                    function.instruction(&Instruction::Drop);
                }
            }
            IrStmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                emit_expr(cond, function, locals, agent_indices, host_plan, string_pool)?;
                function.instruction(&Instruction::If(BlockType::Empty));
                emit_block(then_block, function, locals, agent_indices, host_plan, string_pool)?;
                if let Some(else_block) = else_block {
                    function.instruction(&Instruction::Else);
                    emit_block(else_block, function, locals, agent_indices, host_plan, string_pool)?;
                }
                function.instruction(&Instruction::End);
            }
            IrStmt::Approve { label, args, .. } => {
                for arg in args {
                    emit_expr(arg, function, locals, agent_indices, host_plan, string_pool)?;
                }
                let index = host_plan
                    .approval_indices
                    .get(label)
                    .copied()
                    .ok_or_else(|| {
                        WasmCodegenError::unsupported(format!(
                            "wasm target could not resolve approval import `{label}`"
                        ))
                    })?;
                function.instruction(&Instruction::Call(index));
                function.instruction(&Instruction::If(BlockType::Empty));
                function.instruction(&Instruction::Else);
                function.instruction(&Instruction::Unreachable);
                function.instruction(&Instruction::End);
            }
            IrStmt::Pass { .. } | IrStmt::Dup { .. } | IrStmt::Drop { .. } => {}
            IrStmt::Yield { .. }
            | IrStmt::For { .. }
            | IrStmt::Break { .. }
            | IrStmt::Continue { .. } => {
                return Err(WasmCodegenError::unsupported(format!(
                    "wasm target cannot lower statement `{stmt:?}` yet"
                )));
            }
        }
    }
    Ok(())
}

fn emit_expr(
    expr: &IrExpr,
    function: &mut Function,
    locals: &LocalLayout,
    agent_indices: &HashMap<DefId, u32>,
    host_plan: &HostImportPlan,
    string_pool: &StringPool,
) -> Result<(), WasmCodegenError> {
    match &expr.kind {
        IrExprKind::MapLiteral { .. } => {
            return Err(WasmCodegenError::unsupported(
                "Map<K, V> is interpreter-only in 45g".to_string(),
            ));
        }
        IrExprKind::BuiltinMethod { .. } => {
            return Err(WasmCodegenError::unsupported(
                "builtin methods (String.length() and the 45d/45e/45f batches) are \
                 interpreter-only in 45c"
                    .to_string(),
            ));
        }
        IrExprKind::Literal(IrLiteral::Int(value)) => {
            function.instruction(&Instruction::I64Const(*value));
        }
        IrExprKind::Literal(IrLiteral::Float(value)) => {
            function.instruction(&Instruction::F64Const((*value).into()));
        }
        IrExprKind::Literal(IrLiteral::Bool(value)) => {
            function.instruction(&Instruction::I32Const(i32::from(*value)));
        }
        IrExprKind::Literal(IrLiteral::String(value)) => {
            // Slice 20n-B-2b: lower a string literal as a `(ptr, len)`
            // pair pointing into the compile-time literal pool. The
            // pool was populated during `intern_string_literals`'s
            // pre-codegen walk and lives at memory[HEAP_BASE ..
            // HEAP_BASE + pool.total_bytes()] via the active
            // `DataSection` segment. Both `ptr` and `len` are
            // compile-time constants — no runtime allocation
            // required for literal-only String returns.
            let (offset, len) = string_pool.lookup(value);
            function.instruction(&Instruction::I32Const(allocator::HEAP_BASE + offset as i32));
            function.instruction(&Instruction::I32Const(len as i32));
        }
        IrExprKind::Literal(IrLiteral::Nothing) => {}
        IrExprKind::Local { local_id, name } => {
            let slot = locals.slot(*local_id, name)?;
            // Single-slot scalars push exactly one value; multi-
            // slot `String` pushes `(ptr, len)` in that order so
            // the result types match the function signature's
            // multi-value return shape.
            for offset in 0..slot.count {
                function.instruction(&Instruction::LocalGet(slot.start + offset));
            }
        }
        IrExprKind::Call {
            kind,
            args,
            callee_name,
        } => match kind {
            IrCallKind::Agent { def_id } => {
                for arg in args {
                    emit_expr(arg, function, locals, agent_indices, host_plan, string_pool)?;
                }
                let index = agent_indices.get(def_id).copied().ok_or_else(|| {
                    WasmCodegenError::unsupported(format!(
                        "wasm target could not resolve agent call `{callee_name}`"
                    ))
                })?;
                function.instruction(&Instruction::Call(index));
            }
            IrCallKind::Tool { def_id, .. } => {
                for arg in args {
                    emit_expr(arg, function, locals, agent_indices, host_plan, string_pool)?;
                }
                let index = host_plan.tool_indices.get(def_id).copied().ok_or_else(|| {
                    WasmCodegenError::unsupported(format!(
                        "wasm target could not resolve tool import `{callee_name}`"
                    ))
                })?;
                function.instruction(&Instruction::Call(index));
            }
            IrCallKind::Prompt { def_id } => {
                for arg in args {
                    emit_expr(arg, function, locals, agent_indices, host_plan, string_pool)?;
                }
                let index = host_plan
                    .prompt_indices
                    .get(def_id)
                    .copied()
                    .ok_or_else(|| {
                        WasmCodegenError::unsupported(format!(
                            "wasm target could not resolve prompt import `{callee_name}`"
                        ))
                    })?;
                function.instruction(&Instruction::Call(index));
            }
            IrCallKind::Fixture { .. } => {
                return Err(WasmCodegenError::unsupported(format!(
                    "wasm target cannot lower test fixture call `{callee_name}`"
                )));
            }
            IrCallKind::StructConstructor { .. } | IrCallKind::Unknown => {
                return Err(WasmCodegenError::unsupported(format!(
                    "wasm target cannot lower non-scalar call `{callee_name}`"
                )));
            }
        },
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right } => {
            emit_expr(left, function, locals, agent_indices, host_plan, string_pool)?;
            emit_expr(right, function, locals, agent_indices, host_plan, string_pool)?;
            emit_binary(*op, &left.ty, function)?;
        }
        IrExprKind::UnOp { op, operand } | IrExprKind::WrappingUnOp { op, operand } => {
            emit_unary(*op, operand, function, locals, agent_indices, host_plan, string_pool)?;
        }
        IrExprKind::Decl { .. }
        | IrExprKind::FieldAccess { .. }
        | IrExprKind::Index { .. }
        | IrExprKind::List { .. }
        | IrExprKind::UnwrapGrounded { .. }
        | IrExprKind::WeakNew { .. }
        | IrExprKind::WeakUpgrade { .. }
        | IrExprKind::StreamSplitBy { .. }
        | IrExprKind::StreamMerge { .. }
        | IrExprKind::StreamOrderedBy { .. }
        | IrExprKind::StreamResumeToken { .. }
        | IrExprKind::ResumeStream { .. }
        | IrExprKind::ResultOk { .. }
        | IrExprKind::ResultErr { .. }
        | IrExprKind::OptionSome { .. }
        | IrExprKind::OptionNone
        | IrExprKind::Ask { .. }
        | IrExprKind::Choose { .. }
        | IrExprKind::TryPropagate { .. }
        | IrExprKind::TryRetry { .. }
        | IrExprKind::Replay { .. } => {
            return Err(WasmCodegenError::unsupported(format!(
                "wasm target cannot lower expression `{expr:?}` yet"
            )));
        }
    }
    Ok(())
}

fn emit_binary(
    op: BinaryOp,
    operand_ty: &Type,
    function: &mut Function,
) -> Result<(), WasmCodegenError> {
    let instruction = match (op, operand_ty) {
        (BinaryOp::Add, Type::Int) => Instruction::I64Add,
        (BinaryOp::Sub, Type::Int) => Instruction::I64Sub,
        (BinaryOp::Mul, Type::Int) => Instruction::I64Mul,
        (BinaryOp::Div, Type::Int) => Instruction::I64DivS,
        (BinaryOp::Mod, Type::Int) => Instruction::I64RemS,
        (BinaryOp::Add, Type::Float) => Instruction::F64Add,
        (BinaryOp::Sub, Type::Float) => Instruction::F64Sub,
        (BinaryOp::Mul, Type::Float) => Instruction::F64Mul,
        (BinaryOp::Div, Type::Float) => Instruction::F64Div,
        (BinaryOp::Eq, Type::Int) => Instruction::I64Eq,
        (BinaryOp::NotEq, Type::Int) => Instruction::I64Ne,
        (BinaryOp::Lt, Type::Int) => Instruction::I64LtS,
        (BinaryOp::LtEq, Type::Int) => Instruction::I64LeS,
        (BinaryOp::Gt, Type::Int) => Instruction::I64GtS,
        (BinaryOp::GtEq, Type::Int) => Instruction::I64GeS,
        (BinaryOp::Eq, Type::Float) => Instruction::F64Eq,
        (BinaryOp::NotEq, Type::Float) => Instruction::F64Ne,
        (BinaryOp::Lt, Type::Float) => Instruction::F64Lt,
        (BinaryOp::LtEq, Type::Float) => Instruction::F64Le,
        (BinaryOp::Gt, Type::Float) => Instruction::F64Gt,
        (BinaryOp::GtEq, Type::Float) => Instruction::F64Ge,
        (BinaryOp::Eq, Type::Bool) => Instruction::I32Eq,
        (BinaryOp::NotEq, Type::Bool) => Instruction::I32Ne,
        (BinaryOp::And, Type::Bool) => Instruction::I32And,
        (BinaryOp::Or, Type::Bool) => Instruction::I32Or,
        _ => {
            return Err(WasmCodegenError::unsupported(format!(
                "wasm target cannot lower binary op `{op:?}` for `{}`",
                operand_ty.display_name()
            )));
        }
    };
    function.instruction(&instruction);
    Ok(())
}

fn emit_unary(
    op: UnaryOp,
    operand: &IrExpr,
    function: &mut Function,
    locals: &LocalLayout,
    agent_indices: &HashMap<DefId, u32>,
    host_plan: &HostImportPlan,
    string_pool: &StringPool,
) -> Result<(), WasmCodegenError> {
    match (op, &operand.ty) {
        (UnaryOp::Neg, Type::Int) => {
            function.instruction(&Instruction::I64Const(0));
            emit_expr(operand, function, locals, agent_indices, host_plan, string_pool)?;
            function.instruction(&Instruction::I64Sub);
        }
        (UnaryOp::Neg, Type::Float) => {
            emit_expr(operand, function, locals, agent_indices, host_plan, string_pool)?;
            function.instruction(&Instruction::F64Neg);
        }
        (UnaryOp::Not, Type::Bool) => {
            emit_expr(operand, function, locals, agent_indices, host_plan, string_pool)?;
            function.instruction(&Instruction::I32Eqz);
        }
        _ => {
            return Err(WasmCodegenError::unsupported(format!(
                "wasm target cannot lower unary op `{op:?}` for `{}`",
                operand.ty.display_name()
            )));
        }
    }
    Ok(())
}

/// Single-value scalar mapping. Used by host-import params (which
/// stay scalar in v1 — strings only cross at the agent boundary)
/// and by single-slot local layout for non-String types.
fn wasm_val_type(ty: &Type) -> Result<ValType, WasmCodegenError> {
    match ty {
        Type::Int => Ok(ValType::I64),
        Type::Float => Ok(ValType::F64),
        Type::Bool => Ok(ValType::I32),
        _ => Err(WasmCodegenError::unsupported(format!(
            "unsupported wasm scalar type `{}`",
            ty.display_name()
        ))),
    }
}

/// Multi-slot mapping for agent params and locals. `String`
/// expands to a pair of `i32`s: `(ptr, len)`. Everything else is a
/// single-value scalar matching `wasm_val_type`.
///
/// Phase 20n-B v1 string ABI: UTF-8 bytes in linear memory, addressed
/// by `(ptr, len)` pairs. The convention is owned by the JS loader on
/// the input side (it allocates via `corvid_alloc`, writes UTF-8
/// bytes, passes the two `i32`s to the agent) and read-but-not-freed
/// on the output side (the agent's returned `(ptr, len)` may alias
/// the input or a const-memory literal — JS decodes the bytes via
/// `TextDecoder` then frees only the inputs it allocated).
fn wasm_param_value_types(ty: &Type) -> Result<Vec<ValType>, WasmCodegenError> {
    match ty {
        Type::String => Ok(vec![ValType::I32, ValType::I32]),
        _ => Ok(vec![wasm_val_type(ty)?]),
    }
}

fn wasm_result_types(ty: &Type) -> Result<Vec<ValType>, WasmCodegenError> {
    match ty {
        Type::Nothing => Ok(Vec::new()),
        Type::String => Ok(vec![ValType::I32, ValType::I32]),
        _ => Ok(vec![wasm_val_type(ty)?]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ir::lower;
    use corvid_resolve::resolve;
    use corvid_syntax::{lex, parse_file};
    use corvid_types::typecheck;

    fn lower_src(src: &str) -> IrFile {
        let tokens = lex(src).expect("lex");
        let (file, perr) = parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = resolve(&file);
        assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
        let checked = typecheck(&file, &resolved);
        assert!(checked.errors.is_empty(), "typecheck: {:?}", checked.errors);
        lower(&file, &resolved, &checked)
    }

    #[test]
    fn emits_valid_wasm_for_scalar_agent() {
        let ir = lower_src(
            r#"
agent add_one(x: Int) -> Int:
    y = x + 1
    return y
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "math").expect("wasm artifacts");
        wasmparser::Validator::new()
            .validate_all(&artifacts.wasm)
            .expect("valid wasm");
        assert!(artifacts.js_loader.contains("add_one(x)"));
        assert!(artifacts.ts_types.contains("add_one(x: bigint): bigint"));
        assert!(artifacts
            .manifest_json
            .contains("\"module_name\": \"math\""));
    }

    #[test]
    fn emits_prompt_as_typed_host_import() {
        let ir = lower_src(
            r#"
prompt answer() -> Int:
    """Return 42."""

agent main() -> Int:
    return answer()
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "prompted").expect("wasm artifacts");
        wasmparser::Validator::new()
            .validate_all(&artifacts.wasm)
            .expect("valid wasm");
        assert!(artifacts.js_loader.contains("'prompt.answer'"));
        assert!(artifacts.js_loader.contains("kind: 'llm_call'"));
        assert!(artifacts.js_loader.contains("kind: 'run_started'"));
        assert!(artifacts.ts_types.contains("'answer': () => bigint"));
        assert!(artifacts.ts_types.contains("CorvidWasmTraceSink"));
        assert!(artifacts.js_loader.contains("createIndexedDbStoreHost"));
        assert!(artifacts.ts_types.contains("CorvidWasmStoreHost"));
        assert!(artifacts.manifest_json.contains("\"kind\": \"prompt\""));
    }

    #[test]
    fn lowers_string_pass_through_agent() {
        // Slice 20n-B-2a regression: a String parameter and return
        // must lower to (i32, i32) pairs so the JS loader can pass
        // UTF-8 byte spans across the boundary. The reproduction
        // from L-4: `agent shout(msg: String) -> String: return msg`.
        let ir = lower_src(
            r#"
agent shout(msg: String) -> String:
    return msg
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "shout").expect("wasm artifacts");
        wasmparser::Validator::new()
            .validate_all(&artifacts.wasm)
            .expect("valid wasm");

        let agent_type = find_agent_func_type(&artifacts.wasm, "shout");
        assert_eq!(
            agent_type.params,
            vec![ValType::I32, ValType::I32],
            "String param must lower to (i32, i32)"
        );
        assert_eq!(
            agent_type.results,
            vec![ValType::I32, ValType::I32],
            "String return must lower to multi-value (i32, i32)"
        );
    }

    #[test]
    fn js_loader_emits_string_param_packing_and_return_decoding() {
        // Slice 20n-B-3 regression: the JS loader for a String-
        // pass-through agent must allocate input bytes via
        // corvid_alloc, write the encoded bytes, destructure the
        // multi-value (ptr, len) return, decode via TextDecoder,
        // and free the input in `finally`.
        let ir = lower_src(
            r#"
agent shout(msg: String) -> String:
    return msg
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "shout").expect("wasm artifacts");
        let js = &artifacts.js_loader;
        // User-facing signature stays a single JS arg.
        assert!(js.contains("shout(msg)"), "wrapper signature: {js}");
        // TextEncoder packing.
        assert!(
            js.contains("__corvid_msg_bytes = __corvid_enc.encode(msg)"),
            "packing pre-amble missing: {js}"
        );
        assert!(
            js.contains("__corvid_msg_ptr = exports.corvid_alloc"),
            "alloc call missing: {js}"
        );
        assert!(
            js.contains(
                "new Uint8Array(exports.memory.buffer, __corvid_msg_ptr, __corvid_msg_bytes.length).set(__corvid_msg_bytes)"
            ),
            "byte-write missing: {js}"
        );
        // WASM-side call passes (ptr, len) instead of `msg`.
        assert!(
            js.contains("exports.shout(__corvid_msg_ptr, __corvid_msg_bytes.length)"),
            "wasm call site uses the wrong arg shape: {js}"
        );
        // Multi-value return destructure + TextDecoder decode.
        assert!(
            js.contains("__corvid_dec.decode(") && js.contains("__corvid_result[0]"),
            "decode post-amble missing: {js}"
        );
        // Input free in finally.
        assert!(
            js.contains("finally {")
                && js.contains("exports.corvid_free(__corvid_msg_ptr, __corvid_msg_bytes.length)"),
            "free in finally missing: {js}"
        );
    }

    #[test]
    fn js_loader_keeps_scalar_agent_wrapper_unchanged() {
        // Regression guard: scalar-only agents must still emit the
        // pre-2a wrapper shape so existing consumers don't break.
        let ir = lower_src(
            r#"
agent dbl(x: Int) -> Int:
    return x + x
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "math").expect("wasm artifacts");
        let js = &artifacts.js_loader;
        assert!(js.contains("dbl(x)"), "wrapper signature: {js}");
        // Scalar path: direct call, no String-only locals.
        assert!(
            js.contains("const result = exports.dbl(x);"),
            "scalar agent must call exports.<name>(arg) directly: {js}"
        );
        assert!(
            !js.contains("__corvid_x_ptr"),
            "scalar arg must not be wrapped in alloc/free: {js}"
        );
    }

    #[test]
    fn manifest_carries_kind_discriminator_uniformly() {
        // Slice 20n-B-3 regression for the design-decision-3
        // manifest extension: `kind` populated for every agent
        // param + return.
        let ir = lower_src(
            r#"
agent shout(msg: String) -> String:
    return msg

agent dbl(x: Int) -> Int:
    return x + x
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "kinds").expect("wasm artifacts");
        let manifest = &artifacts.manifest_json;
        // Every export's params should carry kind alongside ty.
        assert!(
            manifest.contains("\"name\": \"msg\"") && manifest.contains("\"kind\": \"string\""),
            "string param kind missing: {manifest}"
        );
        assert!(
            manifest.contains("\"name\": \"x\"") && manifest.contains("\"kind\": \"i64\""),
            "Int param kind missing: {manifest}"
        );
        // Return-kind on each export.
        assert!(
            manifest.contains("\"return_kind\": \"string\""),
            "shout's return kind missing: {manifest}"
        );
        assert!(
            manifest.contains("\"return_kind\": \"i64\""),
            "dbl's return kind missing: {manifest}"
        );
    }

    #[test]
    fn lowers_string_literal_via_data_section() {
        // Slice 20n-B-2b regression: agents can now return a string
        // literal. The literal is interned into the compile-time
        // string pool, written into the module's DataSection at
        // offset HEAP_BASE, and the agent body lowers
        // `IrLiteral::String("hello")` into a constant `(ptr, len)`
        // pair that JS reads from linear memory.
        let ir = lower_src(
            r#"
agent greet() -> String:
    return "hello"
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "greeting").expect("wasm artifacts");
        wasmparser::Validator::new()
            .validate_all(&artifacts.wasm)
            .expect("valid wasm");

        // The `.wasm` binary should contain a Data section whose
        // active segment carries the literal bytes. Walking via
        // `wasmparser` to confirm.
        let pool_bytes = literal_pool_bytes(&artifacts.wasm);
        assert!(
            pool_bytes.windows(5).any(|w| w == b"hello"),
            "literal pool must contain the bytes `hello`; got {pool_bytes:?}"
        );

        let agent_type = find_agent_func_type(&artifacts.wasm, "greet");
        assert_eq!(
            agent_type.results,
            vec![ValType::I32, ValType::I32],
            "String return must lower to multi-value (i32, i32)"
        );
    }

    #[test]
    fn deduplicates_repeated_string_literals() {
        // Two literals with identical content should share storage in
        // the pool — the interner is content-keyed.
        let ir = lower_src(
            r#"
agent first() -> String:
    return "shared"

agent second() -> String:
    return "shared"
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "shared").expect("wasm artifacts");
        wasmparser::Validator::new()
            .validate_all(&artifacts.wasm)
            .expect("valid wasm");

        let pool_bytes = literal_pool_bytes(&artifacts.wasm);
        // Pool should be exactly 6 bytes (= "shared".len()), not 12.
        assert_eq!(
            pool_bytes,
            b"shared",
            "deduplication: pool should hold one copy of 'shared'"
        );
    }

    #[test]
    fn handles_multi_byte_utf8_literal() {
        // Multi-byte UTF-8 literal: the literal pool stores raw bytes
        // and the (ptr, len) pair counts bytes (not chars). The
        // wasmtime end-to-end round-trip in commit 4 will confirm a
        // round-trip through TextDecoder; here we verify the bytes
        // land in the pool unchanged.
        let ir = lower_src(
            r#"
agent crab() -> String:
    return "héllo 🦀"
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "utf8").expect("wasm artifacts");
        wasmparser::Validator::new()
            .validate_all(&artifacts.wasm)
            .expect("valid wasm");

        let pool_bytes = literal_pool_bytes(&artifacts.wasm);
        assert_eq!(
            pool_bytes,
            "héllo 🦀".as_bytes(),
            "multi-byte UTF-8 literal must round-trip its raw bytes"
        );
    }

    /// Decode the wasm module's data section and return the active-
    /// segment bytes (concatenated if there are multiple segments;
    /// in v1 we only emit one segment so this collapses to the
    /// literal pool).
    fn literal_pool_bytes(bytes: &[u8]) -> Vec<u8> {
        use wasmparser::{DataKind, Parser, Payload};

        let mut out = Vec::new();
        for payload in Parser::new(0).parse_all(bytes) {
            if let Payload::DataSection(reader) = payload.expect("parse payload") {
                for entry in reader {
                    let entry = entry.expect("data entry");
                    if let DataKind::Active { .. } = entry.kind {
                        out.extend_from_slice(entry.data);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn lowers_mixed_string_and_int_parameters_in_order() {
        // Param flattening: `(msg: String, count: Int)` becomes
        // `(i32, i32, i64)` in declaration order. Validates that
        // String slots are inserted at the right spot when other
        // scalar params surround them.
        let ir = lower_src(
            r#"
agent count_repeats(msg: String, count: Int) -> Int:
    return count
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "mixed").expect("wasm artifacts");
        wasmparser::Validator::new()
            .validate_all(&artifacts.wasm)
            .expect("valid wasm");

        let agent_type = find_agent_func_type(&artifacts.wasm, "count_repeats");
        assert_eq!(
            agent_type.params,
            vec![ValType::I32, ValType::I32, ValType::I64],
            "mixed (String, Int) params must lower to (i32, i32, i64)"
        );
        assert_eq!(
            agent_type.results,
            vec![ValType::I64],
            "Int return must remain single i64"
        );
    }

    /// Reflective helper: walk the wasm binary's type section + export
    /// section to find the function signature for the named agent
    /// export. Used by 2a regression tests to verify lowering shape
    /// without having to instantiate the module under wasmtime.
    fn find_agent_func_type(bytes: &[u8], agent_name: &str) -> AgentFuncType {
        use wasmparser::{Parser, Payload, TypeRef};

        let mut function_type_indices: Vec<u32> = Vec::new();
        let mut imported_func_count: u32 = 0;
        let mut function_types: Vec<(Vec<ValType>, Vec<ValType>)> = Vec::new();
        let mut export_target: Option<u32> = None;

        for payload in Parser::new(0).parse_all(bytes) {
            let payload = payload.expect("parse payload");
            match payload {
                Payload::TypeSection(reader) => {
                    for ty in reader.into_iter_err_on_gc_types() {
                        let func = ty.expect("function type");
                        let params = func
                            .params()
                            .iter()
                            .map(|t| valtype_from_wasmparser(*t))
                            .collect();
                        let results = func
                            .results()
                            .iter()
                            .map(|t| valtype_from_wasmparser(*t))
                            .collect();
                        function_types.push((params, results));
                    }
                }
                Payload::ImportSection(reader) => {
                    // wasmparser 0.244+ wraps each section entry in
                    // an `Imports` enum to support the compact-imports
                    // proposal — `into_imports()` flattens it back to
                    // a flat iterator of `Import` items.
                    for import in reader.into_imports() {
                        let import = import.expect("import entry");
                        if let TypeRef::Func(_) = import.ty {
                            imported_func_count += 1;
                        }
                    }
                }
                Payload::FunctionSection(reader) => {
                    for type_idx in reader {
                        function_type_indices.push(type_idx.expect("function entry"));
                    }
                }
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export.expect("export entry");
                        if export.name == agent_name
                            && matches!(export.kind, wasmparser::ExternalKind::Func)
                        {
                            export_target = Some(export.index);
                        }
                    }
                }
                _ => {}
            }
        }

        let func_idx = export_target
            .unwrap_or_else(|| panic!("no exported function named `{agent_name}`"));
        // Imports occupy the lowest function-index slots; the
        // `function_type_indices` vector is indexed by
        // `func_idx - imported_func_count`.
        let local_idx = func_idx
            .checked_sub(imported_func_count)
            .expect("agent func index below imported_func_count");
        let type_idx = function_type_indices[local_idx as usize];
        let (params, results) = function_types[type_idx as usize].clone();
        AgentFuncType { params, results }
    }

    #[derive(Debug)]
    struct AgentFuncType {
        params: Vec<ValType>,
        results: Vec<ValType>,
    }

    /// Convert a wasmparser `ValType` into the wasm-encoder
    /// `ValType` we use throughout this crate. The two enums are
    /// distinct types, but for the four core scalar variants we
    /// care about (I32, I64, F32, F64) the variants are
    /// straightforward to map.
    fn valtype_from_wasmparser(t: wasmparser::ValType) -> ValType {
        match t {
            wasmparser::ValType::I32 => ValType::I32,
            wasmparser::ValType::I64 => ValType::I64,
            wasmparser::ValType::F32 => ValType::F32,
            wasmparser::ValType::F64 => ValType::F64,
            other => panic!("unexpected wasmparser ValType {other:?}"),
        }
    }

    #[test]
    fn emits_tool_and_approval_as_typed_host_imports() {
        let ir = lower_src(
            r#"
tool issue_refund(amount: Int) -> Int dangerous

agent refund(amount: Int) -> Int:
    approve IssueRefund(amount)
    return issue_refund(amount)
"#,
        );
        let artifacts = emit_wasm_artifacts(&ir, "refund").expect("wasm artifacts");
        wasmparser::Validator::new()
            .validate_all(&artifacts.wasm)
            .expect("valid wasm");
        assert!(artifacts.js_loader.contains("'approve.IssueRefund'"));
        assert!(artifacts.js_loader.contains("'tool.issue_refund'"));
        assert!(artifacts.js_loader.contains("kind: 'approval_decision'"));
        assert!(artifacts.js_loader.contains("kind: 'tool_result'"));
        assert!(artifacts
            .ts_types
            .contains("'IssueRefund': (arg1: bigint) => boolean"));
        assert!(artifacts
            .ts_types
            .contains("'issue_refund': (amount: bigint) => bigint"));
    }

    /// Slice 33Q17d — when the user passes a `pub extern "c"` agent
    /// to the wasm target, the error must point them at
    /// `docs/reference/exported-abi.md` where the cdylib-only boundary
    /// contract is documented. Pre-33Q17d the message said
    /// "export a normal agent" with no indication of WHY the
    /// restriction exists.
    #[test]
    fn pub_extern_c_rejection_references_exported_abi_doc() {
        let ir = lower_src(
            r#"
pub extern "c"
agent ping() -> Int:
    return 1
"#,
        );
        let err =
            emit_wasm_artifacts(&ir, "extern_test").expect_err("wasm must reject pub extern c");
        let msg = format!("{err}");
        assert!(
            msg.contains("docs/reference/exported-abi.md"),
            "error must point at the doc page that owns the boundary contract. got: {msg}"
        );
        assert!(
            msg.contains("cdylib"),
            "error must name where the boundary IS supported. got: {msg}"
        );
    }
}
