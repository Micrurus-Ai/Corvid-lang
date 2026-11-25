//! `T.from_json(text)`, `T.from_json_strict(text)` and `value.to_json()`
//! in native code.
//!
//! Both directions are the shared public JSON mapping, run by the
//! runtime's `corvid_json_decode_typed` and `corvid_json_encode_typed`.
//! This module embeds, once per type, the plan the runtime follows (the
//! type, the declarations it reaches, and which typeinfo each shape
//! uses, which also tells the encoder which options are boxed). A
//! decode allocates the `Result<T, JsonError>` wrapper and lets the
//! runtime fill its payload; an encode passes the value and receives
//! the text.

use super::*;
use corvid_public_json::{builder_shapes, reachable_types, DecodePlan, Shape};
use std::collections::{BTreeMap, HashSet};

/// Whether native code can decode JSON into `target` and encode it:
/// every shape the mapping builds or reads for it has a native
/// representation. `Grounded` is excluded because native code keeps no
/// provenance to encode, and `JsonValue` has no native form.
pub fn native_public_json_type(target: &Type, types: &[corvid_ir::IrType]) -> bool {
    let by_id: HashMap<DefId, &corvid_ir::IrType> = types.iter().map(|ty| (ty.id, ty)).collect();
    Decodable { types: &by_id, secrets: false }.check(target, &mut HashSet::new())
}

/// Whether native code can decode the answer of a tool the runtime
/// implements whose result it holds as a secret (`secret(name)`,
/// `random_token(bytes)`): the public form, plus `Secret` positions,
/// which only the runtime call's decoder builds (native dispatch N3).
pub fn native_secret_result_type(target: &Type, types: &[corvid_ir::IrType]) -> bool {
    let by_id: HashMap<DefId, &corvid_ir::IrType> = types.iter().map(|ty| (ty.id, ty)).collect();
    Decodable { types: &by_id, secrets: true }.check(target, &mut HashSet::new())
}

/// Whether native code can make `tool`'s runtime call: every parameter
/// has a native JSON form and so does the result, where a tool whose
/// result the runtime holds as a secret may return one. The code
/// generator and the driver's tier picker both ask this.
pub fn native_runtime_call_types(tool: &corvid_ir::IrTool, types: &[corvid_ir::IrType]) -> bool {
    let result_is_native = if corvid_runtime::runtime::is_stdlib_secret_value_tool(&tool.name) {
        native_secret_result_type(&tool.return_ty, types)
    } else {
        native_public_json_type(&tool.return_ty, types)
    };
    result_is_native && tool.params.iter().all(|p| native_public_json_type(&p.ty, types))
}

struct Decodable<'a, 'types> {
    types: &'a HashMap<DefId, &'types corvid_ir::IrType>,
    secrets: bool,
}

impl Decodable<'_, '_> {
    fn check(&self, ty: &Type, seen: &mut HashSet<DefId>) -> bool {
        decodable(ty, self, seen)
    }
}

fn decodable(ty: &Type, rules: &Decodable<'_, '_>, seen: &mut HashSet<DefId>) -> bool {
    let types = rules.types;
    match ty {
        Type::Secret => rules.secrets,
        // B06 — a `Decimal`, time, currency or locale value travels as
        // its canonical text.
        Type::Int | Type::Float | Type::String | Type::Bool => true,
        ty if crate::text_scalar::is_text_scalar(ty) => true,
        Type::Tainted(inner) | Type::List(inner) => decodable(inner, rules, seen),
        Type::Option(inner) => is_native_option_type(ty) && decodable(inner, rules, seen),
        Type::Map(_, value) => crate::native_string_map(ty) && decodable(value, rules, seen),
        Type::Result(ok, err) => {
            is_native_result_type(ty) && decodable(ok, rules, seen) && decodable(err, rules, seen)
        }
        // The driver checks programs before `JsonError` becomes a record
        // and this backend after, so the record is excluded here too:
        // both see it as a type with no native JSON mapping.
        Type::Struct(id) if *id == corvid_types::JSON_ERROR_TYPE_ID => false,
        Type::Struct(id) | Type::ImportedStruct(corvid_types::types::ImportedStructType { def_id: id, .. }) => {
            let Some(decl) = types.get(id) else { return false };
            if !seen.insert(*id) {
                return true;
            }
            decl.fields
                .iter()
                .chain(decl.variants.iter().flat_map(|variant| &variant.fields))
                .all(|field| decodable(&field.ty, rules, seen))
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
    if !native_public_json_type(target, &types) {
        return Err(CodegenError::not_supported(
            format!(
                "decoding JSON into `{}` - a type in it has no native form; use the interpreter tier (`corvid run --tier interp`)",
                target.display_name()
            ),
            expr.span,
        ));
    }
    let (plan, plan_len, table) = json_plan(module, runtime, target, expr.span)?;

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

/// Lower `value.to_json()`; `value` is the receiver, lowered as a slot.
pub(super) fn lower_json_encode(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    receiver_ty: &Type,
    value: ClValue,
    span: Span,
) -> Result<ClValue, CodegenError> {
    let types: Vec<corvid_ir::IrType> = runtime.ir_types.values().cloned().collect();
    if !native_public_json_type(receiver_ty, &types) {
        return Err(CodegenError::not_supported(
            format!(
                "encoding `{}` as JSON - a type in it has no native form; use the interpreter tier (`corvid run --tier interp`)",
                receiver_ty.display_name()
            ),
            span,
        ));
    }
    let slot = match builder.func.dfg.value_type(value) {
        I8 => builder.ins().uextend(I64, value),
        F64 => builder.ins().bitcast(I64, MemFlags::new(), value),
        _ => value,
    };
    let (plan, plan_len, table) = json_plan(module, runtime, receiver_ty, span)?;
    let plan = module.declare_data_in_func(plan, builder.func);
    let plan = builder.ins().symbol_value(I64, plan);
    let plan_len = builder.ins().iconst(I64, plan_len);
    let table = module.declare_data_in_func(table, builder.func);
    let table = builder.ins().symbol_value(I64, table);
    let encode = module.declare_func_in_func(runtime.json_encode_typed, builder.func);
    let call = builder.ins().call(encode, &[slot, plan, plan_len, table]);
    Ok(builder.inst_results(call)[0])
}

/// The embedded plan for `target`: its data, length and typeinfo table.
pub(super) fn json_plan(
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
    // A decode failure allocates a `JsonError` record. Every program that
    // decodes mentions `JsonError` in its `Result`, so the record is
    // declared; a program that only encodes needs none.
    let error_record = Some(Shape::Record(corvid_types::JSON_ERROR_TYPE_ID))
        .filter(|_| runtime.struct_typeinfos.contains_key(&corvid_types::JSON_ERROR_TYPE_ID));
    for shape in builder_shapes(target, &types_by_id).into_iter().chain(error_record) {
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
