"""Exercise Corvid drag surfaces from a built web package."""
import argparse
import os
from pathlib import Path
import shutil
from tempfile import TemporaryDirectory

from playwright.sync_api import expect, sync_playwright
from verify_browser_diagnostics import package
from verify_reading_list import serve


def exercise(page, url):
    page.add_init_script("""Object.defineProperty(document, 'modelContext', {value: {
        registerTool(tool) { (window.tools ??= new Map()).set(tool.name, tool); }
    }})""")
    page.goto(url)
    page.wait_for_function("window.tools?.size === 2")
    canvas = page.locator("canvas").first
    box = canvas.bounding_box()
    page.mouse.move(box["x"] + 4, box["y"] + 5)
    page.mouse.down()
    page.mouse.move(box["x"] + 12, box["y"] + 5)
    page.mouse.up()
    expect(page.locator("output").first).to_have_text("2")
    assert canvas.evaluate("c => c.getContext('2d').getImageData(8,3,1,5).data.some((n,i) => i % 4 === 3 && n > 0)")
    canvas.press("ArrowRight")
    expect(page.locator("output").first).to_have_text("3")
    expect(page.locator("output").last).to_have_text("0")
    assert page.evaluate("""tools.get('add_stroke').execute({
        start:{x:16,y:16}, end:{x:20,y:16}})""") == "4"
    assert canvas.evaluate("c => c.getContext('2d').getImageData(18,16,1,1).data[3] > 0")
    assert page.evaluate("""async () => {
        try { await tools.get('add_stroke').execute({start:{x:-1,y:0}, end:{x:0,y:0}}); }
        catch { return true; } return false;
    }""")
    handle = page.get_by_role("separator")
    handle.press("Shift+ArrowRight")
    expect(handle).to_have_attribute("aria-valuenow", "210")
    box = handle.bounding_box()
    page.mouse.move(box["x"] + 3, box["y"] + 3)
    page.mouse.down()
    page.mouse.move(box["x"] + 53, box["y"] + 3)
    page.mouse.up()
    expect(handle).to_have_attribute("aria-valuenow", "260")
    for delta, width in [(800, "480"), (-800, "96")]:
        assert page.evaluate("""x => tools.get('resize_panel').execute({
            start:{x:0,y:0}, end:{x,y:0}})""", delta) == width
        expect(handle).to_have_attribute("aria-valuenow", width)
        expect(page.locator("[data-corvid-panel]")).to_have_css("width", width + "px")
    handle.evaluate("""el => el.addEventListener('pointerdown', e => {
        el.dispatchEvent(new PointerEvent('pointercancel', {pointerId:e.pointerId}));
    }, {once:true})""")
    box = handle.bounding_box()
    page.mouse.move(box["x"] + 3, box["y"] + 3)
    page.mouse.down()
    page.mouse.move(box["x"] + 50, box["y"] + 3)
    page.mouse.up()
    expect(handle).to_have_attribute("aria-valuenow", "96")
    page.reload()
    expect(handle).to_have_attribute("aria-valuenow", "200")
    expect(page.locator("output").first).to_have_text("0")


def verify(cli, channel):
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        project = root / "workspace"
        shutil.copytree(Path(__file__).resolve().parents[1] / "examples/pointer_components",
                        project, ignore=shutil.ignore_patterns("target"))
        main = project / "src/main.cor"
        main.write_text(main.read_text().replace("[sketch, sidebar]", "[sketch, sidebar, Sketch()]"))
        package(cli, project, root / "bundle")
        with serve(cli, root / "bundle", os.environ.copy()) as url, sync_playwright() as engines:
            for name in ("chromium", "firefox", "webkit"):
                browser = getattr(engines, name).launch(channel=channel if name == "chromium" else None)
                try:
                    exercise(browser.new_page(), url)
                    print(name, "passed", flush=True)
                finally:
                    browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
