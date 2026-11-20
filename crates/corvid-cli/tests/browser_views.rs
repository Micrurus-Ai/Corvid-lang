use std::{fs, process::Command};

#[test]
fn browser_views_escape_content_and_require_typed_children() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("ui.cor"),
        include_str!("../../../std/ui.cor"),
    )
    .unwrap();
    let main = project.path().join("main.cor");
    let source = r#"
import "./ui" as ui

test escapes_fields:
    field = ui.text_field("<Name>", "a\"b", "<x>&")
    assert field.html == "<label>&lt;Name&gt;<input type=\"text\" name=\"a&quot;b\" value=\"&lt;x&gt;&amp;\"></label>"

test preserves_child_order:
    body = ui.disclosure("More", [ui.text("first"), ui.text("second")])
    assert body.html == "<details><summary>More</summary><p>first</p><p>second</p></details>"
"#;
    fs::write(&main, source).unwrap();
    let run = |cmd| {
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(project.path())
            .args([cmd, "main.cor"])
            .output()
            .unwrap()
    };
    let good = run("test");
    assert!(
        good.status.success() && String::from_utf8_lossy(&good.stdout).contains("2 passed"),
        "{good:?}"
    );
    fs::write(main, source.replace("ui.text(\"second\")", "\"second\"")).unwrap();
    let bad = run("check");
    assert!(!bad.status.success(), "{bad:?}");
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("produce a `View`"),
        "{bad:?}"
    );
}
