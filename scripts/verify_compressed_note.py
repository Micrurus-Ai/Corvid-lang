"""Build the typed file importer and exercise it from a separate static Corvid host."""

import argparse
from functools import partial
import gzip
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import shutil
import subprocess
from threading import Thread

from export_browser_component import build


PROJECT = Path(__file__).resolve().parents[1] / "examples/compressed_note"


def check_callback_type(cli):
    fixture = PROJECT / "target/callback-check"
    fixture.mkdir(parents=True, exist_ok=True)
    (fixture / "events.cor").write_text(
        "browser callback page_count(count: Int) -> Int:\n    return count\n", encoding="utf-8")
    (fixture / "component.cor").write_text(
        'import "std:ui" as ui\nimport "./events" use page_count\n'
        'agent main() -> String:\n'
        '    return ui.module_field(ui.text("Note"), "./reader.js", "read", page_count()).html\n',
        encoding="utf-8")
    checked = subprocess.run([cli, "check", "component.cor"], cwd=fixture, capture_output=True,
                             text=True, encoding="utf-8")
    assert checked.returncode and "TextCallback" in checked.stderr and "component.cor:4:" in checked.stderr, checked.stderr


def build_host(cli):
    check_callback_type(cli)
    package = build(cli, PROJECT)
    npm = shutil.which("npm.cmd") or shutil.which("npm")
    if not npm:
        raise RuntimeError("Install npm to build the TypeScript reader")
    subprocess.run([npm, "ci", "--ignore-scripts", "--no-audit", "--no-fund"], cwd=PROJECT, check=True)
    subprocess.run([npm, "run", "build"], cwd=PROJECT, check=True)
    subprocess.run(["node", "read_note.test.mjs"], cwd=PROJECT, check=True)
    result = subprocess.run([cli, "run", "src/host.cor", "--target", "interp"], cwd=PROJECT,
                            capture_output=True, text=True, encoding="utf-8", check=True)
    host = PROJECT / "target/note-host"
    shutil.copytree(package, host / "target/wasm", dirs_exist_ok=True)
    (host / "index.html").write_text(json.loads(result.stdout), encoding="utf-8")
    return host


def verify_browser(host, channel):
    from playwright.sync_api import expect, sync_playwright

    with ThreadingHTTPServer(("127.0.0.1", 0), partial(SimpleHTTPRequestHandler, directory=host)) as server:
        Thread(target=server.serve_forever, daemon=True).start()
        try:
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(channel=channel)
                page = browser.new_page()
                page.goto(f"http://127.0.0.1:{server.server_port}")
                field = page.locator('textarea[name="note"]')
                picker = page.get_by_label("Import a compressed note")
                importer = page.locator("compressed-note")
                output = importer.locator("output")
                expect(output).to_have_text("Draft")
                assert importer.locator("slot").evaluate("slot => slot.assignedElements()[0].textContent") == "Choose a gzip file."
                field.fill("Manual edit")
                expect(output).to_have_text("Manual edit")
                importer.evaluate("""element => {
                    element.changes = [];
                    element.addEventListener('change', event => {
                        element.changes.push([element.value, event.bubbles, event.composed]);
                    });
                }""")
                picker.set_input_files({"name": "broken.gz", "mimeType": "application/gzip", "buffer": b"invalid archive"})
                alert = importer.get_by_role("alert")
                expect(alert).to_contain_text("readNote → imported_text at src/events.cor:2:9:")
                expect(field).to_have_value("Manual edit")
                note = "Café\nShopping list"
                picker.set_input_files({"name": "note.gz", "mimeType": "application/gzip",
                                        "buffer": gzip.compress(note.encode())})
                expect(field).to_have_value(note)
                expect(alert).to_be_empty()
                picker.set_input_files([])
                expect(field).to_have_value("")
                assert importer.evaluate("element => element.changes") == [[note, True, True], ["", True, True]]
                browser.close()
        finally:
            server.shutdown()
    print("Passed slot projection, property binding, gzip import, source-located error, recovery and change events.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--build-only", action="store_true", help="run the Corvid, TypeScript and Node checks")
    args = parser.parse_args()
    host = build_host(args.cli.resolve())
    if not args.build_only:
        verify_browser(host, args.channel)
