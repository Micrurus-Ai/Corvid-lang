//! Time and currency operators and methods (B06), each lowered to one
//! call into the runtime entry points that [`crate::time_methods`] and
//! [`crate::currency_methods`] name.
//!
//! A native `Duration`, `Instant`, `TimeZone`, `ZonedTime`, `Currency`
//! or `Locale` is a String holding its canonical text, and every entry
//! point takes one-word operands: those Strings, `Int`s, and a Result
//! payload slot for the parsers. Operands are lowered in source order,
//! even when the call takes them swapped (`n * d`). Ownership follows
//! the ownership pass: operator operands are consumed, so an owned
//! operand is released after the call; method receivers and arguments
//! are borrowed, so a bare local is read without a retain.

use super::*;
use crate::text_scalar::{scalar_of, BinaryReading, NativeScalarMethod, ScalarBinary};

/// How the String operands of a call are lowered.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Passing {
    /// Operator operands, which the ownership pass treats as consumed.
    Consumed,
    /// Method receivers and arguments, which it treats as borrowed.
    Borrowed,
}

fn call_import(builder: &mut FunctionBuilder, module: &mut ObjectModule, id: FuncId, args: &[ClValue]) -> ClValue {
    let func = module.declare_func_in_func(id, builder.func);
    let call = builder.ins().call(func, args);
    builder.inst_results(call)[0]
}

/// Lowers `operands` in order. Returns each value and whether this
/// site owns a +1 it must release. Earlier owned values stay on the
/// early-exit release list while later operands are lowered.
#[allow(clippy::too_many_arguments)]
fn lower_operands(
    builder: &mut FunctionBuilder,
    operands: &[&IrExpr],
    passing: Passing,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<Vec<(ClValue, bool)>, CodegenError> {
    let mut lowered = Vec::with_capacity(operands.len());
    let mut pushed = 0;
    let mut outcome = Ok(());
    for operand in operands {
        let value = if is_refcounted_type(scalar_of(&operand.ty)) {
            let lowered_operand = match passing {
                Passing::Consumed => lower_string_operand_maybe_borrowed(
                    builder, operand, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                ),
                Passing::Borrowed => lower_container_maybe_borrowed(
                    builder, operand, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                ),
            };
            lowered_operand.map(|(value, borrowed)| (value, !borrowed))
        } else {
            lower_expr(builder, operand, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)
                .map(|value| (value, false))
        };
        match value {
            Ok((value, owned)) => {
                if owned {
                    runtime.return_temporaries.borrow_mut().push(value);
                    pushed += 1;
                }
                lowered.push((value, owned));
            }
            Err(error) => {
                outcome = Err(error);
                break;
            }
        }
    }
    for _ in 0..pushed {
        runtime.return_temporaries.borrow_mut().pop();
    }
    outcome.map(|()| lowered)
}

fn release_owned(builder: &mut FunctionBuilder, module: &mut ObjectModule, runtime: &RuntimeFuncs, operands: &[(ClValue, bool)]) {
    for (value, owned) in operands.iter().rev() {
        if *owned {
            emit_release(builder, module, runtime, *value);
        }
    }
}

/// `left op right` through the runtime call `binary` names.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_scalar_binop<Operation>(
    builder: &mut FunctionBuilder,
    func: FuncId,
    binary: ScalarBinary<Operation>,
    op: BinaryOp,
    left: &IrExpr,
    right: &IrExpr,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    let operands = lower_operands(
        builder, &[left, right], Passing::Consumed, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
    )?;
    let args = if binary.swapped { [operands[1].0, operands[0].0] } else { [operands[0].0, operands[1].0] };
    let result = call_import(builder, module, func, &args);
    let result = match (binary.reading, op) {
        (BinaryReading::Value, _) => result,
        (BinaryReading::Order, BinaryOp::Eq) => builder.ins().icmp_imm(IntCC::Equal, result, 0),
        (BinaryReading::Order, BinaryOp::NotEq) => builder.ins().icmp_imm(IntCC::NotEqual, result, 0),
        (BinaryReading::Order, BinaryOp::Lt) => builder.ins().icmp_imm(IntCC::SignedLessThan, result, 0),
        (BinaryReading::Order, BinaryOp::LtEq) => builder.ins().icmp_imm(IntCC::SignedLessThanOrEqual, result, 0),
        (BinaryReading::Order, BinaryOp::Gt) => builder.ins().icmp_imm(IntCC::SignedGreaterThan, result, 0),
        (BinaryReading::Order, BinaryOp::GtEq) => builder.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, result, 0),
        (BinaryReading::Equality, BinaryOp::Eq) => builder.ins().icmp_imm(IntCC::NotEqual, result, 0),
        (BinaryReading::Equality, BinaryOp::NotEq) => builder.ins().icmp_imm(IntCC::Equal, result, 0),
        (reading, op) => {
            return Err(CodegenError::cranelift(
                format!("`{op:?}` has no {reading:?} reading; the operator tables pair each with its reading"),
                left.span.merge(right.span),
            ))
        }
    };
    release_owned(builder, module, runtime, &operands);
    Ok(result)
}

/// `-d` on a `Duration`: one call on the consumed operand.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_scalar_negate(
    builder: &mut FunctionBuilder,
    func: FuncId,
    operand: &IrExpr,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    let operands = lower_operands(
        builder, &[operand], Passing::Consumed, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
    )?;
    let result = call_import(builder, module, func, &[operands[0].0]);
    release_owned(builder, module, runtime, &operands);
    Ok(result)
}

/// A time or currency builtin method. `func` finds an operation's
/// import, and `is_parse` says whether it fills a `Result` payload slot.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_scalar_method<Operation: Copy + std::fmt::Debug>(
    builder: &mut FunctionBuilder,
    expr: &IrExpr,
    method: NativeScalarMethod<Operation>,
    func: impl Fn(Operation) -> Option<FuncId>,
    is_parse: impl Fn(Operation) -> bool,
    receiver: &IrExpr,
    args: &[IrExpr],
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    let span = expr.span;
    let operation = match method {
        // The value already is its canonical text: return it with the
        // +1 the method result owns.
        NativeScalarMethod::CanonicalText => {
            let (text, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            if borrowed {
                emit_retain(builder, module, runtime, text);
            }
            return Ok(text);
        }
        NativeScalarMethod::Call(operation) => operation,
    };
    let id = func(operation)
        .ok_or_else(|| CodegenError::cranelift(format!("operation {operation:?} was not declared"), span))?;
    let operand_exprs: Vec<&IrExpr> = std::iter::once(receiver).chain(args).collect();
    let operands = lower_operands(
        builder, &operand_exprs, Passing::Borrowed, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
    )?;
    let mut call_args: Vec<ClValue> = operands.iter().map(|(value, _)| *value).collect();
    let result = if is_parse(operation) {
        let Type::Result(ok, _) = &expr.ty else {
            return Err(CodegenError::cranelift(
                format!("a parse method returned `{}`, not a Result", expr.ty.display_name()),
                span,
            ));
        };
        let placeholder = builder.ins().iconst(I64, 0);
        let result = emit_result_wrapper_value(builder, module, runtime, &expr.ty, placeholder, ok, RESULT_TAG_OK, span)?;
        let slot = builder.ins().iadd_imm(result, i64::from(RESULT_PAYLOAD_OFFSET));
        call_args.push(slot);
        let tag = call_import(builder, module, id, &call_args);
        builder.ins().store(MemFlags::trusted(), tag, result, RESULT_TAG_OFFSET);
        result
    } else {
        call_import(builder, module, id, &call_args)
    };
    release_owned(builder, module, runtime, &operands);
    Ok(result)
}

/// `left op right` where either side is a time, `Currency` or `Locale`
/// value. A combination the operator tables do not name is a compiler
/// error, never integer arithmetic on the text pointers.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_time_or_currency_binop(
    builder: &mut FunctionBuilder,
    op: BinaryOp,
    left: &IrExpr,
    right: &IrExpr,
    span: Span,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    let missing = |operation: &dyn std::fmt::Debug| {
        CodegenError::cranelift(format!("operation {operation:?} was not declared"), span)
    };
    if let Some(binary) = crate::time_methods::time_binary(op, &left.ty, &right.ty) {
        let func = *runtime.time_operations.get(&binary.operation).ok_or_else(|| missing(&binary.operation))?;
        return lower_scalar_binop(
            builder, func, binary, op, left, right, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
        );
    }
    if let Some(binary) = crate::currency_methods::currency_binary(op, &left.ty, &right.ty) {
        let func = *runtime.currency_operations.get(&binary.operation).ok_or_else(|| missing(&binary.operation))?;
        return lower_scalar_binop(
            builder, func, binary, op, left, right, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
        );
    }
    Err(CodegenError::cranelift(
        format!(
            "`{op:?}` between `{}` and `{}` reached native lowering; the checker's time and currency tables refuse it",
            left.ty.display_name(),
            right.ty.display_name()
        ),
        span,
    ))
}
