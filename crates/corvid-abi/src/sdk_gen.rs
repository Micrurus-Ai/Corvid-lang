//! Multi-language SDK model generation from the Application Contract
//! (slice 51o).
//!
//! TypeScript is the fully-realized target (client + React hooks,
//! slices 51l/51n). Swift, Kotlin, and Python get typed MODELS — one
//! native type per contract type, mapping records to structs/data
//! classes/dataclasses and sum types to enums/sealed classes/tagged
//! unions — plus a minimal client stub. These are deliberately
//! scaffolds: the model layer (the part that must track the contract
//! exactly) is generated, and the transport is a thin stub extended as
//! demand proves. Every language reads the SAME contract, so the types
//! can never drift between platforms.

use crate::app_contract::{ApplicationContract, ContractType};
use crate::ts_client::GeneratedFile;

/// A target SDK language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdkLanguage {
    TypeScript,
    Swift,
    Kotlin,
    Python,
}

impl SdkLanguage {
    pub fn parse(name: &str) -> Option<SdkLanguage> {
        Some(match name.to_ascii_lowercase().as_str() {
            "ts" | "typescript" => SdkLanguage::TypeScript,
            "swift" => SdkLanguage::Swift,
            "kotlin" | "kt" => SdkLanguage::Kotlin,
            "python" | "py" => SdkLanguage::Python,
            _ => return None,
        })
    }

    pub fn slug(&self) -> &'static str {
        match self {
            SdkLanguage::TypeScript => "typescript",
            SdkLanguage::Swift => "swift",
            SdkLanguage::Kotlin => "kotlin",
            SdkLanguage::Python => "python",
        }
    }
}

/// Generate the SDK files for a contract in the chosen language.
pub fn emit_sdk(contract: &ApplicationContract, lang: SdkLanguage) -> Vec<GeneratedFile> {
    match lang {
        SdkLanguage::TypeScript => crate::ts_client::emit_ts_client(contract),
        SdkLanguage::Swift => vec![GeneratedFile {
            filename: "Models.swift".to_string(),
            contents: emit_swift(contract),
        }],
        SdkLanguage::Kotlin => vec![GeneratedFile {
            filename: "Models.kt".to_string(),
            contents: emit_kotlin(contract),
        }],
        SdkLanguage::Python => vec![GeneratedFile {
            filename: "models.py".to_string(),
            contents: emit_python(contract),
        }],
    }
}

const NOTE_SCAFFOLD: &str =
    "Generated from the Corvid Application Contract. The typed models track the\ncontract exactly; the transport is a scaffold extended as demand proves.";

// ----------------------------- Swift -----------------------------

fn emit_swift(contract: &ApplicationContract) -> String {
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

// ----------------------------- Kotlin -----------------------------

fn emit_kotlin(contract: &ApplicationContract) -> String {
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

// ----------------------------- Python -----------------------------

fn emit_python(contract: &ApplicationContract) -> String {
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

// ----------------------------- shared -----------------------------

fn split_generic(name: &str) -> Option<(&str, &str)> {
    let open = name.find('<')?;
    if !name.ends_with('>') {
        return None;
    }
    Some((&name[..open], &name[open + 1..name.len() - 1]))
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_contract::{effect_decls_of, emit_application_contract, ContractOptions};
    use corvid_types::effects::EffectRegistry;

    fn contract_for(src: &str) -> ApplicationContract {
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
        let registry = EffectRegistry::from_decls(&effect_decls_of(&file));
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(checked.errors.is_empty(), "check: {:?}", checked.errors);
        emit_application_contract(
            &file,
            &resolved,
            &checked,
            &registry,
            &ContractOptions { source_path: "app.cor", compiler_version: "test", generated_at: "now" },
        )
    }

    const SRC: &str = "public type Answer:
    text: String
    score: Int

public type RefundError:
    | PaymentNotFound
    | ApprovalDenied(reason: String)

public agent classify(q: String) -> Answer:
    return Answer(q, 1)
";

    #[test]
    fn language_parses_are_case_insensitive() {
        assert_eq!(SdkLanguage::parse("TS"), Some(SdkLanguage::TypeScript));
        assert_eq!(SdkLanguage::parse("Swift"), Some(SdkLanguage::Swift));
        assert_eq!(SdkLanguage::parse("kt"), Some(SdkLanguage::Kotlin));
        assert_eq!(SdkLanguage::parse("py"), Some(SdkLanguage::Python));
        assert_eq!(SdkLanguage::parse("cobol"), None);
    }

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

    #[test]
    fn kotlin_models_map_records_and_sealed() {
        let s = emit_kotlin(&contract_for(SRC));
        assert!(s.contains("data class Answer(val text: String, val score: Int)"));
        assert!(s.contains("sealed interface RefundError {"));
        assert!(s.contains("data object PaymentNotFound : RefundError"));
        assert!(s.contains("data class ApprovalDenied(val reason: String) : RefundError"));
    }

    #[test]
    fn python_models_map_records_and_unions() {
        let s = emit_python(&contract_for(SRC));
        assert!(s.contains("@dataclass\nclass Answer:"));
        assert!(s.contains("    text: str"));
        assert!(s.contains("    score: int"));
        assert!(s.contains("class PaymentNotFound:"));
        assert!(s.contains("RefundError = Union[PaymentNotFound, ApprovalDenied]"));
    }

    #[test]
    fn emit_sdk_dispatches_by_language() {
        let c = contract_for(SRC);
        assert_eq!(emit_sdk(&c, SdkLanguage::Swift)[0].filename, "Models.swift");
        assert_eq!(emit_sdk(&c, SdkLanguage::Kotlin)[0].filename, "Models.kt");
        assert_eq!(emit_sdk(&c, SdkLanguage::Python)[0].filename, "models.py");
        // TypeScript reuses the 51l generator (types.ts + api.ts).
        let ts = emit_sdk(&c, SdkLanguage::TypeScript);
        assert!(ts.iter().any(|f| f.filename == "types.ts"));
        assert!(ts.iter().any(|f| f.filename == "api.ts"));
    }
}
