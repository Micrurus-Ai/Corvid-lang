//! `T.from_json(text)` and `T.from_json_strict(text)` in native code.
//!
//! The decoding itself is the shared public JSON decoder, run by the
//! runtime's `corvid_json_decode_typed`. This module embeds, once per
//! target type, the decode plan the runtime follows (the target, the
//! declarations it reaches, and which typeinfo each allocated shape
//! uses), then allocates the `Result<T, JsonError>` wrapper and lets
//! the runtime fill its payload.

use super::*;
use corvid_public_json::{builder_shapes, reachable_types, DecodePlan, Shape};
use std::collections::{BTreeMap, HashSet};

/// Whether native code can decode JSON into `target`: every shape the
/// decoder builds for it has a native representation.
pub fn native_json_decode_target(target: &Type, types: &[corvid_ir::IrType]) -> bool {
    let by_id: HashMap<DefId, &corvid_ir::IrType> = types.iter().map(|ty| (ty.id, ty)).collect();
    decodable(target, &by_id, &mut HashSet::new())
}

fn decodable(ty: &Type, types: &HashMap<DefId, &corvid_ir::IrType>, seen: &mut HashSet<DefId>) -> bool {
    match ty {
        Type::Int | Type::Float | Type::String | Type::Bool => true,
        Type::Tainted(inner) | Type::List(inner) => decodable(inner, types, seen),
        Type::Option(inner) => is_native_option_type(ty) && decodable(inner, types, seen),
        Type::Map(_, value) => crate::native_string_map(ty) && decodable(value, types, seen),
        Type::Result(ok, err) => {
            is_native_result_type(ty) && decodable(ok, types, seen) && decodable(err, types, seen)
        }
        Type::Struct(id) | Type::ImportedStruct(corvid_types::types::ImportedStructType { def_id: id, .. }) => {
            let Some(decl) = types.get(id) else { return false };
            if !seen.insert(*id) {
                return true;
            }
            decl.fields
                .iter()
                .chain(decl.variants.iter().flat_map(|variant| &variant.fields))
                .all(|field| decodable(&field.ty, types, seen))
        }
        _ => false,
    }
}

/// Lower a decode whose JSON text is `text`; `expr` is the
/// `from_json` call, typed `Result<T, JsonError>`.
pub(super) fn lower_json_decode(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    expr: &IrExpr,
    text: ClValue,
    strict: bool,
) -> Result<ClValue, CodegenError> {
    let Type::Result(target, _) = &expr.ty else {
        return Err(CodegenError::cranelift(
            format!("`from_json` is typed `{}`, not a Result", expr.ty.display_name()),
            expr.span,
        ));
    };
    let types: Vec<corvid_ir::IrType> = runtime.ir_types.values().cloned().collect();
    if !native_json_decode_target(target, &types) {
        return Err(CodegenError::not_supported(
            format!(
                "decoding JSON into `{}` - a type in it has no native form; use the interpreter tier (`corvid run --tier interp`)",
                target.display_name()
            ),
            expr.span,
        ));
    }
    let (plan, plan_len, table) = decode_plan(module, runtime, target, expr.span)?;

    let empty = builder.ins().iconst(I64, 0);
    let result = emit_result_wrapper_value(builder, module, runtime, &expr.ty, empty, target, RESULT_TAG_OK, expr.span)?;
    let payload = builder.ins().iadd_imm(result, i64::from(RESULT_PAYLOAD_OFFSET));
    let plan = module.declare_data_in_func(plan, builder.func);
    let plan = builder.ins().symbol_value(I64, plan);
    let plan_len = builder.ins().iconst(I64, plan_len);
    let table = module.declare_data_in_func(table, builder.func);
    let table = builder.ins().symbol_value(I64, table);
    let strict = builder.ins().iconst(I64, i64::from(strict));
    let decode = module.declare_func_in_func(runtime.json_decode_typed, builder.func);
    let call = builder.ins().call(decode, &[text, plan, plan_len, table, strict, payload]);
    let tag = builder.inst_results(call)[0];
    builder.ins().store(MemFlags::trusted(), tag, result, RESULT_TAG_OFFSET);
    Ok(result)
}

/// The embedded plan for `target`: its data, length and typeinfo table.
fn decode_plan(
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    target: &Type,
    span: Span,
) -> Result<(cranelift_module::DataId, i64, cranelift_module::DataId), CodegenError> {
    if let Some(cached) = runtime.json_decode_plans.borrow().get(target) {
        return Ok(*cached);
    }
    let types_by_id: HashMap<DefId, &corvid_ir::IrType> = runtime.ir_types.iter().map(|(id, ty)| (*id, ty)).collect();
    let mut table = Vec::new();
    let mut shapes = BTreeMap::new();
    let mut owning_maps = Vec::new();
    let error_record = Shape::Record(corvid_types::JSON_ERROR_TYPE_ID);
    for shape in builder_shapes(target, &types_by_id).into_iter().chain(std::iter::once(error_record)) {
        let typeinfo = match &shape {
            // A nullable-pointer option has no allocation of its own.
            Shape::Type(option @ Type::Option(_)) if !option_uses_wrapper(option) => continue,
            Shape::Type(option @ Type::Option(_)) => runtime.option_typeinfos.get(option).copied(),
            Shape::Type(Type::List(element)) => runtime.list_typeinfos.get(&**element).copied(),
            Shape::Type(result @ Type::Result(_, _)) => runtime.result_typeinfos.get(result).copied(),
            // Maps carry no per-type metadata, only whether they own
            // their values.
            Shape::Type(map @ Type::Map(_, value)) => {
                if is_refcounted_type(value) {
                    owning_maps.push(map.clone());
                }
                continue;
            }
            Shape::Type(_) => None,
            Shape::Record(id) => runtime.struct_typeinfos.get(id).copied(),
            Shape::Variant(id, index) => runtime.variant_typeinfos.get(&(*id, *index)).copied(),
        };
        let typeinfo = typeinfo.ok_or_else(|| {
            CodegenError::cranelift(
                format!(
                    "internal compiler error: no typeinfo was emitted for {shape:?}, which decoding `{}` allocates",
                    target.display_name()
                ),
                span,
            )
        })?;
        shapes.insert(shape, table.len() as u32);
        table.push(typeinfo);
    }
    let plan = DecodePlan {
        root: target.clone(),
        types: reachable_types(target, &types_by_id).into_iter().cloned().collect(),
        shapes,
        owning_maps,
    };
    let text = plan.to_text().map_err(|reason| CodegenError::not_supported(reason, span))?;
    let error = |e: cranelift_module::ModuleError| CodegenError::cranelift(e.to_string(), span);

    let mut plan_data = DataDescription::new();
    plan_data.define(text.clone().into_bytes().into_boxed_slice());
    let plan_id = module.declare_anonymous_data(false, false).map_err(error)?;
    module.define_data(plan_id, &plan_data).map_err(error)?;

    let mut table_data = DataDescription::new();
    table_data.set_align(8);
    table_data.define(vec![0u8; table.len() * 8].into_boxed_slice());
    for (index, typeinfo) in table.iter().enumerate() {
        let address = module.declare_data_in_data(*typeinfo, &mut table_data);
        table_data.write_data_addr((index * 8) as u32, address, 0);
    }
    let table_id = module.declare_anonymous_data(false, false).map_err(error)?;
    module.define_data(table_id, &table_data).map_err(error)?;

    let entry = (plan_id, text.len() as i64, table_id);
    runtime.json_decode_plans.borrow_mut().insert(target.clone(), entry);
    Ok(entry)
}
