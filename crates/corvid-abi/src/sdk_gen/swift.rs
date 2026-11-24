//! Swift models from the Application Contract.

use crate::app_contract::ApplicationContract;
use super::{NOTE_SCAFFOLD, lower_first, split_generic};

pub(super) fn emit_swift(contract: &ApplicationContract) -> String {
    let mut out = format!("// {}\n\nimport Foundation\n\n", NOTE_SCAFFOLD.replace('\n', "\n// "));
    for ty in &contract.types {
        if ty.variants.is_empty() {
            out.push_str(&format!("public struct {}: Codable {{\n", ty.name));
            for f in &ty.fields {
                out.push_str(&format!("    public var {}: {}\n", f.name, swift_type(&f.type_name)));
            }
            out.push_str("}\n\n");
        } else {
            out.push_str(&format!("public enum {}: Codable {{\n", ty.name));
            for v in &ty.variants {
                if v.fields.is_empty() {
                    out.push_str(&format!("    case {}\n", lower_first(&v.name)));
                } else {
                    let params = v
                        .fields
                        .iter()
                        .map(|f| format!("{}: {}", f.name, swift_type(&f.type_name)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("    case {}({})\n", lower_first(&v.name), params));
                }
            }
            out.push_str("}\n\n");
        }
    }
    out.push_str(&format!("// {} public agent(s), {} route(s).\n", contract.agents.len(), contract.routes.len()));
    out
}

fn swift_type(name: &str) -> String {
    match name {
        "Int" => return "Int".into(),
        "Float" => return "Double".into(),
        "String" | "TraceId" => return "String".into(),
        "Bool" => return "Bool".into(),
        "Nothing" => return "Void".into(),
        _ => {}
    }
    if let Some((head, inner)) = split_generic(name) {
        return match head {
            "List" => format!("[{}]", swift_type(inner)),
            "Option" => format!("{}?", swift_type(inner)),
            "Stream" | "Tainted" | "Grounded" => swift_type(inner),
            "Page" => format!("Page<{}>", swift_type(inner)),
            "Upload" => "Data".into(),
            "Map" => {
                let (_k, v) = inner.split_once(',').unwrap_or(("String", inner));
                format!("[String: {}]", swift_type(v.trim()))
            }
            "Result" => {
                let (ok, err) = inner.split_once(',').unwrap_or((inner, "String"));
                format!("Result<{}, {}Error>", swift_type(ok.trim()), swift_type(err.trim()))
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
    fn swift_models_map_records_and_enums() {
        let s = emit_swift(&contract_for(SRC));
        assert!(s.contains("public struct Answer: Codable {"));
        assert!(s.contains("public var text: String"));
        assert!(s.contains("public var score: Int"));
        assert!(s.contains("public enum RefundError: Codable {"));
        assert!(s.contains("case paymentNotFound"));
        assert!(s.contains("case approvalDenied(reason: String)"));
    }
}
