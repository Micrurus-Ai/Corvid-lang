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

test renders_native_lists_in_order:
    label = ui.inline_text("Read <notes> & ")
    first = ui.list_item([label, ui.link("references", "/ref")])
    nested = ui.unordered_list([ui.list_item([ui.inline_text("Notebook")])])
    second = ui.list_item([ui.inline_text("Bring "), nested])
    steps = ui.ordered_list(-2, [first, second])
    assert steps.html == "<ol start=\"-2\"><li>Read &lt;notes&gt; &amp; <a href=\"/ref\" tabindex=\"0\">references</a></li><li>Bring <ul><li>Notebook</li></ul></li></ol>"
    assert ui.unordered_list([]).html == "<ul></ul>"

test renders_native_table_in_order:
    guide = [ui.link("A & B", "/guide"), ui.inline_text("<ready>")]
    notes = [ui.inline_text("Notes"), ui.inline_text("Draft")]
    table = ui.table("Books <today>", ["Title & author", "Status"], [guide, notes])
    assert table.html == "<table><caption>Books &lt;today&gt;</caption><thead><tr><th scope=\"col\">Title &amp; author</th><th scope=\"col\">Status</th></tr></thead><tbody><tr><td><a href=\"/guide\" tabindex=\"0\">A &amp; B</a></td><td>&lt;ready&gt;</td></tr><tr><td>Notes</td><td>Draft</td></tr></tbody></table>"
    assert ui.table("Queue", ["Title"], []).html == "<table><caption>Queue</caption><thead><tr><th scope=\"col\">Title</th></tr></thead><tbody></tbody></table>"

test agent_query_uses_the_same_form:
    form = ui.agent_query_form("find_slots", "Find \"open\" slots", "/slots", [ui.text_field("Day", "day", ""), ui.submit("Find slots")])
    assert form.html == "<form method=\"get\" action=\"/slots\" toolname=\"find_slots\" tooldescription=\"Find &quot;open&quot; slots\"><label>Day<input type=\"text\" name=\"day\" value=\"\"></label><button type=\"submit\">Find slots</button></form>"
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
        good.status.success() && String::from_utf8_lossy(&good.stdout).contains("5 passed"),
        "{good:?}"
    );
    fs::write(
        &main,
        source.replace("ordered_list(-2", "ordered_list(\"two\""),
    )
    .unwrap();
    let bad_start = run("check");
    assert!(!bad_start.status.success(), "{bad_start:?}");
    assert!(
        String::from_utf8_lossy(&bad_start.stderr).contains("expected `Int`, got `String`"),
        "{bad_start:?}"
    );
    fs::write(&main, source.replace("[guide, notes]", "[[\"raw\"]]")).unwrap();
    let bad_cells = run("check");
    assert!(
        !bad_cells.status.success()
            && String::from_utf8_lossy(&bad_cells.stderr)
                .contains("expected `List<List<View>>`, got `List<List<String>>`"),
        "{bad_cells:?}"
    );
    fs::write(main, source.replace("ui.text(\"second\")", "\"second\"")).unwrap();
    let bad = run("check");
    assert!(!bad.status.success(), "{bad:?}");
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("produce a `View`"),
        "{bad:?}"
    );
}

#[test]
fn web_bundle_without_event_handlers_contains_only_application_inputs() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join("migrations")).unwrap();
    fs::write(
        root.join("migrations/001.sql"),
        "create table notes(id integer);",
    )
    .unwrap();
    fs::write(
        root.join("src/main.cor"),
        "server app:\n    route GET \"/\" -> html:\n        return \"Hello\"\n",
    )
    .unwrap();
    fs::write(root.join("src/session.log"), "runtime data").unwrap();
    let build = || {
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(root)
            .args(["build", "src/main.cor", "--target", "web"])
            .output()
            .unwrap()
    };
    let result = build();
    assert!(result.status.success(), "{result:?}");
    let output = root.join("target/web.tar");
    let original = fs::read(&output).unwrap();
    let mut archive = tar::Archive::new(original.as_slice());
    let files: Vec<_> = archive
        .entries()
        .unwrap()
        .map(|entry| entry.unwrap().path().unwrap().into_owned())
        .collect();
    assert_eq!(
        files,
        [
            std::path::PathBuf::from("migrations/001.sql"),
            std::path::PathBuf::from("src/main.cor")
        ]
    );
    fs::write(
        root.join("src/main.cor"),
        "agent main() -> Int:\n    return 3\n",
    )
    .unwrap();
    assert!(!build().status.success());
    assert_eq!(fs::read(output).unwrap(), original);
}

#[test]
fn web_bundle_compiles_events_and_preserves_the_previous_build_on_handler_errors() {
    let project = tempfile::tempdir().unwrap();
    let src = project.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("main.cor"),
        "server app:\n    route GET \"/\" -> html:\n        return \"Counter\"\n",
    )
    .unwrap();
    let handler = "agent increment(current: Int) -> Int:\n    return current + 1\n";
    let events = src.join("events.cor");
    fs::write(&events, handler).unwrap();
    let build = || {
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(project.path())
            .args(["build", "src/main.cor", "--target", "web"])
            .output()
            .unwrap()
    };
    let valid = build();
    assert!(valid.status.success(), "{valid:?}");
    let output = project.path().join("target/web.tar");
    let original = fs::read(&output).unwrap();
    let extracted = tempfile::tempdir().unwrap();
    tar::Archive::new(original.as_slice())
        .unpack(extracted.path())
        .unwrap();
    let assets = extracted.path().join("target/wasm");
    assert!(fs::read(assets.join("events.wasm"))
        .unwrap()
        .starts_with(b"\0asm"));
    let loader = fs::read_to_string(assets.join("events.js")).unwrap();
    assert!(loader.contains("export async function instantiate("));
    assert_eq!(
        fs::read_to_string(extracted.path().join("src/events.cor")).unwrap(),
        handler
    );
    fs::write(events, handler.replace("current + 1", "\"wrong\"")).unwrap();
    let invalid = build();
    assert!(!invalid.status.success(), "{invalid:?}");
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("return type mismatch"));
    assert_eq!(fs::read(output).unwrap(), original);
}

#[test]
fn bound_form_uses_the_route_record_and_reports_stale_bindings() {
    let project = tempfile::tempdir().unwrap();
    let source = r#"
type Filter:
    term: String
    ratio: Float
    enabled: Bool

server app:
    route POST "/save" form Filter -> html:
        return body.term

test initial_controls:
    form = form_for("POST /save", Filter("<milk>", 1.25, false), "Save & close")
    assert form.contains("value=\"&lt;milk&gt;\"")
    assert form.contains("step=\"any\" name=\"ratio\"")
    assert form.contains("<option selected>false</option>")
    assert form.contains("Save &amp; close")
"#;
    let run = |cmd, program: &str| {
        fs::write(project.path().join("main.cor"), program).unwrap();
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(project.path())
            .args([cmd, "main.cor"])
            .output()
            .unwrap()
    };
    let success = run("test", source);
    assert!(success.status.success(), "{success:?}");
    for (before, after, message) in [
        ("POST /save", "POST /stale", "no declared route"),
        ("form Filter", "body Filter", "POST form Record"),
        ("form Filter", "form String", "expected a record"),
        ("ratio: Float", "ratio: List<Float>", "field `ratio`"),
    ] {
        let bad = run("check", &source.replace(before, after));
        assert!(
            !bad.status.success() && String::from_utf8_lossy(&bad.stderr).contains(message),
            "{bad:?}"
        );
    }
}
