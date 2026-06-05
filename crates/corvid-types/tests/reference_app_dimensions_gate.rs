//! Reference-app dimension drift gate — slice 33Q7a.
//!
//! Walks every `examples/backend/*/src/main.cor` and asserts each
//! distinct `trust:` / `data:` value falls into one of two documented
//! sets: the canonical spec values OR the reference-app extensions
//! cataloged at [`docs/internals/effect-spec/reference-app-dimensions.md`].
//!
//! Motivation: maintainer-as-reviewer-2026-06-05 P1.2 caught that the
//! shipped reference apps use trust values (`bounded`, `workspace`,
//! `grounded`, `local`, `readonly`) that aren't in the spec's stated
//! lattice (`autonomous < supervisor_required < human_required`). The
//! v1.0 typechecker accepts any string for `trust:`/`data:` via
//! `DimensionValue::Name(String)` without enforcing canonical
//! membership, so the apps compile silently. A reviewer reading the
//! spec is contradicted by the apps; a reviewer reading the apps is
//! contradicted by the spec. This soft gate catches drift without
//! rejecting the build — when someone adds a new value to a reference
//! app, the test fails and forces them to either (a) update the
//! catalog doc + this file's constants OR (b) rewrite the app to use
//! a documented value.
//!
//! The post-v1.0 slice 33Q7b promotes this to hard typechecker
//! enforcement: non-canonical values will require a `corvid.toml`
//! `[effect-system.dimensions.<name>]` declaration.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Trust values defined in the spec's core lattice at
/// `docs/internals/effect-spec/04-builtin-dimensions.md` § 4.2.
const SPEC_TRUST_VALUES: &[&str] = &[
    "autonomous",
    "supervisor_required",
    "human_required",
    // `autonomous_if_confident(<threshold>)` is parsed as
    // `DimensionValue::ConfidenceGated`, not Name — handled separately
    // by the grep below (the regex only matches Name values).
];

/// Data values defined in the spec's well-known categories at § 4.4.
const SPEC_DATA_VALUES: &[&str] = &[
    "none",
    "public",
    "pii",
    "financial",
    "medical",
    "grounded",
];

/// Reference-app extensions cataloged at
/// `docs/internals/effect-spec/reference-app-dimensions.md` § "Trust".
/// Annotation-only — these compose lexicographically (not by lattice)
/// and don't trigger approval-gate rules. When a new reference app
/// introduces a new trust value, ADD it here AND document it in the
/// catalog doc. The two must stay in sync; this test catches the
/// "added to app but not documented" direction.
const REFERENCE_APP_TRUST_EXTENSIONS: &[&str] = &[
    "bounded",
    "grounded",
    "local",
    "readonly",
    "workspace",
];

/// Reference-app extensions cataloged at the same doc § "Data".
const REFERENCE_APP_DATA_EXTENSIONS: &[&str] = &[
    "code",
    "customer",
    "external",
    "internal",
    "private",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Scan a Corvid source file for `trust:` / `data:` declarations and
/// return the set of distinct values used. Conservative: matches
/// `<dim>:` followed by whitespace + an identifier, stops at end of
/// identifier (`a-zA-Z0-9_`). Skips lines starting with `#` (comments)
/// and lines containing `autonomous_if_confident` (handled separately
/// because they parse as `ConfidenceGated`, not `Name`).
fn extract_dimension_values(source: &str, dimension: &str) -> BTreeSet<String> {
    let mut values = BTreeSet::new();
    let needle = format!("{dimension}:");
    for raw_line in source.lines() {
        let line = raw_line.trim_start();
        if line.starts_with('#') {
            continue;
        }
        // Skip ConfidenceGated occurrences — they're not Name values.
        if line.contains("autonomous_if_confident") {
            continue;
        }
        let Some(idx) = line.find(&needle) else {
            continue;
        };
        let after = &line[idx + needle.len()..];
        let trimmed = after.trim_start();
        let value: String = trimmed
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !value.is_empty() {
            values.insert(value);
        }
    }
    values
}

#[test]
fn reference_apps_use_only_documented_trust_values() {
    let allowed: BTreeSet<String> = SPEC_TRUST_VALUES
        .iter()
        .chain(REFERENCE_APP_TRUST_EXTENSIONS.iter())
        .map(|s| s.to_string())
        .collect();

    let examples_dir = repo_root().join("examples").join("backend");
    let mut undocumented: Vec<(String, String)> = Vec::new();
    let mut walked_any = false;

    for entry in std::fs::read_dir(&examples_dir).expect("read examples/backend") {
        let entry = entry.expect("entry");
        let src = entry.path().join("src").join("main.cor");
        if !src.is_file() {
            continue;
        }
        walked_any = true;
        let source = std::fs::read_to_string(&src).expect("read main.cor");
        let app_name = entry.file_name().to_string_lossy().to_string();
        for value in extract_dimension_values(&source, "trust") {
            if !allowed.contains(&value) {
                undocumented.push((app_name.clone(), value));
            }
        }
    }

    assert!(walked_any, "no reference apps walked — examples/backend missing");
    assert!(
        undocumented.is_empty(),
        "Reference apps use trust values not in the documented sets. \
         Either: (a) add the value to `REFERENCE_APP_TRUST_EXTENSIONS` \
         here AND to `docs/internals/effect-spec/reference-app-dimensions.md`, \
         OR (b) rewrite the reference app to use a documented value. \
         Drift detected: {undocumented:?}"
    );
}

#[test]
fn reference_apps_use_only_documented_data_values() {
    let allowed: BTreeSet<String> = SPEC_DATA_VALUES
        .iter()
        .chain(REFERENCE_APP_DATA_EXTENSIONS.iter())
        .map(|s| s.to_string())
        .collect();

    let examples_dir = repo_root().join("examples").join("backend");
    let mut undocumented: Vec<(String, String)> = Vec::new();

    for entry in std::fs::read_dir(&examples_dir).expect("read examples/backend") {
        let entry = entry.expect("entry");
        let src = entry.path().join("src").join("main.cor");
        if !src.is_file() {
            continue;
        }
        let source = std::fs::read_to_string(&src).expect("read main.cor");
        let app_name = entry.file_name().to_string_lossy().to_string();
        for value in extract_dimension_values(&source, "data") {
            if !allowed.contains(&value) {
                undocumented.push((app_name.clone(), value));
            }
        }
    }

    assert!(
        undocumented.is_empty(),
        "Reference apps use data values not in the documented sets. \
         Either: (a) add the value to `REFERENCE_APP_DATA_EXTENSIONS` \
         here AND to `docs/internals/effect-spec/reference-app-dimensions.md`, \
         OR (b) rewrite the reference app to use a documented value. \
         Drift detected: {undocumented:?}"
    );
}

/// Adversarial guard: when the catalog lists an EXTENSION value, at
/// least one reference app should actually use it. Otherwise the
/// catalog is overclaiming. This catches the reverse direction (value
/// listed in catalog but app removed it).
#[test]
fn every_listed_extension_value_is_used_by_at_least_one_reference_app() {
    let examples_dir = repo_root().join("examples").join("backend");

    let mut trust_used: BTreeSet<String> = BTreeSet::new();
    let mut data_used: BTreeSet<String> = BTreeSet::new();
    for entry in std::fs::read_dir(&examples_dir).expect("read examples/backend") {
        let entry = entry.expect("entry");
        let src = entry.path().join("src").join("main.cor");
        if !src.is_file() {
            continue;
        }
        let source = std::fs::read_to_string(&src).expect("read main.cor");
        trust_used.extend(extract_dimension_values(&source, "trust"));
        data_used.extend(extract_dimension_values(&source, "data"));
    }

    let unused_trust: Vec<&str> = REFERENCE_APP_TRUST_EXTENSIONS
        .iter()
        .copied()
        .filter(|v| !trust_used.contains(*v))
        .collect();
    let unused_data: Vec<&str> = REFERENCE_APP_DATA_EXTENSIONS
        .iter()
        .copied()
        .filter(|v| !data_used.contains(*v))
        .collect();

    assert!(
        unused_trust.is_empty() && unused_data.is_empty(),
        "Catalog lists extension values that no reference app uses anymore. \
         Either: (a) remove the value from the constants here AND the catalog \
         doc, OR (b) restore a reference app's use of it. Unused trust: \
         {unused_trust:?}; unused data: {unused_data:?}"
    );
}
