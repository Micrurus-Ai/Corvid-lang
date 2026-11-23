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

test checkbox_field_escapes_strings_and_respects_initial_state:
    packed = ui.checkbox_field("<Map> & compass", "gear\"list", true)
    assert packed.html == "<label><input type=\"checkbox\" name=\"gear&quot;list\" checked>&lt;Map&gt; &amp; compass</label>"
    assert ui.checkbox_field("Water", "water", false).html == "<label><input type=\"checkbox\" name=\"water\">Water</label>"

test reset_button_escapes_its_label:
    button = ui.reset_button("Restore <note> & map")
    assert button.html == "<button type=\"reset\">Restore &lt;note&gt; &amp; map</button>"

test preserves_child_order:
    body = ui.disclosure("More", [ui.text("first"), ui.text("second")])
    assert body.html == "<details><summary>More</summary><p>first</p><p>second</p></details>"

test paragraph_preserves_inline_markup_and_escaped_text:
    label = ui.trusted_markup("<strong>Note</strong>")
    note = ui.inline_text(": <em>Wind & rain</em>")
    assert ui.paragraph([label, note]).html == "<p><strong>Note</strong>: &lt;em&gt;Wind &amp; rain&lt;/em&gt;</p>"
    assert ui.paragraph([]).html == "<p></p>"

test abbreviation_escapes_text_and_title:
    distance = ui.abbreviation("<km> & mi", "kilometres & \"miles\"")
    assert distance.html == "<abbr title=\"kilometres &amp; &quot;miles&quot;\">&lt;km&gt; &amp; mi</abbr>"

test strong_preserves_inline_children_in_order:
    warning = ui.strong([ui.inline_text("<Closed> & "), ui.link("detour", "/detour")])
    assert warning.html == "<strong>&lt;Closed&gt; &amp; <a href=\"/detour\" tabindex=\"0\">detour</a></strong>"
    assert ui.strong([]).html == "<strong></strong>"

test escapes_multiline_field:
    field = ui.text_area("Notes & maps", "a\"b", "\n</textarea> & more")
    assert field.html == "<label>Notes &amp; maps<textarea name=\"a&quot;b\">\n\n&lt;/textarea&gt; &amp; more</textarea></label>"

test textarea_requirement_controls_boolean_attribute:
    required_field = ui.text_area_with_requirement("Notes & maps", "a\"b", "\n</textarea>", true)
    optional_field = ui.text_area_with_requirement("Note", "note", "", false)
    assert required_field.html == "<label>Notes &amp; maps<textarea name=\"a&quot;b\" required>\n\n&lt;/textarea&gt;</textarea></label>"
    assert optional_field.html == ui.text_area("Note", "note", "").html

test fieldset_escapes_legend_and_preserves_controls:
    first = ui.text_field("Name", "name", "")
    last = ui.submit("Save")
    group = ui.fieldset("Maps & <notes>", [first, last])
    assert group.html == "<fieldset><legend>Maps &amp; &lt;notes&gt;</legend>" + first.html + last.html + "</fieldset>"

test renders_heading_levels:
    headings = [
        ui.h1("Trails & <maps>"), ui.h2("Level 2"), ui.h3("Level 3"),
        ui.h4("Level 4"), ui.h5("Level 5"), ui.h6("")
    ]
    page = ui.page("Guide", headings)
    assert page.contains("<main><h1>Trails &amp; &lt;maps&gt;</h1><h2>Level 2</h2><h3>Level 3</h3><h4>Level 4</h4><h5>Level 5</h5><h6></h6></main>")

test escapes_document_language:
    page = ui.page_in_language("en\" & <tag>", "Map & guide", [ui.text("Map")])
    assert page.contains("<!doctype html><html lang=\"en&quot; &amp; &lt;tag&gt;\"><head>")
    assert page.contains("<title>Map &amp; guide</title></head><body><main><p>Map</p></main>")

test trusted_markup_and_text_in_one_page:
    fragment = "<h1 data-topic=\"notes\">Maps &amp; trails</h1>"
    page = ui.page("Guide", [ui.trusted_markup(fragment), ui.text(fragment)])
    literal = "<p>&lt;h1 data-topic=&quot;notes&quot;&gt;Maps &amp;amp; trails&lt;/h1&gt;</p>"
    assert page.contains("<main>" + fragment + literal + "</main>")

test custom_data_group:
    group = ui.data_group("trail-id2", "<east> & \"west\"", [ui.h2("Map"), ui.text("Route")])
    html = match group:
        Ok(view) -> view.html
        Err(message) -> message
    assert html == "<div data-trail-id2=\"&lt;east&gt; &amp; &quot;west&quot;\"><h2>Map</h2><p>Route</p></div>"

fn rejects_data_name(name: String) -> Bool:
    return match ui.data_group(name, "", []):
        Ok(_) -> false
        Err(_) -> true

test rejects_invalid_data_names:
    assert rejects_data_name("")
    assert rejects_data_name("trail id")
    assert rejects_data_name("x=\"y\"")
    assert rejects_data_name("Trail")
    assert rejects_data_name("café")
    assert rejects_data_name("x>")

test renders_image_attributes:
    map_image = ui.image("/map?day=1&trail=\"east\"", "Bridge <closed> & \"East\"")
    assert map_image.html == "<img src=\"/map?day=1&amp;trail=&quot;east&quot;\" alt=\"Bridge &lt;closed&gt; &amp; &quot;East&quot;\">"
    assert ui.image("/border.png", "").html == "<img src=\"/border.png\" alt=\"\">"

test figure_preserves_child_order_and_escapes_caption:
    figure = ui.figure("Map & <key>", [ui.image("/map.png", "Route"), ui.inline_text("North")])
    assert figure.html == "<figure><img src=\"/map.png\" alt=\"Route\">North<figcaption>Map &amp; &lt;key&gt;</figcaption></figure>"

test renders_labelled_navigation_in_order:
    links = [ui.link("Map", "/map"), ui.link("Trails", "/trails")]
    nav = ui.navigation("Walks & \"rides\" <nearby>", links)
    assert nav.html == "<nav aria-label=\"Walks &amp; &quot;rides&quot; &lt;nearby&gt;\"><a href=\"/map\" tabindex=\"0\">Map</a><a href=\"/trails\" tabindex=\"0\">Trails</a></nav>"
    assert ui.page("Guide", [nav]).contains("<main>" + nav.html + "</main>")

test renders_native_lists_in_order:
    label = ui.inline_text("Read <notes> & ")
    first = ui.list_item([label, ui.link("references", "/ref")])
    nested = ui.unordered_list([ui.list_item([ui.inline_text("Notebook")])])
    second = ui.list_item([ui.inline_text("Bring "), nested])
    steps = ui.ordered_list(-2, [first, second])
    assert steps.html == "<ol start=\"-2\"><li>Read &lt;notes&gt; &amp; <a href=\"/ref\" tabindex=\"0\">references</a></li><li>Bring <ul><li>Notebook</li></ul></li></ol>"
    assert ui.unordered_list([]).html == "<ul></ul>"

test section_names_its_region_and_preserves_order:
    region = ui.section("trail\"&", "Maps <today>", [ui.text("first"), ui.text("second")])
    assert region.html == "<section aria-labelledby=\"trail&quot;&amp;\"><h2 id=\"trail&quot;&amp;\">Maps &lt;today&gt;</h2><p>first</p><p>second</p></section>"

test renders_native_table_in_order:
    guide = [ui.link("A & B", "/guide"), ui.inline_text("<ready>")]
    notes = [ui.inline_text("Notes"), ui.inline_text("Draft")]
    table = ui.table("Books <today>", ["Title & author", "Status"], [guide, notes])
    assert table.html == "<table><caption>Books &lt;today&gt;</caption><thead><tr><th scope=\"col\">Title &amp; author</th><th scope=\"col\">Status</th></tr></thead><tbody><tr><td><a href=\"/guide\" tabindex=\"0\">A &amp; B</a></td><td>&lt;ready&gt;</td></tr><tr><td>Notes</td><td>Draft</td></tr></tbody></table>"
    assert ui.table("Queue", ["Title"], []).html == "<table><caption>Queue</caption><thead><tr><th scope=\"col\">Title</th></tr></thead><tbody></tbody></table>"

test first_cell_labels_each_nonempty_row:
    stop = ui.inline_text("Bridge & park")
    distance = ui.abbreviation("1 km", "1 kilometre")
    table = ui.table_with_row_headers("Distances", ["Stop", "Distance"], [[stop, distance], [], [stop]])
    header = "<th scope=\"row\">Bridge &amp; park</th>"
    assert table.html.contains("<tbody><tr>" + header + "<td>" + distance.html + "</td></tr><tr></tr><tr>" + header + "</tr></tbody>")

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
        good.status.success() && String::from_utf8_lossy(&good.stdout).contains("23 passed"),
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
    fs::write(
        &main,
        source.replace("\"note\", \"\", false)", "\"note\", \"\", \"false\")"),
    )
    .unwrap();
    let bad_requirement = run("check");
    assert!(
        !bad_requirement.status.success()
            && String::from_utf8_lossy(&bad_requirement.stderr)
                .contains("expected `Bool`, got `String`"),
        "{bad_requirement:?}"
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
