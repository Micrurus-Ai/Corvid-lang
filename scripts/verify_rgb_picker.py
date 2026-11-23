"""Check a developer-defined record selection in three browser engines."""
import argparse
import os
from pathlib import Path
import shutil
from tempfile import TemporaryDirectory

from playwright.sync_api import expect, sync_playwright
from verify_browser_diagnostics import package
from verify_reading_list import serve


def check(page, url):
    page.add_init_script("""Object.defineProperty(document, "modelContext", {value: {
        registerTool(tool) { (window.tools ??= new Map()).set(tool.name, tool); }
    }})""")
    page.goto(url)
    page.wait_for_function("window.tools?.has('select_color')")
    picker = page.locator("form").first
    output = picker.get_by_role("status")
    other = page.locator("form").last.get_by_role("status")
    red = picker.get_by_role("slider", name="Red")
    expect(output).to_have_text("RGB 20, 80, 160")
    box = red.bounding_box()
    red.click(position={"x": int(box["width"] * 0.7), "y": int(box["height"] / 2)})
    changed = int(red.input_value())
    assert changed != 20
    expect(output).to_have_text(f"RGB {changed}, 80, 160")
    red.press("ArrowRight")
    expect(output).to_have_text(f"RGB {changed + 1}, 80, 160")
    expect(red).to_be_focused()
    schema = page.evaluate("tools.get('select_color').inputSchema")
    assert schema["required"] == ["red", "green", "blue"]
    assert schema["properties"]["green"] == {"type": "integer", "minimum": 0, "maximum": 255}
    result = page.evaluate("tools.get('select_color').execute({red:0, green:127, blue:255})")
    assert result == "RGB 0, 127, 255"
    expect(output.locator("[aria-hidden]")).to_have_css("background-color", "rgb(0, 127, 255)")
    expect(red).to_have_value("0")
    expect(other).to_have_text("RGB 20, 80, 160")
    for invalid in ({"red": 256, "green": 5, "blue": 2}, {"red": 12}):
        assert page.evaluate("""async v => {
            try { await tools.get('select_color').execute(v); } catch { return true; } return false;
        }""", invalid)
    expect(output).to_have_text(result)
    response = page.request.post(url + "/color", form={"red": "256", "green": "0", "blue": "0"})
    assert 400 <= response.status < 500, response.status
    pending = []
    def reply_in_reverse(route):
        pending.append(route)
        if len(pending) == 2:
            for delayed in reversed(pending):
                delayed.fulfill(response=delayed.fetch())

    page.route("**/color", reply_in_reverse)
    for blue in (33, 44):
        with page.expect_request("**/color"):
            page.evaluate("""blue => { (window.selectionResults ??= []).push(tools.get('select_color')
                .execute({red:0,green:127,blue}).catch(e => e.message)); }""", blue)
    assert page.evaluate("Promise.all(window.selectionResults)") == [
        "Selection was superseded.", "RGB 0, 127, 44"]
    expect(output).to_have_text("RGB 0, 127, 44")
    page.unroute("**/color")
    page.route("**/color", lambda route: route.fulfill(status=500, body="failed"))
    red.press("ArrowRight")
    expect(picker.get_by_role("alert")).to_contain_text("500")
    expect(red).to_have_value("0")
    expect(output).to_have_text("RGB 0, 127, 44")


def verify(cli, channel):
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        project = root / "project"
        shutil.copytree(Path(__file__).resolve().parents[1] / "examples/rgb_picker", project,
                        ignore=shutil.ignore_patterns("target"))
        main = project / "src/main.cor"
        main.write_text(main.read_text().replace('"Choose an RGB color.")]', '"Choose an RGB color."), picker]'))
        package(cli, project, root / "bundle")
        with serve(cli, root / "bundle", os.environ.copy()) as url, sync_playwright() as engines:
            for name in ("chromium", "firefox", "webkit"):
                browser = getattr(engines, name).launch(channel=channel if name == "chromium" else None)
                try:
                    check(browser.new_page(), url)
                    print(name, "passed", flush=True)
                finally:
                    browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
