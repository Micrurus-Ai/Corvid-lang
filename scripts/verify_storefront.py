"""Build the storefront and drive the extracted package in a real browser."""
import argparse
from contextlib import nullcontext
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory
from urllib.error import HTTPError
from urllib.request import urlopen

from verify_reading_list import read, serve
from storefront_editor import check_editor


ROOT = Path(__file__).resolve().parents[1]


def checkout(page, notebooks, pencils):
    page.get_by_role("button", name="Checkout").press("Enter")
    page.get_by_role("heading", name="Checkout summary").wait_for()
    for line in (f"Notebooks: {notebooks}", f"Pencils: {pencils}",
                 f"Total: {450 * notebooks + 125 * pencils} cents"):
        assert page.get_by_text(line, exact=True).is_visible()


def trial(browser, url, width, enhanced):
    page = browser.new_page(java_script_enabled=enhanced, viewport={"width": width, "height": 720})
    try:
        pending = []
        if enhanced:
            page.route("**/events.js", lambda route: pending.append(route))
        page.goto(url)
        page.keyboard.press("Tab")
        assert page.get_by_role("link", name="Open cart").evaluate("e => e === document.activeElement")
        page.keyboard.press("Enter")
        notebook = page.get_by_label("Notebook quantity")
        notebook.wait_for()
        row = page.locator('[data-corvid-key="notebook"]')
        assert row.evaluate("e => getComputedStyle(e).display") == ("grid" if width >= 672 else "list-item")
        page.keyboard.press("Tab")  # Catalog
        page.keyboard.press("Tab")  # First quantity
        assert notebook.evaluate("e => e === document.activeElement")
        page.keyboard.press("Control+a")
        page.keyboard.type("12")
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        if enhanced:
            # Type before the module arrives. Binding must adopt this DOM value.
            field = notebook.element_handle()
            page.keyboard.press("Shift+ArrowLeft")
            assert len(pending) == 1
            pending[0].continue_()
            row.get_by_text("5400", exact=True).wait_for()
            page.keyboard.press("Alt+ArrowDown")
            assert page.locator("ol > li").last.get_attribute("data-corvid-key") == "notebook"
            assert field.evaluate("e => e === document.activeElement && e.selectionStart === 1 && e.selectionEnd === 2")
            page.keyboard.type("3")
            row.get_by_text("5850", exact=True).wait_for()
            row.get_by_role("button", name="Move Notebook earlier").press("Space")
            assert page.locator("ol > li").first.get_attribute("data-corvid-key") == "notebook"
            assert notebook.evaluate("(e, original) => e === original", field)
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        else:
            assert page.get_by_role("button", name="Move Notebook later").count() == 0
        assert page.get_by_label("Pencil quantity").input_value() == "1"
        checkout(page, 13 if enhanced else 12, 1)
    finally:
        page.close()


def picker_trial(browser, url, enhanced):
    page = browser.new_page(java_script_enabled=enhanced)
    try:
        for path, title, label, initial in (
            ("notebook", "Notebook cover", "Cover color", "Ocean"),
            ("pencil", "Pencil finish", "Barrel color", "Clay"),
        ):
            page.goto(f"{url}/{path}")
            page.get_by_role("heading", name=title).wait_for()
            assert page.get_by_text(f"{label}: {initial}", exact=True).is_visible()
            select = page.get_by_role("combobox", name="Color")
            assert select.locator("option:checked").inner_text() == initial
            with page.expect_request(lambda r: r.method == "POST" and r.url == f"{url}/{path}") as request:
                select.press("ArrowDown" if initial == "Ocean" else "ArrowUp")
                if not enhanced:
                    page.get_by_role("button", name="Apply color").press("Enter")
            assert request.value.post_data == "shade=1"
            page.get_by_text(f"{label}: Moss", exact=True).wait_for()
            assert select.locator("option:checked").inner_text() == "Moss"
    finally:
        page.close()


def touch_checkout(browser, url, device):
    from playwright.sync_api import expect

    page = browser.new_page(**device)
    try:
        page.goto(url)
        page.get_by_role("link", name="Open cart").tap()
        quantity = page.get_by_label("Notebook quantity")
        quantity.tap()
        expect(quantity).to_be_focused()
        assert quantity.get_attribute("inputmode") == "numeric"
        quantity.evaluate("field => field.select()")
        page.keyboard.insert_text("12")
        row = page.locator('[data-corvid-key="notebook"]')
        expect(row.get_by_text("5400", exact=True)).to_be_visible()
        row.get_by_role("button", name="Move Notebook later").tap()
        expect(page.get_by_text("notebook, position 2 of 2", exact=True)).to_be_visible()
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
        page.get_by_role("button", name="Checkout").tap()
        expect(page.get_by_text("Total: 5525 cents", exact=True)).to_be_visible()
    finally:
        page.close()


def check_picker_inputs(cli, project):
    for screen in ("notebook", "pencil"):
        path = project / "src" / (screen + ".cor")
        original = path.read_text(encoding="utf-8")
        try:
            path.write_text(original.replace("ColorPicker(value,", 'ColorPicker("Moss",'), encoding="utf-8")
            result = subprocess.run([cli, "check"], cwd=project, capture_output=True, text=True,
                                    encoding="utf-8")
            diagnostic = result.stdout + result.stderr
            assert result.returncode and "Color" in diagnostic and "String" in diagnostic, diagnostic
        finally:
            path.write_text(original, encoding="utf-8")


def check_rebuild(cli, project, installed, rebuilt):
    old_wasm = (installed / "target/wasm/events.wasm").read_bytes()
    for name in ("events.cor", "main.cor"):
        source = project / "src" / name
        source.write_text(source.read_text(encoding="utf-8").replace("450", "475"), encoding="utf-8")
    subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
    with tarfile.open(project / "target/web.tar") as archive:
        archive.extractall(rebuilt, filter="data")
    with serve(cli, rebuilt, os.environ.copy()) as url:
        cart = read(url + "/cart")
        assert "475 cents each" in cart and 'src="/target/wasm/events.js"' in cart
        assert "Total: 5825 cents" in read(url + "/checkout", b"notebook=12&pencil=1")
        for name, mime in (("events.js", "text/javascript"), ("events.wasm", "application/wasm")):
            path = "target/wasm/" + name
            with urlopen(f"{url}/{path}", timeout=5) as response:
                assert response.headers.get_content_type() == mime
                asset = response.read()
            assert asset and asset == (rebuilt / path).read_bytes()
            if name == "events.wasm":
                assert asset != old_wasm
    print("Rebuilt notebook price, checkout and served JS/Wasm assets passed")


def verify(cli, channel, http_only, work_dir, engine="chromium", lsp=None):
    if work_dir is not None:
        work_dir.mkdir(parents=True, exist_ok=False)
    with (nullcontext(work_dir) if work_dir is not None else TemporaryDirectory()) as tmp:
        root = Path(tmp)
        subprocess.run([cli, "new", "storefront"], cwd=root, check=True)
        project = root / "storefront"
        shutil.copytree(ROOT / "examples/storefront/src", project / "src", dirs_exist_ok=True)
        if lsp is not None:
            check_editor(lsp, project)
        check_picker_inputs(cli, project)
        for args in (["check"], ["test", "src/main.cor"], ["build", "src/main.cor", "--target", "web"]):
            subprocess.run([cli, *args], cwd=project, check=True)
        installed = root / "installed"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        check_rebuild(cli, project, installed, root / "rebuilt")
        with serve(cli, installed, os.environ.copy()) as url:
            assert 'href="/cart"' in read(url)
            assert 'action="/checkout"' in read(url + "/cart")
            assert "Total: 5525 cents" in read(url + "/checkout", b"notebook=12&pencil=1")
            for path, label, initial in (("notebook", "Cover", "Ocean"), ("pencil", "Barrel", "Clay")):
                assert f"{label} color: {initial}" in read(f"{url}/{path}")
                assert f"{label} color: Moss" in read(f"{url}/{path}", b"shade=1")
                for invalid in (b"shade=9", b"shade=green"):
                    try:
                        urlopen(f"{url}/{path}", data=invalid, timeout=5)
                    except HTTPError as error:
                        assert error.code == 400
                    else:
                        raise AssertionError(f"{path} accepted {invalid!r}")
            print("Packaged navigation, checkout and both typed Color routes passed")
            if http_only:
                return
            from playwright.sync_api import sync_playwright

            with sync_playwright() as playwright:
                run_browser(playwright, url, channel, engine)


def run_browser(playwright, url, channel, engine="chromium"):
    browser = getattr(playwright, engine).launch(channel=channel)
    try:
        for width in (320, 960):
            for enhanced in (False, True):
                trial(browser, url, width, enhanced)
                print(f"{engine} {browser.version}, {width}px, scripting={enhanced}: passed")
        for enhanced in (False, True):
            picker_trial(browser, url, enhanced)
            print(f"Both ColorPicker screens, scripting={enhanced}: passed")
        devices = {"chromium": "Pixel 7", "webkit": "iPhone 13"}
        if engine in devices:
            device = devices[engine]
            touch_checkout(browser, url, playwright.devices[device])
            print(f"{device} emulation: touch checkout and quantity entry passed")
    finally:
        browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--browser", choices=("chromium", "firefox", "webkit"))
    parser.add_argument("--channel", help="Chromium distribution (defaults to Edge when --browser is omitted)")
    parser.add_argument("--http-only", action="store_true")
    parser.add_argument("--lsp", type=Path, help="also exercise editor navigation and unsaved diagnostics")
    parser.add_argument("--work-dir", type=Path, help="keep the scaffold and extracted archive in a new directory")
    args = parser.parse_args()
    if args.channel and args.browser in ("firefox", "webkit"):
        parser.error("--channel requires a Chromium browser")
    channel = args.channel or ("msedge" if args.browser is None else None)
    verify(args.cli.resolve(), channel, args.http_only, args.work_dir, args.browser or "chromium",
           args.lsp.resolve() if args.lsp else None)
