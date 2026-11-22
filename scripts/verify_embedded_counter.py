"""Exercise a Corvid custom element on a standalone JavaScript host."""

import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import shutil
from tempfile import TemporaryDirectory
from threading import Thread

from playwright.sync_api import expect, sync_playwright

from export_browser_component import build


def verify(cli, channel):
    example = Path(__file__).resolve().parents[1] / "examples/embedded_counter"
    with TemporaryDirectory() as tmp:
        project = Path(tmp) / "component"
        shutil.copytree(example, project, ignore=shutil.ignore_patterns("target"))
        package = build(cli, project)
        host = Path(tmp) / "host"
        shutil.copytree(package, host / "widgets/counter")
        html = (example / "index.html").read_text().replace("./target/wasm/", "./widgets/counter/")
        (host / "index.html").write_text(html)
        handler = partial(SimpleHTTPRequestHandler, directory=host)
        with ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
            Thread(target=server.serve_forever, daemon=True).start()
            try:
                with sync_playwright() as playwright:
                    browser = playwright.chromium.launch(channel=channel)
                    page = browser.new_page()
                    page.goto(f"http://127.0.0.1:{server.server_port}")
                    morning, evening = (page.get_by_role("region", name=name)
                                        for name in ("Morning", "Evening"))
                    morning.get_by_role("button").press("Enter")
                    expect(morning.locator("output")).to_have_text("1")
                    expect(evening.locator("output")).to_have_text("0")
                    morning.locator("reading-counter").evaluate("(el) => el.parentElement.append(el)")
                    morning.get_by_role("button").click()
                    expect(morning.locator("output")).to_have_text("2")
                    evening.get_by_role("button").press("Space")
                    expect(evening.locator("output")).to_have_text("1")
                    page.reload()
                    expect(morning.locator("output")).to_have_text("0")
                    browser.close()
            finally:
                server.shutdown()
    print("Passed static host import, keyboard activation, independent instances, remount and reload.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="chrome")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
