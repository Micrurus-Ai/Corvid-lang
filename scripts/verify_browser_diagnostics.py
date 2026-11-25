"""Trigger a nested checkout error, fix the reported line, and retry in three engines."""
import argparse
from contextlib import ExitStack
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from playwright.sync_api import expect, sync_playwright
from verify_reading_list import serve
from verify_storefront import ROOT, checkout


def package(cli, project, destination):
    subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
    with tarfile.open(project / "target/web.tar") as archive:
        archive.extractall(destination, filter="data")


def verify(cli, channel, engines=("chromium", "firefox", "webkit")):
    with TemporaryDirectory() as tmp, ExitStack() as servers:
        root = Path(tmp)
        subprocess.run([cli, "new", "storefront"], cwd=root, check=True)
        project = root / "storefront"
        src = project / "src"
        shutil.copytree(ROOT / "examples/storefront/src", src, dirs_exist_ok=True)
        main = src / "main.cor"
        main.write_text(main.read_text(encoding="utf-8").replace("server shop:\n", '''server shop:
    route GET "/pencil-quantity" -> html:
        return shop_page("Pencils", [ui.quantity_field("Pencil count", "pencil", 1, "pencil_cents", "Cents", 125), ui.event_module()])

'''), encoding="utf-8")
        events = src / "events.cor"
        events.write_text('import "./pricing" use line_total\n' + events.read_text().replace(
            "quantity * 450", "line_total(quantity, 450)"), encoding="utf-8")
        pricing = src / "pricing.cor"
        broken = "    return quantity * unit_price / (quantity - 2)"
        pricing.write_text("public fn line_total(quantity: Int, unit_price: Int) -> Int:\n"
                           "    if quantity == 4:\n"
                           "        return unit_price % (quantity - 4)\n" +
                           broken + "\n", encoding="utf-8")
        package(cli, project, root / "broken")
        pricing.write_text(pricing.read_text().replace(broken, "    return quantity * unit_price"),
                           encoding="utf-8")
        package(cli, project, root / "fixed")
        urls = [servers.enter_context(serve(cli, root / variant, os.environ.copy()))
                for variant in ("broken", "fixed")]
        with sync_playwright() as playwright:
            for engine in engines:
                browser = getattr(playwright, engine).launch(channel=channel if engine == "chromium" else None)
                try:
                    page = browser.new_page()
                    page.goto(urls[0] + "/pencil-quantity")
                    page.get_by_label("Pencil count").fill("3")
                    expect(page.get_by_text("375", exact=True)).to_be_visible()
                    assets = page.evaluate("""performance.getEntriesByType('resource')
                      .filter(e => e.name.includes('/target/wasm/')).map(e => new URL(e.name).pathname)""")
                    assert assets == ["/target/wasm/events.quantity.js"], assets
                    page.goto(urls[0] + "/cart")
                    notebook = page.get_by_label("Notebook quantity")
                    notebook.fill("2")
                    row = page.locator('[data-corvid-key="notebook"]')
                    alert = row.get_by_role("alert")
                    expect(alert).to_contain_text("src/pricing.cor:4:12 in line_total (integer division)")
                    print(engine, alert.inner_text(), flush=True)
                    notebook.fill("4")
                    expect(alert).to_contain_text("src/pricing.cor:3:16 in line_total (integer remainder)")
                    notebook.fill("3")
                    expect(row.get_by_text("1350", exact=True)).to_be_visible()
                    page.goto(urls[1] + "/cart")
                    page.get_by_label("Notebook quantity").fill("2")
                    expect(row.get_by_text("900", exact=True)).to_be_visible()
                    checkout(page, 2, 1)
                    print(engine, "repaired checkout passed", flush=True)
                finally:
                    browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", help="Installed Chromium distribution, such as chrome")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
