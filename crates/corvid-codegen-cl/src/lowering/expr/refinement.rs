//! Native checks that a value holds its field's refinement (B09
//! Refine-2): `between(min, max)` on an `Int` and `len_between(min, max)`
//! on a `String`, checked where a record is built and where a field is
//! assigned. A value that breaks one stops the program with the
//! interpreter's message, `` `Search.limit` must be between(1, 50), but
//! this value is 500 ``: the compiler embeds the text before and after
//! the value, and a runtime handler writes the three and exits. A refined
//! parameter is checked the same way when a call starts.

use super::*;

/// The runtime handler for an `Int` outside its `between` bounds.
const INT_HANDLER: &str = "corvid_refinement_violated_int";
/// The runtime handler for a `String` outside its `len_between` bounds.
const TEXT_HANDLER: &str = "corvid_refinement_violated_text";

/// Check `value`, about to become `owner.field`, against the field's
/// refinement. A field without one, or of a type the refinement does not
/// constrain, is not checked, as on the interpreter.
pub(in crate::lowering) fn hold_refinement(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    owner: &str,
    field: &corvid_ir::IrField,
    value: ClValue,
    span: Span,
) -> Result<(), CodegenError> {
    let Some(refinement) = &field.refinement else { return Ok(()) };
    let prefix = format!("`{owner}.{}` must be {}, but this value is ", field.name, refinement.describe());
    emit_refinement_check(builder, module, runtime, refinement, &field.ty, value, &prefix, "", span)
}

/// Check `value`, the argument for parameter `param` of `callable`,
/// against the parameter's refinement when the call starts (parameter
/// refinements PR-2), with the interpreter's message:
/// `` `limit` is 500, outside `between(1, 50)`, in this call to `page` ``.
pub(in crate::lowering) fn hold_parameter_refinement(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    callable: &str,
    param: &corvid_ir::IrParam,
    value: ClValue,
    span: Span,
) -> Result<(), CodegenError> {
    let Some(refinement) = &param.refinement else { return Ok(()) };
    let prefix = format!("`{}` is ", param.name);
    let suffix = format!(", outside `{}`, in this call to `{callable}`", refinement.describe());
    emit_refinement_check(builder, module, runtime, refinement, &param.ty, value, &prefix, &suffix, span)
}

/// Stop with `prefix`, the value and `suffix` when `value`, of type `ty`,
/// breaks `refinement`; a type the refinement does not constrain is not
/// checked, as on the interpreter.
#[allow(clippy::too_many_arguments)]
fn emit_refinement_check(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    refinement: &corvid_ast::Refinement,
    ty: &Type,
    value: ClValue,
    prefix: &str,
    suffix: &str,
    span: Span,
) -> Result<(), CodegenError> {
    use corvid_ast::Refinement;
    let (measured, min, max, handler) = match (refinement, untainted(ty)) {
        (Refinement::Between { min, max }, Type::Int) => (value, *min, *max, INT_HANDLER),
        (Refinement::LenBetween { min, max }, Type::String) => {
            let count = module.declare_func_in_func(runtime.string_char_len, builder.func);
            let call = builder.ins().call(count, &[value]);
            let length = builder.inst_results(call)[0];
            (length, *min as i64, *max as i64, TEXT_HANDLER)
        }
        _ => return Ok(()),
    };
    let below = builder.ins().icmp_imm(IntCC::SignedLessThan, measured, min);
    let above = builder.ins().icmp_imm(IntCC::SignedGreaterThan, measured, max);
    let breached = builder.ins().bor(below, above);
    let stop = builder.create_block();
    let held = builder.create_block();
    builder.ins().brif(breached, stop, &[], held, &[]);

    builder.switch_to_block(stop);
    builder.seal_block(stop);
    let prefix = lower_string_literal(builder, module, runtime, prefix, span)?;
    let suffix = lower_string_literal(builder, module, runtime, suffix, span)?;
    let mut signature = module.make_signature();
    signature.params.extend([AbiParam::new(I64), AbiParam::new(I64), AbiParam::new(I64)]);
    let handler = module
        .declare_function(handler, Linkage::Import, &signature)
        .map_err(|e| CodegenError::cranelift(format!("declare {handler}: {e}"), span))?;
    let handler = module.declare_func_in_func(handler, builder.func);
    builder.ins().call(handler, &[prefix, value, suffix]);
    builder.ins().trap(clir::TrapCode::unwrap_user(3));

    builder.switch_to_block(held);
    builder.seal_block(held);
    Ok(())
}

/// `Tainted<T>` holds a `T` at run time.
fn untainted(ty: &Type) -> &Type {
    match ty {
        Type::Tainted(inner) => untainted(inner),
        other => other,
    }
}
