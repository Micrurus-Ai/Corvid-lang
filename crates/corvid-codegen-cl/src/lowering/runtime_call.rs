//! A runtime-owned call from native code (native dispatch): a connector
//! operation, or a tool the runtime implements.
//!
//! Native code does not send the request itself. It encodes each
//! argument with the public JSON mapping, the form the interpreter's
//! connector calls use, and hands the call to `corvid_runtime_call`,
//! which runs `Runtime::call_tool`: the same recording, replay, secret
//! redaction, HTTP policy, timeouts, redirects, `HttpExchange` records and
//! `on failure` mapping the interpreter gets. The runtime decodes the
//! answer into the declared return type with the plan embedded here.
//! See `docs/proposals/native-dispatch.md`.

use super::*;
use super::expr::lower_container_maybe_borrowed;
use super::public_json::{json_plan, lower_json_encode, lower_tool_argument_encode, native_runtime_call_types};

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_runtime_call(
    builder: &mut FunctionBuilder,
    tool: &corvid_ir::IrTool,
    args: &[IrExpr],
    expr: &IrExpr,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    let types: Vec<corvid_ir::IrType> = runtime.ir_types.values().cloned().collect();
    if !native_runtime_call_types(tool, &types) {
        return Err(CodegenError::not_supported(
            format!("`{}` sends or reads a type with no native JSON form; use the interpreter tier", tool.name),
            expr.span,
        ));
    }

    // Arguments: each encoded as public JSON text. A computed argument
    // lives until the call returns. A tool that takes a secret gets each
    // argument twice: the recorded form every record of the call holds,
    // with `<redacted>` in a secret's place, and the revealed form only
    // its implementation receives (N3c).
    let name = super::prompt::emit_string_const(builder, module, runtime, &tool.name, expr.span)?;
    let types_by_id: HashMap<DefId, &corvid_ir::IrType> = runtime.ir_types.iter().map(|(id, ty)| (*id, ty)).collect();
    let takes_secret = tool.params.iter().any(|p| corvid_ir::holds_secret(&p.ty, &types_by_id));
    let outer_temporaries = runtime.return_temporaries.borrow().len();
    let mut encoded = Vec::with_capacity(args.len());
    let mut revealed = Vec::new();
    for (arg, param) in args.iter().zip(&tool.params) {
        let (value, borrowed) = lower_container_maybe_borrowed(
            builder, arg, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
        )?;
        if !borrowed && is_refcounted_type(&arg.ty) {
            runtime.return_temporaries.borrow_mut().push(value);
        }
        if takes_secret {
            encoded.push(lower_tool_argument_encode(builder, module, runtime, name, &param.ty, value, false, arg.span)?);
            revealed.push(lower_tool_argument_encode(builder, module, runtime, name, &param.ty, value, true, arg.span)?);
        } else {
            encoded.push(lower_json_encode(builder, module, runtime, &param.ty, value, arg.span)?);
        }
    }
    let store_texts = |builder: &mut FunctionBuilder, texts: &[ClValue]| {
        let slots = builder.create_sized_stack_slot(clir::StackSlotData::new(
            clir::StackSlotKind::ExplicitSlot,
            (texts.len().max(1) * 8) as u32,
            3,
        ));
        for (index, text) in texts.iter().enumerate() {
            builder.ins().stack_store(*text, slots, (index * 8) as i32);
        }
        builder.ins().stack_addr(I64, slots, 0)
    };
    let args_ptr = store_texts(builder, &encoded);
    let revealed_ptr = if takes_secret { store_texts(builder, &revealed) } else { builder.ins().iconst(I64, 0) };
    let arg_count = builder.ins().iconst(I64, encoded.len() as i64);

    // The answer, decoded by the runtime into the declared return type.
    let (plan, plan_len, table) = json_plan(module, runtime, &tool.return_ty, expr.span)?;
    let plan = module.declare_data_in_func(plan, builder.func);
    let plan = builder.ins().symbol_value(I64, plan);
    let plan_len = builder.ins().iconst(I64, plan_len);
    let table = module.declare_data_in_func(table, builder.func);
    let table = builder.ins().symbol_value(I64, table);
    let payload_slot = builder.create_sized_stack_slot(clir::StackSlotData::new(clir::StackSlotKind::ExplicitSlot, 8, 3));
    let payload = builder.ins().stack_addr(I64, payload_slot, 0);

    let call_ref = module.declare_func_in_func(runtime.runtime_call, builder.func);
    builder
        .ins()
        .call(call_ref, &[name, args_ptr, arg_count, plan, plan_len, table, payload, revealed_ptr]);

    for text in encoded.into_iter().chain(revealed) {
        emit_release(builder, module, runtime, text);
    }
    for value in runtime.return_temporaries.borrow_mut().drain(outer_temporaries..).rev() {
        emit_release(builder, module, runtime, value);
    }
    emit_release(builder, module, runtime, name);

    let bits = builder.ins().stack_load(I64, payload_slot, 0);
    Ok(match cl_type_for(&tool.return_ty, expr.span)? {
        I8 => builder.ins().ireduce(I8, bits),
        F64 => builder.ins().bitcast(F64, MemFlags::new(), bits),
        _ => bits,
    })
}
