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
from storefront_editor import frame, responses


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


def check_package_editor(lsp, project, cached, env):
    main = (project / "src/main.cor").resolve()
    source = '''import "corvid://@trial/color-picker/v1.0.0" as picker
import "std:ui" use View

fn palette() -> View:
    return picker.ColorPicker(picker.Color(0), "/")
'''
    line, call = next((i, s) for i, s in enumerate(source.splitlines()) if "picker.ColorPicker(" in s)
    document = {"uri": main.as_uri()}
    cursor = {"textDocument": document, "position": {"line": line, "character": call.index("ColorPicker") + 3}}
    messages = [
        {"id": 1, "method": "initialize", "params": {"rootUri": project.as_uri(), "capabilities": {}}},
        {"method": "initialized", "params": {}},
        {"method": "textDocument/didOpen", "params": {"textDocument": {
            **document, "text": source, "languageId": "corvid", "version": 1}}},
        {"id": 2, "method": "textDocument/completion", "params": cursor},
        {"id": 3, "method": "textDocument/definition", "params": cursor},
        {"id": 4, "method": "shutdown"}, {"method": "exit"},
    ]
    result = subprocess.run([lsp], cwd=project, env=env, input=b"".join(map(frame, messages)),
                            stdout=subprocess.PIPE, timeout=60, check=True)
    replies = {r["id"]: r["result"] for r in responses(result.stdout) if "id" in r}
    assert "ColorPicker" in [item["label"] for item in replies[2]], replies[2]
    location = replies[3]
    assert location["uri"].lower() == cached.as_uri().lower(), location
    line = cached.read_text(encoding="utf-8").splitlines()[location["range"]["start"]["line"]]
    assert "public fn ColorPicker(" in line, line


def verify(cli, root, browser_channel, lsp=None):
    registry = root / "registry"
    source = ROOT / "examples/storefront/src/color_picker.cor"
    name = "@trial/color-picker"
    key = secrets.token_hex(32)
    def publish(path, version):
        subprocess.run([cli, "package", "publish", path, "--name", name, "--version", version,
                        "--out", registry, "--url-base", registry.as_uri(),
                        "--key", key, "--key-id", "local-trial"], check=True)

    publish(source, "1.0.0")
    subprocess.run([cli, "new", "palette", "--web"], cwd=root, check=True)
    project = root / "palette"
    subprocess.run([cli, "check"], cwd=project, check=True)
    subprocess.run([cli, "test", "src/main.cor"], cwd=project, check=True)
    with serve(cli, project, os.environ.copy()) as url:
        assert "Hello, Corvid!" in read(url)
    subprocess.run([cli, "add", "package", name + "@1.0.0", "--registry", registry / "index.json"],
                   cwd=project, check=True)
    subprocess.run([cli, "package", "verify-lock"], cwd=project, check=True)
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    cached = project / "target/cache/packages" / (digest + ".cor")
    assert cached.read_bytes() == source.read_bytes()
    registry.rename(root / "disconnected-registry")
    shutil.copyfile(ROOT / "examples/palette/src/main.cor", project / "src/main.cor")
    env = dict(os.environ, CORVID_OFFLINE="1")
    if lsp:
        check_package_editor(lsp, project, cached, env)
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

    # Version 2 asks the caller to supply the field label.
    (root / "disconnected-registry").rename(registry)
    revised = root / "picker_v2.cor"
    revised.write_text(source.read_text(encoding="utf-8").replace(
        "change_path: String)", "change_path: String, label: String)").replace(
        'select("Color",', 'select(label,'), encoding="utf-8")
    publish(revised, "2.0.0")
    subprocess.run([cli, "add", "package", name + "@2.0.0", "--registry", registry / "index.json"],
                   cwd=project, check=True)
    subprocess.run([cli, "package", "verify-lock"], cwd=project, check=True)
    main = project / "src/main.cor"
    migrated = main.read_text(encoding="utf-8").replace("/v1.0.0", "/v2.0.0")
    main.write_text(migrated, encoding="utf-8")
    line = next(i for i, s in enumerate(migrated.splitlines(), 1) if "ColorPicker(value," in s)
    check = subprocess.run([cli, "check"], cwd=project, env=env, capture_output=True, encoding="utf-8")
    diagnostic = check.stdout + check.stderr
    assert check.returncode and f"{main}:{line}:" in diagnostic, diagnostic
    assert "wrong number of arguments to `ColorPicker`" in diagnostic, diagnostic
    assert "`ColorPicker` takes 3 arguments" in diagnostic, diagnostic
    main.write_text(migrated.replace('ColorPicker(value, "/")', 'ColorPicker(value, "/", "Cover color")'),
                    encoding="utf-8")
    subprocess.run([cli, "test", "src/main.cor"], cwd=project, env=env, check=True)
    subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, env=env, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", nargs="?", type=Path)
    parser.add_argument("--cli", dest="cli_option", type=Path)
    parser.add_argument("--lsp", type=Path, help="check installed component completion and definition")
    parser.add_argument("--work-dir", type=Path, help="retain the trial in a new directory")
    parser.add_argument("--browser-channel", "--channel", help="also check computed style and keyboard use, e.g. msedge")
    args = parser.parse_args()
    cli = args.cli_option or args.cli
    if cli is None:
        parser.error("supply --cli PATH")
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=False)
        verify(cli.resolve(), args.work_dir.resolve(), args.browser_channel, args.lsp and args.lsp.resolve())
    else:
        with TemporaryDirectory() as directory:
            verify(cli.resolve(), Path(directory).resolve(), args.browser_channel, args.lsp and args.lsp.resolve())
