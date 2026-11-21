"""Check catalog discovery and compare the live preview with the installed Corvid example."""

import argparse
import json
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
from urllib.request import urlopen

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
SERVER = """
import http from "node:http";
import worker from "./web/worker.js";
const server = http.createServer(async (req, res) => {
    const url = "http://127.0.0.1:" + server.address().port + req.url;
    const response = await worker.fetch(new Request(url));
    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(await response.text());
});
server.listen(0, "127.0.0.1", () => console.log(server.address().port));
"""


def check(origin, cli, channel):
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        page = browser.new_page()
        page.goto(origin + "/packages")
        page.get_by_label("Package name or task").fill("expand details")
        page.get_by_role("button", name="Search").click()
        page.get_by_role("link", name="@corvid/ui").click()
        example = page.locator("pre").last.inner_text()
        preview = page.locator("details").evaluate("(el) => el.outerHTML")
        page.locator("summary").press("Enter")
        assert page.locator("details").evaluate("(el) => el.open")
        assert page.goto(origin + "/packages/unknown").status == 404
        browser.close()

    with urlopen(origin + "/packages/catalog.json?q=nonexistent") as response:
        assert json.load(response) == []
    with TemporaryDirectory() as tmp:
        subprocess.run([cli, "new", "sample"], cwd=tmp, check=True)
        app = Path(tmp) / "sample"
        subprocess.run([cli, "add", "package", "@corvid/ui@0.0.1",
                        "--registry", origin + "/packages/index.json"], cwd=app, check=True)
        (app / "src/main.cor").write_text(example + "\n", encoding="utf-8")
        result = subprocess.check_output([cli, "run", "src/main.cor"], cwd=app, text=True)
        assert json.loads(result.strip()) == preview


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="msedge")
    args = parser.parse_args()
    server = subprocess.Popen(["node", "--input-type=module", "-e", SERVER],
                              cwd=ROOT, stdout=subprocess.PIPE, text=True)
    try:
        port = int(server.stdout.readline())
        check(f"http://127.0.0.1:{port}", args.cli.resolve(), args.channel)
        print("Catalog search, keyboard preview and clean-project import passed.")
    finally:
        server.terminate()
        server.wait()
