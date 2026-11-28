use super::*;

#[derive(Debug)]
enum TemplateSegment<'a> {
    Literal(&'a str),
    Param(usize), // index into the prompt's params
}

fn prompt_constant_arg_text(expr: &IrExpr) -> Option<String> {
    match &expr.kind {
        // A parameter renders as its JSON form (the grammar's rule, and
        // the interpreter's): a String is quoted and escaped.
        IrExprKind::Literal(IrLiteral::String(s)) => Some(serde_json::Value::String(s.clone()).to_string()),
        IrExprKind::Literal(IrLiteral::Int(n)) => Some(n.to_string()),
        IrExprKind::Literal(IrLiteral::Bool(b)) => Some(if *b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        _ => None,
    }
}

fn render_prompt_constant(
    segments: &[TemplateSegment<'_>],
    args: &[IrExpr],
) -> Option<String> {
    let mut rendered = String::new();
    for seg in segments {
        match seg {
            TemplateSegment::Literal(text) => rendered.push_str(text),
            TemplateSegment::Param(idx) => {
                rendered.push_str(&prompt_constant_arg_text(&args[*idx])?);
            }
        }
    }
    Some(rendered)
}

/// Parse a prompt template into literal + `{param_name}` segments.
/// Param names that aren't in `params` produce a codegen error —
/// matches what the typechecker should already enforce, kept as
/// belt-and-braces.
fn parse_prompt_template<'a>(
    template: &'a str,
    params: &[corvid_ir::IrParam],
    span: Span,
) -> Result<Vec<TemplateSegment<'a>>, CodegenError> {
    let mut out: Vec<TemplateSegment<'a>> = Vec::new();
    let mut cursor = 0;
    let bytes = template.as_bytes();
    while cursor < bytes.len() {
        if let Some(open_rel) = template[cursor..].find('{') {
            let open = cursor + open_rel;
            if open > cursor {
                out.push(TemplateSegment::Literal(&template[cursor..open]));
            }
            let close_rel = template[open + 1..].find('}').ok_or_else(|| {
                CodegenError::cranelift(
                    format!(
                        "prompt template has unmatched `{{` near offset {open}: `{template}`"
                    ),
                    span,
                )
            })?;
            let close = open + 1 + close_rel;
            let name = template[open + 1..close].trim();
            let idx = params.iter().position(|p| p.name == name).ok_or_else(|| {
                CodegenError::cranelift(
                    format!(
                        "prompt template references `{{{name}}}` but no such parameter — typechecker should have caught this; available: {:?}",
                        params.iter().map(|p| &p.name).collect::<Vec<_>>()
                    ),
                    span,
                )
            })?;
            out.push(TemplateSegment::Param(idx));
            cursor = close + 1;
        } else {
            out.push(TemplateSegment::Literal(&template[cursor..]));
            break;
        }
    }
    Ok(out)
}

pub(super) fn emit_string_const(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    s: &str,
    span: Span,
) -> Result<ClValue, CodegenError> {
    lower_string_literal(builder, module, runtime, s, span)
}

fn emit_stringify_arg(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    arg_value: ClValue,
    arg_ty: &Type,
    span: Span,
) -> Result<ClValue, CodegenError> {
    match arg_ty {
        Type::Grounded(inner) => {
            emit_stringify_arg(builder, module, runtime, arg_value, inner, span)
        }
        Type::String => Ok(arg_value),
        Type::Int => {
            let f = module.declare_func_in_func(runtime.string_from_int, builder.func);
            let call = builder.ins().call(f, &[arg_value]);
            let results: Vec<ClValue> =
                builder.inst_results(call).iter().copied().collect();
            Ok(results[0])
        }
        Type::Bool => {
            let f = module.declare_func_in_func(runtime.string_from_bool, builder.func);
            let call = builder.ins().call(f, &[arg_value]);
            let results: Vec<ClValue> =
                builder.inst_results(call).iter().copied().collect();
            Ok(results[0])
        }
        Type::Float => {
            let f = module.declare_func_in_func(runtime.string_from_float, builder.func);
            let call = builder.ins().call(f, &[arg_value]);
            let results: Vec<ClValue> =
                builder.inst_results(call).iter().copied().collect();
            Ok(results[0])
        }
        other => Err(CodegenError::not_supported(
            format!(
                "prompt argument type `{}` is not yet supported in template interpolation — the native prompt bridge currently supports only Int / Bool / Float / String; Struct / List interpolation is not implemented yet",
                other.display_name()
            ),
            span,
        )),
    }
}

fn is_string_like_prompt_arg(ty: &Type) -> bool {
    match ty {
        Type::String => true,
        Type::Grounded(inner) => is_string_like_prompt_arg(inner),
        _ => false,
    }
}

fn emit_citation_text(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    value: ClValue,
    value_ty: &Type,
    span: Span,
) -> Result<(ClValue, bool), CodegenError> {
    match value_ty {
        Type::Grounded(inner) => {
            emit_citation_text(builder, module, runtime, value, inner, span)
        }
        Type::String => Ok((value, true)),
        Type::Int | Type::Bool | Type::Float => {
            let text = emit_stringify_arg(builder, module, runtime, value, value_ty, span)?;
            Ok((text, false))
        }
        other => Err(CodegenError::not_supported(
            format!(
                "`cites ... strictly` for `{}` is not yet supported by native codegen; use Grounded<String> or a scalar grounded context",
                other.display_name()
            ),
            span,
        )),
    }
}

fn emit_citation_check(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    prompt_name: ClValue,
    context: ClValue,
    response: ClValue,
) {
    let verify_ref = module.declare_func_in_func(runtime.citation_verify_or_panic, builder.func);
    builder
        .ins()
        .call(verify_ref, &[prompt_name, context, response]);
}

fn emit_concat_chain(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    parts: Vec<(ClValue, bool)>,
    span: Span,
) -> Result<(ClValue, bool), CodegenError> {
    if parts.is_empty() {
        return emit_string_const(builder, module, runtime, "", span).map(|v| (v, false));
    }
    let mut parts = parts.into_iter();
    let (mut acc, mut acc_borrowed) = parts.next().expect("parts not empty");
    let concat_fid = module.declare_func_in_func(runtime.string_concat, builder.func);
    for (next, next_borrowed) in parts {
        let call = builder.ins().call(concat_fid, &[acc, next]);
        let results: Vec<ClValue> =
            builder.inst_results(call).iter().copied().collect();
        let new_acc = results[0];
        if !acc_borrowed {
            emit_release(builder, module, runtime, acc);
        }
        if !next_borrowed {
            emit_release(builder, module, runtime, next);
        }
        acc = new_acc;
        acc_borrowed = false;
    }
    Ok((acc, acc_borrowed))
}

fn emit_grounded_prompt_attestation(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    value: ClValue,
    grounded_ty: &Type,
    prompt_name: &str,
    confidence: f64,
    span: Span,
) -> Result<ClValue, CodegenError> {
    let source_name = emit_string_const(builder, module, runtime, prompt_name, span)?;
    let confidence_val = builder.ins().f64const(confidence);
    let attest_id = match grounded_ty {
        Type::Grounded(inner) => match inner.as_ref() {
            Type::Int => runtime.grounded_attest_int,
            Type::Bool => runtime.grounded_attest_bool,
            Type::Float => runtime.grounded_attest_float,
            Type::String => runtime.grounded_attest_string,
            // `Grounded<Struct>` reuses the heap-pointer-keyed
            // `pointer_attestations` map at runtime — same storage
            // path as `Grounded<String>`, just keyed on the raw
            // struct pointer rather than a CorvidString descriptor.
            // The decoder from commit 4 produces the struct
            // pointer; the bridge from commit 3 hands it back; this
            // attestation arm pins the source + confidence onto it
            // before the value reaches the agent body.
            Type::Struct(_) => runtime.grounded_attest_struct,
            other => {
                return Err(CodegenError::not_supported(
                    format!(
                        "prompt grounded return `{}` is not yet supported at the native ABI boundary",
                        other.display_name()
                    ),
                    span,
                ))
            }
        },
        _ => unreachable!("grounded prompt attestation requires Grounded<T>"),
    };
    let attest_ref = module.declare_func_in_func(attest_id, builder.func);
    let attest_call = builder.ins().call(attest_ref, &[value, source_name, confidence_val]);
    let attested = builder.inst_results(attest_call)[0];
    emit_release(builder, module, runtime, source_name);
    Ok(attested)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_prompt_call(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    def_id: DefId,
    callee_name: &str,
    args: &[IrExpr],
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    return_ty: &Type,
    span: Span,
) -> Result<ClValue, CodegenError> {
    // Whether the statement after this call tests the outage slot itself
    // (recoverable failures TE-3c); read before lowering any argument.
    let checked = runtime.outage_checked.replace(false);
    let prompt = runtime
        .ir_prompts
        .get(&def_id)
        .cloned()
        .ok_or_else(|| {
            CodegenError::cranelift(
                format!(
                    "prompt `{callee_name}` metadata missing from ir_prompts — declare-pass invariant violated"
                ),
                span,
            )
        })?;

    if prompt.params.len() != args.len() {
        return Err(CodegenError::cranelift(
            format!(
                "prompt `{callee_name}` declared with {} param(s) but called with {}",
                prompt.params.len(),
                args.len()
            ),
            span,
        ));
    }

    let pinned_locals = runtime.prompt_pins.get(&span);
    let mut arg_vals: Vec<ClValue> = Vec::with_capacity(args.len());
    let outer_temporaries = runtime.return_temporaries.borrow().len();
    for (a, param) in args.iter().zip(&prompt.params) {
        let (v, borrowed) = lower_container_maybe_borrowed(
            builder,
            a,
            current_return_ty,
            env,
            scope_stack,
            func_ids_by_def,
            module,
            runtime,
        )?;
        // Checked before the model is called (parameter refinements PR-2).
        super::expr::hold_parameter_refinement(builder, module, runtime, &prompt.name, param, v, a.span)?;
        let pinned = is_string_like_prompt_arg(&a.ty)
            && matches!(&a.kind, IrExprKind::Local { .. })
            && pinned_locals.is_some_and(|set| {
                matches!(
                    &a.kind,
                    IrExprKind::Local { local_id, .. } if set.contains(local_id)
                )
            });
        arg_vals.push(v);
        if !pinned && is_refcounted_type(&a.ty) {
            if borrowed {
                emit_retain(builder, module, runtime, v);
            }
            runtime.return_temporaries.borrow_mut().push(v);
        }
    }

    // Each argument as the model is shown it: its public JSON text,
    // encoded through the plan the compiler embeds, the form the
    // interpreter renders and records (native scalar boundaries B1b). A
    // `Grounded<T>` argument shows its `T`. The texts fill the template
    // and are the call's recorded arguments, so both tiers send and trace
    // the same text.
    let types: Vec<corvid_ir::IrType> = runtime.ir_types.values().cloned().collect();
    let mut arg_json: Vec<ClValue> = Vec::with_capacity(args.len());
    for (arg, value) in args.iter().zip(&arg_vals) {
        let shown = match &arg.ty {
            Type::Grounded(inner) => inner.as_ref(),
            other => other,
        };
        if !super::public_json::native_public_json_type(shown, &types) {
            return Err(CodegenError::not_supported(
                format!(
                    "prompt `{callee_name}` takes a `{}`, which has no native JSON form; use the interpreter tier",
                    shown.display_name()
                ),
                arg.span,
            ));
        }
        arg_json.push(super::public_json::lower_json_encode(builder, module, runtime, shown, *value, arg.span)?);
    }

    let segments = parse_prompt_template(&prompt.template, &prompt.params, span)?;
    let (rendered, rendered_borrowed) = if let Some(text) =
        render_prompt_constant(&segments, args)
    {
        (emit_string_const(builder, module, runtime, &text, span)?, false)
    } else {
        let mut parts: Vec<(ClValue, bool)> = Vec::with_capacity(segments.len());
        for seg in &segments {
            let part = match seg {
                TemplateSegment::Literal(text) => (
                    emit_string_const(builder, module, runtime, text, span)?,
                    false,
                ),
                // The argument's JSON text, released after the call.
                TemplateSegment::Param(idx) => (arg_json[*idx], true),
            };
            parts.push(part);
        }
        emit_concat_chain(builder, module, runtime, parts, span)?
    };

    let prompt_name_val = emit_string_const(builder, module, runtime, &prompt.name, span)?;
    let model_val = emit_string_const(builder, module, runtime, "", span)?;
    let trace_payload = super::runtime::json_trace_payload(builder, module, runtime, &arg_json, span)?;

    let bridge_return_ty = match return_ty {
        Type::Grounded(inner) => inner.as_ref(),
        other => other,
    };
    // Scalar replies use the four scalar bridges, whose text parsing is
    // the interpreter's for those types. Every other reply (a record, a
    // list, an `Option`, a `Result`, a sum type, a text-held scalar) goes
    // through `corvid_prompt_call_typed`: the model is shown the
    // public-form schema and the reply is read with the public decoder
    // through the embedded plan, as on the interpreter (B1b).
    // Every reply goes through `corvid_prompt_call_typed`, which makes the
    // interpreter's request (the rendered prompt and `reply_schema`'s
    // schema, none for a `String`), reads the reply with the public
    // decoder through the embedded plan, and retries exactly as
    // `with repair N` says.
    let reply_ty = bridge_return_ty;
    if !super::public_json::native_public_json_type(reply_ty, &types) {
        return Err(CodegenError::not_supported(
            format!(
                "prompt `{callee_name}` returns `{}`, which has no native JSON form; use the interpreter tier",
                reply_ty.display_name()
            ),
            span,
        ));
    }
    let types_by_id: std::collections::HashMap<DefId, &corvid_ir::IrType> =
        runtime.ir_types.iter().map(|(k, v)| (*k, v)).collect();
    let schema_text = match corvid_prompt_format::reply_schema(reply_ty, &types_by_id) {
        Some(schema) => serde_json::to_string(&schema).map_err(|e| {
            CodegenError::cranelift(format!("failed to serialise schema for prompt `{callee_name}` return: {e}"), span)
        })?,
        None => String::new(),
    };
    // `.rodata`: an immortal string, never released.
    let schema_val = emit_string_const(builder, module, runtime, &schema_text, span)?;
    let repair_attempts = builder.ins().iconst(I64, prompt.repair_attempts.unwrap_or(0) as i64);
    let (plan, plan_len, table) = super::public_json::json_plan(module, runtime, reply_ty, span)?;
    let plan = module.declare_data_in_func(plan, builder.func);
    let plan = builder.ins().symbol_value(I64, plan);
    let plan_len = builder.ins().iconst(I64, plan_len);
    let table = module.declare_data_in_func(table, builder.func);
    let table = builder.ins().symbol_value(I64, table);
    let payload_slot =
        builder.create_sized_stack_slot(clir::StackSlotData::new(clir::StackSlotKind::ExplicitSlot, 8, 3));
    let payload = builder.ins().stack_addr(I64, payload_slot, 0);
    let bridge_ref = module.declare_func_in_func(runtime.prompt_call_typed, builder.func);
    builder.ins().call(
        bridge_ref,
        &[
            prompt_name_val,
            rendered,
            model_val,
            trace_payload.type_tags,
            trace_payload.count,
            trace_payload.values_ptr,
            schema_val,
            repair_attempts,
            plan,
            plan_len,
            table,
            payload,
        ],
    );
    super::expr::emit_outage_stop_unless_checked(builder, module, runtime, checked);
    let bits = builder.ins().stack_load(I64, payload_slot, 0);
    let result_value = match cl_type_for(reply_ty, span)? {
        I8 => builder.ins().ireduce(I8, bits),
        F64 => builder.ins().bitcast(F64, MemFlags::new(), bits),
        _ => bits,
    };
    let result_vals = [result_value];

    // On a held outage the reply is zero bits, which name no text to
    // check or value to attest: the unwinding path skips both.
    let skip_on_outage = |builder: &mut FunctionBuilder, module: &mut ObjectModule| {
        let pending = module.declare_func_in_func(runtime.outage_pending, builder.func);
        let pending = builder.ins().call(pending, &[]);
        builder.inst_results(pending)[0]
    };
    let citation_join = if checked && prompt.cites_strictly_param.is_some() {
        let pending = skip_on_outage(builder, module);
        let check = builder.create_block();
        let join = builder.create_block();
        builder.ins().brif(pending, join, &[], check, &[]);
        builder.switch_to_block(check);
        builder.seal_block(check);
        Some(join)
    } else {
        None
    };
    if let Some(param_idx) = prompt.cites_strictly_param {
        let (context_text, context_borrowed) = emit_citation_text(
            builder,
            module,
            runtime,
            arg_vals[param_idx],
            &args[param_idx].ty,
            span,
        )?;
        let (response_text, response_borrowed) = emit_citation_text(
            builder,
            module,
            runtime,
            result_vals[0],
            bridge_return_ty,
            span,
        )?;
        emit_citation_check(
            builder,
            module,
            runtime,
            prompt_name_val,
            context_text,
            response_text,
        );
        if !context_borrowed {
            emit_release(builder, module, runtime, context_text);
        }
        if !response_borrowed {
            emit_release(builder, module, runtime, response_text);
        }
    }
    if let Some(join) = citation_join {
        builder.ins().jump(join, &[]);
        builder.switch_to_block(join);
        builder.seal_block(join);
    }

    emit_release(builder, module, runtime, prompt_name_val);
    if !rendered_borrowed {
        emit_release(builder, module, runtime, rendered);
    }
    emit_release(builder, module, runtime, model_val);
    emit_release(builder, module, runtime, trace_payload.type_tags);
    for text in arg_json {
        emit_release(builder, module, runtime, text);
    }

    for value in runtime.return_temporaries.borrow_mut().drain(outer_temporaries..).rev() {
        emit_release(builder, module, runtime, value);
    }

    if matches!(return_ty, Type::Grounded(_)) {
        if !checked {
            return emit_grounded_prompt_attestation(
                builder,
                module,
                runtime,
                result_vals[0],
                return_ty,
                &prompt.name,
                prompt.effect_confidence,
                span,
            );
        }
        let pending = skip_on_outage(builder, module);
        let attest = builder.create_block();
        let done = builder.create_block();
        let result = builder.append_block_param(done, cl_type_for(return_ty, span)?);
        builder.ins().brif(pending, done, &[result_vals[0].into()], attest, &[]);
        builder.switch_to_block(attest);
        builder.seal_block(attest);
        let attested = emit_grounded_prompt_attestation(
            builder,
            module,
            runtime,
            result_vals[0],
            return_ty,
            &prompt.name,
            prompt.effect_confidence,
            span,
        )?;
        builder.ins().jump(done, &[attested.into()]);
        builder.switch_to_block(done);
        builder.seal_block(done);
        return Ok(result);
    }
    Ok(result_vals[0])
}


