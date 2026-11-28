//! Large uploads held on disk (U1): what the checker accepts in
//! `@upload(..., disk_over_mb: N)`, and that the threshold stays server
//! policy. The server half is `crates/corvid-cli/tests/serve_uploads_on_disk.rs`.

use corvid_driver::{compile_to_application_contract_with_config, compile_to_ir};

const PROGRAM: &str = r#"type Receipt:
    size: Int

server files:
    @upload(max_mb: 500, mime: "video/mp4", disk_over_mb: 16)
    route POST "/videos" body Upload<Video> -> json Receipt:
        return Receipt(body.size())
"#;

fn errors(source: &str) -> Vec<String> {
    match compile_to_ir(source) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

#[test]
fn a_threshold_below_the_maximum_compiles_and_is_never_published() {
    assert!(errors(PROGRAM).is_empty(), "{:#?}", errors(PROGRAM));
    let contract = compile_to_application_contract_with_config(PROGRAM, "app.cor", "test", None).unwrap();
    let published = serde_json::to_string(&contract).unwrap();
    assert!(published.contains("524288000"), "the maximum is published for the picker: {published}");
    assert!(!published.contains("disk_over") && !published.contains("16777216"), "the disk threshold is server policy: {published}");
}

#[test]
fn a_threshold_that_could_never_apply_is_refused() {
    for threshold in ["500", "600"] {
        let source = PROGRAM.replace("disk_over_mb: 16", &format!("disk_over_mb: {threshold}"));
        assert!(
            errors(&source).iter().any(|e| e.contains("nothing would be held on disk")),
            "{threshold} MB: {:#?}",
            errors(&source)
        );
    }
}

#[test]
fn a_fields_upload_cannot_be_held_on_disk_yet() {
    let source = "type Submission:\n    @upload(max_mb: 5, disk_over_mb: 1)\n    file: Upload<Pdf>\n";
    assert!(errors(source).iter().any(|e| e.contains("field's upload stays in memory")), "{:#?}", errors(source));
}
