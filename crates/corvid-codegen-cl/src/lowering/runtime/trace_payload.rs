//! Trace-event payloads: the values a trace event records, encoded in
//! the public JSON form by declared type, as the interpreter records
//! them.

use super::*;

/// A trace payload of public JSON texts, each tagged `j` so the runtime
/// parses it back into the JSON value the interpreter records. The texts
/// stay owned by the caller.
pub(in crate::lowering) fn json_trace_payload(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    texts: &[ClValue],
    span: Span,
) -> Result<TracePayload, CodegenError> {
    let tags = "j".repeat(texts.len());
    let type_tags = lower_string_literal(builder, module, runtime, &tags, span)?;
    let count = builder.ins().iconst(I64, texts.len() as i64);
    let values_ptr = if texts.is_empty() {
        builder.ins().iconst(I64, 0)
    } else {
        let slot = builder.create_sized_stack_slot(clir::StackSlotData::new(
            clir::StackSlotKind::ExplicitSlot,
            (texts.len() as u32) * 8,
            3,
        ));
        for (index, text) in texts.iter().enumerate() {
            builder.ins().stack_store(*text, slot, (index * 8) as i32);
        }
        builder.ins().stack_addr(I64, slot, 0)
    };
    Ok(TracePayload { type_tags, count, values_ptr, owned_values: Vec::new() })
}

/// Values recorded in a trace (a run's arguments, an `approve`'s), each
/// in the public JSON form by its declared type, a `Secret` as
/// `<redacted>`, a value the form cannot write as `null`: what the
/// interpreter records for the same values. The
/// payload's texts are returned in `owned_values` for the caller to
/// release after the call.
pub(in crate::lowering) fn recorded_values_payload(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    values: &[ClValue],
    tys: &[Type],
    span: Span,
) -> Result<TracePayload, CodegenError> {
    let types: Vec<corvid_ir::IrType> = runtime.ir_types.values().cloned().collect();
    let mut texts = Vec::with_capacity(values.len());
    for (value, ty) in values.iter().zip(tys) {
        let shown = match ty {
            Type::Grounded(inner) | Type::Tainted(inner) => inner.as_ref(),
            other => other,
        };
        if !crate::lowering::public_json::native_form_with_secrets(shown, &types) {
            return Err(CodegenError::not_supported(
                format!("recording a `{}` value, which has no native JSON form; use the interpreter tier", shown.display_name()),
                span,
            ));
        }
        texts.push(crate::lowering::public_json::lower_recorded_encode(builder, module, runtime, shown, *value, span)?);
    }
    let mut payload = json_trace_payload(builder, module, runtime, &texts, span)?;
    payload.owned_values = texts;
    Ok(payload)
}

