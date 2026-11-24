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

use crate::app_contract::ApplicationContract;
use crate::ts_client::GeneratedFile;

mod type_name;

mod swift;
use swift::emit_swift;

mod kotlin;
use kotlin::emit_kotlin;

mod python;
use python::emit_python;

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

// ----------------------------- shared -----------------------------

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app_contract::{effect_decls_of, emit_application_contract, ContractOptions};
    use corvid_types::effects::EffectRegistry;

    pub(crate) fn contract_for(src: &str) -> ApplicationContract {
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

    pub(crate) const SRC: &str = "public type Answer:
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
