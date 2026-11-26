use super::*;

pub(super) fn signature(module: &ObjectModule, arity: usize, result: clir::Type) -> Signature {
    let mut sig = module.make_signature();
    // Slot zero is the closure itself, including for a named function.
    sig.params.resize(arity + 1, AbiParam::new(I64));
    sig.returns.push(AbiParam::new(result));
    sig
}

pub(super) fn create(
    builder: &mut FunctionBuilder,
    expr: &IrExpr,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    functions: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
) -> Result<ClValue, CodegenError> {
    let Type::Function {
        params: types, ret, ..
    } = &expr.ty
    else {
        unreachable!()
    };
    let sig = signature(module, types.len(), cl_type_for(ret, expr.span)?);
    let func = module
        .declare_anonymous_function(&sig)
        .map_err(|e| CodegenError::cranelift(e.to_string(), expr.span))?;
    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(UserFuncName::user(0, func.as_u32()), sig);
    let mut builder_ctx = FunctionBuilderContext::new();
    let mut inner = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
    let entry = inner.create_block();
    inner.append_block_params_for_function_params(entry);
    inner.switch_to_block(entry);
    inner.seal_block(entry);
    let arguments = inner.block_params(entry).to_vec();
    let mut captures = Vec::new();
    let value = match &expr.kind {
        IrExprKind::Decl { def_id, .. } => {
            let id = functions
                .get(def_id)
                .ok_or_else(|| CodegenError::cranelift("missing callback definition", expr.span))?;
            let target = module.declare_func_in_func(*id, inner.func);
            let call = inner.ins().call(target, &arguments[1..]);
            inner.inst_results(call)[0]
        }
        IrExprKind::Lambda { params, body } => {
            captures = crate::native_lambda_locals(body).ok_or_else(|| {
                CodegenError::not_supported("this lambda body requires the interpreter", expr.span)
            })?;
            captures.retain(|(id, _)| !params.iter().any(|p| p.local_id == *id));
            let mut locals = HashMap::new();
            for (param, arg) in params.iter().zip(&arguments[1..]) {
                let var = Variable::from_u32(locals.len() as u32);
                inner.declare_var(var, I64);
                inner.def_var(var, *arg);
                locals.insert(param.local_id, (var, I64));
            }
            for (index, (id, ty)) in captures.iter().enumerate() {
                let layout = cl_type_for(ty, expr.span)?;
                let value = inner.ins().load(
                    layout,
                    MemFlags::trusted(),
                    arguments[0],
                    capture_offset(index),
                );
                if is_refcounted_type(ty) {
                    inner.declare_value_needs_stack_map(value);
                }
                let var = Variable::from_u32(locals.len() as u32);
                inner.declare_var(var, layout);
                inner.def_var(var, value);
                locals.insert(*id, (var, layout));
            }
            lower_expr(
                &mut inner,
                body,
                ret,
                &locals,
                &Vec::new(),
                functions,
                module,
                runtime,
            )?
        }
        _ => unreachable!(),
    };
    inner.ins().return_(&[value]);
    inner.finalize();
    define_function_with_stack_maps(module, func, &mut ctx, runtime, expr.span, "callback")?;

    let info = if captures.iter().any(|(_, ty)| is_refcounted_type(ty)) {
        // The environment has the same slot layout as a record. Its first
        // slot holds code and the remaining slots own the captured values.
        let fields = std::iter::once(Type::Int)
            .chain(captures.iter().map(|(_, ty)| ty.clone()))
            .enumerate()
            .map(|(index, ty)| corvid_ir::IrField {
                name: index.to_string(),
                ty,
                refinement: None,
                json_key: None,
                span: expr.span,
            })
            .collect();
        let layout = corvid_ir::IrType {
            id: DefId(func.as_u32()),
            name: format!("callback.env.{}", func.as_u32()),
            fields,
            variants: Vec::new(),
            span: expr.span,
        };
        let destroy = define_struct_destructor(module, &layout, runtime)?;
        let trace = define_struct_trace(module, &layout, runtime)?;
        emit_struct_typeinfo(module, &layout, Some(destroy), trace, runtime)?
    } else {
        runtime.callback_typeinfo
    };
    let info = module.declare_data_in_func(info, builder.func);
    let info = builder.ins().symbol_value(I64, info);
    let bytes = builder.ins().iconst(I64, 8 * (captures.len() as i64 + 1));
    let alloc = module.declare_func_in_func(runtime.alloc_typed, builder.func);
    let call = builder.ins().call(alloc, &[bytes, info]);
    let handle = builder.inst_results(call)[0];
    let target = module.declare_func_in_func(func, builder.func);
    let address = builder.ins().func_addr(I64, target);
    builder.ins().store(MemFlags::trusted(), address, handle, 0);
    for (index, (id, ty)) in captures.iter().enumerate() {
        let var = env
            .get(id)
            .ok_or_else(|| CodegenError::cranelift("unbound lambda capture", expr.span))?
            .0;
        let value = builder.use_var(var);
        if is_refcounted_type(ty) {
            emit_retain(builder, module, runtime, value);
        }
        builder
            .ins()
            .store(MemFlags::trusted(), value, handle, capture_offset(index));
    }
    Ok(handle)
}

fn capture_offset(index: usize) -> i32 {
    8 * (index as i32 + 1)
}
