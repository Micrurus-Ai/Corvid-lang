"""Package the reading counter and exercise its CLI, keyboard and WebMCP bindings."""
import argparse
from contextlib import nullcontext
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from urllib.request import urlopen
from verify_reading_list import read, serve


class ActionControls(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.controls = []
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag in ("button", "input"):
            self.controls.append(attrs)

    @property
    def bindings(self):
        return [(attrs.get("data-corvid-action"), attrs.get("data-corvid-tool")) for attrs in self.controls]


def trial(page, url, adapter):
    from playwright.sync_api import expect
    if not adapter:
        page.add_init_script("Object.defineProperty(document, 'modelContext', {value: undefined})")
    page.goto(url)
    record = page.get_by_role("button", name="Read one page")
    output = page.locator("output")
    expect(record).to_be_enabled()
    page.keyboard.press("Tab")
    expect(record).to_be_focused()
    page.keyboard.press("Enter")
    expect(output).to_have_text("1")
    if adapter:
        page.wait_for_function("""async () =>
            (await document.modelContext.getTools()).some(tool => tool.name === 'record_page_read')""")
        tools = page.evaluate("async () => (await document.modelContext.getTools()).map(t => t.name)")
        assert tools == ["record_page_read"], tools
        page.evaluate("""async () => {
            const context = document.modelContext;
            const [tool] = await context.getTools();
            const invoke = name => context.executeTool({...tool, source:tool.source, origin:tool.origin, name}, {});
            window.callTool = () => context.executeTool(tool, {});
            window.denied = async (action) => {
                try { await action(); } catch { return true; }
                return false;
            };
            // Prove the request shape works before changing only its name.
            await invoke(tool.name);
            if (!await denied(() => invoke('reset_pages')))
                throw Error('Private action accepted');
        }""")
        expect(output).to_have_text("2")
        record.evaluate("""button => {
            button.textContent = 'Log reading';
            button.parentElement.append(button);
        }""")
        assert page.evaluate("async () => (await document.modelContext.getTools()).map(t => t.name)") == tools
        page.evaluate("() => callTool()")
        expect(output).to_have_text("3")
        renamed = page.get_by_role("button", name="Log reading")
        renamed.evaluate("button => button.disabled = true")
        assert page.evaluate("() => denied(callTool)")
        expect(output).to_have_text("3")
    else:
        assert page.evaluate("document.modelContext === undefined")
    page.get_by_role("button", name="Start over").press("Enter")
    expect(output).to_have_text("0")


def check_worker(cli, installed, expected):
    def jobs(*args):
        return subprocess.check_output([cli, "jobs", *args], cwd=installed, text=True, timeout=20)

    queued = jobs("enqueue", "--task", "read_page", "--payload", "[40]")
    job = next(line.removeprefix("job: ") for line in queued.splitlines() if line.startswith("job: "))
    jobs("run", "--source", "src/events.cor", "--workers", "1", "--max-runtime-ms", "1500")
    receipt = jobs("inspect", "--job", job).splitlines()
    fingerprint = hashlib.sha256(expected.encode()).hexdigest()
    for field in ("status: succeeded", "attempts: 1", "output_kind: number",
                  f"output_fingerprint: sha256:{fingerprint}"):
        assert field in receipt, receipt


def check_contract(cli, project):
    def public_action():
        output = subprocess.check_output([cli, "contract", "app", "src/events.cor", "--out", "-"],
                                         cwd=project, text=True)
        action, = json.loads(output)["agents"]
        assert action["name"] == "read_page", action
        return action

    def check_types(ty):
        action = public_action()
        assert action["inputs"] == [{"name": "current", "type": ty}], action
        assert action["output_type"] == ty, action

    def check_binding(kind, redesigned=False):
        subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
        assets = project / "target" / "binding-check"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(assets, filter="data")
        with serve(cli, assets, dict(os.environ)) as url:
            html = read(url)
        parsed = ActionControls(html)
        if redesigned:
            assert ">Log reading</button>" in html and ">Read one page</button>" not in html
            assert parsed.bindings == [("reset_pages", None), ("read_page", "record_page_read")]
        command = ["node", Path(__file__).with_name("check_action_binding.mjs").resolve(),
                   (assets / "target/wasm/events.js").resolve(), kind]
        controls = json.dumps(parsed.controls)
        subprocess.run(command, input=controls, text=True, check=True)
        if kind == "counter":
            subprocess.run([*command, "absent"], input=controls, text=True, check=True)

    check_types("Int")
    check_binding("counter")
    events = project / "src/events.cor"
    original = events.read_bytes()
    main = project / "src/main.cor"
    counter_page = main.read_bytes()
    try:
        redesigned = counter_page.replace(b"Read one page", b"Log reading").replace(
            b"[record, clear]", b"[clear, record]")
        main.write_bytes(redesigned)
        check_binding("counter", redesigned=True)
        events.write_bytes(original.replace(b"read_page(current: Int) -> Int",
                                            b"read_page(current: String) -> String")
                                   .replace(b"return current + 1", b"return current"))
        check_types("String")
        main.write_text('''import "./std/ui" as ui

server progress:
    route GET "/" -> html:
        note = ui.assistant_text_state("Reading note", "first", "read_page", "record_page_read", "Edit the reading note.")
        return ui.interactive_page("Reading progress", [note])
''', encoding="utf-8")
        check_binding("text")
        result = subprocess.run([cli, "check", "src/read_page_cli.cor"], cwd=project,
                                capture_output=True, text=True)
        assert result.returncode != 0, result
        assert "type mismatch in argument 1 to `read_page`" in result.stderr, result.stderr
        assert "read_page_cli.cor:4:" in result.stderr, result.stderr
    finally:
        events.write_bytes(original)
        main.write_bytes(counter_page)


def verify(cli, cdp, channel, work_dir, package_only):
    example = Path(__file__).resolve().parents[1] / "examples/reading_list/src"
    if work_dir:
        work_dir.mkdir(parents=True, exist_ok=False)
    with (nullcontext(work_dir) if work_dir else TemporaryDirectory()) as directory:
        root = Path(directory)
        subprocess.run([cli, "new", "progress"], cwd=root, check=True)
        project = root / "progress"
        for name in ("events.cor", "read_page_cli.cor"):
            shutil.copyfile(example / name, project / "src" / name)
        shutil.copyfile(example / "progress.cor", project / "src/main.cor")
        cmd = [cli, "run", "--target", "interp", "src/read_page_cli.cor", "40"]
        result = subprocess.check_output(cmd, cwd=project, text=True).strip()
        assert result == "41", result
        check_contract(cli, project)
        subprocess.run([cli, "check"], cwd=project, check=True)
        subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
        installed = root / "installed"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        check_worker(cli, installed, result)
        with serve(cli, installed, dict(os.environ)) as url:
            assert ActionControls(read(url)).bindings == [("read_page", "record_page_read"), ("reset_pages", None)]
            with urlopen(url + "/target/wasm/events.wasm", timeout=5) as response:
                assert response.read(4) == b"\0asm"
            assert "registerTool" in read(url + "/target/wasm/events.js")
            print("CLI, durable worker receipt, public contract, stale caller, packaged bindings "
                  "and served WebAssembly assets passed.", flush=True)
            if package_only:
                return
            from playwright.sync_api import sync_playwright
            with sync_playwright() as playwright:
                run_browser(playwright, url, cdp, channel)


def run_browser(playwright, url, cdp, channel):
    browser = (playwright.chromium.connect_over_cdp(cdp, timeout=10000) if cdp else
               playwright.chromium.launch(channel=channel, args=["--enable-blink-features=WebMCP,WebMCPTesting"]))
    for adapter in (False, True):
        page = browser.contexts[0].new_page() if cdp else browser.new_page()
        try:
            trial(page, url, adapter)
            print(f"Browser {browser.version}, adapter present={adapter}: passed", flush=True)
        finally:
            page.close()
    if not cdp:
        browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--cdp", help="connect to a dedicated local browser with WebMCP enabled")
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--work-dir", type=Path, help="retain the package in a new directory")
    parser.add_argument("--package-only", action="store_true", help="skip browser checks")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.cdp, args.channel, args.work_dir, args.package_only)
