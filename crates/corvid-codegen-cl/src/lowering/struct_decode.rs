//! Per-struct JSON decoder emission for prompt struct returns.
//!
//! When a prompt agent declares a `Type::Struct(def_id)` return, the
//! native code generator emits a small Cranelift function with the
//! signature `extern "C" fn(json: CorvidString) -> i64`. The runtime
//! `corvid_prompt_call_struct` bridge invokes this decoder on each
//! retry attempt, treating a non-zero return as the heap-allocated
//! struct pointer and a zero return as a parse failure (retry).
//!
//! Decoders are cached on `RuntimeFuncs.struct_decoders` keyed by
//! `DefId` so multiple prompts returning the same struct re-use one
//! decoder. The runtime never learns the struct's field layout — the
//! language-aware decoding logic lives entirely here.
//!
//! The emitted control flow:
//!
//! ```text
//! entry:
//!   handle = corvid_json_parse(json_text)
//!   if handle == 0: jump fail
//!
//! check_field_0:
//!   present = corvid_json_field_present(handle, "<f0>")
//!   if present == 0: jump release_fail
//! ...
//! check_field_N:  (one block per field)
//!
//! all_present:
//!   ptr = corvid_alloc_typed(size, &typeinfo)
//!   for each field: read via the type-appropriate getter, store at offset
//!   corvid_json_release(handle)
//!   return ptr
//!
//! release_fail:
//!   corvid_json_release(handle)
//!   jump fail
//!
//! fail:
//!   return 0
//! ```
//!
//! Field type support in v1 mirrors the four scalar prompt bridges:
//! `Int`, `Bool`, `Float`, `String`. Nested struct fields, list
//! fields, optional fields, etc. are out of scope and rejected at
//! emit time with a clear error message pointing at the unsupported
//! field.

use super::*;
use crate::lowering::expr::lower_string_literal;
use crate::lowering::runtime::{define_function_with_stack_maps, struct_payload_bytes};

/// Look up an existing decoder for `def_id` or emit one and cache it.
/// First-call emits; subsequent calls reuse the cached `FuncId` so
/// repeated prompt returns of the same struct don't generate
/// duplicate decoders.
pub(super) fn lookup_or_emit_struct_decoder(
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    def_id: DefId,
    span: Span,
) -> Result<FuncId, CodegenError> {
    if let Some(&fid) = runtime.struct_decoders.borrow().get(&def_id) {
        return Ok(fid);
    }
    let ir_type = runtime.ir_types.get(&def_id).cloned().ok_or_else(|| {
        CodegenError::cranelift(
            format!("struct metadata missing for def_id {def_id:?} when emitting prompt-return decoder"),
            span,
        )
    })?;
    let fid = emit_struct_decoder(module, runtime, &ir_type)?;
    runtime.struct_decoders.borrow_mut().insert(def_id, fid);
    Ok(fid)
}

/// Emit the decoder function body for `ir_type`.
fn emit_struct_decoder(
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    ir_type: &corvid_ir::IrType,
) -> Result<FuncId, CodegenError> {
    // Reject any unsupported field type up-front with a clear
    // message. v1 only supports Int / Bool / Float / String fields,
    // matching the four scalar prompt bridges. Nested structs,
    // lists, options, results, etc. need their own decoder primitives
    // and are filed as future slices.
    for field in &ir_type.fields {
        match &field.ty {
            Type::Int | Type::Bool | Type::Float | Type::String => {}
            other => {
                return Err(CodegenError::not_supported(
                    format!(
                        "field `{}: {}` on struct `{}` returned from a prompt — native struct decoder v1 supports `Int` / `Bool` / `Float` / `String` field types only; nested structs, lists, options, and results need their own decoder primitives (filed as later slices)",
                        field.name,
                        other.display_name(),
                        ir_type.name,
                    ),
                    field.span,
                ));
            }
        }
    }

    let typeinfo_id = *runtime.struct_typeinfos.get(&ir_type.id).ok_or_else(|| {
        CodegenError::cranelift(
            format!(
                "struct `{}` has no typeinfo emitted — every struct should have a typeinfo per the post-17a uniform allocation rule",
                ir_type.name,
            ),
            ir_type.span,
        )
    })?;

    // Decoder signature: extern "C" fn(json_text: CorvidString as i64) -> i64
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(I64));
    sig.returns.push(AbiParam::new(I64));

    // Symbol mangled with the DefId number so two structs with the
    // same name (e.g., one local and one imported, or two via
    // shadowing in different scopes) never collide. The struct name
    // is included for readability in stack traces and `nm` output.
    let symbol = format!("corvid_decode_{}__{}", ir_type.name, ir_type.id.0);
    let func_id = module
        .declare_function(&symbol, Linkage::Local, &sig)
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare struct decoder `{symbol}`: {e}"),
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

        // Blocks:
        //   entry: parse and check handle
        //   check_field_<i>: per-field presence checks (one per field)
        //   all_present: allocate, store fields, release, return ptr
        //   release_fail: release the JSON handle, fall through to fail
        //   fail: return 0
        let entry_b = builder.create_block();
        let check_blocks: Vec<clir::Block> = ir_type
            .fields
            .iter()
            .map(|_| builder.create_block())
            .collect();
        // After the last per-field check we jump into all_present.
        let all_present_b = builder.create_block();
        let release_fail_b = builder.create_block();
        let fail_b = builder.create_block();

        // ---- entry: parse the JSON ------------------------------------
        builder.append_block_params_for_function_params(entry_b);
        builder.switch_to_block(entry_b);
        let json_text = builder.block_params(entry_b)[0];

        let parse_ref = module.declare_func_in_func(runtime.json_parse, builder.func);
        let parse_call = builder.ins().call(parse_ref, &[json_text]);
        let handle = builder.inst_results(parse_call)[0];
        let zero_i64 = builder.ins().iconst(I64, 0);
        let parse_failed = builder.ins().icmp(IntCC::Equal, handle, zero_i64);
        // On parse failure, jump straight to fail (no handle to release).
        // Otherwise jump to the first per-field check (or all_present
        // if the struct has no fields).
        let first_check = check_blocks
            .first()
            .copied()
            .unwrap_or(all_present_b);
        builder
            .ins()
            .brif(parse_failed, fail_b, &[], first_check, &[]);
        builder.seal_block(entry_b);

        // ---- per-field presence checks --------------------------------
        // Pre-emit each field-name string literal once, before the
        // builder switches blocks. lower_string_literal allocates a
        // .rodata symbol; the emitted ClValue (a symbol reload) is
        // valid within the block where it was emitted.
        //
        // We need each field-name literal in TWO places: the presence
        // check block AND the all_present block (to feed the getters).
        // Re-emitting inside each block produces fresh symbol_value
        // instructions referring to the same .rodata datum — Cranelift
        // doesn't dedupe symbol_value across blocks, but the underlying
        // .rodata datum is shared, so this is purely a per-block
        // ClValue rematerialisation, not a duplicate string allocation.
        for (i, field) in ir_type.fields.iter().enumerate() {
            let block = check_blocks[i];
            builder.switch_to_block(block);
            builder.seal_block(block);

            let name_val =
                lower_string_literal(&mut builder, module, runtime, &field.name, field.span)?;
            let present_ref = module.declare_func_in_func(runtime.json_field_present, builder.func);
            let present_call = builder.ins().call(present_ref, &[handle, name_val]);
            let present_i32 = builder.inst_results(present_call)[0];
            let zero_i32 = builder.ins().iconst(I32, 0);
            let absent = builder.ins().icmp(IntCC::Equal, present_i32, zero_i32);
            // Next destination on the success branch: the next per-
            // field check, or all_present after the last field.
            let next_block = if i + 1 < check_blocks.len() {
                check_blocks[i + 1]
            } else {
                all_present_b
            };
            builder
                .ins()
                .brif(absent, release_fail_b, &[], next_block, &[]);
        }

        // ---- all_present: allocate, fill fields, release, return -----
        builder.switch_to_block(all_present_b);
        builder.seal_block(all_present_b);

        let size = builder
            .ins()
            .iconst(I64, struct_payload_bytes(ir_type.fields.len()));
        let ti_gv = module.declare_data_in_func(typeinfo_id, builder.func);
        let ti_addr = builder.ins().symbol_value(I64, ti_gv);
        let alloc_ref = module.declare_func_in_func(runtime.alloc_typed, builder.func);
        let alloc_call = builder.ins().call(alloc_ref, &[size, ti_addr]);
        let struct_ptr = builder.inst_results(alloc_call)[0];

        for (i, field) in ir_type.fields.iter().enumerate() {
            let name_val =
                lower_string_literal(&mut builder, module, runtime, &field.name, field.span)?;
            let offset = (i as i32) * STRUCT_FIELD_SLOT_BYTES;

            // Each branch reads the field via the type-appropriate
            // getter, normalises to the struct slot's storage width,
            // and stores. Field-type rejection at the top of this
            // function ensures `_ =>` is unreachable here.
            match &field.ty {
                Type::Int => {
                    let getter = runtime.json_get_field_int;
                    let r = module.declare_func_in_func(getter, builder.func);
                    let c = builder.ins().call(r, &[handle, name_val]);
                    let v = builder.inst_results(c)[0]; // I64
                    builder.ins().store(MemFlags::trusted(), v, struct_ptr, offset);
                }
                Type::Bool => {
                    let getter = runtime.json_get_field_bool;
                    let r = module.declare_func_in_func(getter, builder.func);
                    let c = builder.ins().call(r, &[handle, name_val]);
                    let v_i32 = builder.inst_results(c)[0]; // I32
                    // Struct fields store Bool as I8 (the slot is
                    // 8 bytes wide; the I8 occupies the low byte).
                    let v_i8 = builder.ins().ireduce(I8, v_i32);
                    builder
                        .ins()
                        .store(MemFlags::trusted(), v_i8, struct_ptr, offset);
                }
                Type::Float => {
                    let getter = runtime.json_get_field_float;
                    let r = module.declare_func_in_func(getter, builder.func);
                    let c = builder.ins().call(r, &[handle, name_val]);
                    let v = builder.inst_results(c)[0]; // F64
                    builder.ins().store(MemFlags::trusted(), v, struct_ptr, offset);
                }
                Type::String => {
                    let getter = runtime.json_get_field_str;
                    let r = module.declare_func_in_func(getter, builder.func);
                    let c = builder.ins().call(r, &[handle, name_val]);
                    let v = builder.inst_results(c)[0]; // I64 (CorvidString descriptor ptr)
                    builder.ins().store(MemFlags::trusted(), v, struct_ptr, offset);
                }
                _ => unreachable!("field-type validation rejected unsupported types up-front"),
            }
        }

        let release_ref = module.declare_func_in_func(runtime.json_release, builder.func);
        builder.ins().call(release_ref, &[handle]);
        builder.ins().return_(&[struct_ptr]);

        // ---- release_fail: release the handle, then fall through ----
        builder.switch_to_block(release_fail_b);
        builder.seal_block(release_fail_b);
        let release_ref = module.declare_func_in_func(runtime.json_release, builder.func);
        builder.ins().call(release_ref, &[handle]);
        builder.ins().jump(fail_b, &[]);

        // ---- fail: return 0 (the bridge-defined parse-failure code) -
        builder.switch_to_block(fail_b);
        builder.seal_block(fail_b);
        let zero = builder.ins().iconst(I64, 0);
        builder.ins().return_(&[zero]);

        builder.finalize();
    }

    define_function_with_stack_maps(
        module,
        func_id,
        &mut ctx,
        runtime,
        ir_type.span,
        &format!("struct decoder `{symbol}`"),
    )?;
    Ok(func_id)
}
