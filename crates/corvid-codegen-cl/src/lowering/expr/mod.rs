use super::*;

mod callback;
mod binop;
mod constructors;
mod decimal;
mod match_value;
mod operand;
mod overflow;
mod scalar_call;
mod try_propagate;
mod try_retry;
mod wrappers;
pub(super) use binop::{
    lower_binop_strict, lower_binop_wrapping, lower_short_circuit, lower_unop, lower_unop_wrapping,
};
pub(super) use constructors::{
    emit_grounded_value_attestation, emit_option_wrapper_value, emit_result_wrapper_value,
    lower_result_constructor, lower_string_literal, lower_struct_constructor,
};
pub(super) use operand::{
    lower_container_maybe_borrowed, lower_string_binop_with_ownership,
    lower_string_operand_maybe_borrowed,
};
pub(super) use overflow::{trap_if, with_overflow_trap};
pub(super) use try_propagate::{lower_try_propagate_option, lower_try_propagate_result};
pub(super) use try_retry::{lower_try_retry_option, lower_try_retry_result};
use wrappers::tool_wrapper_symbol;

fn allocate_slots(
    builder: &mut FunctionBuilder, module: &mut ObjectModule, runtime: &RuntimeFuncs,
    element: &Type, capacity: ClValue, span: Span,
) -> Result<ClValue, CodegenError> {
    let info = runtime.list_typeinfos.get(element).ok_or_else(|| {
        CodegenError::cranelift(format!("missing list element typeinfo for {}", element.display_name()), span)
    })?;
    let data = module.declare_data_in_func(*info, builder.func);
    let addr = builder.ins().symbol_value(I64, data);
    let alloc = module.declare_func_in_func(runtime.list_new, builder.func);
    let call = builder.ins().call(alloc, &[capacity, addr]);
    Ok(builder.inst_results(call)[0])
}

pub(super) fn checked_list_slot(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    list: ClValue,
    index: ClValue,
) -> ClValue {
    let length = builder.ins().load(I64, MemFlags::trusted(), list, 0);
    // Negative signed indices also exceed the length as unsigned integers.
    let valid = builder.ins().icmp(IntCC::UnsignedLessThan, index, length);
    let trap = builder.create_block();
    let next = builder.create_block();
    builder.ins().brif(valid, next, &[], trap, &[]);
    builder.switch_to_block(trap);
    builder.seal_block(trap);
    let report = module.declare_func_in_func(runtime.list_bounds_error, builder.func);
    builder.ins().call(report, &[index, length]);
    builder.ins().trap(clir::TrapCode::unwrap_user(1));
    builder.switch_to_block(next);
    builder.seal_block(next);
    let offset = builder.ins().imul_imm(index, 8);
    let buffer = builder.ins().load(I64, MemFlags::trusted(), list, 8);
    builder.ins().iadd(buffer, offset)
}

pub(super) fn record_field<'a>(runtime: &'a RuntimeFuncs, ty: &Type, name: &str, span: Span)
    -> Result<(i32, &'a corvid_ir::IrField), CodegenError>
{
    let id = match ty {
        Type::Struct(id) => *id,
        Type::ImportedStruct(imported) => imported.def_id,
        _ => return Err(CodegenError::cranelift("field access requires a record", span)),
    };
    let field = runtime.ir_types.get(&id).and_then(|record| {
        record.fields.iter().enumerate().find(|(_, field)| field.name == name)
    });
    field.map(|(i, field)| (i as i32 * STRUCT_FIELD_SLOT_BYTES, field))
        .ok_or_else(|| CodegenError::cranelift(format!("missing native layout for field `{name}`"), span))
}

pub(super) fn lower_expr(
    builder: &mut FunctionBuilder,
    expr: &IrExpr,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    if let Some((receiver, index)) = crate::native_string_map_lookup(expr) {
        let (map, map_borrowed) = lower_container_maybe_borrowed(builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
        let (key, key_borrowed) = lower_container_maybe_borrowed(builder, index, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
        let find = module.declare_func_in_func(runtime.map_find, builder.func);
        let call = builder.ins().call(find, &[map, key]);
        let slot = builder.inst_results(call)[0];
        let result = if let Type::Option(element) = &expr.ty {
            let found = builder.create_block();
            let done = builder.create_block();
            let result = builder.append_block_param(done, I64);
            builder.ins().brif(slot, found, &[], done, &[slot.into()]);
            builder.switch_to_block(found);
            builder.seal_block(found);
            let item = builder.ins().load(I64, MemFlags::trusted(), slot, 0);
            if is_refcounted_type(element) { emit_retain(builder, module, runtime, item); }
            let some = if option_uses_wrapper(&expr.ty) {
                emit_option_wrapper_value(builder, module, runtime, &expr.ty, item, element, expr.span)?
            } else { item };
            builder.ins().jump(done, &[some.into()]);
            builder.switch_to_block(done);
            builder.seal_block(done);
            result
        } else {
            builder.ins().icmp_imm(IntCC::NotEqual, slot, 0)
        };
        if !key_borrowed { emit_release(builder, module, runtime, key); }
        if !map_borrowed { emit_release(builder, module, runtime, map); }
        return Ok(result);
    }
    match &expr.kind {
        IrExprKind::Match { scrutinee, arms } => match_value::lower_match(
            builder, expr, scrutinee, arms, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
        ),
        IrExprKind::Lambda { .. } | IrExprKind::Decl { .. } if crate::native_callback_type(&expr.ty) =>
            callback::create(builder, expr, env, func_ids_by_def, module, runtime),
        IrExprKind::Lambda { .. } => Err(CodegenError::not_supported(
            "lambdas are interpreter-only in 45j",
            expr.span,
        )),
        IrExprKind::StructLiteral { .. } => Err(CodegenError::not_supported(
            "named struct literals are interpreter-only in 45n (positional construction compiles natively)",
            expr.span,
        )),
        IrExprKind::PageNew { .. } => Err(CodegenError::not_supported(
            "Page<Item> responses are interpreter-only in 52c-2 (served via corvid serve)",
            expr.span,
        )),
        IrExprKind::TableOp { .. } | IrExprKind::TableQuery { .. } => Err(CodegenError::not_supported(
            "table operations run on the interpreter until native tables land (B09 P6); use `corvid run --tier interp`",
            expr.span,
        )),
        IrExprKind::MapLiteral { keys, values } if crate::native_string_map(&expr.ty) => {
            let capacity = builder.ins().iconst(I64, keys.len() as i64);
            let new = module.declare_func_in_func(runtime.map_new, builder.func);
            let Type::Map(_, element) = &expr.ty else { unreachable!() };
            let owns_values = builder.ins().iconst(I64, i64::from(is_refcounted_type(element)));
            let call = builder.ins().call(new, &[capacity, owns_values]);
            let map = builder.inst_results(call)[0];
            let set = module.declare_func_in_func(runtime.map_set, builder.func);
            for (key, value) in keys.iter().zip(values) {
                let key = lower_expr(builder, key, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
                let value = lower_expr(builder, value, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
                builder.ins().call(set, &[map, key, value]);
            }
            Ok(map)
        }
        IrExprKind::BuiltinMethod { kind, receiver, .. } if crate::native_numeric_conversion(*kind) => {
            let input = lower_expr(builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            overflow::convert_number(builder, module, *kind, input, expr.span)
        }
        IrExprKind::BuiltinMethod { kind, receiver, args }
            if crate::native_integer_list_transform(*kind, &receiver.ty, args) => {
                let (list, borrowed_list) = lower_container_maybe_borrowed(
                    builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                )?;
                use corvid_types::BuiltinMethodKind::*;
                let (helper, extra, callback_expr) = match kind {
                    ListMap => (runtime.list_map_int, None, &args[0]),
                    ListFilter => (runtime.list_filter_int, None, &args[0]),
                    ListAny | ListAll => {
                        let all = builder.ins().iconst(I64, i64::from(*kind == ListAll));
                        (runtime.list_test_int, Some(all), &args[0])
                    }
                    ListFold => {
                        let init = lower_expr(builder, &args[0], current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
                        (runtime.list_fold_int, Some(init), &args[1])
                    }
                    _ => unreachable!(),
                };
                let (callback, borrowed_callback) = lower_container_maybe_borrowed(
                    builder, callback_expr, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                )?;
                let info = runtime.list_typeinfos.get(&Type::Int).ok_or_else(||
                    CodegenError::cranelift("missing integer list typeinfo", expr.span))?;
                let info = module.declare_data_in_func(*info, builder.func);
                let info = builder.ins().symbol_value(I64, info);
                let target = builder.ins().load(I64, MemFlags::trusted(), callback, 0);
                let helper = module.declare_func_in_func(helper, builder.func);
                let mut operands = vec![list, callback, target, info];
                operands.extend(extra);
                let call = builder.ins().call(helper, &operands);
                let output = builder.inst_results(call)[0];
                if !borrowed_callback { emit_release(builder, module, runtime, callback); }
                if !borrowed_list { emit_release(builder, module, runtime, list); }
                Ok(output)
            }
        IrExprKind::BuiltinMethod { kind, receiver, .. } if crate::native_string_map(&receiver.ty) => {
            use corvid_types::BuiltinMethodKind::*;
            let (map, borrowed) = lower_container_maybe_borrowed(builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            let value = match kind {
                MapLength => {
                    let entries = builder.ins().load(I64, MemFlags::trusted(), map, 0);
                    builder.ins().load(I64, MemFlags::trusted(), entries, 0)
                }
                MapKeys | MapValues => {
                    let Type::List(element) = &expr.ty else { unreachable!() };
                    let info = runtime.list_typeinfos.get(element.as_ref()).ok_or_else(|| CodegenError::cranelift("missing map snapshot list typeinfo", expr.span))?;
                    let global = module.declare_data_in_func(*info, builder.func);
                    let addr = builder.ins().symbol_value(I64, global);
                    let values = builder.ins().iconst(I64, i64::from(*kind == MapValues));
                    let snapshot = module.declare_func_in_func(runtime.map_snapshot, builder.func);
                    let call = builder.ins().call(snapshot, &[map, values, addr]);
                    builder.inst_results(call)[0]
                }
                _ => return Err(CodegenError::not_supported("map method requires the interpreter", expr.span)),
            };
            if !borrowed { emit_release(builder, module, runtime, map); }
            Ok(value)
        }
        IrExprKind::MapLiteral { .. } => Err(CodegenError::not_supported(
            crate::NATIVE_MAP_REQUIREMENTS,
            expr.span,
        )),
        IrExprKind::BuiltinMethod { kind: kind @ (corvid_types::BuiltinMethodKind::ListFirst | corvid_types::BuiltinMethodKind::ListLast), receiver, .. } => {
            let Type::Option(element) = &expr.ty else { unreachable!() };
            if !is_native_option_type(&expr.ty) {
                return Err(CodegenError::not_supported("first or last with this element type", expr.span));
            }
            let (list, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let length = builder.ins().load(I64, MemFlags::trusted(), list, 0);
            let present = builder.create_block();
            let join = builder.create_block();
            let output = builder.append_block_param(join, I64);
            let none = builder.ins().iconst(I64, 0);
            builder.ins().brif(length, present, &[], join, &[none.into()]);
            builder.switch_to_block(present);
            builder.seal_block(present);
            // The descriptor owns a growable buffer of eight-byte slots.
            let slots = builder.ins().load(I64, MemFlags::trusted(), list, 8);
            let ty = cl_type_for(element, expr.span)?;
            let slot = if *kind == corvid_types::BuiltinMethodKind::ListLast {
                let index = builder.ins().iadd_imm(length, -1);
                let offset = builder.ins().imul_imm(index, 8);
                builder.ins().iadd(slots, offset)
            } else {
                slots
            };
            let item = builder.ins().load(ty, MemFlags::trusted(), slot, 0);
            if is_refcounted_type(element) {
                emit_retain(builder, module, runtime, item);
            }
            let some = if option_uses_wrapper(&expr.ty) {
                emit_option_wrapper_value(builder, module, runtime, &expr.ty, item, element, expr.span)?
            } else {
                item
            };
            builder.ins().jump(join, &[some.into()]);
            builder.switch_to_block(join);
            builder.seal_block(join);
            if !borrowed {
                emit_release(builder, module, runtime, list);
            }
            Ok(output)
        }
        IrExprKind::BuiltinMethod { kind, receiver, args } if crate::is_native_time_method(*kind) => {
            let method = crate::time_methods::native_time_method(*kind).expect("checked by the guard");
            scalar_call::lower_scalar_method(
                builder, expr, method,
                |operation| runtime.time_operations.get(&operation).copied(),
                crate::time_methods::TimeOperation::is_parse,
                receiver, args, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )
        }
        IrExprKind::BuiltinMethod { kind, receiver, args } if crate::is_native_currency_method(*kind) => {
            let method = crate::currency_methods::native_currency_method(*kind).expect("checked by the guard");
            scalar_call::lower_scalar_method(
                builder, expr, method,
                |operation| runtime.currency_operations.get(&operation).copied(),
                crate::currency_methods::CurrencyOperation::is_parse,
                receiver, args, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )
        }
        IrExprKind::BuiltinMethod { kind, receiver, args } if crate::is_native_bytes_method(*kind) => {
            // B04 — a native `Secret` does not exist, so an HMAC keyed by
            // one never reaches here; refuse it by name if it does.
            if let Some(key) = args.iter().find(|arg| matches!(crate::text_scalar::scalar_of(&arg.ty), Type::Secret)) {
                return Err(CodegenError::not_supported(crate::secret_boundary::SECRET_REQUIREMENT, key.span));
            }
            let method = crate::bytes_methods::native_bytes_method(*kind).expect("checked by the guard");
            scalar_call::lower_scalar_method(
                builder, expr, method,
                |operation| runtime.bytes_operations.get(&operation).copied(),
                crate::bytes_methods::BytesOperation::is_parse,
                receiver, args, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )
        }
        IrExprKind::BuiltinMethod { kind, receiver, args } if crate::is_native_decimal_method(*kind) => {
            let method = crate::decimal_methods::native_decimal_method(*kind).expect("checked by the guard");
            decimal::lower_decimal_method(
                builder, expr, method, receiver, args, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )
        }
        IrExprKind::BuiltinMethod { kind, receiver, .. }
            if crate::native_text_method(*kind) && !crate::text_methods::native_regex_method(*kind) => {
            // B03 encoders and decoders: one runtime call into
            // `corvid-text`. A decoder fills a `Result<String, String>`
            // the way `parse_int` fills its Result.
            let (text, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let method = runtime
                .text_methods
                .iter()
                .find(|(declared, _)| declared == kind)
                .map(|(_, id)| *id)
                .ok_or_else(|| CodegenError::cranelift(format!("text method {kind:?} was not declared"), expr.span))?;
            let func = module.declare_func_in_func(method, builder.func);
            let value = if kind.is_text_decoder() {
                let placeholder = builder.ins().iconst(I64, 0);
                let result = emit_result_wrapper_value(
                    builder, module, runtime, &expr.ty, placeholder, &Type::String, RESULT_TAG_OK, expr.span,
                )?;
                let slot = builder.ins().iadd_imm(result, i64::from(RESULT_PAYLOAD_OFFSET));
                let call = builder.ins().call(func, &[text, slot]);
                let tag = builder.inst_results(call)[0];
                builder.ins().store(MemFlags::trusted(), tag, result, RESULT_TAG_OFFSET);
                result
            } else {
                let call = builder.ins().call(func, &[text]);
                builder.inst_results(call)[0]
            };
            if !borrowed {
                emit_release(builder, module, runtime, text);
            }
            Ok(value)
        }
        IrExprKind::BuiltinMethod { kind, receiver, args }
            if crate::native_string_method(*kind) || crate::text_methods::native_regex_method(*kind) => {
            use corvid_types::BuiltinMethodKind::*;
            let outer_temporaries = runtime.return_temporaries.borrow().len();
            let (text, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            if !borrowed {
                runtime.return_temporaries.borrow_mut().push(text);
            }
            let mut operands = vec![text];
            for arg in args {
                let (value, borrowed) = lower_container_maybe_borrowed(
                    builder, arg, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                )?;
                if !borrowed && is_refcounted_type(&arg.ty) {
                    runtime.return_temporaries.borrow_mut().push(value);
                }
                operands.push(value);
            }
            let splits = matches!(kind, StringSplit | StringSplitRegex);
            if splits {
                let capacity = builder.ins().iconst(I64, 0);
                operands.push(allocate_slots(builder, module, runtime, &Type::String, capacity, expr.span)?);
            }
            // B03 regex methods share this argument handling; their
            // runtime entry points come from the text-method table.
            let regex_method = runtime.text_methods.iter().find(|(declared, _)| declared == kind).map(|(_, id)| *id);
            let method = match kind {
                _ if crate::text_methods::native_regex_method(*kind) => regex_method.ok_or_else(|| {
                    CodegenError::cranelift(format!("regex method {kind:?} was not declared"), expr.span)
                })?,
                StringLength => runtime.string_char_len,
                StringSubstring => runtime.string_substring,
                StringToUpper => runtime.string_to_upper,
                StringToLower => runtime.string_to_lower,
                StringTrim => runtime.string_trim,
                StringContains => runtime.string_contains,
                StringStartsWith => runtime.string_starts_with,
                StringEndsWith => runtime.string_ends_with,
                StringReplace => runtime.string_replace,
                StringSplit => runtime.string_split,
                _ => unreachable!(),
            };
            let func = module.declare_func_in_func(method, builder.func);
            let call = builder.ins().call(func, &operands);
            let result = builder.inst_results(call)[0];
            for value in runtime.return_temporaries.borrow_mut().drain(outer_temporaries..).rev() {
                emit_release(builder, module, runtime, value);
            }
            if splits {
                builder.ins().trapz(result, clir::TrapCode::unwrap_user(2));
            }
            Ok(result)
        }
        IrExprKind::BuiltinMethod {
            kind: kind @ (corvid_types::BuiltinMethodKind::ListLength
                | corvid_types::BuiltinMethodKind::ResultIsOk
                | corvid_types::BuiltinMethodKind::ResultIsErr),
            receiver, ..
        } => {
            use corvid_types::BuiltinMethodKind::*;
            let (container, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let value = match kind {
                ListLength => builder.ins().load(I64, MemFlags::trusted(), container, 0),
                _ => {
                    let tag = builder.ins().load(I64, MemFlags::trusted(), container, RESULT_TAG_OFFSET);
                    let expected = if *kind == ResultIsOk { RESULT_TAG_OK } else { RESULT_TAG_ERR };
                    builder.ins().icmp_imm(IntCC::Equal, tag, expected)
                }
            };
            if !borrowed {
                emit_release(builder, module, runtime, container);
            }
            Ok(value)
        }
        IrExprKind::BuiltinMethod { kind: corvid_types::BuiltinMethodKind::StringParseInt, receiver, .. } => {
            let (text, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let zero = builder.ins().iconst(I64, 0);
            let result = emit_result_wrapper_value(
                builder, module, runtime, &expr.ty, zero, &Type::Int, RESULT_TAG_OK, expr.span,
            )?;
            let slot = builder.ins().iadd_imm(result, i64::from(RESULT_PAYLOAD_OFFSET));
            let parse = module.declare_func_in_func(runtime.string_parse_int, builder.func);
            let call = builder.ins().call(parse, &[text, slot]);
            let tag = builder.inst_results(call)[0];
            builder.ins().store(MemFlags::trusted(), tag, result, RESULT_TAG_OFFSET);
            if !borrowed {
                emit_release(builder, module, runtime, text);
            }
            Ok(result)
        }
        IrExprKind::BuiltinMethod { kind: corvid_types::BuiltinMethodKind::ListAppend, receiver, args } => {
            let item = &args[0];
            if !crate::native_list_append(&receiver.ty, &item.ty) {
                return Err(CodegenError::not_supported("append with this element type", expr.span));
            }
            let (list, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let mut slot = lower_expr(builder, item, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            if matches!(&receiver.ty, Type::List(t) if **t == Type::Float) && item.ty == Type::Int {
                slot = builder.ins().fcvt_from_sint(F64, slot);
            }
            slot = match builder.func.dfg.value_type(slot) {
                I8 => builder.ins().uextend(I64, slot),
                F64 => builder.ins().bitcast(I64, MemFlags::new(), slot),
                _ => slot,
            };
            let append = module.declare_func_in_func(runtime.list_append, builder.func);
            builder.ins().call(append, &[list, slot]);
            if !borrowed {
                emit_release(builder, module, runtime, list);
            }
            Ok(builder.ins().iconst(I64, 0))
        }
        IrExprKind::BuiltinMethod {
            kind: kind @ (corvid_types::BuiltinMethodKind::JsonDecode | corvid_types::BuiltinMethodKind::JsonDecodeStrict),
            receiver,
            ..
        } => {
            let (text, borrowed) = lower_container_maybe_borrowed(
                builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let strict = *kind == corvid_types::BuiltinMethodKind::JsonDecodeStrict;
            let result = lower_json_decode(builder, module, runtime, expr, text, strict)?;
            if !borrowed {
                emit_release(builder, module, runtime, text);
            }
            Ok(result)
        }
        IrExprKind::BuiltinMethod { kind: corvid_types::BuiltinMethodKind::JsonEncode, receiver, .. } => {
            if is_refcounted_type(&receiver.ty) {
                let (value, borrowed) = lower_container_maybe_borrowed(
                    builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                )?;
                let text = lower_json_encode(builder, module, runtime, &receiver.ty, value, expr.span)?;
                if !borrowed {
                    emit_release(builder, module, runtime, value);
                }
                Ok(text)
            } else {
                let value = lower_expr(builder, receiver, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
                lower_json_encode(builder, module, runtime, &receiver.ty, value, expr.span)
            }
        }
        IrExprKind::BuiltinMethod { .. } => Err(CodegenError::not_supported(
            "builtin methods (String.length() and the 45d/45e/45f batches) are interpreter-only in 45c",
            expr.span,
        )),
        IrExprKind::Literal(IrLiteral::Int(n)) => Ok(builder.ins().iconst(I64, *n)),
        IrExprKind::Literal(IrLiteral::Bool(b)) => {
            Ok(builder.ins().iconst(I8, if *b { 1 } else { 0 }))
        }
        IrExprKind::Literal(IrLiteral::Float(n)) => Ok(builder.ins().f64const(*n)),
        // B06 — a native zone, currency or locale is its canonical
        // text: the IANA name, the ISO 4217 code, the BCP 47 tag.
        IrExprKind::Literal(IrLiteral::TimeZone(zone)) => {
            lower_string_literal(builder, module, runtime, &zone.to_string(), expr.span)
        }
        IrExprKind::Literal(IrLiteral::Currency(currency)) => {
            lower_string_literal(builder, module, runtime, &currency.to_string(), expr.span)
        }
        IrExprKind::Literal(IrLiteral::Locale(locale)) => {
            lower_string_literal(builder, module, runtime, &locale.to_string(), expr.span)
        }
        // B06 — a native `Decimal` is its canonical text, the digits the
        // literal was written with (`2.50`).
        IrExprKind::Literal(IrLiteral::Decimal(d)) => {
            lower_string_literal(builder, module, runtime, &d.to_string(), expr.span)
        }
        IrExprKind::Literal(IrLiteral::String(s)) => {
            lower_string_literal(builder, module, runtime, s, expr.span)
        }
        IrExprKind::Literal(IrLiteral::Nothing) => Err(CodegenError::not_supported(
            "`nothing` literal is not supported yet",
            expr.span,
        )),
        IrExprKind::Local { local_id, name } => {
            let (var, _ty) = env.get(local_id).ok_or_else(|| {
                CodegenError::cranelift(
                    format!("no variable for local `{name}` ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â compiler bug"),
                    expr.span,
                )
            })?;
            let v = builder.use_var(*var);
            // Three-state ownership: `use_var` produces a Borrowed
            // reference. Convert to Owned by retaining so the caller
            // (bind / return / call-arg / discard) can dispose of it
            // uniformly. For non-refcounted types this is a no-op.
            //
            // Under the .6d unified pass, we rely on the pass's Dup
            // insertion at non-last consuming uses instead of
            // retaining on every read ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â the last use of a local
            // consumes the existing +1 directly.
            if (!runtime.dup_drop_enabled || runtime.in_match_operand.get()) && is_refcounted_type(&expr.ty) {
                emit_retain(builder, module, runtime, v);
            }
            Ok(v)
        }
        IrExprKind::Decl { .. } => Err(CodegenError::not_supported(
            "declaration reference (imports/functions as values)",
            expr.span,
        )),
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right }
            if crate::requires_grounded_string_concat(*op, &left.ty, &right.ty) => {
                let (l, borrowed_l) = lower_string_operand_maybe_borrowed(
                    builder, left, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                )?;
                if !borrowed_l {
                    runtime.return_temporaries.borrow_mut().push(l);
                }
                let (r, borrowed_r) = lower_string_operand_maybe_borrowed(
                    builder, right, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
                )?;
                if !borrowed_l {
                    runtime.return_temporaries.borrow_mut().pop();
                }
                let mask = i64::from(matches!(left.ty, Type::Grounded(_)))
                    | (i64::from(matches!(right.ty, Type::Grounded(_))) << 1);
                let mask = builder.ins().iconst(clir::types::I8, mask);
                let callee = module.declare_func_in_func(runtime.grounded_string_concat, builder.func);
                let call = builder.ins().call(callee, &[l, r, mask]);
                let result = builder.inst_results(call)[0];
                if !borrowed_l {
                    emit_release(builder, module, runtime, l);
                }
                if !borrowed_r {
                    emit_release(builder, module, runtime, r);
                }
                builder.ins().trapz(result, clir::TrapCode::unwrap_user(3));
                Ok(result)
            }
        // B06, B04 — time, currency, locale and `Bytes` operators are
        // runtime calls; `@wrapping` does not apply to them, and `==` on
        // `Bytes` is the runtime's constant-time comparison.
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right }
            if crate::time_methods::has_time_operand(&left.ty, &right.ty)
                || crate::currency_methods::has_currency_operand(&left.ty, &right.ty)
                || crate::bytes_methods::has_bytes_operand(&left.ty, &right.ty) => scalar_call::lower_text_scalar_binop(
                builder, *op, left, right, expr.span, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            ),
        // B06 — `@wrapping` does not apply to decimals; both forms are checked.
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right }
            if crate::decimal_methods::is_decimal_binary(*op, &left.ty, &right.ty) => decimal::lower_decimal_binop(
                builder, *op, left, right, expr.span, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            ),
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right }
            if uses_structural_equality(*op, &left.ty) => super::equality::lower_comparison(
                builder, *op, left, right, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime, expr.span,
            ),
        IrExprKind::BinOp { op, left, right } => {
            if matches!(op, BinaryOp::And | BinaryOp::Or) {
                return lower_short_circuit(
                    builder,
                    *op,
                    left,
                    right,
                    current_return_ty,
                    env,
                    scope_stack,
                    func_ids_by_def,
                    module,
                    runtime,
                );
            }
            // String operands route through the String runtime helpers
            // and have their own ownership semantics (release inputs
            // after the helper call). Dispatch here so we have access
            // to the IR type information.
            //
            // Borrow-at-use-site peephole for
            // consuming string BinOps. If an operand is a bare
            // `IrExprKind::Local` of a refcounted String, we lower
            // the Local WITHOUT emitting the ownership-conversion
            // retain, and correspondingly skip the post-op release.
            // The comparison/concat runtime helpers only read their
            // inputs (they don't mutate the operand's refcount or
            // store the pointer), so reading the Variable directly
            // is semantically equivalent to retain-then-release with
            // zero observable refcount net effect. The Local's
            // binding remains Live in its scope, governed by the
            // scope-exit release that's already in place.
            //
            // Non-bare-Local operands (literals, nested expressions,
            // call results) still produce an Owned +1 and are
            // released by the helper as before ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â the original
            // ownership contract.
            if matches!(&left.ty, Type::String) && matches!(&right.ty, Type::String) {
                let (l, l_borrowed) =
                    lower_string_operand_maybe_borrowed(builder, left, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
                let (r, r_borrowed) =
                    lower_string_operand_maybe_borrowed(builder, right, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
                return lower_string_binop_with_ownership(
                    builder, *op, l, r, l_borrowed, r_borrowed, expr.span, module, runtime,
                );
            }
            let l = lower_expr(builder, left, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            let r = lower_expr(builder, right, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            let result = lower_binop_strict(builder, *op, l, r, expr.span, module, runtime)?;
            if is_refcounted_type(&left.ty) {
                emit_release(builder, module, runtime, l);
            }
            if is_refcounted_type(&right.ty) {
                emit_release(builder, module, runtime, r);
            }
            Ok(result)
        }
        IrExprKind::WrappingBinOp { op, left, right } => {
            let l = lower_expr(builder, left, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            let r = lower_expr(builder, right, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            lower_binop_wrapping(builder, *op, l, r, expr.span)
        }
        IrExprKind::UnOp { op: UnaryOp::Neg, operand } | IrExprKind::WrappingUnOp { op: UnaryOp::Neg, operand }
            if *crate::text_scalar::scalar_of(&operand.ty) == Type::Duration => {
            let func = *runtime.time_operations.get(&crate::time_methods::TimeOperation::DurationNegate).ok_or_else(|| {
                CodegenError::cranelift("Duration negation was not declared", expr.span)
            })?;
            scalar_call::lower_scalar_negate(
                builder, func, operand, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )
        }
        IrExprKind::UnOp { op, operand } | IrExprKind::WrappingUnOp { op, operand } if operand.ty == Type::Decimal => {
            decimal::lower_decimal_unop(
                builder, *op, operand, expr.span, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )
        }
        IrExprKind::UnOp { op, operand } => {
            let v = lower_expr(builder, operand, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            lower_unop(builder, *op, v, expr.span, module, runtime)
        }
        IrExprKind::WrappingUnOp { op, operand } => {
            let v = lower_expr(builder, operand, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?;
            lower_unop_wrapping(builder, *op, v, expr.span)
        }
        IrExprKind::UnwrapGrounded { value } => {
            lower_expr(
                builder,
                value,
                current_return_ty,
                env,
                scope_stack,
                func_ids_by_def,
                module,
                runtime,
            )
        }
        IrExprKind::Call { kind, callee_name, args } => match kind {
            IrCallKind::ClosureLocal { local_id, signature } if crate::native_callback_type(signature) => {
                let var = env.get(local_id).ok_or_else(|| CodegenError::cranelift("unbound callback", expr.span))?.0;
                let handle = builder.use_var(var);
                let mut operands = vec![handle];
                for arg in args {
                    operands.push(lower_expr(builder, arg, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime)?);
                }
                let sig = builder.import_signature(callback::signature(module, args.len(), cl_type_for(&expr.ty, expr.span)?));
                let address = builder.ins().load(I64, MemFlags::trusted(), handle, 0);
                let call = builder.ins().call_indirect(sig, address, &operands);
                Ok(builder.inst_results(call)[0])
            }
            IrCallKind::ClosureLocal { .. } => {
                return Err(CodegenError::not_supported(
                    "closure calls are interpreter-only in 45j",
                    expr.span,
                ));
            }
            IrCallKind::Agent { def_id } => {
                let callee_id = func_ids_by_def.get(def_id).ok_or_else(|| {
                    CodegenError::cranelift(
                        format!("agent `{callee_name}` has no declared function id"),
                        expr.span,
                    )
                })?;
                let callee_ref = module.declare_func_in_func(*callee_id, builder.func);
                // Caller-side borrow peephole. For
                // each refcounted arg whose callee slot is Borrowed
                // (per the callee's `borrow_sig` populated by the
                // ownership pass) AND whose argument expression is a
                // bare `IrExprKind::Local`, we lower the Local
                // directly from its Variable without a retain, and
                // skip the paired post-call release. The callee
                // already skips its entry-retain + scope-exit release
                // for Borrowed params (17b-1b.1), so the whole
                // retain/release pair collapses on both sides.
                //
                // For Owned callee slots OR non-bare-Local args, the
                // original +0 ABI applies: lower_expr produces a +1
                // Owned, callee retains on entry (17b-1b.1 keeps this
                // for Owned), caller releases its +1 after the call.
                let callee_sig = runtime.agent_borrow_sigs.get(def_id);
                let mut arg_vals = Vec::with_capacity(args.len());
                // `needs_post_release[i]` = true iff the caller
                // produced a +1 that must be released post-call
                // (false when borrow peephole applies).
                let mut needs_post_release: Vec<bool> = Vec::with_capacity(args.len());
                for (i, a) in args.iter().enumerate() {
                    let is_ref = is_refcounted_type(&a.ty);
                    let callee_borrowed = callee_sig
                        .and_then(|s| s.get(i).copied())
                        .map(|b| matches!(b, corvid_ir::ParamBorrow::Borrowed))
                        .unwrap_or(false);
                    let arg_is_bare_local =
                        matches!(&a.kind, IrExprKind::Local { .. });
                    if is_ref && callee_borrowed && arg_is_bare_local {
                        // Borrow path: read the Variable directly, no retain.
                        if let IrExprKind::Local { local_id, name } = &a.kind {
                            let (var, _ty) = env.get(local_id).ok_or_else(|| {
                                CodegenError::cranelift(
                                    format!("no variable for local `{name}` ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â compiler bug"),
                                    a.span,
                                )
                            })?;
                            let v = builder.use_var(*var);
                            arg_vals.push(v);
                            needs_post_release.push(false);
                            continue;
                        }
                    }
                    // Normal path: +1 Owned, caller releases after call.
                    let v = lower_expr(
                        builder,
                        a,
                        current_return_ty,
                        env,
                        scope_stack,
                        func_ids_by_def,
                        module,
                        runtime,
                    )?;
                    arg_vals.push(v);
                    needs_post_release.push(is_ref);
                }
                let call = builder.ins().call(callee_ref, &arg_vals);
                let results = builder.inst_results(call);
                if matches!(expr.ty, Type::Nothing) {
                    if !results.is_empty() {
                        return Err(CodegenError::cranelift(
                            format!(
                                "agent `{callee_name}` returned {} values; native lowering expects 0 for `Nothing`",
                                results.len()
                            ),
                            expr.span,
                        ));
                    }
                } else if results.len() != 1 {
                    return Err(CodegenError::cranelift(
                        format!(
                            "agent `{callee_name}` returned {} values; native lowering expects exactly 1",
                            results.len()
                        ),
                        expr.span,
                    ));
                }
                let result = if matches!(expr.ty, Type::Nothing) {
                    builder.ins().iconst(I64, 0)
                } else {
                    results[0]
                };
                if !runtime.dup_drop_enabled {
                    for (v, needs) in arg_vals.iter().zip(needs_post_release.iter()) {
                        if *needs {
                            emit_release(builder, module, runtime, *v);
                        }
                    }
                }
                Ok(result)
            }
            IrCallKind::Tool { def_id, .. } => {
                // Emit a direct typed call to the tool's
                // `#[tool]`-generated wrapper symbol. No JSON, no
                // dynamic dispatch ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â just a `call` instruction against
                // a named import. Link-time symbol resolution catches
                // missing tool implementations; Cranelift-level
                // type-matching catches wrong-type mismatches at
                // parity-harness or codegen time.
                let tool = runtime.ir_tools.get(def_id).cloned().ok_or_else(|| {
                    CodegenError::cranelift(
                        format!(
                            "tool `{callee_name}` metadata missing from ir_tools ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â declare-pass invariant violated"
                        ),
                        expr.span,
                    )
                })?;

                // Arity cross-check ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â belt-and-braces vs. the
                // typechecker's check that already ran.
                if tool.params.len() != args.len() {
                    return Err(CodegenError::cranelift(
                        format!(
                            "tool `{callee_name}` declared with {} param(s) but called with {}",
                            tool.params.len(),
                            args.len()
                        ),
                        expr.span,
                    ));
                }
                let types = |id: DefId| runtime.ir_types.get(&id).cloned();
                if let Some(held) = tool.params.iter().map(|p| &p.ty).chain(std::iter::once(&tool.return_ty))
                    .find_map(|ty| crate::text_scalar_boundary::held_text_scalar(ty, &types)) {
                    return Err(CodegenError::not_supported(
                        format!(
                            "tool `{callee_name}`: {}",
                            crate::text_scalar_boundary::text_scalar_boundary_requirement(&held.display_name())
                        ),
                        expr.span,
                    ));
                }

                // Native binaries call the linked `__corvid_tool_<name>`
                // wrapper directly; library targets dispatch through the
                // runtime registry (below) and emit no such import.
                let wrapper_id = if runtime.tools_via_registry {
                    None
                } else {
                    let mut cache = runtime.tool_wrapper_ids.borrow_mut();
                    let id = if let Some(id) = cache.get(def_id) {
                        *id
                    } else {
                        let mut sig = module.make_signature();
                        for p in &tool.params {
                            sig.params.push(AbiParam::new(cl_type_for(&p.ty, p.span)?));
                        }
                        if !matches!(tool.return_ty, Type::Nothing) {
                            sig.returns
                                .push(AbiParam::new(cl_type_for(&tool.return_ty, tool.span)?));
                        }
                        let symbol = tool_wrapper_symbol(&tool.name);
                        let id = module
                            .declare_function(&symbol, Linkage::Import, &sig)
                            .map_err(|e| {
                                CodegenError::cranelift(
                                    format!("declare tool wrapper `{symbol}`: {e}"),
                                    expr.span,
                                )
                            })?;
                        cache.insert(*def_id, id);
                        id
                    };
                    Some(id)
                };

                // The tool borrows its inputs. Bare locals stay owned by
                // their scope; computed arguments must survive until the
                // call returns (or be released if a later argument exits).
                let outer_temporaries = runtime.return_temporaries.borrow().len();
                let mut arg_vals = Vec::with_capacity(args.len());
                for arg in args {
                    let (value, borrowed) = lower_container_maybe_borrowed(
                        builder, arg, current_return_ty, env, scope_stack,
                        func_ids_by_def, module, runtime,
                    )?;
                    if !borrowed && is_refcounted_type(&arg.ty) {
                        runtime.return_temporaries.borrow_mut().push(value);
                    }
                    arg_vals.push(value);
                }
                let trace_arg_tys = args.iter().map(|arg| arg.ty.clone()).collect::<Vec<_>>();
                let trace_payload =
                    emit_trace_payload(builder, module, runtime, &arg_vals, &trace_arg_tys, expr.span)?;
                let tool_name_val =
                    emit_string_const(builder, module, runtime, &tool.name, expr.span)?;
                let runtime_is_replay_ref =
                    module.declare_func_in_func(runtime.runtime_is_replay, builder.func);
                let runtime_is_replay_call = builder.ins().call(runtime_is_replay_ref, &[]);
                let runtime_is_replay = builder.inst_results(runtime_is_replay_call)[0];
                let replay_b = builder.create_block();
                let live_b = builder.create_block();
                let join_b = builder.create_block();
                let result_ty = if matches!(expr.ty, Type::Nothing) {
                    None
                } else {
                    Some(cl_type_for(&expr.ty, expr.span)?)
                };
                if let Some(result_ty) = result_ty {
                    builder.append_block_param(join_b, result_ty);
                }
                // A struct-returning tool under registry dispatch
                // records/replays its result as JSON (the dispatch and
                // replay bridges return a JSON string the per-struct
                // decoder turns into a struct handle).
                let registry_struct_def_id = if runtime.tools_via_registry {
                    match &expr.ty {
                        Type::Struct(id) => Some(*id),
                        Type::ImportedStruct(imported) => Some(imported.def_id),
                        _ => None,
                    }
                } else {
                    None
                };
                let replay_cond = builder.ins().icmp_imm(IntCC::NotEqual, runtime_is_replay, 0);
                builder
                    .ins()
                    .brif(replay_cond, replay_b, &[], live_b, &[]);

                builder.switch_to_block(replay_b);
                builder.seal_block(replay_b);
                if let Some(struct_def_id) = registry_struct_def_id {
                    // Replay a struct-returning tool: read the recorded
                    // JSON string and decode it into a struct handle.
                    let decoder_fid =
                        lookup_or_emit_struct_decoder(module, runtime, struct_def_id, expr.span)?;
                    let replay_ref =
                        module.declare_func_in_func(runtime.replay_tool_call_string, builder.func);
                    let replay_call = builder.ins().call(
                        replay_ref,
                        &[
                            tool_name_val,
                            trace_payload.type_tags,
                            trace_payload.count,
                            trace_payload.values_ptr,
                        ],
                    );
                    let json_str = builder.inst_results(replay_call)[0];
                    let decoder_ref = module.declare_func_in_func(decoder_fid, builder.func);
                    let decode_call = builder.ins().call(decoder_ref, &[json_str]);
                    let struct_ptr = builder.inst_results(decode_call)[0];
                    emit_release(builder, module, runtime, json_str);
                    builder.ins().jump(join_b, &[struct_ptr]);
                } else {
                let replay_func = match &expr.ty {
                    Type::Nothing => runtime.replay_tool_call_nothing,
                    Type::Int => runtime.replay_tool_call_int,
                    Type::Bool => runtime.replay_tool_call_bool,
                    Type::Float => runtime.replay_tool_call_float,
                    Type::String => runtime.replay_tool_call_string,
                    Type::Grounded(inner) => match &**inner {
                        Type::Int => runtime.replay_tool_call_int,
                        Type::Bool => runtime.replay_tool_call_bool,
                        Type::Float => runtime.replay_tool_call_float,
                        Type::String => runtime.replay_tool_call_string,
                        _ => {
                            return Err(CodegenError::not_supported(
                                format!(
                                    "native replay for tool `{}` with return type `{}` is not implemented yet",
                                    tool.name,
                                    expr.ty.display_name()
                                ),
                                expr.span,
                            ))
                        }
                    },
                    _ => {
                        return Err(CodegenError::not_supported(
                            format!(
                                "native replay for tool `{}` with return type `{}` is not implemented yet",
                                tool.name,
                                expr.ty.display_name()
                            ),
                            expr.span,
                        ))
                    }
                };
                let replay_ref = module.declare_func_in_func(replay_func, builder.func);
                let replay_call = builder.ins().call(
                    replay_ref,
                    &[
                        tool_name_val,
                        trace_payload.type_tags,
                        trace_payload.count,
                        trace_payload.values_ptr,
                    ],
                );
                if matches!(expr.ty, Type::Nothing) {
                    builder.ins().jump(join_b, &[]);
                } else {
                    let replay_value = builder.inst_results(replay_call)[0];
                    let replay_value = if matches!(expr.ty, Type::Grounded(_)) {
                        emit_grounded_value_attestation(
                            builder,
                            module,
                            runtime,
                            replay_value,
                            &expr.ty,
                            &tool.name,
                            1.0,
                            expr.span,
                        )?
                    } else {
                        replay_value
                    };
                    builder.ins().jump(join_b, &[replay_value]);
                }
                }

                builder.switch_to_block(live_b);
                builder.seal_block(live_b);
                let trace_tool_call_ref =
                    module.declare_func_in_func(runtime.trace_tool_call, builder.func);
                builder.ins().call(
                    trace_tool_call_ref,
                    &[
                        tool_name_val,
                        trace_payload.type_tags,
                        trace_payload.count,
                        trace_payload.values_ptr,
                    ],
                );

                if runtime.tools_via_registry {
                    // Library target: dispatch through the runtime tool
                    // registry; no link-time `__corvid_tool_<name>` symbol.
                    let invoke_func = match &expr.ty {
                        Type::Nothing => runtime.invoke_tool_nothing,
                        Type::Int => runtime.invoke_tool_int,
                        Type::Bool => runtime.invoke_tool_bool,
                        Type::Float => runtime.invoke_tool_float,
                        Type::String => runtime.invoke_tool_string,
                        Type::Struct(_) | Type::ImportedStruct(_) => runtime.invoke_tool_struct,
                        Type::Grounded(inner) => match &**inner {
                            Type::Int => runtime.invoke_tool_int,
                            Type::Bool => runtime.invoke_tool_bool,
                            Type::Float => runtime.invoke_tool_float,
                            Type::String => runtime.invoke_tool_string,
                            _ => {
                                return Err(CodegenError::not_supported(
                                    format!(
                                        "native dispatch for tool `{}` with return type `{}` is not implemented yet",
                                        tool.name,
                                        expr.ty.display_name()
                                    ),
                                    expr.span,
                                ))
                            }
                        },
                        _ => {
                            return Err(CodegenError::not_supported(
                                format!(
                                    "native dispatch for tool `{}` with return type `{}` is not implemented yet",
                                    tool.name,
                                    expr.ty.display_name()
                                ),
                                expr.span,
                            ))
                        }
                    };
                    let invoke_ref = module.declare_func_in_func(invoke_func, builder.func);
                    let invoke_call = builder.ins().call(
                        invoke_ref,
                        &[
                            tool_name_val,
                            trace_payload.type_tags,
                            trace_payload.count,
                            trace_payload.values_ptr,
                        ],
                    );
                    if matches!(expr.ty, Type::Nothing) {
                        let trace_ref = module
                            .declare_func_in_func(runtime.trace_tool_result_null, builder.func);
                        builder.ins().call(trace_ref, &[tool_name_val]);
                        builder.ins().jump(join_b, &[]);
                    } else if let Some(struct_def_id) = registry_struct_def_id {
                        // Record the result JSON for replay, then decode
                        // it into a struct handle. Recorder + decoder both
                        // borrow the JSON; release the owned (+1) string.
                        let json_str = builder.inst_results(invoke_call)[0];
                        let trace_ref = module
                            .declare_func_in_func(runtime.trace_tool_result_string, builder.func);
                        builder.ins().call(trace_ref, &[tool_name_val, json_str]);
                        let decoder_fid = lookup_or_emit_struct_decoder(
                            module,
                            runtime,
                            struct_def_id,
                            expr.span,
                        )?;
                        let decoder_ref = module.declare_func_in_func(decoder_fid, builder.func);
                        let decode_call = builder.ins().call(decoder_ref, &[json_str]);
                        let struct_ptr = builder.inst_results(decode_call)[0];
                        emit_release(builder, module, runtime, json_str);
                        builder.ins().jump(join_b, &[struct_ptr]);
                    } else {
                        let result_val = builder.inst_results(invoke_call)[0];
                        let trace_result_ref = match &expr.ty {
                            Type::Int => Some(runtime.trace_tool_result_int),
                            Type::Bool => Some(runtime.trace_tool_result_bool),
                            Type::Float => Some(runtime.trace_tool_result_float),
                            Type::String => Some(runtime.trace_tool_result_string),
                            Type::Grounded(inner) => match &**inner {
                                Type::Int => Some(runtime.trace_tool_result_int),
                                Type::Bool => Some(runtime.trace_tool_result_bool),
                                Type::Float => Some(runtime.trace_tool_result_float),
                                Type::String => Some(runtime.trace_tool_result_string),
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(trace_func) = trace_result_ref {
                            let trace_result_call =
                                module.declare_func_in_func(trace_func, builder.func);
                            builder
                                .ins()
                                .call(trace_result_call, &[tool_name_val, result_val]);
                        }
                        let live_result = if matches!(expr.ty, Type::Grounded(_)) {
                            emit_grounded_value_attestation(
                                builder,
                                module,
                                runtime,
                                result_val,
                                &expr.ty,
                                &tool.name,
                                1.0,
                                expr.span,
                            )?
                        } else {
                            result_val
                        };
                        builder.ins().jump(join_b, &[live_result]);
                    }
                } else {
                    // Native binary: direct call to the linked tool wrapper.
                    let wrapper_id =
                        wrapper_id.expect("native build declares the tool wrapper import");
                    let fref = module.declare_func_in_func(wrapper_id, builder.func);
                    let call = builder.ins().call(fref, &arg_vals);
                    let result_vals: Vec<ClValue> =
                        builder.inst_results(call).iter().copied().collect();
                    let trace_result_ref = match &expr.ty {
                        Type::Nothing => Some(runtime.trace_tool_result_null),
                        Type::Int => Some(runtime.trace_tool_result_int),
                        Type::Bool => Some(runtime.trace_tool_result_bool),
                        Type::Float => Some(runtime.trace_tool_result_float),
                        Type::String => Some(runtime.trace_tool_result_string),
                        Type::Grounded(inner) => match &**inner {
                            Type::Int => Some(runtime.trace_tool_result_int),
                            Type::Bool => Some(runtime.trace_tool_result_bool),
                            Type::Float => Some(runtime.trace_tool_result_float),
                            Type::String => Some(runtime.trace_tool_result_string),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(trace_func) = trace_result_ref {
                        let trace_result_call =
                            module.declare_func_in_func(trace_func, builder.func);
                        let trace_args = if matches!(expr.ty, Type::Nothing) {
                            vec![tool_name_val]
                        } else {
                            vec![tool_name_val, result_vals[0]]
                        };
                        builder.ins().call(trace_result_call, &trace_args);
                    }
                    let live_result = if matches!(expr.ty, Type::Grounded(_)) {
                        emit_grounded_value_attestation(
                            builder,
                            module,
                            runtime,
                            result_vals[0],
                            &expr.ty,
                            &tool.name,
                            1.0,
                            expr.span,
                        )?
                    } else {
                        result_vals[0]
                    };
                    if matches!(expr.ty, Type::Nothing) {
                        builder.ins().jump(join_b, &[]);
                    } else if result_vals.len() == 1 {
                        builder.ins().jump(join_b, &[live_result]);
                    } else {
                        return Err(CodegenError::cranelift(
                            format!(
                                "tool `{callee_name}` wrapper returned {} values; expected 1 for type `{}`",
                                result_vals.len(),
                                expr.ty.display_name()
                            ),
                            expr.span,
                        ));
                    }
                }

                builder.switch_to_block(join_b);
                builder.seal_block(join_b);
                emit_release(builder, module, runtime, trace_payload.type_tags);

                for value in runtime.return_temporaries.borrow_mut().drain(outer_temporaries..).rev() {
                    emit_release(builder, module, runtime, value);
                }
                emit_release(builder, module, runtime, tool_name_val);

                // Return-value unpacking. For Nothing-returning tools
                // there's no result to hand back; synthesize a
                // zero-Int so the expr-result contract stays uniform.
                if matches!(expr.ty, Type::Nothing) {
                    Ok(builder.ins().iconst(I64, 0))
                } else {
                    Ok(builder.block_params(join_b)[0])
                }
            }
            IrCallKind::Prompt { def_id, .. } => {
                let types = |id: DefId| runtime.ir_types.get(&id).cloned();
                if let Some(prompt) = runtime.ir_prompts.get(def_id) {
                    if let Some(held) = prompt.params.iter().map(|p| &p.ty).chain(std::iter::once(&prompt.return_ty))
                        .find_map(|ty| crate::text_scalar_boundary::held_text_scalar(ty, &types)) {
                        return Err(CodegenError::not_supported(
                            format!(
                                "prompt `{callee_name}`: {}",
                                crate::text_scalar_boundary::text_scalar_boundary_requirement(&held.display_name())
                            ),
                            expr.span,
                        ));
                    }
                }
                lower_prompt_call(
                    builder,
                    module,
                    runtime,
                    *def_id,
                    callee_name,
                    args,
                    current_return_ty,
                    env,
                    scope_stack,
                    func_ids_by_def,
                    &expr.ty,
                    expr.span,
                )
            }
            IrCallKind::StructConstructor { def_id } | IrCallKind::EnumConstructor { def_id, .. } => {
                let ty = runtime.ir_types.get(def_id).ok_or_else(||
                    CodegenError::cranelift(format!("missing layout for `{callee_name}`"), expr.span))?;
                let (fields, info, tag) = if let IrCallKind::EnumConstructor { variant_index, .. } = kind {
                    let variant = ty.variants.get(*variant_index as usize).ok_or_else(||
                        CodegenError::cranelift("unknown constructor tag", expr.span))?;
                    (&variant.fields, runtime.variant_typeinfos.get(&(*def_id, *variant_index)), Some(*variant_index))
                } else {
                    (&ty.fields, runtime.struct_typeinfos.get(def_id), None)
                };
                let info = info.ok_or_else(|| CodegenError::cranelift("missing constructor typeinfo", expr.span))?;
                lower_struct_constructor(
                    builder, module, runtime, fields, *info, tag,
                    args,
                    current_return_ty,
                    env,
                    scope_stack,
                    func_ids_by_def,
                    expr.span,
                )
            }
            IrCallKind::Fixture { .. } => Err(CodegenError::cranelift(
                format!("test fixture `{callee_name}` is interpreter-only and cannot be lowered natively"),
                expr.span,
            )),
            IrCallKind::Unknown => Err(CodegenError::cranelift(
                format!("call to `{callee_name}` did not resolve ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â typecheck should have caught this"),
                expr.span,
            )),
        },
        IrExprKind::FieldAccess { target, field } => {
            let (offset, field_meta) = record_field(runtime, &target.ty, field, expr.span)?;
            let field_cl_ty = cl_type_for(&field_meta.ty, field_meta.span)?;

            // Borrow-at-use-site peephole: if the target is a bare
            // `IrExprKind::Local`, read the Variable directly with
            // no ownership-conversion retain, and skip the symmetric
            // post-extract release of the struct pointer. The load
            // of the field only reads the struct's memory; we never
            // mutate the struct's refcount. The Local's binding
            // stays Live ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â its scope-exit release handles cleanup.
            let (struct_ptr, struct_borrowed) = lower_container_maybe_borrowed(
                builder, target, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let field_val = builder.ins().load(
                field_cl_ty,
                cranelift_codegen::ir::MemFlags::trusted(),
                struct_ptr,
                offset,
            );
            // Retain refcounted field so caller gets an Owned ref.
            // This retain is NOT redundant pass traffic ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â it's the
            // ownership conversion from "field slot inside a parent"
            // to "standalone result value the caller owns." The .6d
            // pass cannot replace this because the extracted value
            // never gets an IR Local; it's an intermediate temp
            // known only to codegen. Pairs with the struct_ptr
            // release below when struct_ptr was a fresh temp.
            if is_refcounted_type(&field_meta.ty) {
                emit_retain(builder, module, runtime, field_val);
            }
            // Release the temp +1 on the struct pointer only if we
            // created one. `struct_borrowed = false` means the target
            // was a fresh temp (call result, constructor, nested field
            // access) not a bare Local ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â the .6d pass doesn't see
            // those, so codegen must drop unconditionally. Borrowed
            // reads have no +1 to release.
            if !struct_borrowed {
                emit_release(builder, module, runtime, struct_ptr);
            }
            Ok(field_val)
        }
        IrExprKind::Index { target, index } => {
            // Element type from the Index expression's annotated type
            // (the type checker attaches the element type).
            let elem_ty = expr.ty.clone();
            let elem_cl_ty = cl_type_for(&elem_ty, expr.span)?;
            let elem_refcounted = is_refcounted_type(&elem_ty);

            // Same borrow-at-use-site trick
            // as FieldAccess ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â if the list target is a bare Local,
            // borrow it (no retain) and skip the post-extract release.
            // The bounds-check + load only read the list's memory;
            // never mutate its refcount or escape the pointer.
            let (list_ptr, list_borrowed) = lower_container_maybe_borrowed(
                builder, target, current_return_ty, env, scope_stack, func_ids_by_def, module, runtime,
            )?;
            let idx_val = lower_expr(
                builder,
                index,
                current_return_ty,
                env,
                scope_stack,
                func_ids_by_def,
                module,
                runtime,
            )?;

            let elem_addr = checked_list_slot(builder, module, runtime, list_ptr, idx_val);
            let elem_val = builder.ins().load(
                elem_cl_ty,
                cranelift_codegen::ir::MemFlags::trusted(),
                elem_addr,
                0,
            );
            // Retain extracted element as ownership conversion from
            // list slot ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â‚¬Å¾Ã‚Â¢ standalone caller-owned value. The .6d
            // pass can't replace this because the extracted value
            // never gets an IR Local. Pairs with list_ptr release.
            if elem_refcounted {
                emit_retain(builder, module, runtime, elem_val);
            }
            // Release the temp +1 on the list pointer only if we
            // actually took one. `list_borrowed = false` means the
            // target was a fresh temp, not a bare Local ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â the .6d
            // pass doesn't see internal temps, so codegen drops
            // unconditionally. Borrowed reads have no +1 to drop.
            if !list_borrowed {
                emit_release(builder, module, runtime, list_ptr);
            }
            Ok(elem_val)
        }
        IrExprKind::List { items } => {
            let Type::List(elem_ty) = &expr.ty else {
                return Err(CodegenError::not_supported("list literal with this element type", expr.span));
            };
            let count = builder.ins().iconst(I64, items.len() as i64);
            let list_ptr = allocate_slots(builder, module, runtime, elem_ty, count, expr.span)?;
            let slots = builder.ins().load(I64, MemFlags::trusted(), list_ptr, 8);
            for (i, item) in items.iter().enumerate() {
                let item_val = lower_expr(
                    builder,
                    item,
                    current_return_ty,
                    env,
                    scope_stack,
                    func_ids_by_def,
                    module,
                    runtime,
                )?;
                let offset = (i as i32) * 8;
                builder.ins().store(
                    cranelift_codegen::ir::MemFlags::trusted(),
                    item_val,
                    slots,
                    offset,
                );
                let initialized = builder.ins().iconst(I64, (i + 1) as i64);
                builder.ins().store(MemFlags::trusted(), initialized, list_ptr, 0);
            }
            Ok(list_ptr)
        }
        IrExprKind::WeakNew { strong } => {
            let strong_val = lower_expr(
                builder,
                strong,
                current_return_ty,
                env,
                scope_stack,
                func_ids_by_def,
                module,
                runtime,
            )?;
            let weak_new_ref = module.declare_func_in_func(runtime.weak_new, builder.func);
            let call = builder.ins().call(weak_new_ref, &[strong_val]);
            let weak_ptr = builder.inst_results(call)[0];
            if !runtime.dup_drop_enabled && is_refcounted_type(&strong.ty) {
                emit_release(builder, module, runtime, strong_val);
            }
            Ok(weak_ptr)
        }
        IrExprKind::WeakUpgrade { weak } => {
            let weak_val = lower_expr(
                builder,
                weak,
                current_return_ty,
                env,
                scope_stack,
                func_ids_by_def,
                module,
                runtime,
            )?;
            let weak_upgrade_ref =
                module.declare_func_in_func(runtime.weak_upgrade, builder.func);
            let call = builder.ins().call(weak_upgrade_ref, &[weak_val]);
            let upgraded = builder.inst_results(call)[0];
            if !runtime.dup_drop_enabled && is_refcounted_type(&weak.ty) {
                emit_release(builder, module, runtime, weak_val);
            }
            Ok(upgraded)
        }
        IrExprKind::ResultOk { inner } if is_native_result_type(&expr.ty) => lower_result_constructor(
            builder,
            expr,
            inner,
            RESULT_TAG_OK,
            current_return_ty,
            env,
            scope_stack,
            func_ids_by_def,
            module,
            runtime,
        ),
        IrExprKind::ResultErr { inner } if is_native_result_type(&expr.ty) => lower_result_constructor(
            builder,
            expr,
            inner,
            RESULT_TAG_ERR,
            current_return_ty,
            env,
            scope_stack,
            func_ids_by_def,
            module,
            runtime,
        ),
        // Result/Option construction and `?` / `try-retry` control
        // flow are native only for the current principled subset:
        //   * nullable-pointer `Option<T>` when `T` is refcounted
        //   * wide scalar `Option<Int|Bool|Float>`
        //   * one-word `Result<T, E>`
        //   * `try ... retry` over the native `Result<T, E>` subset
        //
        // Any shape outside that subset still gets a clean,
        // actionable boundary here. The `cl_type_for` path above
        // will typically fire first for bindings, but these arms are
        // the load-bearing fallback for intermediate expressions and
        // other positions the typecheck doesn't intercept.
        IrExprKind::ResultOk { .. } | IrExprKind::ResultErr { .. } => {
            Err(CodegenError::not_supported(
                "`Result<T, E>` construction (`Ok(...)` / `Err(...)`) ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â native tagged-union lowering is not implemented yet; use the interpreter tier (`corvid run --tier interp`) until then",
                expr.span,
            ))
        }
        IrExprKind::OptionSome { inner } => {
            if option_uses_wrapper(&expr.ty) {
                let payload_ty = match &expr.ty {
                    Type::Option(inner_ty) => &**inner_ty,
                    _ => unreachable!("option wrapper gate requires Option<T> type"),
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
                emit_option_wrapper_value(builder, module, runtime, &expr.ty, payload, payload_ty, expr.span)
            } else if is_refcounted_type(&expr.ty) {
                lower_expr(
                    builder,
                    inner,
                    current_return_ty,
                    env,
                    scope_stack,
                    func_ids_by_def,
                    module,
                    runtime,
                )
            } else {
                Err(CodegenError::not_supported(
                    "`Some(...)` ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡Ãƒâ€šÃ‚Â¬ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€šÃ‚Â native codegen currently supports nullable-pointer `Option<T>` when `T` is refcounted plus wide scalar `Option<Int|Bool|Float>`",
                    expr.span,
                ))
            }
        }
        IrExprKind::OptionNone => {
            Ok(builder.ins().iconst(I64, 0))
        }
        IrExprKind::TryPropagate { inner } => match &inner.ty {
            Type::Result(_, _) => lower_try_propagate_result(
                builder,
                expr,
                inner,
                current_return_ty,
                env,
                scope_stack,
                func_ids_by_def,
                module,
                runtime,
            ),
            _ => lower_try_propagate_option(
                builder,
                expr,
                inner,
                current_return_ty,
                env,
                scope_stack,
                func_ids_by_def,
                module,
                runtime,
            ),
        },
        IrExprKind::TrustBoundary { inner } => lower_expr(
            builder,
            inner,
            current_return_ty,
            env,
            scope_stack,
            func_ids_by_def,
            module,
            runtime,
        ),
        IrExprKind::TryRetry {
            body,
            attempts,
            backoff,
            timeout_ms,
        } => {
            if timeout_ms.is_some() {
                return Err(CodegenError::not_supported(
                    "`try ... timeout <ms>` is interpreter-only today — the native tier's                      synchronous call lowering has no cancellation point; run through                      `corvid run --target=interpreter` (the default for effectful programs)"
                        .to_string(),
                    expr.span,
                ));
            }
            match &body.ty {
            Type::Result(_, _) => lower_try_retry_result(
                builder,
                expr,
                body,
                *attempts,
                *backoff,
                current_return_ty,
                env,
                scope_stack,
                func_ids_by_def,
                module,
                runtime,
            ),
            Type::Option(_) => lower_try_retry_option(
                builder,
                expr,
                body,
                *attempts,
                *backoff,
                current_return_ty,
                env,
                scope_stack,
                func_ids_by_def,
                module,
                runtime,
            ),
            _ => Err(CodegenError::not_supported(
                "`try ... on error retry ...` in native code currently supports only native `Result<T, E>` and `Option<T>` bodies",
                expr.span,
            )),
        }
        }
        IrExprKind::Replay { .. } => Err(CodegenError::not_supported(
            "`replay` expressions require runtime pattern-dispatch over a recorded trace; native tier lowering lands in a follow-up to 21-inv-E-runtime. Use the interpreter tier (`corvid run --tier interp`) until then.",
            expr.span,
        )),
        IrExprKind::StreamSplitBy { .. }
        | IrExprKind::StreamMerge { .. }
        | IrExprKind::StreamOrderedBy { .. }
        | IrExprKind::StreamResumeToken { .. }
        | IrExprKind::ResumeStream { .. }
        | IrExprKind::Ask { .. }
        | IrExprKind::Choose { .. } => {
            Err(CodegenError::not_supported(
                "human-boundary and stream combinators are interpreter-backed in this release; use the interpreter tier (`corvid run --tier interp`)",
                expr.span,
            ))
        }
    }
}
