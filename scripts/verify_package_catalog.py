"""Check catalog discovery and compare the live preview with the installed Corvid example."""

import argparse
import json
import os
import shutil
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


def check(origin, cli, channel, editor):
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
        main = app / "src/main.cor"
        main.write_text(example + "\n", encoding="utf-8")
        if editor:
            main.write_text(example.split("\n", 1)[1].lstrip("\n") + "\n", encoding="utf-8")
            profile = Path(tmp) / "profile"
            settings = profile / "User/settings.json"
            settings.parent.mkdir(parents=True)
            settings.write_text(json.dumps({"corvid.catalog.url": origin + "/packages/catalog.json"}))
            extension = ROOT / "extensions/vscode-corvid"
            editor_env = os.environ.copy()
            editor_env.pop("ELECTRON_RUN_AS_NODE", None)
            subprocess.run([editor, "--disable-extensions", "--disable-workspace-trust",
                            "--disable-updates", "--skip-welcome", "--wait",
                            "--user-data-dir=" + str(profile),
                            "--extensions-dir=" + str(Path(tmp) / "extensions"),
                            "--extensionDevelopmentPath=" + str(extension),
                            "--extensionTestsPath=" + str(extension / "scripts/catalog-host.cjs"),
                            str(app)], check=True, timeout=100, env=editor_env)
            assert main.read_text(encoding="utf-8").startswith(example.splitlines()[0])
        env = os.environ.copy()
        env.pop("CORVID_PACKAGE_REGISTRY", None)
        env.pop("CORVID_OFFLINE", None)

        def run(directory, environment):
            result = subprocess.check_output([cli, "run", "src/main.cor"], cwd=directory,
                                             text=True, env=environment)
            assert json.loads(result.strip()) == preview

        # The import itself must be enough, with no add/install step.
        offline = subprocess.run([cli, "run", "src/main.cor"], cwd=app, capture_output=True,
                                 text=True, env=dict(env, CORVID_OFFLINE="1"))
        assert offline.returncode and "offline" in offline.stderr
        assert not (app / "Corvid.lock").exists() or "@corvid/ui" not in (app / "Corvid.lock").read_text()
        run(app, env)
        lock = (app / "Corvid.lock").read_bytes()
        checkout = Path(tmp) / "checkout"
        shutil.copytree(app, checkout, ignore=shutil.ignore_patterns("target"))
        # A fresh checkout must use the pinned artifact even if its registry is unavailable.
        env["CORVID_PACKAGE_REGISTRY"] = "http://127.0.0.1:1/unavailable"
        run(checkout, env)
        run(checkout, dict(env, CORVID_OFFLINE="1"))
        assert (checkout / "Corvid.lock").read_bytes() == lock
        source = checkout / "src/main.cor"
        source.write_text(example.replace("/v0.0.1", "/v0.0.2"), encoding="utf-8")
        conflict = subprocess.run([cli, "run", "src/main.cor"], cwd=checkout, capture_output=True,
                                  text=True, env=env)
        assert conflict.returncode and "conflicts with a locked version" in conflict.stderr
        assert (checkout / "Corvid.lock").read_bytes() == lock

        # An explicitly selected registry serves the same import as the built-in catalog.
        subprocess.run([cli, "new", "custom"], cwd=tmp, check=True)
        custom = Path(tmp) / "custom"
        (custom / "src/main.cor").write_text(example, encoding="utf-8")
        run(custom, dict(env, CORVID_PACKAGE_REGISTRY=origin + "/packages/index.json"))
        assert origin + "/packages/index.json" in (custom / "Corvid.lock").read_text()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--editor", type=Path, help="also exercise Add Package Import in a VS Code extension host")
    args = parser.parse_args()
    server = subprocess.Popen(["node", "--input-type=module", "-e", SERVER],
                              cwd=ROOT, stdout=subprocess.PIPE, text=True)
    try:
        port = int(server.stdout.readline())
        check(f"http://127.0.0.1:{port}", args.cli.resolve(), args.channel, args.editor)
        print("Catalog search, keyboard preview and clean-project import passed.")
    finally:
        server.terminate()
        server.wait()
