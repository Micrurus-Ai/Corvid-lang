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
    # The packaged fixture below contains only the application under test.
    probe.unlink()
    model.unlink()


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
        pages = []
        with serve(cli, root / "bundle", os.environ.copy()) as url:
            for engine in ("chromium", "firefox", "webkit"):
                browser = getattr(engines, engine).launch(channel=channel if engine == "chromium" else None)
                browsers.callback(browser.close)
                page = browser.new_page()
                page.add_init_script("""Object.defineProperty(document, 'modelContext', {value: {
                    registerTool(tool) { (window.tools ??= new Map()).set(tool.name, tool); }
                }})""")
                page.goto(url)
                page.wait_for_function("window.tools?.has('select_color')")
                pages.append((engine, page))
        for engine, page in pages:
            exercise(page)
            print(engine, "passed with server stopped", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
