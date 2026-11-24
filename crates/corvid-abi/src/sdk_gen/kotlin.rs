//! Kotlin models from the Application Contract.

use crate::app_contract::ApplicationContract;
use super::{NOTE_SCAFFOLD, split_generic};

pub(super) fn emit_kotlin(contract: &ApplicationContract) -> String {
    let mut out = format!("// {}\n\n", NOTE_SCAFFOLD.replace('\n', "\n// "));
    for ty in &contract.types {
        if ty.variants.is_empty() {
            let fields = ty
                .fields
                .iter()
                .map(|f| format!("val {}: {}", f.name, kotlin_type(&f.type_name)))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("data class {}({})\n\n", ty.name, fields));
        } else {
            out.push_str(&format!("sealed interface {} {{\n", ty.name));
            for v in &ty.variants {
                if v.fields.is_empty() {
                    out.push_str(&format!("    data object {} : {}\n", v.name, ty.name));
                } else {
                    let fields = v
                        .fields
                        .iter()
                        .map(|f| format!("val {}: {}", f.name, kotlin_type(&f.type_name)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("    data class {}({}) : {}\n", v.name, fields, ty.name));
                }
            }
            out.push_str("}\n\n");
        }
    }
    out
}

fn kotlin_type(name: &str) -> String {
    match name {
        "Int" => return "Int".into(),
        "Float" => return "Double".into(),
        "String" | "TraceId" => return "String".into(),
        "Bool" => return "Boolean".into(),
        "Nothing" => return "Unit".into(),
        _ => {}
    }
    if let Some((head, inner)) = split_generic(name) {
        return match head {
            "List" => format!("List<{}>", kotlin_type(inner)),
            "Option" => format!("{}?", kotlin_type(inner)),
            "Stream" | "Tainted" | "Grounded" => kotlin_type(inner),
            "Page" => format!("Page<{}>", kotlin_type(inner)),
            "Upload" => "ByteArray".into(),
            "Map" => {
                let (_k, v) = inner.split_once(',').unwrap_or(("String", inner));
                format!("Map<String, {}>", kotlin_type(v.trim()))
            }
            _ => name.to_string(),
        };
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk_gen::tests::{contract_for, SRC};

    #[test]
    fn kotlin_models_map_records_and_sealed() {
        let s = emit_kotlin(&contract_for(SRC));
        assert!(s.contains("data class Answer(val text: String, val score: Int)"));
        assert!(s.contains("sealed interface RefundError {"));
        assert!(s.contains("data object PaymentNotFound : RefundError"));
        assert!(s.contains("data class ApprovalDenied(val reason: String) : RefundError"));
    }
}
