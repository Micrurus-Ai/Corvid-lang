"""Load a Corvid record editor, stop its server, then exercise three browser engines."""
import argparse
from contextlib import ExitStack
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory

from playwright.sync_api import expect, sync_playwright
from verify_browser_diagnostics import package
from verify_reading_list import serve


def exercise(page):
    requests = []
    page.on("request", lambda request: requests.append(request.url))
    page.context.set_offline(True)
    picker, other = page.locator("[data-local-selection]").all()
    output = picker.get_by_role("status")
    red = picker.get_by_role("slider", name="Red")
    expect(output).to_have_text("RGB 20, 80, 160")
    box = red.bounding_box()
    red.click(position={"x": int(box["width"] * .75), "y": int(box["height"] / 2)})
    value = int(red.input_value())
    assert value != 20
    expect(output).to_have_text(f"RGB {value}, 80, 160")
    red.press("ArrowLeft")
    expect(output).to_have_text(f"RGB {value - 1}, 80, 160")
    expect(red).to_be_focused()
    expected = {"red": 0, "green": 127, "blue": 255}
    result = page.evaluate("v => tools.get('select_color').execute(v)", expected)
    assert result == "RGB 0, 127, 255"
    expect(output.locator("[data-record-swatch]")).to_have_css("background-color", "rgb(0, 127, 255)")
    expect(red).to_have_value("0")
    expect(other.get_by_role("status")).to_have_text("RGB 20, 80, 160")
    for invalid in (dict(expected, red=256), dict(expected, blue=0.5), {"red": 1}, dict(expected, extra=1)):
        assert page.evaluate("""async v => { try { await tools.get('select_color').execute(v); }
            catch { return true; } return false; }""", invalid)
    expect(output).to_have_text(result)
    assert not requests, requests


def check_bindings(cli, project):
    model = project / "src/shared_dimensions.cor"
    model.write_text("public type Dimensions:\n    width: Int where between(1, 800)\n")
    probe = project / "src/state_bindings.cor"
    probe.write_text("""import "./shared_dimensions" use Dimensions
import "std:ui" as ui

local type Legacy:
    width: Int where between(1, 800)

fn describe(initial: Dimensions) -> ui.RecordState:
    browser state selected = initial
    return selected

test state_bindings:
    current = describe(Dimensions(31))
    legacy = Legacy_state(Legacy(31))
    assert current.fields[0].name == legacy.fields[0].name
    assert current.fields[0].value == 31
    assert current.fields[0].maximum == 800
""")
    subprocess.run([cli, "test", probe], cwd=project, check=True)
    model.write_text(model.read_text().replace("Int where between(1, 800)", "Int"))
    rejected = subprocess.run([cli, "check", probe], cwd=project, capture_output=True, text=True)
    assert rejected.returncode and "browser state" in rejected.stderr, rejected
    model.write_text("""public choice Mode:
    Manual
    Scheduled

public type Profile:
    name: String

public type Settings:
    profile: Profile
    enabled: Bool
    mode: Mode

public fn defaults() -> Settings:
    return Settings(Profile("Scout"), false, Manual)
""")
    probe.write_text("""import "./shared_dimensions" use defaults
import "std:ui" as ui

fn describe() -> ui.RecordState:
    browser state selected = defaults()
    key = bind selected.profile.name
    return selected

test imported_choice:
    state = describe()
    assert state.descriptors.length() == 3
""")
    subprocess.run([cli, "test", probe], cwd=project, check=True)
    model.write_text(model.read_text().replace("    Scheduled", "    Scheduled(at: String)"))
    rejected = subprocess.run([cli, "check", probe], cwd=project, capture_output=True, text=True)
    assert rejected.returncode and "browser state" in rejected.stderr, rejected
    # The packaged fixture below contains only the application under test.
    probe.unlink()
    model.unlink()


def check_field_diagnostics(cli, project):
    main = project / "src/main.cor"
    source = main.read_text()
    for before, after, diagnostic in [
        ("bind settings.profile.name", "bind settings.profile.nmae", "nmae"),
        ("bind settings.profile.name", "bind settings.profil.name", "profil"),
        ("bind settings.tools.web_search", "bind settings.profile.name", "checkbox field binding"),
        ("bind settings.mode", "bind settings.retries", "segmented_choice"),
        ("bind settings.profile.name", "bind initial.profile.name", "browser state"),
    ]:
        assert before in source
        main.write_text(source.replace(before, after))
        result = subprocess.run([cli, "check", main], cwd=project, capture_output=True, text=True)
        assert result.returncode and diagnostic in result.stderr, (after, result)
    main.write_text(source)


def check_child_blocks(cli, project):
    probe = project / "src/layout_test.cor"
    probe.write_text("""import "std:ui" as ui

fn settings_panel(children: List<ui.View>) -> ui.View:
    return ui.stack(children)

fn panel_preview() -> ui.View:
    return settings_panel:
        ui.field("Name", "name")
        ui.row:
            ui.checkbox("Enabled", "enabled")
            ui.text("Manual")
        ui.text("Scheduled")

test layout_order:
    row = ui.row([ui.local_checkbox("Enabled", "enabled"), ui.text("Manual")])
    children = [ui.local_field("Name", "name"), row, ui.text("Scheduled")]
    assert panel_preview().html == settings_panel(children).html
""")
    subprocess.run([cli, "test", probe], cwd=project, check=True)
    probe.write_text(probe.read_text().replace('        ui.text("Scheduled")', '        42'))
    invalid = subprocess.run([cli, "check", probe], cwd=project, capture_output=True, text=True)
    assert invalid.returncode and "Int" in invalid.stderr, invalid
    probe.unlink()


def exercise_settings(page):
    requests = []
    page.on("request", lambda request: requests.append(request.url))
    page.context.set_offline(True)
    name = page.get_by_role("textbox", name="Agent name")
    enabled = page.get_by_role("checkbox", name="Allow web search")
    mode = page.get_by_role("group", name="Run mode")
    scheduled = mode.get_by_role("radio", name="Scheduled")
    preview = page.get_by_role("status")
    expect(preview).to_have_text("Assistant / Manual / web search: false / retries: 1")
    schema = page.evaluate("tools.get('update_settings').inputSchema")
    assert schema["properties"]["mode"]["enum"] == ["Manual", "Scheduled"]
    assert schema["properties"]["tools"]["properties"]["web_search"] == {"type": "boolean"}
    desired = {"profile": {"name": "Scout"}, "tools": {"web_search": True}, "mode": "Scheduled", "retries": 3}
    select = "v => tools.get('update_settings').execute(v)"
    result = page.evaluate(select, desired)
    expect(preview).to_have_text("Scout / Scheduled / web search: true / retries: 3")
    expect(name).to_have_value("Scout")
    expect(enabled).to_be_checked()
    expect(scheduled).to_be_checked()
    for patch in ({"tools": {"web_search": "true"}}, {"mode": "Auto"}, {"profile": {"name": ""}},
                  {"profile": {"name": "x" * 61}}, {"profile": {}}, {"tools": {"web_search": True, "extra": 1}},
                  {"retries": 6}, {"extra": True}):
        assert page.evaluate("""async v => { try { await tools.get('update_settings').execute(v); }
            catch { return true; } return false; }""", dict(desired, **patch))
        expect(preview).to_have_text(result)
    scheduled.press("ArrowLeft")
    enabled.click()
    expect(name).to_have_value("Scout")
    expect(preview).to_have_text("Scout / Manual / web search: false / retries: 3")
    name.fill("<Scout & Co>")
    expect(preview).to_have_text("<Scout & Co> / Manual / web search: false / retries: 3")
    expect(enabled).not_to_be_checked()
    assert not requests, requests


def verify(cli, channel):
    with TemporaryDirectory() as tmp, sync_playwright() as engines, ExitStack() as browsers:
        root = Path(tmp)
        project = root / "project"
        shutil.copytree(Path(__file__).resolve().parents[1] / "examples/rgb_picker", project,
                        ignore=shutil.ignore_patterns("target"))
        check_bindings(cli, project)
        main = project / "src/main.cor"
        # No server-side selection route exists in this fixture.
        main.write_text(main.read_text().split('    route GET "/server"')[0].replace(
            '"Choose an RGB color.")]', '"Choose an RGB color."), picker]'))
        package(cli, project, root / "bundle")
        settings = root / "settings"
        shutil.copytree(Path(__file__).resolve().parents[1] / "examples/agent_settings", settings,
                        ignore=shutil.ignore_patterns("target"))
        check_child_blocks(cli, settings)
        check_field_diagnostics(cli, settings)
        package(cli, settings, root / "settings-bundle")
        pages = []
        with ExitStack() as servers:
            url = servers.enter_context(serve(cli, root / "bundle", os.environ.copy()))
            settings_url = servers.enter_context(serve(cli, root / "settings-bundle", os.environ.copy()))
            for engine in ("chromium", "firefox", "webkit"):
                browser = getattr(engines, engine).launch(channel=channel if engine == "chromium" else None)
                browsers.callback(browser.close)
                for address, tool, test in [(url, "select_color", exercise),
                                            (settings_url, "update_settings", exercise_settings)]:
                    page = browser.new_page()
                    page.add_init_script("""Object.defineProperty(document, 'modelContext', {value: {
                        registerTool(tool) { (window.tools ??= new Map()).set(tool.name, tool); }
                    }})""")
                    page.goto(address)
                    page.wait_for_function("name => window.tools?.has(name)", arg=tool)
                    pages.append((engine, page, test))
        for engine, page, test in pages:
            test(page)
            print(engine, "passed with server stopped", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
