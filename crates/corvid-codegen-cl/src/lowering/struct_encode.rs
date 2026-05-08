//! Per-struct JSON encoder emission for entry-boundary struct
//! returns.
//!
//! When an entry agent declares a `Type::Struct(_)` return, the
//! native code generator emits a small Cranelift function with the
//! signature `extern "C" fn(struct_ptr: i64) -> CorvidString`. The
//! emitted `main` calls this encoder on the agent's return value
//! and prints the resulting JSON string via `corvid_print_string`,
//! producing a `{"field": value, ...}` line on stdout that mirrors
//! what the interpreter's `value_to_json` produces in source-order.
//!
//! Encoders are cached on `RuntimeFuncs.struct_to_json` keyed by
//! `DefId` — mirror of the decoder cache in `struct_decode.rs`.
//! Together they form a complete pair: a struct can cross the JSON
//! boundary in either direction (LLM JSON → struct via decoder;
//! struct → stdout JSON via encoder) without the runtime ever
//! learning the struct's field layout.
//!
//! Field-type support in v1 mirrors the decoder side: `Int`, `Bool`,
//! `Float`, `String`. Nested structs, lists, options, etc. are
//! rejected at emit time — same future-slice deferral.
//!
//! Refcount discipline. Each `String` field is loaded as a borrowed
//! descriptor pointer (the struct still owns it). The runtime's
//! `corvid_json_object_set_str` consumes the string at +0 ABI
//! (read_corvid_string moves bytes out, then implicitly releases on
//! its own internal cleanup path). To prevent the struct's
//! destructor from double-freeing the field after the encoder
//! returns, the encoder calls `corvid_retain` before each `set_str`
//! so the field's refcount goes from N → N+1, the setter consumes
//! the +1 it gained, and the struct's original refcount is
//! preserved for its own destructor to release later.

use super::*;
use crate::lowering::expr::lower_string_literal;
use crate::lowering::runtime::define_function_with_stack_maps;

/// Look up an existing encoder for `def_id` or emit one and cache it.
pub(super) fn lookup_or_emit_struct_to_json(
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    def_id: DefId,
    span: Span,
) -> Result<FuncId, CodegenError> {
    if let Some(&fid) = runtime.struct_to_json.borrow().get(&def_id) {
        return Ok(fid);
    }
    let ir_type = runtime.ir_types.get(&def_id).cloned().ok_or_else(|| {
        CodegenError::cranelift(
            format!(
                "struct metadata missing for def_id {def_id:?} when emitting entry-boundary encoder"
            ),
            span,
        )
    })?;
    let fid = emit_struct_to_json(module, runtime, &ir_type)?;
    runtime.struct_to_json.borrow_mut().insert(def_id, fid);
    Ok(fid)
}

/// Emit the encoder function body for `ir_type`.
fn emit_struct_to_json(
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    ir_type: &corvid_ir::IrType,
) -> Result<FuncId, CodegenError> {
    // Reject any unsupported field type up-front. Mirror of the
    // decoder's policy — v1 covers `Int` / `Bool` / `Float` /
    // `String` only.
    for field in &ir_type.fields {
        match &field.ty {
            Type::Int | Type::Bool | Type::Float | Type::String => {}
            other => {
                return Err(CodegenError::not_supported(
                    format!(
                        "field `{}: {}` on struct `{}` returned at the entry boundary — native struct encoder v1 supports `Int` / `Bool` / `Float` / `String` field types only; nested structs, lists, options, and results need their own encoder primitives (filed as later slices)",
                        field.name,
                        other.display_name(),
                        ir_type.name,
                    ),
                    field.span,
                ));
            }
        }
    }

    // Encoder signature: extern "C" fn(struct_ptr: i64) -> CorvidString as i64
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(I64));
    sig.returns.push(AbiParam::new(I64));

    let symbol = format!("corvid_{}__{}_to_json", ir_type.name, ir_type.id.0);
    let func_id = module
        .declare_function(&symbol, Linkage::Local, &sig)
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare struct encoder `{symbol}`: {e}"),
                ir_type.span,
            )
        })?;

    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(
        UserFuncName::user(0, func_id.as_u32()),
        module
            .declarations()
            .get_function_decl(func_id)
            .signature
            .clone(),
    );
    let mut bctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut bctx);
        let entry_b = builder.create_block();
        builder.append_block_params_for_function_params(entry_b);
        builder.switch_to_block(entry_b);
        builder.seal_block(entry_b);

        let struct_ptr = builder.block_params(entry_b)[0];

        // Build an empty JSON object handle.
        let new_ref = module.declare_func_in_func(runtime.json_object_new, builder.func);
        let new_call = builder.ins().call(new_ref, &[]);
        let builder_handle = builder.inst_results(new_call)[0];

        for (i, field) in ir_type.fields.iter().enumerate() {
            let offset = (i as i32) * STRUCT_FIELD_SLOT_BYTES;
            let name_val =
                lower_string_literal(&mut builder, module, runtime, &field.name, field.span)?;

            match &field.ty {
                Type::Int => {
                    let v = builder.ins().load(
                        I64,
                        cranelift_codegen::ir::MemFlags::trusted(),
                        struct_ptr,
                        offset,
                    );
                    let setter = module
                        .declare_func_in_func(runtime.json_object_set_int, builder.func);
                    builder.ins().call(setter, &[builder_handle, name_val, v]);
                }
                Type::Bool => {
                    let v_i8 = builder.ins().load(
                        I8,
                        cranelift_codegen::ir::MemFlags::trusted(),
                        struct_ptr,
                        offset,
                    );
                    // The setter expects i32 (0/1). Zero-extend the i8
                    // so true (1) and false (0) widen unambiguously
                    // — sign-extending a bool's high bits would be
                    // wrong if the slot's other 7 bytes hold garbage.
                    let v_i32 = builder.ins().uextend(I32, v_i8);
                    let setter = module
                        .declare_func_in_func(runtime.json_object_set_bool, builder.func);
                    builder.ins().call(setter, &[builder_handle, name_val, v_i32]);
                }
                Type::Float => {
                    let v = builder.ins().load(
                        F64,
                        cranelift_codegen::ir::MemFlags::trusted(),
                        struct_ptr,
                        offset,
                    );
                    let setter = module
                        .declare_func_in_func(runtime.json_object_set_float, builder.func);
                    builder.ins().call(setter, &[builder_handle, name_val, v]);
                }
                Type::String => {
                    let v = builder.ins().load(
                        I64,
                        cranelift_codegen::ir::MemFlags::trusted(),
                        struct_ptr,
                        offset,
                    );
                    // The struct still owns this String field; the
                    // setter consumes a +1 refcount via its
                    // `read_corvid_string` move. Retain so the
                    // setter's consumption doesn't deplete the
                    // struct's own count — the destructor will
                    // release when the struct drops.
                    let retain_ref = module.declare_func_in_func(runtime.retain, builder.func);
                    builder.ins().call(retain_ref, &[v]);
                    let setter = module
                        .declare_func_in_func(runtime.json_object_set_str, builder.func);
                    builder.ins().call(setter, &[builder_handle, name_val, v]);
                }
                _ => unreachable!("field-type validation rejected unsupported types up-front"),
            }
        }

        let finish_ref = module.declare_func_in_func(runtime.json_object_finish, builder.func);
        let finish_call = builder.ins().call(finish_ref, &[builder_handle]);
        let json_str = builder.inst_results(finish_call)[0];
        builder.ins().return_(&[json_str]);

        builder.finalize();
    }

    define_function_with_stack_maps(
        module,
        func_id,
        &mut ctx,
        runtime,
        ir_type.span,
        &format!("struct encoder `{symbol}`"),
    )?;
    Ok(func_id)
}
