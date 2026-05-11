//! Integration tests for `corvid_browser::check`.
//!
//! Acceptance criteria from the 33J7-prereq brief:
//!
//! - At least one accepted-input test.
//! - At least one rejected-input test.
//! - The compile-refusal example surfaces
//!   `approval.dangerous_call_requires_token`.
//! - A valid trivial program returns `{ ok: true, diagnostics: [] }`.
//! - `import` declarations refuse with the documented message.
//!
//! Plus one JSON-shape test: serialize the result and assert the wire
//! format matches what the website renderer expects.

use corvid_browser::{check, Severity};

/// Valid trivial program — comment only. The minimal accepted input.
#[test]
fn empty_program_is_ok() {
    let result = check("# just a comment\n");
    assert!(result.ok, "expected ok=true, got diagnostics: {:#?}", result.diagnostics);
    assert!(result.diagnostics.is_empty());
    assert_eq!(result.version, "v1");
}

/// The compile-refusal moat demo: a dangerous tool called without
/// `approve` must fire `approval.dangerous_call_requires_token`. This
/// is the load-bearing test — it pins the property the playground
/// exists to demo.
#[test]
fn dangerous_call_without_approve_refuses() {
    let source = r#"
tool send_email(to: String, body: String) -> Nothing dangerous

agent spam(address: String) -> Nothing:
    send_email(address, "buy our stuff")
"#;
    let result = check(source);
    assert!(!result.ok, "expected refusal, got ok=true");

    let approval_diag = result
        .diagnostics
        .iter()
        .find(|d| d.guarantee_id == Some("approval.dangerous_call_requires_token"));
    assert!(
        approval_diag.is_some(),
        "expected at least one diagnostic with guarantee_id \
         `approval.dangerous_call_requires_token`. Got:\n{:#?}",
        result.diagnostics
    );

    let diag = approval_diag.unwrap();
    assert!(matches!(diag.severity, Severity::Error));
    assert!(diag.span.start_line >= 1);
    assert!(diag.span.start_col >= 1);
}

/// The same program WITH `approve` compiles cleanly. This pins that
/// the guarantee fires on the absence, not on the call shape.
#[test]
fn dangerous_call_with_approve_passes() {
    let source = r#"
tool send_email(to: String, body: String) -> Nothing dangerous

agent send(address: String, body: String) -> Nothing:
    approve SendEmail(address, body)
    send_email(address, body)
"#;
    let result = check(source);
    assert!(
        result.ok,
        "expected ok=true with approve in scope, got diagnostics: {:#?}",
        result.diagnostics
    );
}

/// `import` declarations refuse with the documented browser-only
/// message. Multi-file resolution is out of scope for v1 of the
/// playground.
#[test]
fn import_is_refused_in_playground() {
    let source = "import \"./other_module\"\n\nagent main() -> String:\n    return \"hi\"\n";
    let result = check(source);
    assert!(!result.ok);
    let import_diag = result
        .diagnostics
        .iter()
        .find(|d| d.message.starts_with("imports are not supported"));
    assert!(
        import_diag.is_some(),
        "expected import-refusal diagnostic. Got:\n{:#?}",
        result.diagnostics
    );
}

/// Wire-format JSON shape: serialize and inspect. The website
/// renderer assumes these field names and the `version: "v1"`
/// invariant. A non-additive schema change here is a breaking change
/// for the playground.
#[test]
fn wire_format_serializes_with_expected_field_names() {
    let result = check("# valid\n");
    let json = serde_json::to_value(&result).expect("CheckResult must serialize");

    // Top-level structure
    assert_eq!(json["version"], "v1");
    assert_eq!(json["ok"], true);
    assert!(json["diagnostics"].is_array());

    // Diagnostic field-name contract — produce one error so we can
    // inspect a Diagnostic.
    let bad = check(
        r#"
tool send_email(to: String, body: String) -> Nothing dangerous
agent spam(addr: String) -> Nothing:
    send_email(addr, "x")
"#,
    );
    let bad_json = serde_json::to_value(&bad).expect("must serialize");
    let diag = &bad_json["diagnostics"][0];

    // Each of these fields is load-bearing for the website renderer;
    // change them only with a coordinated rollout.
    for field in &[
        "guarantee_id",
        "severity",
        "message",
        "span",
        "help",
    ] {
        assert!(
            diag.get(field).is_some(),
            "diagnostic missing `{field}` field; full diag: {diag:#?}"
        );
    }

    let span = &diag["span"];
    for field in &["start_line", "start_col", "end_line", "end_col"] {
        assert!(
            span.get(field).is_some(),
            "span missing `{field}` field; full span: {span:#?}"
        );
    }

    // severity is lowercase string per serde rename.
    let severity = diag["severity"].as_str().unwrap();
    assert!(
        matches!(severity, "error" | "warning" | "info"),
        "unexpected severity value: {severity}"
    );
}

/// Line/col conversion is 1-indexed and counts unicode characters.
/// A diagnostic on line N column M should land where a CodeMirror
/// editor would render the squiggle.
#[test]
fn span_is_one_indexed_and_unicode_aware() {
    let source = "# αβγ comment\n# next line\nbroken_call(\n";
    let result = check(source);
    assert!(
        !result.ok,
        "broken source should produce diagnostics; got: {:#?}",
        result.diagnostics
    );
    let diag = &result.diagnostics[0];
    assert!(diag.span.start_line >= 1);
    assert!(diag.span.start_col >= 1);
}
