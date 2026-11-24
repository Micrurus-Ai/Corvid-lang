"""Build the trail report and pointer examples and exercise their packaged controls."""
import argparse
from contextlib import ExitStack, nullcontext
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from verify_reading_list import read, serve


def check_report(page, url):
    from playwright.sync_api import expect

    page.goto(url)
    field = page.get_by_role("combobox", name="Trail")
    form = page.get_by_role("form", name="Find a trail")
    page.keyboard.press("Tab")
    expect(field).to_be_focused()
    page.keyboard.press("Enter")
    expect(field).to_have_attribute("aria-invalid", "true")
    assert page.get_by_role("alert").inner_text() == field.evaluate("el => el.validationMessage")
    assert field.evaluate("el => el.ariaDescribedByElements[0].getAttribute('role') === 'alert'")

    session = page.context.new_cdp_session(page)
    session.send("Input.imeSetComposition", {"text": "Ri", "selectionStart": 2, "selectionEnd": 2})
    expect(field).to_have_value("Ri")
    expect(field).to_have_attribute("aria-expanded", "false")
    assert field.evaluate("""el => el.dispatchEvent(new KeyboardEvent('keydown',
        {key: 'Enter', bubbles: true, isComposing: true, cancelable: true}))""")
    session.send("Input.insertText", {"text": "Ri"})
    choices = page.get_by_role("listbox", name="Trail")
    expect(choices.get_by_role("option")).to_have_text(["River walk", "Ridge path"])
    expect(page.get_by_role("alert")).to_be_empty()
    form.evaluate("""form => {
        window.choiceEvents = [];
        for (const type of ['input', 'change', 'keydown'])
            form.addEventListener(type, e => choiceEvents.push([e.type, e.defaultPrevented]));
    }""")
    page.keyboard.press("ArrowDown")
    selected = choices.get_by_role("option", selected=True)
    expect(selected).to_have_text("River walk")
    expect(field).to_have_attribute("aria-activedescendant", selected.get_attribute("id"))
    page.keyboard.press("Enter")
    expect(field).to_have_value("River walk")
    assert page.evaluate("choiceEvents") == [
        ["keydown", True], ["input", False], ["change", False], ["keydown", True]]
    expect(field).to_be_focused()
    expect(choices).to_be_hidden()
    page.keyboard.press("Enter")
    expect(page).to_have_url(url + "/trail?trail=River+walk")
    expect(page.get_by_role("heading")).to_have_text("River walk")

    page.goto(url)
    field.click()
    choices.get_by_role("option", name="Lake loop").click()
    expect(field).to_have_value("Lake loop")
    field.fill("R")
    choices.get_by_role("option", name="Ridge path").tap()
    expect(field).to_have_value("Ridge path")
    form.evaluate("el => el.requestSubmit()")
    expect(page).to_have_url(url + "/trail?trail=Ridge+path")

    page.goto(url)
    opener = page.get_by_role("button", name="Edit field notes")
    for _ in range(3):
        page.keyboard.press("Tab")
    expect(opener).to_be_focused()
    page.keyboard.press("Enter")
    dialog = page.get_by_role("dialog", name="Field journal")
    editor = dialog.get_by_role("textbox", name="Field notes")
    expect(editor).to_be_focused()
    assert dialog.evaluate("el => el.matches(':modal')")
    page.keyboard.press("Control+a")
    assert page.evaluate("getSelection().toString()") == "Trail conditions"
    page.keyboard.press("Control+b")
    assert editor.locator("b, strong").inner_text() == "Trail conditions"
    page.keyboard.press("Control+z")
    expect(editor.locator("b, strong")).to_have_count(0)
    expect(editor).to_have_text("Trail conditions")
    page.keyboard.press("Tab")
    files = dialog.get_by_label("Choose or drop photos")
    expect(files).to_be_focused()
    page.keyboard.press("Tab")
    expect(dialog.get_by_role("button", name="Close journal")).to_be_focused()
    page.keyboard.press("Escape")
    expect(dialog).to_be_hidden()
    expect(opener).to_be_focused()

    opener.evaluate("el => el.click()")
    files.set_input_files({"name": "ridge.jpg", "mimeType": "image/jpeg", "buffer": b"photo"})
    assert files.evaluate("el => el.files[0].name") == "ridge.jpg"
    assert dialog.evaluate("""dialog => {
        const label = dialog.querySelector('label');
        const transfer = new DataTransfer();
        transfer.items.add(new File(['photo'], 'lake.jpg'));
        const changes = [];
        dialog.addEventListener('change', e => changes.push(e.target.files[0].name));
        const dispatch = type => label.dispatchEvent(new DragEvent(type, {
            dataTransfer: transfer, bubbles: true, cancelable: true}));
        return !dispatch('dragover') && !dispatch('drop') && changes.join() === 'lake.jpg';
    }""")
    dialog.get_by_role("button", name="Close journal").tap()
    expect(opener).to_be_focused()


def check_touch_resize(page, url):
    from playwright.sync_api import expect

    page.add_init_script("""Object.defineProperty(document, 'modelContext', {value: {
        registerTool(tool) { (window.registeredActions ??= new Map()).set(tool.name, tool); }
    }})""")
    page.goto(url)
    page.wait_for_function("window.registeredActions?.has('resize_panel')")
    handle = page.get_by_role("separator", name="Resize panel")
    handle.press("ArrowRight")
    expect(handle).to_have_attribute("aria-valuenow", "201")
    handle.evaluate("""el => el.addEventListener('gotpointercapture', e => {
        el.dataset.captured = e.pointerType;
    }, {once: true})""")
    box = handle.bounding_box()
    point = {"x": box["x"] + 4, "y": box["y"] + 4}
    session = page.context.new_cdp_session(page)
    session.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [point]})
    point["x"] += 60
    session.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [point]})
    session.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    expect(handle).to_have_attribute("data-captured", "touch")
    expect(handle).to_have_attribute("aria-valuenow", "261")


def verify(cli, channel, http_only, work_dir):
    examples = Path(__file__).resolve().parents[1] / "examples"
    if work_dir:
        work_dir.mkdir(parents=True, exist_ok=False)
    with nullcontext(work_dir) if work_dir else TemporaryDirectory() as tmp, ExitStack() as servers:
        urls = {}
        for name in ("trail_report", "pointer_components"):
            project = Path(tmp) / name
            shutil.copytree(examples / name, project, ignore=shutil.ignore_patterns("target"))
            subprocess.run([cli, "check", "src/main.cor"], cwd=project, check=True)
            if name == "trail_report":
                subprocess.run([cli, "test", "src/main.cor"], cwd=project, check=True)
            subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
            installed = project / "installed"
            with tarfile.open(project / "target/web.tar") as archive:
                archive.extractall(installed, filter="data")
            urls[name] = servers.enter_context(serve(cli, installed, os.environ))
        report = read(urls["trail_report"])
        for markup in ['role="combobox"', 'role="alert"', 'command="show-modal"',
                       'type="file"', 'contenteditable="true"']:
            assert markup in report, markup
        assert "<h1>Ridge path</h1>" in read(urls["trail_report"] + "/trail?trail=Ridge+path")
        assert "corvid-combobox" in read(urls["trail_report"] + "/target/wasm/events.js")
        assert 'role="separator"' in read(urls["pointer_components"])
        print("Packaged report, browser module, query route and resize control passed HTTP checks.", flush=True)
        if not http_only:
            from playwright.sync_api import sync_playwright

            with sync_playwright() as engines:
                browser = engines.chromium.launch(channel=channel)
                page = browser.new_page(has_touch=True)
                check_report(page, urls["trail_report"])
                check_touch_resize(page, urls["pointer_components"])
                browser.close()
            print("Report interactions and emulated touch resize passed.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--http-only", action="store_true")
    parser.add_argument("--work-dir", type=Path, help="Keep packaged examples in a new directory")
    args = parser.parse_args()
    verify(Path(shutil.which(args.cli) or args.cli).resolve(), args.channel, args.http_only, args.work_dir)
