use super::*;

/// Each key occupies one pair in insertion order.
/// Return the address of the matching value slot, or zero on a miss.
pub(super) fn value_slot(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    map: ClValue,
    key: ClValue,
) -> ClValue {
    let scan = builder.create_block();
    let compare = builder.create_block();
    let found = builder.create_block();
    let done = builder.create_block();
    let remaining = builder.append_block_param(scan, I64);
    let result = builder.append_block_param(done, I64);
    let length = builder.ins().load(I64, MemFlags::trusted(), map, 0);
    let slots = builder.ins().load(I64, MemFlags::trusted(), map, 8);
    let none = builder.ins().iconst(I64, 0);
    builder.ins().jump(scan, &[length.into()]);

    builder.switch_to_block(scan);
    builder
        .ins()
        .brif(remaining, compare, &[], done, &[none.into()]);
    builder.switch_to_block(compare);
    builder.seal_block(compare);
    let index = builder.ins().iadd_imm(remaining, -2);
    let offset = builder.ins().imul_imm(index, 8);
    let pair = builder.ins().iadd(slots, offset);
    let stored = builder.ins().load(I64, MemFlags::trusted(), pair, 0);
    let eq = module.declare_func_in_func(runtime.string_eq, builder.func);
    let call = builder.ins().call(eq, &[stored, key]);
    let equal = builder.inst_results(call)[0];
    builder.ins().brif(equal, found, &[], scan, &[index.into()]);
    builder.seal_block(scan);

    builder.switch_to_block(found);
    builder.seal_block(found);
    let slot = builder.ins().iadd_imm(pair, 8);
    builder.ins().jump(done, &[slot.into()]);
    builder.switch_to_block(done);
    builder.seal_block(done);
    result
}

pub(super) fn load_value(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    slot: ClValue,
) -> ClValue {
    let hit = builder.create_block();
    let done = builder.create_block();
    let result = builder.append_block_param(done, I64);
    builder.ins().brif(slot, hit, &[], done, &[slot.into()]);
    builder.switch_to_block(hit);
    builder.seal_block(hit);
    let value = builder.ins().load(I64, MemFlags::trusted(), slot, 0);
    emit_retain(builder, module, runtime, value);
    builder.ins().jump(done, &[value.into()]);
    builder.switch_to_block(done);
    builder.seal_block(done);
    result
}

/// Store an owned Some(value) representation alongside its key. A lookup
/// can then retain the slot directly, including for scalar zero.
pub(in crate::lowering) fn store(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    map: ClValue,
    key: ClValue,
    value: ClValue,
    value_ty: &Type,
    op: Option<BinaryOp>,
    span: Span,
) -> Result<(), CodegenError> {
    let option_ty = Type::Option(Box::new(value_ty.clone()));
    let slot = value_slot(builder, module, runtime, map, key);
    if let Some(op) = op {
        let found = builder.create_block();
        let missing = builder.create_block();
        builder.ins().brif(slot, found, &[], missing, &[]);
        builder.switch_to_block(missing);
        builder.seal_block(missing);
        let report = module.declare_func_in_func(runtime.missing_map_key, builder.func);
        builder.ins().call(report, &[key]);
        builder.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
        builder.switch_to_block(found);
        builder.seal_block(found);
        let previous = builder.ins().load(I64, MemFlags::trusted(), slot, 0);
        let counter = builder.ins().load(I64, MemFlags::trusted(), previous, OPTION_PAYLOAD_OFFSET);
        let updated = lower_binop_strict(builder, op, counter, value, span, module, runtime)?;
        let boxed = emit_option_wrapper_value(builder, module, runtime, &option_ty, updated, value_ty, span)?;
        builder.ins().store(MemFlags::trusted(), boxed, slot, 0);
        emit_release(builder, module, runtime, previous);
        emit_release(builder, module, runtime, key);
        return Ok(());
    }
    let value = if option_uses_wrapper(&option_ty) {
        emit_option_wrapper_value(builder, module, runtime, &option_ty, value, value_ty, span)?
    } else {
        value
    };
    let replace = builder.create_block();
    let insert = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(slot, replace, &[], insert, &[]);
    builder.switch_to_block(replace);
    builder.seal_block(replace);
    let previous = builder.ins().load(I64, MemFlags::trusted(), slot, 0);
    builder.ins().store(MemFlags::trusted(), value, slot, 0);
    emit_release(builder, module, runtime, previous);
    emit_release(builder, module, runtime, key);
    builder.ins().jump(done, &[]);
    builder.switch_to_block(insert);
    builder.seal_block(insert);
    let append = module.declare_func_in_func(runtime.list_append, builder.func);
    builder.ins().call(append, &[map, key]);
    builder.ins().call(append, &[map, value]);
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    builder.seal_block(done);
    Ok(())
}

/// Copy the selected half of each pair into an independently owned list.
pub(super) fn snapshot(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    map: ClValue,
    len: ClValue,
    values: bool,
    element: &Type,
    span: Span,
) -> Result<ClValue, CodegenError> {
    let list = allocate_slots(builder, module, runtime, element, len, span)?;
    let source = builder.ins().load(I64, MemFlags::trusted(), map, 8);
    let append = module.declare_func_in_func(runtime.list_append, builder.func);
    let check = builder.create_block();
    let copy = builder.create_block();
    let done = builder.create_block();
    let index = builder.append_block_param(check, I64);
    let zero = builder.ins().iconst(I64, 0);
    builder.ins().jump(check, &[zero.into()]);
    builder.switch_to_block(check);
    let more = builder.ins().icmp(IntCC::UnsignedLessThan, index, len);
    builder.ins().brif(more, copy, &[], done, &[]);
    builder.switch_to_block(copy);
    builder.seal_block(copy);
    let offset = builder.ins().imul_imm(index, 16);
    let pair = builder.ins().iadd(source, offset);
    let item = builder
        .ins()
        .load(I64, MemFlags::trusted(), pair, if values { 8 } else { 0 });
    let item = if values && option_uses_wrapper(&Type::Option(Box::new(element.clone()))) {
        builder.ins().load(I64, MemFlags::trusted(), item, OPTION_PAYLOAD_OFFSET)
    } else {
        item
    };
    if is_refcounted_type(element) {
        emit_retain(builder, module, runtime, item);
    }
    builder.ins().call(append, &[list, item]);
    let next = builder.ins().iadd_imm(index, 1);
    builder.ins().jump(check, &[next.into()]);
    builder.seal_block(check);
    builder.switch_to_block(done);
    builder.seal_block(done);
    Ok(list)
}
