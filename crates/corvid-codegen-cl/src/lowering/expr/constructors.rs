//! Constructor lowering for refcounted heap values — struct,
//! result, option, and string-literal sites — plus the
//! grounded-value attestation emission that wraps the bare struct
//! payload with provenance metadata.
//!
//! Each constructor allocates the value as Owned (refcount = 1)
//! and either uses `corvid_alloc_with_destructor` (when the type
//! has refcounted children that need a per-instance destructor) or
//! the bare allocator. Argument values transfer ownership into the
//! container without extra retain/release.

use super::*;

pub fn emit_grounded_value_attestation(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    value: ClValue,
    grounded_ty: &Type,
    source_name: &str,
    confidence: f64,
    span: Span,
) -> Result<ClValue, CodegenError> {
    let source_name_val = emit_string_const(builder, module, runtime, source_name, span)?;
    let confidence_val = builder.ins().f64const(confidence);
    let attest_id = match grounded_ty {
        Type::Grounded(inner) => match inner.as_ref() {
            Type::Int => runtime.grounded_attest_int,
            Type::Bool => runtime.grounded_attest_bool,
            Type::Float => runtime.grounded_attest_float,
            Type::String => runtime.grounded_attest_string,
            other => {
                return Err(CodegenError::not_supported(
                    format!(
                        "grounded return `{}` is not yet supported at the native ABI boundary",
                        other.display_name()
                    ),
                    span,
                ))
            }
        },
        _ => unreachable!("grounded attestation requires Grounded<T>"),
    };
    let attest_ref = module.declare_func_in_func(attest_id, builder.func);
    let attest_call = builder
        .ins()
        .call(attest_ref, &[value, source_name_val, confidence_val]);
    let attested = builder.inst_results(attest_call)[0];
    emit_release(builder, module, runtime, source_name_val);
    Ok(attested)
}

/// Lower a struct constructor: allocate, store each field at its
/// offset, return the struct pointer (refcount = 1, Owned).
///
/// Each constructor argument is lowered as an Owned temp; the store
/// transfers ownership into the struct (no extra retain, no release).
/// If the struct has a per-type destructor (at least one refcounted
/// field), we use `corvid_alloc_with_destructor` so `corvid_release`
/// will release those fields when the struct is eventually freed.
pub fn lower_struct_constructor(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    owner: &str,
    fields: &[corvid_ir::IrField],
    typeinfo: cranelift_module::DataId,
    tag: Option<u32>,
    args: &[IrExpr],
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    span: Span,
) -> Result<ClValue, CodegenError> {
    if args.len() != fields.len() {
        return Err(CodegenError::cranelift("constructor arity differs from its layout", span));
    }
    // A later argument may propagate an error before construction is complete.
    let outer_temporaries = runtime.return_temporaries.borrow().len();
    let mut values = Vec::with_capacity(args.len());
    for (arg, field) in args.iter().zip(fields) {
        let value = lower_expr(
            builder, arg, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
        )?;
        let value = if field.ty == Type::Float && arg.ty == Type::Int {
            builder.ins().fcvt_from_sint(F64, value)
        } else {
            value
        };
        if is_refcounted_type(&field.ty) {
            builder.declare_value_needs_stack_map(value);
            runtime.return_temporaries.borrow_mut().push(value);
        }
        // Every value of a refined field holds its refinement (B09).
        super::hold_refinement(builder, module, runtime, owner, field, value, span)?;
        values.push(value);
    }
    runtime.return_temporaries.borrow_mut().truncate(outer_temporaries);
    let header_slots = usize::from(tag.is_some());
    let size = builder.ins().iconst(I64, struct_payload_bytes(header_slots + fields.len()));
    let data = module.declare_data_in_func(typeinfo, builder.func);
    let info = builder.ins().symbol_value(I64, data);
    let alloc = module.declare_func_in_func(runtime.alloc_typed, builder.func);
    let call = builder.ins().call(alloc, &[size, info]);
    let struct_ptr = builder.inst_results(call)[0];
    builder.declare_value_needs_stack_map(struct_ptr);
    if let Some(tag) = tag {
        let tag = builder.ins().iconst(I64, i64::from(tag));
        builder.ins().store(MemFlags::trusted(), tag, struct_ptr, 0);
    }
    for (i, value) in values.into_iter().enumerate() {
        let offset = ((header_slots + i) as i32) * STRUCT_FIELD_SLOT_BYTES;
        builder.ins().store(
            cranelift_codegen::ir::MemFlags::trusted(),
            value,
            struct_ptr,
            offset,
        );
    }
    Ok(struct_ptr)
}

pub fn lower_result_constructor(
    builder: &mut FunctionBuilder,
    expr: &IrExpr,
    inner: &IrExpr,
    tag: i64,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    if !is_native_result_type(&expr.ty) {
        return Err(CodegenError::not_supported(
            "`Result<T, E>` construction outside the supported native subset — native lowering currently supports one-word payload shapes only",
            expr.span,
        ));
    }

    let payload_ty = match (&expr.ty, tag) {
        (Type::Result(ok, _), RESULT_TAG_OK) => &**ok,
        (Type::Result(_, err), RESULT_TAG_ERR) => &**err,
        (Type::Result(_, _), _) => {
            return Err(CodegenError::cranelift(
                format!("invalid native Result tag {tag}"),
                expr.span,
            ))
        }
        _ => {
            return Err(CodegenError::cranelift(
                format!(
                    "Result constructor expression has non-Result type `{}` — typecheck should have caught this",
                    expr.ty.display_name()
                ),
                expr.span,
            ))
        }
    };

    let payload = lower_expr(
        builder,
        inner,
        current_return_ty,
        env,
        scope_stack,
        func_ids_by_def,
        module,
        runtime,
    )?;
    let payload = if *payload_ty == Type::Float && inner.ty == Type::Int {
        builder.ins().fcvt_from_sint(F64, payload)
    } else {
        payload
    };
    emit_result_wrapper_value(
        builder, module, runtime, &expr.ty, payload, payload_ty, tag, expr.span,
    )
}

pub fn emit_result_wrapper_value(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    result_ty: &Type,
    payload: ClValue,
    payload_ty: &Type,
    tag: i64,
    span: Span,
) -> Result<ClValue, CodegenError> {
    let result_ti_id = runtime
        .result_typeinfos
        .get(result_ty)
        .copied()
        .ok_or_else(|| {
            CodegenError::cranelift(
                format!(
                    "no typeinfo pre-emitted for Result type `{}`",
                    result_ty.display_name()
                ),
                span,
            )
        })?;
    let size = builder.ins().iconst(I64, RESULT_PAYLOAD_BYTES);
    let ti_gv = module.declare_data_in_func(result_ti_id, builder.func);
    let ti_addr = builder.ins().symbol_value(I64, ti_gv);
    let alloc_ref = module.declare_func_in_func(runtime.alloc_typed, builder.func);
    let call = builder.ins().call(alloc_ref, &[size, ti_addr]);
    let result_ptr = builder.inst_results(call)[0];
    let tag_val = builder.ins().iconst(I64, tag);
    builder.ins().store(
        cranelift_codegen::ir::MemFlags::trusted(),
        tag_val,
        result_ptr,
        RESULT_TAG_OFFSET,
    );
    let payload_cl_ty = cl_type_for(payload_ty, span)?;
    builder.ins().store(
        cranelift_codegen::ir::MemFlags::trusted(),
        payload,
        result_ptr,
        RESULT_PAYLOAD_OFFSET,
    );
    // Keep `payload_cl_ty` used explicitly for the correctness check
    // above: mismatched payload widths should fail before store.
    let _ = payload_cl_ty;
    Ok(result_ptr)
}

pub fn emit_option_wrapper_value(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    option_ty: &Type,
    payload: ClValue,
    payload_ty: &Type,
    span: Span,
) -> Result<ClValue, CodegenError> {
    let option_ti_id = runtime
        .option_typeinfos
        .get(option_ty)
        .copied()
        .ok_or_else(|| {
            CodegenError::cranelift(
                format!(
                    "no typeinfo pre-emitted for wide Option type `{}`",
                    option_ty.display_name()
                ),
                span,
            )
        })?;
    let size = builder.ins().iconst(I64, OPTION_PAYLOAD_BYTES);
    let ti_gv = module.declare_data_in_func(option_ti_id, builder.func);
    let ti_addr = builder.ins().symbol_value(I64, ti_gv);
    let alloc_ref = module.declare_func_in_func(runtime.alloc_typed, builder.func);
    let call = builder.ins().call(alloc_ref, &[size, ti_addr]);
    let option_ptr = builder.inst_results(call)[0];
    let payload_cl_ty = cl_type_for(payload_ty, span)?;
    builder.ins().store(
        cranelift_codegen::ir::MemFlags::trusted(),
        payload,
        option_ptr,
        OPTION_PAYLOAD_OFFSET,
    );
    let _ = payload_cl_ty;
    Ok(option_ptr)
}

pub fn lower_string_literal(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    s: &str,
    span: Span,
) -> Result<ClValue, CodegenError> {
    let id = runtime.next_literal_id();
    let symbol_name = format!("corvid_lit_{id}");
    let bytes = s.as_bytes();
    let len = bytes.len() as i64;
    let total = 32 + bytes.len();
    let mut data = vec![0u8; total];
    // refcount = i64::MIN (immortal)
    data[0..8].copy_from_slice(&i64::MIN.to_le_bytes());
    // typeinfo_ptr (offset 8) — relocation below points it at
    // `corvid_typeinfo_String` so runtime tracers can dispatch
    // uniformly through the same typeinfo path as heap-allocated
    // strings.
    // bytes_ptr placeholder at offset 16 — written by the relocation
    // length at offset 24
    data[24..32].copy_from_slice(&len.to_le_bytes());
    // bytes at offset 32
    if !bytes.is_empty() {
        data[32..].copy_from_slice(bytes);
    }

    let data_id = module
        .declare_data(&symbol_name, Linkage::Local, false, false)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare literal `{symbol_name}`: {e}"), span)
        })?;
    let mut desc = DataDescription::new();
    desc.set_align(8);
    desc.define(data.into_boxed_slice());
    // typeinfo_ptr at offset 8 → &corvid_typeinfo_String
    let ti_gv = module.declare_data_in_data(runtime.string_typeinfo, &mut desc);
    desc.write_data_addr(8, ti_gv, 0);
    // Self-relative relocation: at offset 16, write the address of
    // (this same symbol + 32) so `bytes_ptr` points at the inline bytes.
    let self_gv = module.declare_data_in_data(data_id, &mut desc);
    desc.write_data_addr(16, self_gv, 32);
    module.define_data(data_id, &desc).map_err(|e| {
        CodegenError::cranelift(format!("define literal `{symbol_name}`: {e}"), span)
    })?;

    // The String value is the address of the descriptor (symbol + 16),
    // matching what `corvid_alloc` returns for heap strings.
    let gv = module.declare_data_in_func(data_id, builder.func);
    let symbol_addr = builder.ins().symbol_value(I64, gv);
    Ok(builder.ins().iadd_imm(symbol_addr, 16))
}
