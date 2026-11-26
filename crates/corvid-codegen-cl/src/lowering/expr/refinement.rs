//! Native checks that a value holds its field's refinement (B09
//! Refine-2): `between(min, max)` on an `Int` and `len_between(min, max)`
//! on a `String`, checked where a record is built and where a field is
//! assigned. A value that breaks one stops the program with the
//! interpreter's message, `` `Search.limit` must be between(1, 50), but
//! this value is 500 ``: the compiler embeds the message up to the value,
//! and a runtime handler appends the value and exits.

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
    use corvid_ast::Refinement;
    let Some(refinement) = &field.refinement else { return Ok(()) };
    let (measured, min, max, handler) = match (refinement, untainted(&field.ty)) {
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
    let message = format!("`{owner}.{}` must be {}, but this value is ", field.name, refinement.describe());
    let message = lower_string_literal(builder, module, runtime, &message, span)?;
    let mut signature = module.make_signature();
    signature.params.extend([AbiParam::new(I64), AbiParam::new(I64)]);
    let handler = module
        .declare_function(handler, Linkage::Import, &signature)
        .map_err(|e| CodegenError::cranelift(format!("declare {handler}: {e}"), span))?;
    let handler = module.declare_func_in_func(handler, builder.func);
    builder.ins().call(handler, &[message, value]);
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
