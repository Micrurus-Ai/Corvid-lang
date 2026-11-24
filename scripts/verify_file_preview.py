"""Build the Corvid file preview and verify its browser bindings in Edge."""

import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import shutil
import subprocess
import sys
from threading import Thread

PROJECT = Path(__file__).resolve().parents[1] / "examples/file_preview"


def build(cli, tsc):
    host = PROJECT / "target/site"
    host.mkdir(parents=True, exist_ok=True)

    def run(*command, **options):
        return subprocess.run(command, cwd=PROJECT, check=True, **options)

    run(sys.executable, Path(__file__).with_name("derive_browser_bindings.py"),
        "file_access.webidl", "target/file_access.d.ts")
    if tsc is None:
        run(shutil.which("npm.cmd") or "npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund")
        tsc = PROJECT / "node_modules/typescript/bin/tsc"
    for source, library in [("file_preview.ts", "DOM"), ("decode_file.ts", "WebWorker")]:
        run("node", tsc, "--strict", "--target", "ES2022", "--module", "ES2022",
            "--lib", f"ES2022,{library}", "--outDir", host, source)
    run("node", "verify_decoder.mjs")
    run(cli, "check", "src/main.cor")
    run(cli, "build", "src/events.cor", "--target", "wasm")
    shutil.copytree(PROJECT / "target/wasm", host / "target/wasm", dirs_exist_ok=True)
    rendered = run(cli, "run", "src/main.cor", "--target", "interp", capture_output=True,
                   encoding="utf-8")
    (host / "index.html").write_text(json.loads(rendered.stdout), encoding="utf-8")
    return host


def exercise(page, url):
    from playwright.sync_api import expect

    # Forward to the real browser operations while recording transfer and cleanup.
    page.add_init_script("""window.transfers = []; window.terminations = 0;
        window.Worker = class extends Worker {
            postMessage(buffer, transfer) {
                const size = buffer.byteLength;
                super.postMessage(buffer, transfer);
                window.transfers.push([size, buffer.byteLength]);
            }
            terminate() { window.terminations++; super.terminate(); }
        };""")
    page.goto(url)
    picker = page.get_by_label("Preview a UTF-8 file")
    preview = page.locator("pre")
    copy = page.get_by_role("button", name="Copy preview")
    status = page.locator("output")
    sample = {"name": "note.txt", "mimeType": "text/plain", "buffer": "Café".encode()}
    # Wait for the asynchronous Corvid lifecycle mount before selecting a file.
    preview.evaluate("p => p.textContent = 'Manual selection'")
    expect(copy).to_be_enabled()
    preview.evaluate("p => p.textContent = ''")
    expect(copy).to_be_disabled()
    picker.set_input_files(sample)
    expect(preview).to_have_text("Café")
    assert page.evaluate("window.transfers") == [[5, 0]]
    assert page.evaluate("window.terminations") == 1
    page.context.grant_permissions(["clipboard-read", "clipboard-write"], origin=url)
    copy.click()
    expect(status).to_have_text("Copied.")
    assert page.evaluate("navigator.clipboard.readText()") == "Café"
    session = page.context.new_cdp_session(page)
    context_id = session.send("Target.getTargetInfo")["targetInfo"]["browserContextId"]
    for unrestricted in (False, True):
        session.send("Browser.setPermission", {
            "permission": {"name": "clipboard-write", "allowWithoutSanitization": unrestricted},
            "setting": "denied", "origin": url, "browserContextId": context_id,
        })
    copy.click()
    expect(status).to_have_text("Copy failed. Select and copy the preview.")
    expect(preview).to_have_text("Café")
    page.evaluate("Object.defineProperty(navigator, 'clipboard', { value: undefined })")
    copy.click()
    expect(status).to_contain_text("Clipboard unavailable")
    picker.set_input_files([])
    expect(copy).to_be_disabled()

    # Keep this worker alive until the Corvid owner releases it.
    page.route("**/decode_file.js", lambda route: route.fulfill(
        content_type="text/javascript", body="self.onmessage = () => {};"))
    picker.set_input_files(sample)
    page.wait_for_function("window.transfers.length === 2")
    owner = page.locator("corvid-owned-element").element_handle()
    owner.evaluate("el => el.remove()")
    assert page.evaluate("window.terminations") == 2
    owner.evaluate("el => el.querySelector('pre').textContent = 'Restored'")
    assert owner.evaluate("el => el.querySelector('button').disabled")
    page.unroute("**/decode_file.js")
    page.evaluate("window.MutationObserver = undefined")
    owner.evaluate("el => document.body.append(el)")
    expect(copy).to_be_enabled()
    picker.set_input_files([])
    expect(copy).to_be_disabled()
    picker.set_input_files(sample)
    expect(preview).to_have_text("Café")
    expect(copy).to_be_enabled()
    page.evaluate("window.Worker = undefined")
    picker.set_input_files([])
    picker.set_input_files(sample)
    expect(status).to_contain_text("Preview unavailable")


def verify(host, channel):
    from playwright.sync_api import sync_playwright

    handler = partial(SimpleHTTPRequestHandler, directory=host)
    with ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
        Thread(target=server.serve_forever, daemon=True).start()
        try:
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(channel=channel)
                try:
                    exercise(browser.new_page(), f"http://127.0.0.1:{server.server_port}")
                finally:
                    browser.close()
        finally:
            server.shutdown()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--tsc", type=Path, help="reuse an installed TypeScript compiler entry point")
    parser.add_argument("--build-only", action="store_true")
    args = parser.parse_args()
    site = build(args.cli.resolve(), args.tsc.resolve() if args.tsc else None)
    print(f"Built {site}", flush=True)
    if not args.build_only:
        verify(site, args.channel)
        print("Passed file preview, transfer, clipboard, fallback and cleanup checks.")
