"""Build the storefront and drive the extracted package in a real browser."""
import argparse
import base64
from contextlib import nullcontext
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory
from urllib.error import HTTPError
from urllib.request import Request, urlopen

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
            page.route("**/events.quantity.js", lambda route: pending.append(route))
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
        updated = source.read_text(encoding="utf-8").replace("450", "475")
        if name == "events.cor":
            updated = updated.replace("return quantity * 125", "units = quantity\n    return units * 125")
        source.write_text(updated, encoding="utf-8")
    subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
    with tarfile.open(project / "target/web.tar") as archive:
        archive.extractall(rebuilt, filter="data")
    with serve(cli, rebuilt, os.environ.copy()) as url:
        cart = read(url + "/cart")
        assert "475 cents each" in cart and 'src="/target/wasm/events.quantity.js"' in cart
        slim = read(url + "/target/wasm/events.quantity.js")
        assert "adjusted_units" not in slim and "registerControlTool" not in slim
        digest = hashlib.sha256(slim.encode()).digest()
        assert f'integrity="sha256-{base64.b64encode(digest).decode()}"' in cart
        runtime = (rebuilt / "target/wasm/events.js").read_bytes()
        pinned = "./events.js?v=" + hashlib.sha256(runtime).hexdigest()
        assert pinned in slim
        for module in (slim, runtime.decode()):
            encoded = module.split("//# sourceMappingURL=data:application/json;base64,")[1]
            source_map = json.loads(base64.b64decode(encoded))
            assert source_map["version"] == 3 and "src/events.cor" in source_map["sources"]
            assert source_map["mappings"].strip(";")
        runtime_url = url + "/target/wasm/" + pinned[2:]
        with urlopen(runtime_url, timeout=5) as response:
            assert response.read() == runtime and "immutable" in response.headers["Cache-Control"]
            etag = response.headers["ETag"]
        old_runtime = (installed / "target/wasm/events.js").read_bytes()
        stale_url = url + "/target/wasm/events.js?v=" + hashlib.sha256(old_runtime).hexdigest()
        for request, status in ((Request(runtime_url, headers={"If-None-Match": etag}), 304),
                                (stale_url, 404)):
            try:
                urlopen(request, timeout=5)
            except HTTPError as error:
                assert error.code == status
                if status == 404:
                    assert error.headers["Cache-Control"] == "no-store"
            else:
                raise AssertionError(f"expected HTTP {status}")
        assert 'src="/target/wasm/events.js"' in read(url + "/notebook")
        assert "Total: 5825 cents" in read(url + "/checkout", b"notebook=12&pencil=1")
        for name, mime in (("events.quantity.js", "text/javascript"), ("events.js", "text/javascript"), ("events.wasm", "application/wasm")):
            path = "target/wasm/" + name
            with urlopen(f"{url}/{path}", timeout=5) as response:
                assert response.headers.get_content_type() == mime
                asset = response.read()
            assert asset and asset == (rebuilt / path).read_bytes()
            if name == "events.wasm":
                assert asset != old_wasm
    print("Rebuilt notebook price, checkout and served JS/Wasm assets passed")


def check_browser_imports(cli, project):
    events = project / "src/events.cor"
    source = events.read_bytes()
    helper = project / "src/pricing.cor"
    helper.write_text('import "std:secrets.cor" as secrets\n\n'
                      'public fn price() -> Int:\n    return 1\n', encoding="utf-8")
    try:
        events.write_bytes(b'import "./pricing" use price\n' + source)
        build = subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project,
                               capture_output=True, text=True, encoding="utf-8")
        assert build.returncode and "pricing.cor:1:" in build.stderr and "server-only" in build.stderr, build.stderr
    finally:
        events.write_bytes(source)
        helper.unlink()


def check_static_catalog(cli, project):
    entry = project / "src/static_catalog.cor"
    entry.write_text('''import "std:io" use io_write_text
import "std:ui" as ui
import "./layout" use shop_page

agent main() -> Result<String, String>:
    catalog = shop_page("Catalog", [ui.h1("Catalog"), ui.link("Open cart", "/cart")])
    io_write_text("catalog.html", catalog)?
    return Ok("catalog.html")
''', encoding="utf-8")
    try:
        subprocess.run([cli, "run", "--target", "interp", "src/static_catalog.cor"],
                       env={**os.environ, "CORVID_IO_ROOT": str(project.resolve())}, cwd=project, check=True)
        markup = (project / "catalog.html").read_text(encoding="utf-8")
        assert 'href="/cart"' in markup and "<script" not in markup
    finally:
        entry.unlink()


def check_component_locations(cli, root):
    project = root / "reading_counter"
    subprocess.run([cli, "new", project.name], cwd=root, check=True)
    shutil.copyfile(ROOT / "examples/reading_counter/src/main.cor", project / "src/main.cor")
    subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
    with tarfile.open(project / "target/web.tar") as archive:
        loader = archive.extractfile("target/wasm/events.js").read().decode()
        encoded = loader.split("//# sourceMappingURL=data:application/json;base64,")[1]
        assert json.loads(base64.b64decode(encoded))["sources"] == ["src/main.cor"]
        assert '"__click_ReadingCounter_1","src/main.cor:6:5"' in loader
        assert '"__click_ReadingCounter_2","src/main.cor:9:5"' in loader


def verify(cli, channel, http_only, work_dir, engine="chromium", lsp=None):
    if work_dir is not None:
        work_dir.mkdir(parents=True, exist_ok=False)
    with (nullcontext(work_dir) if work_dir is not None else TemporaryDirectory()) as tmp:
        root = Path(tmp)
        check_component_locations(cli, root)
        subprocess.run([cli, "new", "storefront"], cwd=root, check=True)
        project = root / "storefront"
        shutil.copytree(ROOT / "examples/storefront/src", project / "src", dirs_exist_ok=True)
        check_static_catalog(cli, project)
        main = project / "src/main.cor"
        main.write_text(main.read_text(encoding="utf-8").replace("server shop:", '''server shop:
    route GET "/empty-catalog" -> html:
        products: List<View> = []
        return catalog(products)

    route GET "/large-catalog" -> html:
        editions: List<View> = []
        for number in range(1, 1001):
            editions.append(ui.link("Notebook edition " + number.to_string(), "/notebook"))
        return catalog(editions)

    route GET "/cart-note" -> html:
        quantity = ui.quantity_field("Quantity", "quantity", 1, "pencil_cents", "Total", 125)
        note = ui.portal("notice", [ui.text("Cart note")])
        return ui.interactive_page("Cart", [quantity, note])
'''), encoding="utf-8")
        with (project / "src/events.cor").open("a", encoding="utf-8") as source:
            source.write("\npublic agent adjusted_units(units: Int, offset: Int) -> Int:\n"
                         "    return (units + offset) * 2 - units\n")
        if lsp is not None:
            check_editor(lsp, project)
        check_picker_inputs(cli, project)
        for args in (["check"], ["test", "src/main.cor"], ["build", "src/main.cor", "--target", "web"]):
            subprocess.run([cli, *args], cwd=project, check=True)
        installed = root / "installed"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        subprocess.run(["node", ROOT / "scripts/check_storefront_source_maps.mjs", installed], check=True)
        archive_path = project / "target/web.tar"
        first_build = archive_path.read_bytes()
        subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
        assert archive_path.read_bytes() == first_build
        check_browser_imports(cli, project)
        assert archive_path.read_bytes() == first_build
        check_rebuild(cli, project, installed, root / "rebuilt")
        with serve(cli, installed, os.environ.copy()) as url:
            assert 'href="/cart"' in read(url)
            assert "No products available." in read(url + "/empty-catalog")
            assert read(url + "/large-catalog").count('href="/notebook"') == 1000
            assert 'action="/checkout"' in read(url + "/cart")
            assert 'src="/target/wasm/events.js"' in read(url + "/cart-note")
            assert "Total: 5525 cents" in read(url + "/checkout", b"notebook=12&pencil=1")
            assert "Your cart is empty" in read(url + "/checkout", b"notebook=0&pencil=0")
            try:
                read(url + "/checkout", b"notebook=100&pencil=99")
            except HTTPError as error:
                assert error.code == 400
            else:
                raise AssertionError("Checkout accepted 100 notebooks")
            assert "Total: 56925 cents" in read(url + "/checkout", b"notebook=99&pencil=99")
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
                with serve(cli, root / "rebuilt", os.environ.copy()) as rebuilt_url:
                    run_browser(playwright, url, channel, engine, rebuilt_url)


def run_browser(playwright, url, channel, engine="chromium", rebuilt_url=None):
    browser = getattr(playwright, engine).launch(channel=channel)
    try:
        check_quantity_mismatch(browser, url)
        with browser.new_page(viewport={"width": 320, "height": 720}) as page:
            page.goto(url + "/large-catalog")
            page.get_by_role("link", name="Notebook edition 1000", exact=True).click()
            page.get_by_role("heading", name="Notebook cover").wait_for()
            page.goto(url + "/empty-catalog")
            page.get_by_text("No products available.", exact=True).wait_for()
            page.get_by_role("link", name="Open cart").click()
            for label in ("Notebook quantity", "Pencil quantity"):
                page.get_by_label(label).fill("0")
            page.get_by_role("button", name="Checkout").press("Enter")
            page.get_by_role("heading", name="Your cart is empty").wait_for()
            page.get_by_role("link", name="Return to cart").click()
            for label in ("Notebook quantity", "Pencil quantity"):
                page.get_by_label(label).fill("99")
            checkout(page, 99, 99)
        if rebuilt_url:
            check_split_runtime(browser, rebuilt_url)
        with browser.new_page() as page:
            page.goto(url)
            observed = page.evaluate("""async () => {
                const {instantiate} = await import('/target/wasm/events.js');
                const runs = [];
                for (const backend of [undefined, 'wasm']) {
                    const trace = [];
                    const handlers = await instantiate({}, {backend, trace});
                    const results = [];
                    for (const n of [0n, -1n, (1n<<63n)-1n, -(1n<<63n), 1n<<65n, '12']) {
                        const result = handlers.notebook_cents(n);
                        if (result !== BigInt.asIntN(64, BigInt(n) * 450n)) throw Error('incorrect i64 result');
                        results.push(String(result));
                    }
                    try { handlers.notebook_cents(1); } catch (e) {
                        if (!(e instanceof TypeError)) throw e;
                        results.push(e.name);
                    }
                    if (results.at(-1) !== 'TypeError' || trace.length !== 21 ||
                        trace.at(-1).ok !== false || !trace.at(-1).error) throw Error('missing failure trace');
                    for (const [units, offset] of [[(1n<<63n)-1n, 1n], [-(1n<<63n), -3n], [-7n, 2n]]) {
                        const adjusted = handlers.adjusted_units(units, offset);
                        if (adjusted !== BigInt.asIntN(64, units + 2n * offset)) throw Error('nested arithmetic');
                        results.push(String(adjusted));
                    }
                    runs.push([results, trace.map(({ts_ms, run_id, error, ...event}) => event)]);
                }
                return runs;
            }""")
            assert observed[0] == observed[1], observed
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


def check_split_runtime(browser, url):
    from playwright.sync_api import expect

    for wasm_allowed in (True, False):
        with browser.new_page() as page:
            def serve_cart(route):
                response = route.fetch()
                policy = "script-src 'self'" + (" 'wasm-unsafe-eval'" if wasm_allowed else "")
                route.fulfill(response=response, headers={**response.headers, "content-security-policy": policy})

            page.route("**/cart", serve_cart)
            with page.expect_request("**/events.js?v=*"):
                page.goto(url + "/cart")
            page.get_by_label("Notebook quantity").fill("2")
            expect(page.get_by_text("950", exact=True)).to_be_visible()
            pencil = page.locator('[data-corvid-key="pencil"]')
            page.get_by_label("Pencil quantity").fill("3")
            if wasm_allowed:
                expect(pencil.get_by_text("375", exact=True)).to_be_visible()
            else:
                expect(pencil.get_by_role("alert")).to_contain_text("Could not load Corvid event handlers")
            page.get_by_role("button", name="Checkout").press("Enter")
            expect(page.get_by_text("Total: 1325 cents", exact=True)).to_be_visible()


def check_quantity_mismatch(browser, url):
    from playwright.sync_api import expect

    with browser.new_page() as page:
        def serve_cart(route):
            response = route.fetch()
            markup = response.text().replace("<output>450</output>", "", 1)
            assert markup != response.text()
            route.fulfill(body=markup, content_type="text/html; charset=utf-8",
                          headers={"content-security-policy": "script-src 'self'; object-src 'none'"})

        errors = []
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.route("**/cart", serve_cart)
        page.goto(url + "/cart")
        damaged = page.locator('[data-corvid-key="notebook"]')
        expect(damaged.get_by_role("alert")).to_contain_text("src/events.cor:")
        page.get_by_label("Pencil quantity").fill("3")
        expect(page.get_by_text("375", exact=True)).to_be_visible()
        checkout(page, 1, 3)
        assert not errors, errors

    # A tab obtained its HTML before the cached entry module changed.
    with browser.new_page() as page:
        page.route("**/events.quantity.js", lambda route: route.fulfill(
            content_type="text/javascript", body="document.body.dataset.wrongBuild = 'yes'"))
        with page.expect_request("**/events.quantity.js"):
            page.goto(url + "/cart")
        assert page.locator("body").get_attribute("data-wrong-build") is None
        page.get_by_label("Notebook quantity").fill("2")
        checkout(page, 2, 1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", nargs="?", type=Path)
    parser.add_argument("--cli", dest="cli_option", type=Path)
    parser.add_argument("--browser", choices=("chromium", "firefox", "webkit"))
    parser.add_argument("--channel", help="Chromium distribution (defaults to Edge when --browser is omitted)")
    parser.add_argument("--http-only", action="store_true")
    parser.add_argument("--lsp", type=Path, help="also exercise editor navigation and unsaved diagnostics")
    parser.add_argument("--work-dir", type=Path, help="keep the scaffold and extracted archive in a new directory")
    args = parser.parse_args()
    if args.channel and args.browser in ("firefox", "webkit"):
        parser.error("--channel requires a Chromium browser")
    channel = args.channel or ("msedge" if args.browser is None else None)
    cli = args.cli_option or args.cli
    if cli is None:
        parser.error("supply --cli PATH")
    verify(cli.resolve(), channel, args.http_only, args.work_dir, args.browser or "chromium",
           args.lsp.resolve() if args.lsp else None)
