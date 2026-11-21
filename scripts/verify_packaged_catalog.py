"""Test the installed Windows CLI and VSIX with an empty editor profile and the live catalog."""
import argparse
import json
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
import time
import zipfile

from playwright.sync_api import sync_playwright


def verify(archive, editor):
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        with zipfile.ZipFile(archive) as package:
            package.extractall(root / "install")
        cli, = (root / "install").rglob("corvid.exe")
        vsix, = (root / "install").rglob("*.vsix")
        env = os.environ.copy()
        for key in ("ELECTRON_RUN_AS_NODE", "CORVID_PACKAGE_REGISTRY", "CORVID_OFFLINE",
                    "CORVID_LSP_PATH", "CORVID_STD_PATH"):
            env.pop(key, None)
        env["PATH"] = str(cli.parent) + os.pathsep + env["PATH"]
        subprocess.run([cli, "new", "reader"], cwd=root, env=env, check=True)
        project = root / "reader"
        main = project / "src/main.cor"
        main.write_text('agent main() -> String:\n    return ui.disclosure("Details", [ui.text("Hello")]).html\n')
        profile = root / "profile"
        args = ["--user-data-dir=" + str(profile), "--extensions-dir=" + str(root / "extensions")]
        cli_script, = editor.parent.glob("**/resources/app/out/cli.js")
        subprocess.run([editor, cli_script, *args, "--install-extension", vsix],
                       cwd=root, env=dict(env, ELECTRON_RUN_AS_NODE="1"), check=True, timeout=120)
        app = subprocess.Popen([editor, *args, "--disable-workspace-trust", "--disable-updates",
                                "--skip-welcome", "--skip-release-notes", "--remote-debugging-port=0",
                                project, main], cwd=root, env=env)
        try:
            port_file = profile / "DevToolsActivePort"
            for _ in range(200):
                if port_file.exists() and port_file.stat().st_size:
                    break
                if app.poll() is not None:
                    raise RuntimeError(f"VS Code exited with {app.returncode}")
                time.sleep(0.25)
            port = int(port_file.read_text().splitlines()[0])
            with sync_playwright() as playwright:
                browser = playwright.chromium.connect_over_cdp(f"http://127.0.0.1:{port}")
                context = browser.contexts[0]
                for _ in range(200):
                    pages = [p for p in context.pages if "workbench" in p.url]
                    if pages:
                        break
                    time.sleep(0.25)
                page = pages[0]
                page.locator(".monaco-workbench").wait_for()
                page.bring_to_front()
                page.locator(".editor-instance .view-lines .view-line").first.click()
                page.keyboard.press("F1")
                field = page.locator(".quick-input-widget input:visible").first
                field.wait_for()
                page.keyboard.insert_text("Corvid: Add Package Import")
                page.get_by_text("Corvid: Add Package Import", exact=True).last.click()
                page.get_by_text("@corvid/ui", exact=True).last.wait_for()
                field.fill("expand details")
                page.get_by_text("@corvid/ui", exact=True).last.click()
                page.keyboard.press("Control+s")
                for _ in range(40):
                    if "corvid://@corvid/ui" in main.read_text():
                        break
                    time.sleep(0.25)
                assert "corvid://@corvid/ui" in main.read_text(), main.read_text()
                browser.close()
        finally:
            if app.poll() is None:
                app.terminate()
            app.wait(timeout=30)
        result = subprocess.check_output([cli, "run", "src/main.cor"], cwd=project, env=env, text=True)
        assert json.loads(result.strip()) == "<details><summary>Details</summary><p>Hello</p></details>"
        assert "corvid://@corvid/ui" in (project / "Corvid.lock").read_text()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("editor", type=Path)
    args = parser.parse_args()
    verify(args.archive.resolve(), args.editor.resolve())
    print("Installed VSIX found the live package by task, inserted it, and the installed CLI ran it.")
