//! Per-type trace synthesis for the cycle collector.
//!
//! Trace functions (`corvid_trace_<TypeName>`) drive the cycle
//! collector's mark walk — for each refcounted slot they invoke
//! an indirect marker function pointer instead of releasing it.
//! Trace fns are emitted for every refcounted struct / result /
//! option (including ones with zero refcounted children) so the
//! collector can dispatch uniformly without a per-object NULL
//! check.

use super::*;

/// Emit `corvid_trace_<TypeName>(payload, marker, ctx)` for
/// a refcounted struct type. Mirrors `define_struct_destructor` but
/// dispatches through an indirect marker function pointer on each
/// refcounted field instead of releasing it.
///
/// Trace fns are emitted for every refcounted struct — including
/// structs with zero refcounted fields — so the future (17d) mark
/// collector can dispatch uniformly without a per-object NULL check.
/// The linker folds duplicate empty bodies, so the cost is ~zero.
///
/// Marker signature: `fn(obj: i64, ctx: i64) -> ()`. Context-passing
/// (rather than stateless) so 17d's collector can thread a worklist
/// pointer through the walk without TLS or globals.
pub fn define_struct_trace(
    module: &mut ObjectModule,
    ty: &corvid_ir::IrType,
    runtime: &RuntimeFuncs,
) -> Result<FuncId, CodegenError> {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(I64)); // payload
    sig.params.push(AbiParam::new(I64)); // marker fn ptr
    sig.params.push(AbiParam::new(I64)); // ctx

    let symbol = super::type_query::record_symbol("corvid_trace_", ty);
    let func_id = module
        .declare_function(&symbol, Linkage::Local, &sig)
        .map_err(|e| CodegenError::cranelift(format!("declare trace `{symbol}`: {e}"), ty.span))?;

    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(
        UserFuncName::user(0, func_id.as_u32()),
        module
            .declarations()
            .get_function_decl(func_id)
            .signature
            .clone(),
    );
    let mut bctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut bctx);

        // Marker signature: fn(obj: i64, ctx: i64) -> ()
        let mut marker_sig = Signature::new(module.isa().default_call_conv());
        marker_sig.params.push(AbiParam::new(I64));
        marker_sig.params.push(AbiParam::new(I64));
        let marker_sigref = builder.import_signature(marker_sig);

        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        let payload = builder.block_params(entry)[0];
        let marker = builder.block_params(entry)[1];
        let marker_ctx = builder.block_params(entry)[2];

        for (i, field) in ty.fields.iter().enumerate() {
            if is_refcounted_type(&field.ty) {
                let offset = (i as i32) * STRUCT_FIELD_SLOT_BYTES;
                let v = builder.ins().load(
                    I64,
                    cranelift_codegen::ir::MemFlags::trusted(),
                    payload,
                    offset,
                );
                builder
                    .ins()
                    .call_indirect(marker_sigref, marker, &[v, marker_ctx]);
            }
        }

        builder.ins().return_(&[]);
        builder.finalize();
    }

    define_function_with_stack_maps(
        module,
        func_id,
        &mut ctx,
        runtime,
        ty.span,
        &format!("trace `{symbol}`"),
    )?;
    Ok(func_id)
}

pub fn define_result_trace(
    module: &mut ObjectModule,
    result_ty: &Type,
    ok_ty: &Type,
    err_ty: &Type,
    runtime: &RuntimeFuncs,
) -> Result<FuncId, CodegenError> {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(I64));
    sig.params.push(AbiParam::new(I64));
    sig.params.push(AbiParam::new(I64));

    let symbol = format!("corvid_trace_{}", mangle_type_name(result_ty));
    let func_id = module
        .declare_function(&symbol, Linkage::Local, &sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare trace `{symbol}`: {e}"), Span::new(0, 0))
        })?;

    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(
        UserFuncName::user(0, func_id.as_u32()),
        module
            .declarations()
            .get_function_decl(func_id)
            .signature
            .clone(),
    );
    let mut bctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut bctx);

        let mut marker_sig = Signature::new(module.isa().default_call_conv());
        marker_sig.params.push(AbiParam::new(I64));
        marker_sig.params.push(AbiParam::new(I64));
        let marker_sigref = builder.import_signature(marker_sig);

        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let payload = builder.block_params(entry)[0];
        let marker = builder.block_params(entry)[1];
        let marker_ctx = builder.block_params(entry)[2];
        let tag = builder.ins().load(
            I64,
            cranelift_codegen::ir::MemFlags::trusted(),
            payload,
            RESULT_TAG_OFFSET,
        );

        if is_refcounted_type(ok_ty) || is_refcounted_type(err_ty) {
            let ok_block = builder.create_block();
            let err_block = builder.create_block();
            let done_block = builder.create_block();
            let is_ok = builder.ins().icmp_imm(IntCC::Equal, tag, RESULT_TAG_OK);
            builder.ins().brif(is_ok, ok_block, &[], err_block, &[]);

            builder.switch_to_block(ok_block);
            builder.seal_block(ok_block);
            if is_refcounted_type(ok_ty) {
                let v = builder.ins().load(
                    I64,
                    cranelift_codegen::ir::MemFlags::trusted(),
                    payload,
                    RESULT_PAYLOAD_OFFSET,
                );
                builder
                    .ins()
                    .call_indirect(marker_sigref, marker, &[v, marker_ctx]);
            }
            builder.ins().jump(done_block, &[]);

            builder.switch_to_block(err_block);
            builder.seal_block(err_block);
            if is_refcounted_type(err_ty) {
                let v = builder.ins().load(
                    I64,
                    cranelift_codegen::ir::MemFlags::trusted(),
                    payload,
                    RESULT_PAYLOAD_OFFSET,
                );
                builder
                    .ins()
                    .call_indirect(marker_sigref, marker, &[v, marker_ctx]);
            }
            builder.ins().jump(done_block, &[]);

            builder.switch_to_block(done_block);
            builder.seal_block(done_block);
        }

        builder.ins().return_(&[]);
        builder.finalize();
    }

    define_function_with_stack_maps(
        module,
        func_id,
        &mut ctx,
        runtime,
        Span::new(0, 0),
        &format!("trace `{symbol}`"),
    )?;
    Ok(func_id)
}

pub fn define_option_trace(
    module: &mut ObjectModule,
    option_ty: &Type,
    payload_ty: &Type,
    runtime: &RuntimeFuncs,
) -> Result<FuncId, CodegenError> {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(I64)); // payload wrapper ptr
    sig.params.push(AbiParam::new(I64)); // marker fn ptr
    sig.params.push(AbiParam::new(I64)); // ctx

    let symbol = format!("corvid_trace_{}", mangle_type_name(option_ty));
    let func_id = module
        .declare_function(&symbol, Linkage::Local, &sig)
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare option trace `{symbol}`: {e}"),
                Span::new(0, 0),
            )
        })?;

    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(
        UserFuncName::user(0, func_id.as_u32()),
        module
            .declarations()
            .get_function_decl(func_id)
            .signature
            .clone(),
    );
    let mut bctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut bctx);

        let mut marker_sig = Signature::new(module.isa().default_call_conv());
        marker_sig.params.push(AbiParam::new(I64));
        marker_sig.params.push(AbiParam::new(I64));
        let marker_sigref = builder.import_signature(marker_sig);

        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        let payload = builder.block_params(entry)[0];
        let marker = builder.block_params(entry)[1];
        let marker_ctx = builder.block_params(entry)[2];

        if is_refcounted_type(payload_ty) {
            let payload_val = builder.ins().load(
                I64,
                cranelift_codegen::ir::MemFlags::trusted(),
                payload,
                OPTION_PAYLOAD_OFFSET,
            );
            builder
                .ins()
                .call_indirect(marker_sigref, marker, &[payload_val, marker_ctx]);
        }

        builder.ins().return_(&[]);
        builder.finalize();
    }

    define_function_with_stack_maps(
        module,
        func_id,
        &mut ctx,
        runtime,
        Span::new(0, 0),
        &format!("option trace `{symbol}`"),
    )?;
    Ok(func_id)
}
