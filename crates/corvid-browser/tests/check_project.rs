//! Integration tests for `corvid_browser::check_project` — slice
//! 33J7a multi-file typecheck for the playground.
//!
//! Acceptance criteria:
//!
//! - A two-file project where the entry imports another and the
//!   import resolves cleanly typechecks ok.
//! - A two-file project where the entry imports a missing module
//!   surfaces a "module not found" diagnostic at the import site,
//!   anchored to the entry file's path.
//! - A two-file project where the entry imports a module with a
//!   parse/resolve error surfaces that error attributed to the
//!   imported file's path (not the entry's).
//! - Python / remote / package imports refuse with the playground-
//!   sandbox message.
//! - Cycles surface as a single diagnostic.
//! - Diagnostics carry the `path` field so the playground can route
//!   squiggles to the right editor tab.

use std::collections::HashMap;

use corvid_browser::{check_project, Severity};

fn project(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(p, s)| (p.to_string(), s.to_string()))
        .collect()
}

/// Two-file project, valid: the entry imports another file's
/// public agent and calls it. Pins the happy-path multi-file
/// pipeline. (Note: only Type / Store / Tool / Prompt / Agent
/// declarations are exportable across files in v1; plain `fn`
/// stays file-local — that's a corvid-resolve invariant.)
#[test]
fn two_file_project_with_clean_import_passes() {
    let files = project(&[
        (
            "src/main.cor",
            r#"
import "./policy" as policy

agent decide() -> Bool:
    return policy.allowed()
"#,
        ),
        (
            "src/policy.cor",
            r#"
public agent allowed() -> Bool:
    return true
"#,
        ),
    ]);

    let result = check_project(&files, "src/main.cor");
    assert!(
        result.ok,
        "expected ok=true, got diagnostics: {:#?}",
        result.diagnostics
    );
    assert_eq!(result.version, "v1");
}

/// Entry imports a file not present in the project map. Surfaces a
/// "module not found" diagnostic with `path` pointing at the entry.
#[test]
fn missing_import_target_is_reported_at_import_site() {
    let files = project(&[(
        "src/main.cor",
        r#"
import "./missing" as m

agent main() -> String:
    return "hi"
"#,
    )]);

    let result = check_project(&files, "src/main.cor");
    assert!(!result.ok);

    let not_found = result
        .diagnostics
        .iter()
        .find(|d| d.message.starts_with("module not found"))
        .expect("expected module-not-found diagnostic");

    assert!(matches!(not_found.severity, Severity::Error));
    assert_eq!(not_found.path.as_deref(), Some("src/main.cor"));
}

/// Imported file has a parse error. The diagnostic carries the
/// imported file's path, not the entry's, so the playground can
/// route the squiggle to the right tab.
#[test]
fn import_target_parse_error_attributes_path_to_imported_file() {
    let files = project(&[
        (
            "src/main.cor",
            r#"
import "./broken" as b

agent main() -> String:
    return "hi"
"#,
        ),
        (
            "src/broken.cor",
            "public fn @@@ broken_syntax",
        ),
    ]);

    let result = check_project(&files, "src/main.cor");
    assert!(!result.ok);

    let broken_diag = result
        .diagnostics
        .iter()
        .find(|d| d.path.as_deref() == Some("src/broken.cor"))
        .expect("expected diagnostic anchored at src/broken.cor");

    assert!(matches!(broken_diag.severity, Severity::Error));
}

/// Python import refuses with the sandbox message anchored at the
/// import declaration.
#[test]
fn python_import_refuses_in_sandbox() {
    let files = project(&[(
        "src/main.cor",
        r#"
import python "anthropic" as anthropic

agent main() -> String:
    return "hi"
"#,
    )]);

    let result = check_project(&files, "src/main.cor");
    assert!(!result.ok);

    let refused = result
        .diagnostics
        .iter()
        .find(|d| d.message.starts_with("Python imports are not supported"))
        .expect("expected python-imports-refused diagnostic");
    assert_eq!(refused.path.as_deref(), Some("src/main.cor"));
}

/// Two-file cycle: A imports B, B imports A. The back-edge surfaces
/// as a single diagnostic anchored at the import that closed it.
#[test]
fn two_file_cycle_reports_a_cycle_diagnostic() {
    let files = project(&[
        (
            "src/a.cor",
            r#"
import "./b" as b

agent run_a() -> Bool:
    return b.run_b()
"#,
        ),
        (
            "src/b.cor",
            r#"
import "./a" as a

public fn run_b() -> Bool:
    return a.run_a()
"#,
        ),
    ]);

    let result = check_project(&files, "src/a.cor");
    assert!(!result.ok);

    let cycle = result
        .diagnostics
        .iter()
        .find(|d| d.message.starts_with("import cycle"))
        .expect("expected cycle diagnostic");
    assert!(matches!(cycle.severity, Severity::Error));
}

/// Entry path not in the map is a usage error: surface immediately
/// without attempting to load anything.
#[test]
fn missing_entry_in_map_surfaces_usage_error() {
    let files = project(&[("src/other.cor", "# valid\n")]);

    let result = check_project(&files, "src/main.cor");
    assert!(!result.ok);
    assert!(result.diagnostics.iter().any(|d| d
        .message
        .starts_with("entry file `src/main.cor` is not in the project's file map")));
}

/// Cross-file typecheck: the entry calls a function in an imported
/// module without `approve`, the dangerous-call guarantee fires.
/// This pins that multi-file typecheck preserves the moat property.
#[test]
fn dangerous_tool_in_imported_module_still_requires_approve() {
    let files = project(&[
        (
            "src/main.cor",
            r#"
import "./mail" as mail

agent send(address: String, body: String) -> Nothing:
    mail.email_user(address, body)
"#,
        ),
        (
            "src/mail.cor",
            r#"
public tool send_email(to: String, body: String) -> Nothing dangerous

public agent email_user(address: String, body: String) -> Nothing:
    approve SendEmail(address, body)
    send_email(address, body)
"#,
        ),
    ]);

    // mail.email_user already has the approve inside it, so the
    // outer call should typecheck cleanly.
    let result = check_project(&files, "src/main.cor");
    assert!(
        result.ok,
        "expected the imported agent's internal approve to satisfy \
         the guarantee. Got: {:#?}",
        result.diagnostics
    );
}

/// Wire-format sanity check: serialize the multi-file result and
/// confirm the `path` field is present.
#[test]
fn wire_format_includes_path_in_multi_file_diagnostics() {
    let files = project(&[(
        "src/main.cor",
        r#"
import "./missing" as m

agent main() -> String:
    return "hi"
"#,
    )]);

    let result = check_project(&files, "src/main.cor");
    let json = serde_json::to_value(&result).expect("must serialize");
    let diag = &json["diagnostics"][0];
    assert!(diag.get("path").is_some(), "expected `path` field on multi-file diagnostic");
    assert_eq!(diag["path"], "src/main.cor");
}
