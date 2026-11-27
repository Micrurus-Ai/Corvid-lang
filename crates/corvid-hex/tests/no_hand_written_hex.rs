//! Every crate writes hex through `corvid_hex`, and percent-encodes a URL
//! component through `corvid_text::encode_url_component`. This test fails when a new
//! hand-written encoder or decoder appears in a crate's source, so the
//! copies that drifted before cannot come back one at a time.

use std::path::{Path, PathBuf};

/// Text that only a hand-written hex encoder or decoder contains.
const HAND_WRITTEN: &[&str] = &[":02x}", ":02X}", "b\"0123456789abcdef\"", "hex::encode(", "hex::decode("];

/// Files that write these as text for another program, not as code.
const EMITTED_CODE: &[&str] = &[
    // A Rust source template written into a user's generated bindings.
    "crates/corvid-bind/src/rust_backend/common_template.rs",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Whether `pattern` occurs not as the tail of a longer name, so
/// `corvid_hex::encode(` is not taken for the `hex` crate's `hex::encode(`.
fn occurs_on_its_own(text: &str, pattern: &str) -> bool {
    if !pattern.starts_with("hex::") {
        return text.contains(pattern);
    }
    text.match_indices(pattern).any(|(at, _)| {
        !text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

#[test]
fn no_crate_writes_hex_by_hand() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut offenders = Vec::new();
    for krate in std::fs::read_dir(workspace.join("crates")).unwrap().flatten() {
        let name = krate.file_name().to_string_lossy().to_string();
        if name == "corvid-hex" {
            continue;
        }
        let mut files = Vec::new();
        rust_files(&krate.path().join("src"), &mut files);
        for file in files {
            let relative = file.strip_prefix(&workspace).unwrap().to_string_lossy().replace('\\', "/");
            if EMITTED_CODE.contains(&relative.as_str()) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap().replace("\r\n", "\n");
            for pattern in HAND_WRITTEN {
                if occurs_on_its_own(&text, pattern) {
                    offenders.push(format!("{relative}: `{pattern}`"));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "hand-written hex; call corvid_hex::encode, encode_prefix or decode, or for a URL corvid_text::encode_url_component, instead:\n{}",
        offenders.join("\n")
    );
}
