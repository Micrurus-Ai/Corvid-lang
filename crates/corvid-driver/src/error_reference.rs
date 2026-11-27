//! The error reference (`docs/reference/errors.md`), generated from the
//! diagnostic kinds themselves: each kind's declared code (its `code()`
//! match) and its doc comment. A drift test regenerates it and fails when
//! the committed file differs, so the reference cannot fall behind a new
//! or reworded error ([diagnostic codes](../../../docs/proposals/diagnostic-codes.md)
//! DC-3). Run the test with `CORVID_UPDATE_SNAPSHOTS=1` to rewrite it.

use std::collections::BTreeMap;

/// The diagnostic kind enums, read as source: (area label, enum name, file).
const KIND_SOURCES: &[(&str, &str, &str)] = &[
    ("lexer", "LexErrorKind", include_str!("../../corvid-syntax/src/errors.rs")),
    ("parser", "ParseErrorKind", include_str!("../../corvid-syntax/src/errors.rs")),
    ("names", "ResolveErrorKind", include_str!("../../corvid-resolve/src/errors.rs")),
    ("checker", "TypeErrorKind", include_str!("../../corvid-types/src/errors/error_kind.rs")),
    ("checker", "TypeWarningKind", include_str!("../../corvid-types/src/errors/warning_kind.rs")),
];

/// Diagnostics the driver builds itself, and startup refusals, which have
/// no kind enum to carry a doc comment. `every_written_code_is_described`
/// holds this table to the codes the source actually writes.
const OTHER_CODES: &[(&str, &str, &str)] = &[
    ("E0330", "ModuleLoadError::FileNotFound", "An imported module's file does not exist."),
    ("E0331", "ModuleLoadError::ReadError", "An imported module's file could not be read."),
    ("E0332", "ModuleLoadError::HashMismatch", "An imported module's content does not match its pinned `hash:sha256:`."),
    ("E0333", "ModuleLoadError::RemoteImportMissingHash", "A remote import has no `hash:sha256:` pin, so it is not fetched."),
    ("E0334", "ModuleLoadError::RemoteFetchError", "A remote import could not be fetched."),
    ("E0335", "ModuleLoadError::PackageLockMissing", "A package import, with no package lock file to resolve it."),
    ("E0336", "ModuleLoadError::PackageLockError", "The package lock file could not be read or is invalid."),
    ("E0337", "ModuleLoadError::PackageNotLocked", "A package import that the lock file does not list."),
    ("E0338", "ModuleLoadError::Cycle", "Modules import each other in a cycle."),
    ("E0650", "table schema", "Two modules of the program declare a table with the same name."),
    ("E0651", "table schema", "A table's indexes cannot be derived from its keys, unique rules and queries."),
    ("E1001", "native build", "The program cannot be built as native code yet; the message names what stops it."),
    ("E5204", "startup refusal", "`corvid serve` refuses to start: a declared contract (such as a route's `requires`) has no runtime path that enforces it."),
    ("E5205", "startup refusal", "`corvid serve` refuses to start: a connector's mode, host or credential is not allowed or not resolved."),
];

/// The areas codes are grouped by, by their first three characters.
const AREAS: &[(&str, &str)] = &[
    ("E00", "Lexer and parser"),
    ("E01", "Effects, approval and secrets"),
    ("E02", "Types"),
    ("E03", "Names and imports"),
    ("E04", "JSON and records"),
    ("E05", "Scalars: decimals, time, currency, bytes, text"),
    ("E06", "Tables and databases"),
    ("E07", "Actions, routes and the browser"),
    ("E08", "Connectors"),
    ("E09", "Streams, prompts, tools and agents"),
    ("E10", "Builds and targets"),
    ("E52", "Identity, access and startup"),
    ("W01", "Warnings: effects and replay"),
    ("W02", "Warnings: types"),
    ("W07", "Warnings: identity"),
    ("W09", "Warnings: streams and schedules"),
];

struct Entry {
    kind: String,
    meaning: String,
}

/// Every declared code: its kind and what it means.
fn entries() -> BTreeMap<String, Entry> {
    let mut entries = BTreeMap::new();
    for (_, enum_name, source) in KIND_SOURCES {
        let source = source.replace("\r\n", "\n");
        let docs = variant_docs(&source, enum_name);
        for (kind, code) in code_match(&source, enum_name) {
            let meaning = docs.get(&kind).cloned().unwrap_or_default();
            entries.insert(code, Entry { kind: format!("{enum_name}::{kind}"), meaning });
        }
    }
    for (code, kind, meaning) in OTHER_CODES {
        entries.insert(code.to_string(), Entry { kind: kind.to_string(), meaning: meaning.to_string() });
    }
    entries
}

/// The `Self::Kind { .. } => "CODE"` arms of `enum_name`'s `code()`.
fn code_match(source: &str, enum_name: &str) -> Vec<(String, String)> {
    let header = format!("impl {enum_name} {{");
    let body = source
        .match_indices(&header)
        .map(|(start, _)| {
            let block = &source[start..];
            &block[..block[1..].find("\nimpl ").map_or(block.len(), |end| end + 1)]
        })
        .find_map(|block| block.find("pub fn code(&self)").map(|at| &block[at..]))
        .unwrap_or_else(|| panic!("`{enum_name}::code()`"));
    let body = &body[..body.find("\n    }\n").expect("end of code()")];
    body.lines()
        .filter(|line| line.contains("=> \""))
        .map(|line| {
            let kind = line.split_whitespace().next().unwrap().trim_start_matches("Self::").to_string();
            (kind, line.split('"').nth(1).unwrap().to_string())
        })
        .collect()
}

/// Each variant of `enum_name` and its doc comment, joined into one line.
fn variant_docs(source: &str, enum_name: &str) -> BTreeMap<String, String> {
    let start = source.find(&format!("pub enum {enum_name}")).unwrap_or_else(|| panic!("`enum {enum_name}`"));
    let body = &source[start..];
    let body = &body[..body.find("\n}\n").expect("end of enum")];
    let mut docs = BTreeMap::new();
    let mut pending: Vec<&str> = Vec::new();
    let mut depth = 0usize;
    for line in body.lines().skip(1) {
        let trimmed = line.trim();
        if depth == 0 {
            if let Some(doc) = trimmed.strip_prefix("///") {
                pending.push(doc.trim());
                continue;
            }
            if trimmed.starts_with("#[") || trimmed.is_empty() {
                continue;
            }
            let name: String = trimmed.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if !name.is_empty() && name.chars().next().unwrap().is_ascii_uppercase() {
                docs.insert(name, pending.join(" ").trim().to_string());
            }
            pending.clear();
        }
        depth += line.matches('{').count();
        depth = depth.saturating_sub(line.matches('}').count());
    }
    docs
}

/// The reference, as `docs/reference/errors.md` holds it.
pub fn error_reference() -> String {
    let entries = entries();
    let mut out = String::from(
        "# Error codes\n\n\
         Every diagnostic Corvid prints carries a code, such as `[E0208] error: type mismatch`.\n\
         Each error declares its own code, so a code never changes when its message is reworded,\n\
         and no two errors share one. A code is never reused for a different error.\n\
         Codes are grouped by area. `E` marks an error and `W` a warning.\n\n\
         This page is generated from the compiler's diagnostic kinds by\n\
         `corvid-driver/src/error_reference.rs`, and a test fails when it falls behind.\n",
    );
    for (prefix, title) in AREAS {
        let in_area: Vec<_> = entries.iter().filter(|(code, _)| code.starts_with(prefix)).collect();
        if in_area.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {prefix}xx: {title}\n\n| Code | Error | Meaning |\n| --- | --- | --- |\n"));
        for (code, entry) in in_area {
            let meaning = entry.meaning.replace('|', "\\|");
            out.push_str(&format!("| `{code}` | `{}` | {meaning} |\n", entry.kind));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/errors.md")
    }

    /// The committed reference is what the kinds generate.
    #[test]
    fn the_error_reference_matches_the_diagnostic_kinds() {
        let generated = error_reference();
        if std::env::var("CORVID_UPDATE_SNAPSHOTS").is_ok() {
            std::fs::write(reference_path(), &generated).unwrap();
        }
        let committed = std::fs::read_to_string(reference_path()).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            committed == generated,
            "docs/reference/errors.md is out of date; rerun this test with CORVID_UPDATE_SNAPSHOTS=1"
        );
    }

    /// Every code is in an area the reference lists, and every kind says
    /// what it means.
    #[test]
    fn every_code_has_an_area_and_a_meaning() {
        for (code, entry) in entries() {
            assert!(AREAS.iter().any(|(prefix, _)| code.starts_with(prefix)), "{code} ({}) is in no area", entry.kind);
            assert!(!entry.meaning.is_empty(), "{code} ({}) has no doc comment to say what it means", entry.kind);
        }
    }

    /// Every code the driver and its startup checks write literally is in
    /// the table above, and the table names no code nothing writes.
    #[test]
    fn every_written_code_is_described() {
        let sources = [
            include_str!("pipeline/mod.rs"),
            include_str!("table_schema.rs"),
            include_str!("build/mod.rs"),
            include_str!("contract_closure.rs"),
            include_str!("connector_startup.rs"),
        ]
        .map(|source| source.replace("\r\n", "\n"));
        let mut written = std::collections::BTreeSet::new();
        for source in &sources {
            for piece in source.split("code: \"").skip(1) {
                written.insert(piece[..5].to_string());
            }
            for piece in source.split("\"E52").skip(1) {
                written.insert(format!("E52{}", &piece[..2]));
            }
        }
        let described: std::collections::BTreeSet<String> = OTHER_CODES.iter().map(|(code, ..)| code.to_string()).collect();
        assert_eq!(written, described);
    }
}
