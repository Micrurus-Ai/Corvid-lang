//! Cranelift-based native codegen for Corvid.
//!
//! AOT-first: IR → relocatable object file → system linker →
//! `target/bin/<stem>[.exe]`. No JIT detour. The interpreter in
//! `corvid-vm` remains the oracle — the parity harness
//! (`tests/parity.rs`) runs every fixture through both tiers and
//! asserts identical results.
//!
//! Overflow policy: every `Int` arithmetic op uses Cranelift's
//! `sadd_overflow` / `ssub_overflow` / `smul_overflow` and branches to
//! a runtime handler (`corvid_runtime_overflow`, linked from the C
//! shim) on overflow. Division and modulo also trap on a zero divisor.
//! This matches the interpreter's `Arithmetic("integer overflow")`
//! semantics byte-for-byte.
//!
//! See `ARCHITECTURE.md` §4 (pipeline).

#![forbid(unsafe_code)]

pub mod bytes_methods;
pub use bytes_methods::is_native_bytes_method;
pub mod cdylib;
pub mod dataflow;
pub mod currency_methods;
pub use currency_methods::is_native_currency_method;
pub mod decimal_methods;
pub use decimal_methods::is_native_decimal_method;
pub mod dup_drop;
pub mod errors;
pub mod latency_rc;
pub mod link;
pub mod lowering;
pub mod module;
pub mod ownership;
pub mod pair_elim;
pub mod result_match;
pub mod secret_boundary;
pub mod text_scalar;
pub mod text_scalar_boundary;
pub mod time_methods;
pub use time_methods::is_native_time_method;

mod assignment;
pub use assignment::{native_indexed_assignment, native_list_append, native_local_compound, native_record_assignment};
pub use lowering::{native_public_json_type, native_runtime_call_types};
pub use lowering::{is_native_option_type_for, is_native_value_type_for, is_refcounted_type_for, NativeTypeQuestion};
mod json_error_record;
mod response_record;
mod regex_match_record;
mod map_support;
pub use map_support::{native_string_map, native_string_map_lookup, NATIVE_MAP_REQUIREMENTS};
pub mod scope_reduce;
pub mod staticlib_discovery;
pub mod target;
pub mod text_methods;
pub use text_methods::native_text_method;

pub use errors::{CodegenError, CodegenErrorKind};
pub use target::BuildTarget;

use corvid_ir::IrFile;
use cranelift_module::{DataDescription, Linkage, Module};
use std::path::{Path, PathBuf};

/// Compile `ir` to a relocatable object file at `object_path`. If
/// `entry_agent_name` is provided, the object exports a `corvid_entry`
/// trampoline symbol pointing at that agent — the C shim's link target.
pub fn compile_to_object(
    ir: &IrFile,
    module_name: &str,
    object_path: &Path,
    entry_agent_name: Option<&str>,
    embedded_descriptor: Option<&[u8]>,
    embedded_attestation: Option<&[u8]>,
    // True for library targets (cdylib/staticlib): tool calls dispatch
    // through the runtime registry so the binary links without the
    // host's tools. False for native binaries (linked tool wrappers).
    tools_via_registry: bool,
) -> Result<(), CodegenError> {
    if let Some(tool) = ir.tools.iter().find(|tool| tool.python.is_some()) {
        return Err(CodegenError::not_supported("typed Python imports (use the interpreter)", tool.span));
    }
    // Run the ownership analysis pass before codegen.
    // Output is a transformed IrFile with borrow_sig populated on
    // every agent. Codegen downstream consults borrow_sig at call
    // sites to skip the callee-entry retain + scope-exit release
    // pair for Borrowed parameters.
    // `JsonError` becomes an ordinary record before ownership analysis,
    // so its values are reference counted like any record's.
    // A program with connectors embeds their dispatch specs, the
    // projection the interpreter's runtime gets (native dispatch N1). It
    // reads the program as checked, before the rewrites below: a spec
    // says whether its operation returns a `Response`, which the rewrite
    // turns into a record.
    let connector_specs = corvid_runtime::connectors::connector_calls_json(ir);
    let mut ir = ir.clone();
    json_error_record::represent_json_error_as_record(&mut ir);
    // `Response<Body>` becomes a record per body type, after `JsonError`
    // so a body holding one already names its record (native dispatch N4).
    response_record::represent_responses_as_records(&mut ir);
    regex_match_record::represent_regex_match_as_record(&mut ir);
    let (ir_analyzed, _summaries) = ownership::analyze(ir);

    let mut module = module::make_host_object_module(module_name)?;
    let _func_ids =
        lowering::lower_file(&ir_analyzed, &mut module, entry_agent_name, tools_via_registry, connector_specs.as_deref())?;
    if let Some(bytes) = embedded_descriptor {
        define_embedded_descriptor(&mut module, bytes)?;
    }
    if let Some(bytes) = embedded_attestation {
        define_embedded_attestation(&mut module, bytes)?;
    }
    let product = module.finish();
    let bytes = product
        .emit()
        .map_err(|e| CodegenError::cranelift(format!("object emit: {e}"), span_zero()))?;
    if let Some(parent) = object_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CodegenError::io(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(object_path, bytes)
        .map_err(|e| CodegenError::io(format!("write {}: {e}", object_path.display())))?;
    Ok(())
}

/// Compile + link `ir` into a native binary at `bin_path`. Picks the
/// entry agent automatically: the sole agent, or the one named `main`
/// if multiple are present.
///
/// `extra_tool_libs` holds additional `.lib` / `.a` paths the linker
/// should pull in. Native tool dispatch uses this for the tool-implementation
/// staticlib: `cargo build` produces `libuser_tools.a`, the path goes
/// here, and the linker resolves `__corvid_tool_<name>` symbols
/// emitted by `IrCallKind::Tool` codegen. An empty list means no
/// user-provided tools — tool-using programs will fail to link with
/// an unresolved-symbol error, which is the correct outcome. The
/// driver surfaces a friendlier error earlier.
pub fn build_native_to_disk(
    ir: &IrFile,
    module_name: &str,
    bin_path: &Path,
    extra_tool_libs: &[&Path],
) -> Result<PathBuf, CodegenError> {
    let entry = pick_entry_agent(ir)?;
    // The native command-line boundary currently supports the four
    // scalar types. Structs and lists at the entry boundary remain
    // blocked until a dedicated serialization layer exists. The
    // codegen-emitted main itself enforces the same boundary; this
    // guard surfaces the error earlier with the agent name.
    for p in &entry.params {
        if matches!(
            &p.ty,
            corvid_types::Type::Struct(_) | corvid_types::Type::List(_)
        ) {
            return Err(CodegenError::not_supported(
                format!(
                    "entry agent `{}` parameter `{}` is `{}` — the native command-line boundary currently supports only Int/Bool/Float/String; structured input needs a dedicated serialization layer (use a wrapper agent that takes a String and parses internally)",
                    entry.name,
                    p.name,
                    p.ty.display_name()
                ),
                p.span,
            ));
        }
    }
    // `Type::Struct(_)` is now accepted at the entry-return
    // boundary as of Phase 20n-C commit 5; the codegen-emitted main
    // calls a per-struct JSON encoder (`crate::lowering::struct_encode`)
    // and prints the result via `print_string`. `Type::List(_)`
    // returns still need their own encoder primitives and are filed
    // as a follow-up slice.
    if matches!(&entry.return_ty, corvid_types::Type::List(_)) {
        return Err(CodegenError::not_supported(
            format!(
                "entry agent `{}` returns `{}` - the native command-line boundary supports `Int` / `Bool` / `Float` / `String` / `Struct` returns; `List` returns need their own dedicated encoder primitives and are tracked as a follow-up slice",
                entry.name,
                entry.return_ty.display_name()
            ),
            entry.span,
        ));
    }
    let out_bin = link::binary_path_for(
        bin_path.parent().unwrap_or(Path::new(".")),
        bin_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("program"),
    );
    let obj_dir = tempfile::Builder::new()
        .prefix("corvid-obj-")
        .tempdir()
        .map_err(|e| CodegenError::io(format!("tempdir: {e}")))?;
    let object_path = obj_dir.path().join(format!("{module_name}.o"));
    // Native binary: tools are linked in via `extra_tool_libs`, so
    // dispatch calls the typed `__corvid_tool_<name>` wrapper directly.
    compile_to_object(ir, module_name, &object_path, Some(&entry.name), None, None, false)?;
    link::link_binary(&object_path, &entry.name, &out_bin, extra_tool_libs)?;
    Ok(out_bin)
}

pub fn build_library_to_disk(
    ir: &IrFile,
    module_name: &str,
    output_path: &Path,
    target: BuildTarget,
    extra_tool_libs: &[&Path],
    embedded_descriptor: Option<&[u8]>,
    embedded_attestation: Option<&[u8]>,
) -> Result<PathBuf, CodegenError> {
    cdylib::build_library_to_disk(
        ir,
        module_name,
        output_path,
        target,
        extra_tool_libs,
        embedded_descriptor,
        embedded_attestation,
    )
}

fn define_embedded_attestation(
    module: &mut cranelift_object::ObjectModule,
    bytes: &[u8],
) -> Result<(), CodegenError> {
    let data_id = module
        .declare_data(
            corvid_abi::CORVID_ABI_ATTESTATION_SYMBOL,
            Linkage::Export,
            false,
            false,
        )
        .map_err(|e| {
            CodegenError::cranelift(format!("declare embedded attestation: {e}"), span_zero())
        })?;
    let mut desc = DataDescription::new();
    desc.set_align(8);
    desc.define(bytes.to_vec().into_boxed_slice());
    module.define_data(data_id, &desc).map_err(|e| {
        CodegenError::cranelift(format!("define embedded attestation: {e}"), span_zero())
    })
}

fn define_embedded_descriptor(
    module: &mut cranelift_object::ObjectModule,
    bytes: &[u8],
) -> Result<(), CodegenError> {
    let data_id = module
        .declare_data(
            corvid_abi::CORVID_ABI_DESCRIPTOR_SYMBOL,
            Linkage::Export,
            false,
            false,
        )
        .map_err(|e| {
            CodegenError::cranelift(format!("declare embedded descriptor: {e}"), span_zero())
        })?;
    let mut desc = DataDescription::new();
    desc.set_align(8);
    desc.define(bytes.to_vec().into_boxed_slice());
    module.define_data(data_id, &desc).map_err(|e| {
        CodegenError::cranelift(format!("define embedded descriptor: {e}"), span_zero())
    })
}

fn pick_entry_agent(ir: &IrFile) -> Result<&corvid_ir::IrAgent, CodegenError> {
    if ir.agents.is_empty() {
        return Err(CodegenError::not_supported(
            "no agents declared — compiled binaries need an entry agent",
            span_zero(),
        ));
    }
    if ir.agents.len() == 1 {
        return Ok(&ir.agents[0]);
    }
    if let Some(main) = ir.agents.iter().find(|a| a.name == "main") {
        return Ok(main);
    }
    Err(CodegenError::not_supported(
        format!(
            "multiple agents declared and none is named `main`; available: {}",
            ir.agents
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        span_zero(),
    ))
}

/// Build-time fingerprint of this native backend's source and target triple.
pub const NATIVE_BACKEND_ID: &str = include_str!(concat!(env!("OUT_DIR"), "/native-backend-id.txt"));

fn span_zero() -> corvid_ast::Span {
    corvid_ast::Span::new(0, 0)
}

/// Concatenating a grounded string needs ownership and provenance for the new value.
pub fn requires_grounded_string_concat(
    op: corvid_ast::BinaryOp,
    left: &corvid_types::Type,
    right: &corvid_types::Type,
) -> bool {
    use corvid_types::Type;
    op == corvid_ast::BinaryOp::Add
        && [left, right].iter().any(|ty| matches!(ty, Type::Grounded(inner) if **inner == Type::String))
}

/// Integer callbacks share one native ABI: environment handle followed by arguments.
pub fn native_callback_type(ty: &corvid_types::Type) -> bool {
    use corvid_types::Type;
    matches!(ty, Type::Function { params, ret, .. } if matches!(**ret, Type::Int | Type::Bool) && params.iter().all(|p| *p == Type::Int))
}

/// Collect each local needed by an integer callback, with its storage type.
pub fn native_lambda_locals(body: &corvid_ir::IrExpr) -> Option<Vec<(corvid_resolve::LocalId, corvid_types::Type)>> {
    use corvid_ir::{IrExpr, IrExprKind};
    use corvid_resolve::LocalId;
    use corvid_types::{BuiltinMethodKind, Type};

    fn local(expr: &IrExpr, locals: &mut Vec<(LocalId, Type)>) -> Option<()> {
        let IrExprKind::Local { local_id, .. } = &expr.kind else { return None };
        if !locals.iter().any(|(id, _)| id == local_id) {
            locals.push((*local_id, expr.ty.clone()));
        }
        Some(())
    }

    fn list(expr: &IrExpr, locals: &mut Vec<(LocalId, Type)>) -> Option<()> {
        if !matches!(&expr.ty, Type::List(element) if **element == Type::Int) { return None; }
        local(expr, locals)
    }

    fn collect(expr: &IrExpr, locals: &mut Vec<(LocalId, Type)>) -> Option<()> {
        if !matches!(expr.ty, Type::Int | Type::Bool) { return None; }
        match &expr.kind {
            IrExprKind::Literal(corvid_ir::IrLiteral::Int(_) | corvid_ir::IrLiteral::Bool(_)) => {}
            IrExprKind::Local { .. } => local(expr, locals)?,
            IrExprKind::Index { target, index } => {
                list(target, locals)?;
                collect(index, locals)?;
            }
            IrExprKind::BuiltinMethod { kind: BuiltinMethodKind::ListLength, receiver, .. } => {
                list(receiver, locals)?;
            }
            IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
                collect(left, locals)?;
                collect(right, locals)?;
            }
            IrExprKind::Call { kind: corvid_ir::IrCallKind::Agent { .. }, args, .. } => {
                for arg in args { collect(arg, locals)?; }
            }
            IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => collect(operand, locals)?,
            _ => return None,
        }
        Some(())
    }
    let mut locals = Vec::new();
    collect(body, &mut locals)?;
    Some(locals)
}

pub fn native_integer_list_transform(
    method: corvid_types::BuiltinMethodKind, receiver: &corvid_types::Type, args: &[corvid_ir::IrExpr],
) -> bool {
    use corvid_types::{BuiltinMethodKind, Type};
    let (callback, arity, result) = match (method, args) {
        (BuiltinMethodKind::ListMap, [f]) => (f, 1, Type::Int),
        (BuiltinMethodKind::ListFilter | BuiltinMethodKind::ListAny | BuiltinMethodKind::ListAll, [f]) => (f, 1, Type::Bool),
        (BuiltinMethodKind::ListFold, [init, f]) if init.ty == Type::Int => (f, 2, Type::Int),
        _ => return false,
    };
    matches!(receiver, Type::List(element) if **element == Type::Int)
        && native_callback_type(&callback.ty)
        && matches!(&callback.ty, Type::Function { params, ret, .. }
            if params.len() == arity && **ret == result)
}

pub fn native_string_method(kind: corvid_types::BuiltinMethodKind) -> bool {
    use corvid_types::BuiltinMethodKind::*;
    matches!(kind, StringLength | StringSubstring | StringToUpper | StringToLower | StringTrim
        | StringContains | StringStartsWith | StringEndsWith | StringSplit | StringReplace)
}

/// `int.to_string()`, `float.to_string()` and `bool.to_string()`: one
/// runtime call each, writing the interpreter's text
/// (`corvid_runtime::number_text`).
pub fn native_scalar_to_string(kind: corvid_types::BuiltinMethodKind) -> bool {
    use corvid_types::BuiltinMethodKind::*;
    matches!(kind, IntToString | FloatToString | BoolToString)
}

pub fn native_numeric_conversion(kind: corvid_types::BuiltinMethodKind) -> bool {
    use corvid_types::BuiltinMethodKind::*;
    matches!(kind, IntToFloat | FloatToIntTruncated | FloatFloor | FloatCeil | FloatRound)
}

/// Whether the unified ownership pass runs: on unless `CORVID_DUP_DROP_PASS`
/// is `0` or `false`, which selects the legacy ownership path. Lowering and
/// `corvid run`'s native cache key both read the setting here, so a binary
/// built under one setting is never reused under the other.
pub fn dup_drop_pass_enabled() -> bool {
    std::env::var("CORVID_DUP_DROP_PASS").map(|v| v != "0" && v != "false").unwrap_or(true)
}
