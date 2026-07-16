use super::*;

/// Mangle a user agent's name into a link-safe symbol. Prevents
/// collisions with C runtime symbols (`main`, `printf`, `malloc`, ...).
///
/// Include the agent's `DefId` in the symbol because methods
/// declared inside `extend T:` blocks share their unmangled names
/// across types (`Order.total`, `Line.total` both get the AST name
/// `total`). Including the DefId disambiguates without changing the
/// emitted object file's user-visible behavior: these symbols are
/// internal-only (`Linkage::Local`) so the suffix never leaks into a
/// public API.
pub(super) fn mangle_agent_symbol(user_name: &str, def_id: DefId) -> String {
    format!("corvid_agent_{user_name}_{}", def_id.0)
}

pub(super) fn define_extern_c_wrapper(
    module: &mut ObjectModule,
    agent: &IrAgent,
    inner_func_id: FuncId,
    runtime: &RuntimeFuncs,
) -> Result<(), CodegenError> {
    let Some(IrExternAbi::C) = agent.extern_abi else {
        return Ok(());
    };

    let mut sig = module.make_signature();
    for param in &agent.params {
        sig.params
            .push(AbiParam::new(extern_c_abi_type(&param.ty, param.span)?));
    }
    let grounded_return_inner = grounded_extern_c_return_inner(&agent.return_ty);
    if grounded_return_inner.is_some() {
        sig.params.push(AbiParam::new(I64));
    }
    sig.params.push(AbiParam::new(I64));
    if !matches!(agent.return_ty, Type::Nothing) {
        sig.returns.push(AbiParam::new(extern_c_abi_type(
            &agent.return_ty,
            agent.span,
        )?));
    }

    let wrapper_id = module
        .declare_function(&agent.name, Linkage::Export, &sig)
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare extern-c wrapper `{}`: {e}", agent.name),
                agent.span,
            )
        })?;

    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(
        UserFuncName::user(0, wrapper_id.as_u32()),
        module
            .declarations()
            .get_function_decl(wrapper_id)
            .signature
            .clone(),
    );

    let mut builder_ctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let embed_init_ref = module.declare_func_in_func(runtime.runtime_embed_init, builder.func);
        builder.ins().call(embed_init_ref, &[]);
        let begin_observation_ref =
            module.declare_func_in_func(runtime.begin_direct_observation, builder.func);
        let cost_bound = agent.cost_budget.unwrap_or(f64::NAN);
        let bound_value = if cost_bound.is_finite() {
            builder.ins().f64const(cost_bound)
        } else {
            builder.ins().f64const(f64::NAN)
        };
        builder.ins().call(begin_observation_ref, &[bound_value]);

        let mut call_args = Vec::with_capacity(agent.params.len());
        let mut converted_string_params = Vec::new();
        let mut converted_struct_params = Vec::new();
        for (idx, param) in agent.params.iter().enumerate() {
            let raw = builder.block_params(entry)[idx];
            match &param.ty {
                Type::String => {
                    let from_cstr_ref =
                        module.declare_func_in_func(runtime.string_from_cstr, builder.func);
                    let call = builder.ins().call(from_cstr_ref, &[raw]);
                    let value = builder.inst_results(call)[0];
                    call_args.push(value);
                    converted_string_params.push(value);
                }
                // Slice 33Q8: struct extern-c parameter — receive a
                // `*const c_char` JSON buffer from the C caller,
                // convert to an owned CorvidString, hand to the
                // 20n-C struct decoder, release the temporary
                // string, then pass the decoded struct pointer to
                // the inner agent. On parse failure the decoder
                // returns NULL; the wrapper traps so a C caller
                // sees an honest abort instead of a delayed crash
                // when the inner reads a NULL field. The traceable
                // v1 contract is documented in
                // docs/reference/exported-abi.md.
                Type::Struct(def_id) => {
                    let from_cstr_ref =
                        module.declare_func_in_func(runtime.string_from_cstr, builder.func);
                    let from_cstr_call = builder.ins().call(from_cstr_ref, &[raw]);
                    let json_str = builder.inst_results(from_cstr_call)[0];

                    let decoder_fid =
                        lookup_or_emit_struct_decoder(module, runtime, *def_id, param.span)?;
                    let decoder_ref = module.declare_func_in_func(decoder_fid, builder.func);
                    let decode_call = builder.ins().call(decoder_ref, &[json_str]);
                    let struct_ptr = builder.inst_results(decode_call)[0];

                    // The decoder reads from json_str without taking
                    // ownership — release the temporary now.
                    emit_release(&mut builder, module, runtime, json_str);

                    let zero_i64 = builder.ins().iconst(I64, 0);
                    let decode_failed =
                        builder.ins().icmp(IntCC::Equal, struct_ptr, zero_i64);
                    let ok_b = builder.create_block();
                    let fail_b = builder.create_block();
                    builder.ins().brif(decode_failed, fail_b, &[], ok_b, &[]);

                    builder.switch_to_block(fail_b);
                    builder.seal_block(fail_b);
                    builder
                        .ins()
                        .trap(cranelift_codegen::ir::TrapCode::INTEGER_OVERFLOW);

                    builder.switch_to_block(ok_b);
                    builder.seal_block(ok_b);
                    call_args.push(struct_ptr);
                    converted_struct_params.push(struct_ptr);
                }
                _ => call_args.push(raw),
            }
        }

        let inner_ref = module.declare_func_in_func(inner_func_id, builder.func);
        let call = builder.ins().call(inner_ref, &call_args);
        let results: Vec<ClValue> = builder.inst_results(call).iter().copied().collect();
        let grounded_handle_ptr =
            grounded_return_inner.map(|_| builder.block_params(entry)[agent.params.len()]);
        let observation_handle_ptr = builder.block_params(entry)
            [agent.params.len() + grounded_return_inner.map(|_| 1).unwrap_or(0)];
        let finish_observation_ref =
            module.declare_func_in_func(runtime.finish_direct_observation, builder.func);
        builder
            .ins()
            .call(finish_observation_ref, &[observation_handle_ptr]);

        match &agent.return_ty {
            Type::Nothing => {
                if !results.is_empty() {
                    return Err(CodegenError::cranelift(
                        format!(
                            "extern-c wrapper `{}` expected no result, got {} value(s)",
                            agent.name,
                            results.len()
                        ),
                        agent.span,
                    ));
                }
                builder.ins().return_(&[]);
            }
            Type::String => {
                let result = *results.first().ok_or_else(|| {
                    CodegenError::cranelift(
                        format!(
                            "extern-c wrapper `{}` expected one String result, got {}",
                            agent.name,
                            results.len()
                        ),
                        agent.span,
                    )
                })?;
                let into_cstr_ref =
                    module.declare_func_in_func(runtime.string_into_cstr, builder.func);
                let converted = builder.ins().call(into_cstr_ref, &[result]);
                let converted_value = builder.inst_results(converted)[0];
                builder.ins().return_(&[converted_value]);
            }
            Type::Grounded(inner) if matches!(&**inner, Type::String) => {
                let result = *results.first().ok_or_else(|| {
                    CodegenError::cranelift(
                        format!(
                            "extern-c wrapper `{}` expected one grounded String result, got {}",
                            agent.name,
                            results.len()
                        ),
                        agent.span,
                    )
                })?;
                let capture_ref = module
                    .declare_func_in_func(runtime.grounded_capture_string_handle, builder.func);
                let capture_call = builder.ins().call(capture_ref, &[result]);
                let handle = builder.inst_results(capture_call)[0];
                emit_grounded_handle_store(&mut builder, grounded_handle_ptr, handle);
                let into_cstr_ref =
                    module.declare_func_in_func(runtime.string_into_cstr, builder.func);
                let converted = builder.ins().call(into_cstr_ref, &[result]);
                let converted_value = builder.inst_results(converted)[0];
                builder.ins().return_(&[converted_value]);
            }
            Type::Grounded(_) => {
                let result = *results.first().ok_or_else(|| {
                    CodegenError::cranelift(
                        format!(
                            "extern-c wrapper `{}` expected one grounded scalar result, got {}",
                            agent.name,
                            results.len()
                        ),
                        agent.span,
                    )
                })?;
                let capture_ref = module
                    .declare_func_in_func(runtime.grounded_capture_scalar_handle, builder.func);
                let capture_call = builder.ins().call(capture_ref, &[]);
                let handle = builder.inst_results(capture_call)[0];
                emit_grounded_handle_store(&mut builder, grounded_handle_ptr, handle);
                builder.ins().return_(&[result]);
            }
            // Slice 33Q8: struct extern-c return — call the 20n-C
            // encoder to turn the struct pointer into a CorvidString
            // JSON value, convert that to a C string for the C ABI
            // caller, then release the JSON CorvidString and the
            // struct itself (the wrapper owns the result the inner
            // produced). The caller frees the returned `*mut c_char`
            // via the runtime's standard `corvid_string_free` helper.
            Type::Struct(def_id) => {
                let result = *results.first().ok_or_else(|| {
                    CodegenError::cranelift(
                        format!(
                            "extern-c wrapper `{}` expected one struct result, got {}",
                            agent.name,
                            results.len()
                        ),
                        agent.span,
                    )
                })?;
                let to_json_fid =
                    lookup_or_emit_struct_to_json(module, runtime, *def_id, agent.span)?;
                let to_json_ref = module.declare_func_in_func(to_json_fid, builder.func);
                let to_json_call = builder.ins().call(to_json_ref, &[result]);
                let json_str = builder.inst_results(to_json_call)[0];

                let into_cstr_ref =
                    module.declare_func_in_func(runtime.string_into_cstr, builder.func);
                let into_cstr_call = builder.ins().call(into_cstr_ref, &[json_str]);
                let cstr = builder.inst_results(into_cstr_call)[0];

                emit_release(&mut builder, module, runtime, result);
                builder.ins().return_(&[cstr]);
            }
            _ => {
                let result = *results.first().ok_or_else(|| {
                    CodegenError::cranelift(
                        format!(
                            "extern-c wrapper `{}` expected one scalar result, got {}",
                            agent.name,
                            results.len()
                        ),
                        agent.span,
                    )
                })?;
                builder.ins().return_(&[result]);
            }
        }

        builder.finalize();
    }

    define_function_with_stack_maps(
        module,
        wrapper_id,
        &mut ctx,
        runtime,
        agent.span,
        &format!("extern-c wrapper `{}`", agent.name),
    )?;
    Ok(())
}

pub(super) fn reject_unsupported_types(agent: &IrAgent) -> Result<(), CodegenError> {
    for p in &agent.params {
        cl_type_for(&p.ty, p.span).map_err(|_| {
            CodegenError::not_supported(
                format!(
                    "parameter `{}: {}` — this native lowering path supports scalar and refcounted values here",
                    p.name,
                    p.ty.display_name()
                ),
                p.span,
            )
        })?;
    }
    if !matches!(agent.return_ty, Type::Nothing) {
        cl_type_for(&agent.return_ty, agent.span).map_err(|_| {
            CodegenError::not_supported(
                format!(
                    "agent `{}` returns `{}` — this native lowering path supports scalar and refcounted returns here",
                    agent.name,
                    agent.return_ty.display_name()
                ),
                agent.span,
            )
        })?;
    }
    Ok(())
}

fn extern_c_abi_type(ty: &Type, span: Span) -> Result<clir::Type, CodegenError> {
    match ty {
        Type::Int => Ok(I64),
        Type::Float => Ok(F64),
        Type::Bool => Ok(I8),
        Type::String => Ok(I64),
        // Slice 33Q8: struct boundaries travel the C ABI as
        // `*const c_char` (param) or `*mut c_char` (return) JSON
        // buffers, decoded/encoded by the per-DefId routines that
        // 20n-C already shipped for internal sites. Both are I64-
        // shaped on the wire.
        Type::Struct(_) => Ok(I64),
        Type::Grounded(inner) => extern_c_abi_type(inner, span),
        Type::Tainted(inner) => extern_c_abi_type(inner, span),
        Type::Nothing => Err(CodegenError::cranelift(
            "`Nothing` is only valid as an extern-c return type",
            span,
        )),
        other => Err(CodegenError::not_supported(
            format!(
                "extern-c wrapper lowering does not support `{}` at this ABI boundary",
                other.display_name()
            ),
            span,
        )),
    }
}

fn grounded_extern_c_return_inner(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Grounded(inner) => Some(inner.as_ref()),
        _ => None,
    }
}

fn emit_grounded_handle_store(
    builder: &mut FunctionBuilder,
    grounded_handle_ptr: Option<ClValue>,
    handle: ClValue,
) {
    let Some(out_ptr) = grounded_handle_ptr else {
        return;
    };
    let zero = builder.ins().iconst(I64, 0);
    let is_non_null = builder.ins().icmp(IntCC::NotEqual, out_ptr, zero);
    let write_b = builder.create_block();
    let done_b = builder.create_block();
    builder.ins().brif(is_non_null, write_b, &[], done_b, &[]);
    builder.switch_to_block(write_b);
    builder.seal_block(write_b);
    builder.ins().store(
        cranelift_codegen::ir::MemFlags::trusted(),
        handle,
        out_ptr,
        0,
    );
    builder.ins().jump(done_b, &[]);
    builder.switch_to_block(done_b);
    builder.seal_block(done_b);
}

/// Map a Corvid `Type` to the Cranelift IR type width we compile it to.
/// `Int` -> `I64`, `Bool` -> `I8`. Everything else raises
/// `CodegenError::NotSupported` with a descriptive feature boundary.
pub(super) fn cl_type_for(ty: &Type, span: Span) -> Result<clir::Type, CodegenError> {
    match ty {
        Type::Map(_, _) => Err(CodegenError::not_supported(
            "Map<K, V> is interpreter-only in 45g",
            Span::new(0, 0),
        )),
        Type::Int => Ok(I64),
        Type::Bool => Ok(I8),
        Type::Float => Ok(F64),
        Type::String => Ok(I64),
        Type::Struct(_) => Ok(I64),
        // Compile-time-only wrapper: native width is the inner's.
        Type::Tainted(inner) => cl_type_for(inner, span),
        // Imported structs are refcounted heap handles exactly like
        // file-local structs (width I64). Their field layout now lives in
        // the merged `ir.types` table under the same cross-module-remapped
        // DefId the type carries (see corvid-ir slice G0-1), so the
        // construction / field-access / glue machinery resolves them the
        // same way it resolves local structs.
        Type::ImportedStruct(_) => Ok(I64),
        Type::List(_) => Ok(I64),
        Type::Weak(_, _) => Ok(I64),
        Type::Result(_, _) if is_native_result_type(ty) => Ok(I64),
        Type::Option(inner) if is_refcounted_type(inner) => Ok(I64),
        Type::Option(_) if is_native_wide_option_type(ty) => Ok(I64),
        Type::Grounded(inner) => cl_type_for(inner, span),
        Type::Partial(_) => Err(CodegenError::not_supported(
            "`Partial<T>` - progressive structured lowering is interpreter-only until native tagged field-state layout lands",
            span,
        )),
        Type::ResumeToken(_) => Err(CodegenError::not_supported(
            "`ResumeToken<T>` - stream resumption is interpreter-only until native stream handles land",
            span,
        )),
        Type::Stream(_) => Err(CodegenError::not_supported(
            "`Stream<T>` - Stream lowering is not yet implemented",
            span,
        )),
        Type::Nothing => Err(CodegenError::not_supported(
            "`Nothing` — use a bare `return` instead",
            span,
        )),
        Type::Function { .. } => Err(CodegenError::not_supported(
            "function types as values — first-class callables are not implemented in native codegen yet",
            span,
        )),
        Type::Result(_, _) => Err(CodegenError::not_supported(
            "`Result<T, E>` — native tagged-union lowering is not implemented yet; use the interpreter tier (`corvid run --tier interp`) until then",
            span,
        )),
        Type::Option(_) => Err(CodegenError::not_supported(
            "`Option<T>` — native codegen currently supports nullable-pointer `Option<T>` when `T` is refcounted plus wide scalar `Option<Int|Bool|Float>`; other payload shapes still need the interpreter tier",
            span,
        )),
        Type::RouteParams(_) => Err(CodegenError::not_supported(
            "`route path params` - native HTTP route lowering lands in the backend-runtime slices",
            span,
        )),
        Type::TraceId => Err(CodegenError::not_supported(
            "`TraceId` - native codegen for replay expressions lands in Phase 21 slice 21-inv-E-4; use the interpreter tier (`corvid run --tier interp`) until then",
            span,
        )),
        Type::DbHandle => Err(CodegenError::not_supported(
            "`DbHandle` - the executing SQLite surface (`db_open` / `db_query` / `db_execute` from std/db.cor) is interpreter-only in 33S3; a future slice lands C-ABI opaque-pointer codegen so the handle can cross cdylib boundaries. Use the interpreter tier (`corvid run --tier interp`) until then.",
            span,
        )),
        Type::JsonValue => Err(CodegenError::not_supported(
            "`JsonValue` - the executing JSON surface (`json_parse` / `json_get_*` from std/json.cor) is interpreter-only in 33R5b; the corvid_json_* C-ABI exports already exist in ffi_bridge::json_exports, so the cdylib bridging is plumbing in a follow-up slice. Use the interpreter tier (`corvid run --tier interp`) until then.",
            span,
        )),
        Type::JsonBuilder => Err(CodegenError::not_supported(
            "`JsonBuilder` - the JSON builder surface (`json_object_new` / `json_object_set_*` / `json_object_finish` from std/json.cor) is interpreter-only in 33R5b; cdylib bridging lands in a follow-up slice. Use the interpreter tier (`corvid run --tier interp`) until then.",
            span,
        )),
        Type::Upload(_) => Err(CodegenError::not_supported(
            "`Upload<Format>` - file uploads are an HTTP-boundary type served by `corvid serve` (multipart), not the native backend; the application contract and its OpenAPI projection describe the surface. Use the interpreter/serve tier until native upload lowering lands.",
            span,
        )),
        Type::Page(_) => Err(CodegenError::not_supported(
            "`Page<Item>` - cursor pagination is an HTTP-boundary type served by `corvid serve`, not the native backend; the application contract describes the surface. Use the interpreter/serve tier until native pagination lowering lands.",
            span,
        )),
        Type::Unknown => Err(CodegenError::cranelift(
            "encountered `Unknown` type at codegen — typecheck should have caught this",
            span,
        )),
    }
}

pub(super) fn define_agent(
    module: &mut ObjectModule,
    agent: &IrAgent,
    func_id: FuncId,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    runtime: &RuntimeFuncs,
) -> Result<(), CodegenError> {
    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(
        UserFuncName::user(0, func_id.as_u32()),
        module
            .declarations()
            .get_function_decl(func_id)
            .signature
            .clone(),
    );

    let mut builder_ctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let mut env: HashMap<LocalId, (Variable, clir::Type)> = HashMap::new();
        let mut var_idx: usize = 0;
        let mut scope_stack: Vec<Vec<(LocalId, Variable)>> = vec![Vec::new()];
        let mut loop_stack: Vec<LoopCtx> = Vec::new();

        for (i, p) in agent.params.iter().enumerate() {
            let block_arg = builder.block_params(entry)[i];
            let var = Variable::from_u32(var_idx as u32);
            var_idx += 1;
            let ty = cl_type_for(&p.ty, p.span)?;
            builder.declare_var(var, ty);
            if is_refcounted_type(&p.ty) {
                builder.declare_value_needs_stack_map(block_arg);
            }
            builder.def_var(var, block_arg);
            env.insert(p.local_id, (var, ty));
            if is_refcounted_type(&p.ty) {
                let is_borrowed = agent
                    .borrow_sig
                    .as_ref()
                    .and_then(|v| v.get(i).copied())
                    .map(|b| matches!(b, corvid_ir::ParamBorrow::Borrowed))
                    .unwrap_or(false);
                if !is_borrowed {
                    if !runtime.dup_drop_enabled {
                        emit_retain(&mut builder, module, runtime, block_arg);
                    }
                    scope_stack[0].push((p.local_id, var));
                }
            }
        }

        let lowered = lower_block(
            &mut builder,
            &agent.body,
            &agent.return_ty,
            &mut env,
            &mut var_idx,
            &mut scope_stack,
            &mut loop_stack,
            func_ids_by_def,
            module,
            runtime,
        )?;

        match lowered {
            BlockOutcome::Normal => {
                if builder.current_block().is_some() {
                    let cur = builder.current_block().unwrap();
                    let last_inst = builder.func.layout.last_inst(cur);
                    let terminated = last_inst
                        .map(|i| builder.func.dfg.insts[i].opcode().is_terminator())
                        .unwrap_or(false);
                    if !terminated {
                        builder
                            .ins()
                            .trap(cranelift_codegen::ir::TrapCode::INTEGER_OVERFLOW);
                    }
                }
            }
            BlockOutcome::Terminated => {}
        }

        builder.finalize();
    }

    define_function_with_stack_maps(
        module,
        func_id,
        &mut ctx,
        runtime,
        agent.span,
        &format!("agent `{}`", agent.name),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_types::types::ImportedStructType;

    /// Slice G0-2: a `pub extern "c"`-free agent that mentions an
    /// imported struct in its signature must lower to the same handle
    /// width as a file-local struct, not be rejected. Cross-file field
    /// layout now lives in the merged `ir.types` table (corvid-ir slice
    /// G0-1), so there is no longer a reason to reject the type here.
    #[test]
    fn cl_type_for_treats_imported_struct_like_local_struct() {
        let span = corvid_ast::Span::new(0, 0);
        let local = cl_type_for(&Type::Struct(DefId(3)), span)
            .expect("local struct lowers to a handle");
        let imported = cl_type_for(
            &Type::ImportedStruct(ImportedStructType {
                module_path: "std/auth.cor".to_string(),
                def_id: DefId(42),
                name: "Actor".to_string(),
            }),
            span,
        )
        .expect("imported struct lowers to a handle (G0-2)");
        assert_eq!(
            imported, local,
            "imported struct must lower identically to a local struct"
        );
        assert_eq!(imported, I64);
    }
}
