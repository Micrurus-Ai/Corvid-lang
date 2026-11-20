use super::*;

/// Each failed constructor branches to the next arm before reading its payload.
fn match_pattern(
    builder: &mut FunctionBuilder,
    runtime: &RuntimeFuncs,
    pattern: &corvid_ir::IrPattern,
    ty: &Type,
    value: ClValue,
    miss: clir::Block,
    env: &mut HashMap<LocalId, (Variable, clir::Type)>,
    span: Span,
) -> Result<bool, CodegenError> {
    use corvid_ir::IrPattern;
    let (condition, payload) = match (pattern, ty) {
        (IrPattern::Wildcard, _) => return Ok(false),
        (IrPattern::Bind { local_id, .. }, _) => {
            let layout = cl_type_for(ty, span)?;
            let var = runtime.declare_variable(builder, layout);
            builder.def_var(var, value);
            env.insert(*local_id, (var, layout));
            return Ok(false);
        }
        (IrPattern::None_, Type::Option(_)) => {
            (builder.ins().icmp_imm(IntCC::Equal, value, 0), None)
        }
        (IrPattern::Some_(inner), Type::Option(t)) => {
            let offset = option_uses_wrapper(ty).then_some(OPTION_PAYLOAD_OFFSET);
            (
                builder.ins().icmp_imm(IntCC::NotEqual, value, 0),
                Some((inner, t, offset)),
            )
        }
        (IrPattern::Ok_(inner), Type::Result(t, _))
        | (IrPattern::Err_(inner), Type::Result(_, t)) => {
            let tag = builder
                .ins()
                .load(I64, MemFlags::trusted(), value, RESULT_TAG_OFFSET);
            let test = if matches!(pattern, IrPattern::Ok_(_)) {
                IntCC::Equal
            } else {
                IntCC::NotEqual
            };
            (
                builder.ins().icmp_imm(test, tag, RESULT_TAG_OK),
                Some((inner, t, Some(RESULT_PAYLOAD_OFFSET))),
            )
        }
        _ => {
            return Err(CodegenError::not_supported(
                "this pattern requires the interpreter",
                span,
            ))
        }
    };
    let matched = builder.create_block();
    builder.ins().brif(condition, matched, &[], miss, &[]);
    builder.switch_to_block(matched);
    builder.seal_block(matched);
    if let Some((inner, t, offset)) = payload {
        if !matches!(inner.as_ref(), IrPattern::Wildcard) {
            let value = match offset {
                Some(offset) => {
                    builder
                        .ins()
                        .load(cl_type_for(t, span)?, MemFlags::trusted(), value, offset)
                }
                None => value,
            };
            match_pattern(builder, runtime, inner, t, value, miss, env, span)?;
        }
    }
    Ok(true)
}

pub(super) fn lower_match(
    builder: &mut FunctionBuilder,
    expr: &IrExpr,
    scrutinee: &IrExpr,
    arms: &[corvid_ir::IrMatchArm],
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    if !crate::result_match::supports_match(&scrutinee.ty, arms)
        || !(is_native_result_type(&scrutinee.ty) || is_native_option_type(&scrutinee.ty))
    {
        return Err(CodegenError::not_supported(
            "this match requires the interpreter",
            expr.span,
        ));
    }
    let (result, borrowed) = lower_container_maybe_borrowed(
        builder,
        scrutinee,
        current_return_ty,
        env,
        scope_stack,
        func_ids_by_def,
        module,
        runtime,
    )?;
    let join = builder.create_block();
    let ty = if expr.ty == Type::Nothing {
        I64
    } else {
        cl_type_for(&expr.ty, expr.span)?
    };
    let output = builder.append_block_param(join, ty);
    for (arm_index, arm) in arms.iter().enumerate() {
        let next = builder.create_block();
        let mut arm_env = env.clone();
        let can_miss = match_pattern(
            builder,
            runtime,
            &arm.pattern,
            &scrutinee.ty,
            result,
            next,
            &mut arm_env,
            arm.span,
        )?;
        let lower_operand =
            |builder: &mut FunctionBuilder, operand: &IrExpr, module: &mut ObjectModule| {
                let enclosing = runtime.in_match_operand.replace(true);
                let value = lower_expr(
                    builder,
                    operand,
                    current_return_ty,
                    &arm_env,
                    scope_stack,
                    func_ids_by_def,
                    module,
                    runtime,
                );
                runtime.in_match_operand.set(enclosing);
                value
            };
        if let Some(guard) = &arm.guard {
            let test = lower_operand(builder, guard, module)?;
            let body = builder.create_block();
            builder.ins().brif(test, body, &[], next, &[]);
            builder.switch_to_block(body);
            builder.seal_block(body);
        }
        let value = lower_operand(builder, &arm.body, module)?;
        builder.ins().jump(join, &[value.into()]);
        if !can_miss && arm.guard.is_none() {
            break;
        }
        builder.switch_to_block(next);
        builder.seal_block(next);
        if arm_index + 1 == arms.len() {
            // Exhaustiveness is checked before lowering.
            builder.ins().trap(clir::TrapCode::unwrap_user(2));
        }
    }
    builder.switch_to_block(join);
    builder.seal_block(join);
    if !borrowed {
        emit_release(builder, module, runtime, result);
    }
    Ok(output)
}
