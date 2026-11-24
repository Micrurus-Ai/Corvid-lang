//! Python models from the Application Contract.

use crate::app_contract::ApplicationContract;
use super::{NOTE_SCAFFOLD, split_generic};

pub(super) fn emit_python(contract: &ApplicationContract) -> String {
    let mut out = String::from("\"\"\"");
    out.push_str(NOTE_SCAFFOLD);
    out.push_str("\"\"\"\nfrom __future__ import annotations\nfrom dataclasses import dataclass\nfrom typing import Optional, Union\n\n");
    for ty in &contract.types {
        if ty.variants.is_empty() {
            out.push_str(&format!("@dataclass\nclass {}:\n", ty.name));
            if ty.fields.is_empty() {
                out.push_str("    pass\n\n");
            } else {
                for f in &ty.fields {
                    out.push_str(&format!("    {}: {}\n", f.name, python_type(&f.type_name)));
                }
                out.push('\n');
            }
        } else {
            for v in &ty.variants {
                out.push_str(&format!("@dataclass\nclass {}:\n", v.name));
                out.push_str(&format!("    tag: str = \"{}\"\n", v.name));
                for f in &v.fields {
                    out.push_str(&format!("    {}: {} = None  # type: ignore\n", f.name, python_type(&f.type_name)));
                }
                out.push('\n');
            }
            let names = ty.variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", ");
            out.push_str(&format!("{} = Union[{}]\n\n", ty.name, names));
        }
    }
    out
}

fn python_type(name: &str) -> String {
    match name {
        "Int" => return "int".into(),
        "Float" => return "float".into(),
        "String" | "TraceId" => return "str".into(),
        "Bool" => return "bool".into(),
        "Nothing" => return "None".into(),
        _ => {}
    }
    if let Some((head, inner)) = split_generic(name) {
        return match head {
            "List" => format!("list[{}]", python_type(inner)),
            "Option" => format!("Optional[{}]", python_type(inner)),
            "Stream" | "Tainted" | "Grounded" => python_type(inner),
            "Page" => format!("Page[{}]", python_type(inner)),
            "Upload" => "bytes".into(),
            "Map" => {
                let (_k, v) = inner.split_once(',').unwrap_or(("str", inner));
                format!("dict[str, {}]", python_type(v.trim()))
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
    fn python_models_map_records_and_unions() {
        let s = emit_python(&contract_for(SRC));
        assert!(s.contains("@dataclass\nclass Answer:"));
        assert!(s.contains("    text: str"));
        assert!(s.contains("    score: int"));
        assert!(s.contains("class PaymentNotFound:"));
        assert!(s.contains("RefundError = Union[PaymentNotFound, ApprovalDenied]"));
    }
}
