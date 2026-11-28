"""Publish the storefront picker to a test registry and exercise a separate consumer."""
import argparse
import os
from pathlib import Path
import secrets
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from playwright.sync_api import sync_playwright
from verify_reading_list import serve

PICKER = Path(__file__).resolve().parents[1] / "examples/storefront/src/color_picker.cor"
SOURCE = '''import "corvid://@studio/color-picker/v1.0.0" use ColorPicker, Color, color_name
import "std:ui" as ui

fn settings(value: Color) -> String:
    return ui.interactive_page("Theme", [ui.text("Theme: " + color_name(value)), ColorPicker(value, "/theme")])

agent main() -> String:
    return color_name(Color(0))

traces: none

server theme_editor:
    route GET "/" -> html:
        return settings(Color(0))
    route POST "/theme" form Color -> html:
        return settings(body)
'''


def verify(cli, channel):
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        registry = root / "registry"
        result = subprocess.run([
            cli, "package", "publish", PICKER, "--name", "@studio/color-picker",
            "--version", "1.0.0", "--out", registry, "--url-base", registry.as_uri(),
            "--key", secrets.token_hex(32), "--key-id", "trial",
        ], cwd=root, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(result.stderr)
        subprocess.run([cli, "new", "theme"], cwd=root, check=True)
        project = root / "theme"
        main = project / "src/main.cor"
        main.write_text(SOURCE, encoding="utf-8")
        env = dict(os.environ, CORVID_PACKAGE_REGISTRY=str(registry))
        subprocess.run([cli, "run", "src/main.cor"], cwd=project, env=env, check=True)
        main.write_text(SOURCE.replace("ColorPicker(value,", 'ColorPicker("Moss",'), encoding="utf-8")
        bad = subprocess.run([cli, "check"], cwd=project, capture_output=True, text=True)
        assert bad.returncode and "Color" in bad.stderr and "String" in bad.stderr, bad.stderr
        main.write_text(SOURCE, encoding="utf-8")
        subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
        installed = root / "installed"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        with serve(cli, installed, env) as url, sync_playwright() as playwright:
            browser = playwright.chromium.launch(channel=channel)
            for enhanced in (False, True):
                page = browser.new_page(java_script_enabled=enhanced)
                page.goto(url)
                select = page.get_by_role("combobox", name="Color")
                with page.expect_request(lambda r: r.method == "POST" and r.url == url + "/theme"):
                    select.press("ArrowDown")
                    if not enhanced:
                        page.get_by_role("button", name="Apply color").press("Enter")
                page.get_by_text("Theme: Moss", exact=True).wait_for()
                assert select.locator("option:checked").inner_text() == "Moss"
                page.close()
            browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="msedge")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
    print("Signed picker resolved, packaged, type-checked and operated by keyboard.")
