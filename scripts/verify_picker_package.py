"""Install the storefront picker into a new app using a disposable file registry."""
import argparse
import hashlib
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory
from urllib.error import HTTPError

from verify_reading_list import read, serve


ROOT = Path(__file__).resolve().parents[1]


def check_binding(cli, project, env):
    main = project / "src/main.cor"
    source = main.read_text(encoding="utf-8")
    line = next(i for i, text in enumerate(source.splitlines(), 1) if "ColorPicker(value," in text)
    try:
        main.write_text(source.replace("ColorPicker(value,", 'ColorPicker("Moss",'), encoding="utf-8")
        result = subprocess.run([cli, "check"], cwd=project, env=env, capture_output=True,
                                text=True, encoding="utf-8")
        diagnostic = result.stdout + result.stderr
        assert result.returncode != 0, diagnostic
        assert f"{main}:{line}:" in diagnostic, diagnostic
        assert "argument 1 to `ColorPicker`: expected `Color`, got `String`" in diagnostic, diagnostic
    finally:
        main.write_text(source, encoding="utf-8")


def check_keyboard(url, channel):
    from playwright.sync_api import expect, sync_playwright

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        for enhanced in (True, False):
            page = browser.new_page(java_script_enabled=enhanced)
            page.goto(url)
            select = page.get_by_role("combobox", name="Color")
            expect(page.locator("form > div")).to_have_css("flex-wrap", "wrap")
            page.keyboard.press("Tab")
            expect(select).to_be_focused()
            with page.expect_request(lambda r: r.method == "POST" and r.url == url + "/") as request:
                page.keyboard.press("ArrowDown")
                if not enhanced:
                    page.keyboard.press("Tab")
                    page.keyboard.press("Enter")
            assert request.value.post_data == "shade=1"
            expect(page.get_by_text("Selected: Moss", exact=True)).to_be_visible()
            expect(select).to_have_value("1")
            page.close()
        browser.close()


def verify(cli, root, browser_channel):
    registry = root / "registry"
    source = ROOT / "examples/storefront/src/color_picker.cor"
    name = "@trial/color-picker"
    subprocess.run([cli, "package", "publish", source, "--name", name, "--version", "1.0.0",
                    "--out", registry, "--url-base", registry.as_uri(),
                    "--key", secrets.token_hex(32), "--key-id", "local-trial"], check=True)
    subprocess.run([cli, "new", "palette"], cwd=root, check=True)
    project = root / "palette"
    subprocess.run([cli, "add", "package", name + "@1.0.0", "--registry", registry / "index.json"],
                   cwd=project, check=True)
    subprocess.run([cli, "package", "verify-lock"], cwd=project, check=True)
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    cached = project / "target/cache/packages" / (digest + ".cor")
    assert cached.read_bytes() == source.read_bytes()
    registry.rename(root / "disconnected-registry")
    shutil.copyfile(ROOT / "examples/palette/src/main.cor", project / "src/main.cor")
    env = dict(os.environ, CORVID_OFFLINE="1")
    check_binding(cli, project, env)
    for args in (["check"], ["test", "src/main.cor"], ["build", "src/main.cor", "--target", "web"]):
        subprocess.run([cli, *args], cwd=project, env=env, check=True)
    installed = root / "installed"
    with tarfile.open(project / "target/web.tar") as archive:
        archive.extractall(installed, filter="data")
    with serve(cli, installed, env) as url:
        initial = read(url)
        assert "Selected: Ocean" in initial
        assert '<form method="post" action="/"><div style="display:flex;flex-wrap:wrap;' in initial
        assert 'name="shade" data-corvid-submit-change' in initial
        assert 'src="/target/wasm/events.js"' in initial
        assert "data-corvid-submit-change" in read(url + "/target/wasm/events.js")
        updated = read(url, b"shade=1")
        assert "Selected: Moss" in updated and '<option value="1" selected>Moss</option>' in updated
        try:
            read(url, b"shade=3")
        except HTTPError as error:
            assert error.code == 400, error
        else:
            raise AssertionError("out-of-range shade was accepted")
        if browser_channel:
            check_keyboard(url, browser_channel)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--work-dir", type=Path, help="retain the trial in a new directory")
    parser.add_argument("--browser-channel", help="also check computed style and keyboard use, e.g. msedge")
    args = parser.parse_args()
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=False)
        verify(args.cli.resolve(), args.work_dir.resolve(), args.browser_channel)
    else:
        with TemporaryDirectory() as directory:
            verify(args.cli.resolve(), Path(directory).resolve(), args.browser_channel)
