//! `Decimal` operators and methods (B06), each lowered to one call into
//! the runtime's `corvid-decimal` entry points that
//! [`crate::decimal_methods`] names.
//!
//! A native `Decimal` is a String holding its canonical text. An `Int`
//! operand is widened exactly by `corvid_decimal_from_int` first, as the
//! interpreter widens it. Ownership follows the ownership pass: operator
//! operands are consumed, so an owned operand is released after the
//! call; method receivers and arguments are borrowed, so a bare local is
//! read without a retain.

use super::*;
use crate::decimal_methods::{DecimalOperation, NativeDecimalMethod};

fn call(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    operation: DecimalOperation,
    args: &[ClValue],
    span: Span,
) -> Result<ClValue, CodegenError> {
    let id = runtime.decimal_operations.get(&operation).ok_or_else(|| {
        CodegenError::cranelift(format!("Decimal operation {operation:?} was not declared"), span)
    })?;
    let func = module.declare_func_in_func(*id, builder.func);
    let call = builder.ins().call(func, args);
    Ok(builder.inst_results(call)[0])
}

/// A `Decimal` operand, or an `Int` operand widened to one. Returns the
/// value and whether this site owns a +1 it must release.
#[allow(clippy::too_many_arguments)]
fn lower_operand(
    builder: &mut FunctionBuilder,
    expr: &IrExpr,
    consumed: bool,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<(ClValue, bool), CodegenError> {
    match &expr.ty {
        Type::Decimal => {
            let (value, borrowed) = if consumed {
                lower_string_operand_maybe_borrowed(
                    builder, expr, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                )?
            } else {
                lower_container_maybe_borrowed(
                    builder, expr, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                )?
            };
            Ok((value, !borrowed))
        }
        Type::Int => {
            let n = lower_expr(builder, expr, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            Ok((call(builder, module, runtime, DecimalOperation::FromInt, &[n], expr.span)?, true))
        }
        other => Err(CodegenError::cranelift(
            format!("a Decimal operand of type `{}` reached native lowering; the checker accepts only Decimal and Int", other.display_name()),
            expr.span,
        )),
    }
}

/// Lower two operands, keeping the first alive (and released on an
/// early exit) while the second is lowered.
#[allow(clippy::too_many_arguments)]
fn lower_pair(
    builder: &mut FunctionBuilder,
    left: &IrExpr,
    right: &IrExpr,
    consumed: bool,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<[(ClValue, bool); 2], CodegenError> {
    let first = lower_operand(builder, left, consumed, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
    if first.1 {
        runtime.return_temporaries.borrow_mut().push(first.0);
    }
    let second = lower_operand(builder, right, consumed, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime);
    if first.1 {
        runtime.return_temporaries.borrow_mut().pop();
    }
    Ok([first, second?])
}

fn release_owned(builder: &mut FunctionBuilder, module: &mut ObjectModule, runtime: &RuntimeFuncs, operands: &[(ClValue, bool)]) {
    for (value, owned) in operands.iter().rev() {
        if *owned {
            emit_release(builder, module, runtime, *value);
        }
    }
}

/// `a op b` where either side is a `Decimal`: arithmetic returns the
/// result's canonical text, a comparison the numeric order as a Bool.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_decimal_binop(
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
    let operation = DecimalOperation::for_binary(op).ok_or_else(|| {
        CodegenError::cranelift(format!("`{op:?}` has no Decimal form; the checker fuses `/` with its rounding"), span)
    })?;
    let operands = lower_pair(builder, left, right, true, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
    let result = call(builder, module, runtime, operation, &[operands[0].0, operands[1].0], span)?;
    let result = match op {
        BinaryOp::Eq => builder.ins().icmp_imm(IntCC::Equal, result, 0),
        BinaryOp::NotEq => builder.ins().icmp_imm(IntCC::NotEqual, result, 0),
        BinaryOp::Lt => builder.ins().icmp_imm(IntCC::SignedLessThan, result, 0),
        BinaryOp::LtEq => builder.ins().icmp_imm(IntCC::SignedLessThanOrEqual, result, 0),
        BinaryOp::Gt => builder.ins().icmp_imm(IntCC::SignedGreaterThan, result, 0),
        BinaryOp::GtEq => builder.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, result, 0),
        _ => result,
    };
    release_owned(builder, module, runtime, &operands);
    Ok(result)
}

/// `-d` and `+d` on a `Decimal`.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_decimal_unop(
    builder: &mut FunctionBuilder,
    op: UnaryOp,
    operand: &IrExpr,
    span: Span,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    let (value, owned) = lower_operand(builder, operand, true, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
    match op {
        UnaryOp::Neg => {
            let result = call(builder, module, runtime, DecimalOperation::Negate, &[value], span)?;
            release_owned(builder, module, runtime, &[(value, owned)]);
            Ok(result)
        }
        // `+d` is `d`: hand on the +1 the consumed operand carries.
        UnaryOp::Pos => {
            if !owned {
                emit_retain(builder, module, runtime, value);
            }
            Ok(value)
        }
        UnaryOp::Not => Err(CodegenError::cranelift("`not` applied to a Decimal; the checker rejects it", span)),
    }
}

/// A `Decimal` builtin method, or a conversion into `Decimal`.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_decimal_method(
    builder: &mut FunctionBuilder,
    expr: &IrExpr,
    method: NativeDecimalMethod,
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
        NativeDecimalMethod::CanonicalText => {
            let (text, owned) = lower_operand(builder, receiver, false, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            if !owned {
                emit_retain(builder, module, runtime, text);
            }
            return Ok(text);
        }
        NativeDecimalMethod::Call(operation) => operation,
    };
    let lower_plain = |builder: &mut FunctionBuilder, module: &mut ObjectModule, expr: &IrExpr| {
        lower_expr(builder, expr, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)
    };
    match operation {
        DecimalOperation::FromInt | DecimalOperation::FromFloat => {
            let number = lower_plain(builder, module, receiver)?;
            call(builder, module, runtime, operation, &[number], span)
        }
        DecimalOperation::ParseText => {
            let (text, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let placeholder = builder.ins().iconst(I64, 0);
            let result = emit_result_wrapper_value(
                builder, module, runtime, &expr.ty, placeholder, &Type::Decimal, RESULT_TAG_OK, span,
            )?;
            let slot = builder.ins().iadd_imm(result, i64::from(RESULT_PAYLOAD_OFFSET));
            let tag = call(builder, module, runtime, operation, &[text, slot], span)?;
            builder.ins().store(MemFlags::trusted(), tag, result, RESULT_TAG_OFFSET);
            if !borrowed {
                emit_release(builder, module, runtime, text);
            }
            Ok(result)
        }
        DecimalOperation::DivideRound(_) => {
            let [divisor_expr, places_expr] = args else {
                return Err(CodegenError::cranelift("a rounded Decimal division takes a divisor and places", span));
            };
            let operands = lower_pair(
                builder, receiver, divisor_expr, false, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            for (value, owned) in &operands {
                if *owned {
                    runtime.return_temporaries.borrow_mut().push(*value);
                }
            }
            let places = lower_plain(builder, module, places_expr);
            for (_, owned) in &operands {
                if *owned {
                    runtime.return_temporaries.borrow_mut().pop();
                }
            }
            let result = call(builder, module, runtime, operation, &[operands[0].0, operands[1].0, places?], span)?;
            release_owned(builder, module, runtime, &operands);
            Ok(result)
        }
        DecimalOperation::Round(_) => {
            let [places_expr] = args else {
                return Err(CodegenError::cranelift("a Decimal rounding takes places", span));
            };
            let receiver = lower_operand(builder, receiver, false, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            if receiver.1 {
                runtime.return_temporaries.borrow_mut().push(receiver.0);
            }
            let places = lower_plain(builder, module, places_expr);
            if receiver.1 {
                runtime.return_temporaries.borrow_mut().pop();
            }
            let result = call(builder, module, runtime, operation, &[receiver.0, places?], span)?;
            release_owned(builder, module, runtime, &[receiver]);
            Ok(result)
        }
        DecimalOperation::Abs
        | DecimalOperation::Scale
        | DecimalOperation::ToFloat
        | DecimalOperation::ToIntTruncated => {
            let receiver = lower_operand(builder, receiver, false, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            let result = call(builder, module, runtime, operation, &[receiver.0], span)?;
            release_owned(builder, module, runtime, &[receiver]);
            Ok(result)
        }
        DecimalOperation::Add
        | DecimalOperation::Sub
        | DecimalOperation::Mul
        | DecimalOperation::Rem
        | DecimalOperation::Negate
        | DecimalOperation::Compare => Err(CodegenError::cranelift(
            format!("{operation:?} is an operator, not a Decimal method"),
            span,
        )),
    }
}
