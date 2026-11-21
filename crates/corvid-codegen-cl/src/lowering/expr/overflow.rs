//! Overflow / divide-by-zero trap synthesis.
//!
//! `with_overflow_trap` wraps a Cranelift overflow-producing
//! arithmetic op (`sadd_overflow`, `ssub_overflow`,
//! `smul_overflow`) and conditionally jumps to a runtime overflow
//! handler when the flag is set, then traps with
//! `INTEGER_OVERFLOW`. `trap_on_zero` is the divide-by-zero
//! analogue: it branches on `divisor == 0` and routes through the
//! same handler. Both keep the happy-path block flat so calling
//! sites get a plain `ClValue` back.

use super::*;

pub(super) fn convert_number(
    builder: &mut FunctionBuilder, module: &mut ObjectModule,
    kind: corvid_types::BuiltinMethodKind, input: ClValue, span: Span,
) -> Result<ClValue, CodegenError> {
    use corvid_types::BuiltinMethodKind::*;
    let rounded = match kind {
        IntToFloat => return Ok(builder.ins().fcvt_from_sint(F64, input)),
        FloatToIntTruncated => input,
        FloatFloor => builder.ins().floor(input),
        FloatCeil => builder.ins().ceil(input),
        FloatRound => {
            // Comparing the fraction first preserves the float immediately below 0.5.
            let whole = builder.ins().trunc(input);
            let remainder = builder.ins().fsub(input, whole);
            let magnitude = builder.ins().fabs(remainder);
            let half = builder.ins().f64const(0.5);
            let adjust = builder.ins().fcmp(FloatCC::GreaterThanOrEqual, magnitude, half);
            let one = builder.ins().f64const(1.0);
            let step = builder.ins().fcopysign(one, input);
            let next = builder.ins().fadd(whole, step);
            builder.ins().select(adjust, next, whole)
        }
        _ => unreachable!(),
    };
    // 2^63 is representable as a Float, while i64::MAX rounds up to it.
    // Ordered bounds reject NaN and both infinities as well.
    let min = builder.ins().f64const(i64::MIN as f64);
    let end = builder.ins().f64const(-(i64::MIN as f64));
    let above_min = builder.ins().fcmp(FloatCC::GreaterThanOrEqual, rounded, min);
    let below_end = builder.ins().fcmp(FloatCC::LessThan, rounded, end);
    let valid = builder.ins().band(above_min, below_end);
    let success = builder.create_block();
    let failure = builder.create_block();
    builder.ins().brif(valid, success, &[], failure, &[]);
    builder.switch_to_block(failure);
    builder.seal_block(failure);
    let sig = module.make_signature();
    let handler = module.declare_function("corvid_float_out_of_int_range", Linkage::Import, &sig)
        .map_err(|e| CodegenError::cranelift(e.to_string(), span))?;
    let handler = module.declare_func_in_func(handler, builder.func);
    builder.ins().call(handler, &[]);
    builder.ins().trap(clir::TrapCode::INTEGER_OVERFLOW);
    builder.switch_to_block(success);
    builder.seal_block(success);
    Ok(builder.ins().fcvt_to_sint(I64, rounded))
}

/// Run an overflow-producing Cranelift op, branch to an overflow handler
/// block on the flag, and return the sum/diff/product value.
pub fn with_overflow_trap<F>(
    builder: &mut FunctionBuilder,
    _l: ClValue,
    _r: ClValue,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    op: F,
) -> Result<ClValue, CodegenError>
where
    F: FnOnce(&mut FunctionBuilder) -> (ClValue, ClValue),
{
    let (result, overflow) = op(builder);
    let overflow_block = builder.create_block();
    let cont_block = builder.create_block();
    builder
        .ins()
        .brif(overflow, overflow_block, &[], cont_block, &[]);

    builder.switch_to_block(overflow_block);
    builder.seal_block(overflow_block);
    let callee_ref = module.declare_func_in_func(runtime.overflow, builder.func);
    builder.ins().call(callee_ref, &[]);
    builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::INTEGER_OVERFLOW);

    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
    Ok(result)
}

pub fn trap_on_zero(
    builder: &mut FunctionBuilder,
    divisor: ClValue,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) {
    let zero = builder.ins().iconst(I64, 0);
    let is_zero = builder.ins().icmp(IntCC::Equal, divisor, zero);
    let trap_block = builder.create_block();
    let cont_block = builder.create_block();
    builder
        .ins()
        .brif(is_zero, trap_block, &[], cont_block, &[]);
    builder.switch_to_block(trap_block);
    builder.seal_block(trap_block);
    let callee_ref = module.declare_func_in_func(runtime.overflow, builder.func);
    builder.ins().call(callee_ref, &[]);
    builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::INTEGER_OVERFLOW);
    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
}
