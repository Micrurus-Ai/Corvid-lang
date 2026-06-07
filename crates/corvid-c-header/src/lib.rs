mod scalar_marshal;
mod template;

use corvid_ir::{IrAgent, IrExternAbi, IrFile, IrType};
use corvid_resolve::DefId;
use corvid_types::Type;
pub use scalar_marshal::ScalarAbiType;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct HeaderOptions {
    pub library_name: String,
}

#[derive(Debug, Clone)]
pub struct HeaderAgent {
    pub name: String,
    pub signature_comment: String,
    /// Slice 33Q8: when the agent has a struct parameter or return,
    /// this carries the JSON-schema block comments that document what
    /// shape the C caller must send / will receive. Empty for
    /// pre-33Q8 scalar-only signatures.
    pub json_schema_comments: String,
    pub return_c_type: &'static str,
    pub params_c: String,
    pub uses_grounded_handle: bool,
}

pub fn emit_header(ir: &IrFile, opts: &HeaderOptions) -> String {
    let types_by_id: HashMap<DefId, &IrType> =
        ir.types.iter().map(|t| (t.id, t)).collect();
    let agents = ir
        .agents
        .iter()
        .filter(|agent| matches!(agent.extern_abi, Some(IrExternAbi::C)))
        .map(|agent| exported_agent(agent, &types_by_id))
        .collect::<Vec<_>>();
    template::render_header(opts, &agents)
}

fn exported_agent(
    agent: &IrAgent,
    types_by_id: &HashMap<DefId, &IrType>,
) -> HeaderAgent {
    let mut schema_lines = String::new();

    let params_c = if agent.params.is_empty() {
        "void".to_string()
    } else {
        agent
            .params
            .iter()
            .map(|param| {
                let c_ty = param_c_type(&param.ty, types_by_id, &mut schema_lines, &param.name);
                format!("{c_ty} {}", param.name)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (return_c_type, is_grounded_return) =
        return_c_type(&agent.return_ty, types_by_id, &mut schema_lines);
    let mut params_c = params_c;
    if is_grounded_return {
        params_c = if params_c == "void" {
            "uint64_t* out_grounded_handle".to_string()
        } else {
            format!("{params_c}, uint64_t* out_grounded_handle")
        };
    }
    params_c = if params_c == "void" {
        "uint64_t* out_observation_handle".to_string()
    } else {
        format!("{params_c}, uint64_t* out_observation_handle")
    };
    let signature_comment = format!(
        "agent {}({}) -> {}",
        agent.name,
        agent
            .params
            .iter()
            .map(|param| format!("{}: {}", param.name, param.ty.display_name()))
            .collect::<Vec<_>>()
            .join(", "),
        agent.return_ty.display_name()
    );
    HeaderAgent {
        name: agent.name.clone(),
        signature_comment,
        json_schema_comments: schema_lines,
        return_c_type,
        params_c,
        uses_grounded_handle: is_grounded_return,
    }
}

/// Slice 33Q8 — render a struct's JSON Schema as a /* */ block
/// comment so a C caller can see exactly what shape to send / decode.
fn schema_comment(name: &str, role: &str, ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> String {
    let schema = corvid_prompt_format::schema_for(ty, types_by_id);
    let pretty =
        serde_json::to_string_pretty(&schema).unwrap_or_else(|_| schema.to_string());
    let indented = pretty
        .lines()
        .map(|l| format!("//   {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "// JSON shape for {role} `{name}`:\n{indented}\n"
    )
}

fn param_c_type(
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    schema_lines: &mut String,
    param_name: &str,
) -> &'static str {
    if matches!(ty, Type::Struct(_) | Type::ImportedStruct(_)) {
        schema_lines.push_str(&schema_comment(
            param_name,
            "parameter",
            ty,
            types_by_id,
        ));
        // Slice 33Q8: struct extern-c parameters cross the boundary
        // as caller-owned `const char*` JSON buffers. The cdylib's
        // generated wrapper decodes them on entry via the per-DefId
        // routine 20n-C already ships.
        "const char*"
    } else {
        ScalarAbiType::from_param_type(ty)
            .expect("extern-c checker guarantees param is scalar or struct")
            .c_param_type()
    }
}

fn return_c_type(
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    schema_lines: &mut String,
) -> (&'static str, bool) {
    if matches!(ty, Type::Struct(_) | Type::ImportedStruct(_)) {
        schema_lines.push_str(&schema_comment(
            "return",
            "return value",
            ty,
            types_by_id,
        ));
        // Slice 33Q8: struct extern-c returns leave the cdylib as
        // owned `const char*` JSON buffers. Free with the existing
        // `corvid_free_string(...)` runtime helper documented at the
        // top of this header.
        ("const char*", false)
    } else {
        let return_abi = ScalarAbiType::from_return_type(ty)
            .expect("extern-c checker guarantees return is scalar/grounded-scalar/struct");
        (return_abi.c_return_type(), return_abi.is_grounded_return())
    }
}
