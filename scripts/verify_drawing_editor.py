"""Build the sketchbook and exercise offline editing, synchronization and upgrades."""

import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
from threading import Lock, Thread

from playwright.sync_api import expect, sync_playwright


class SketchFixture(SimpleHTTPRequestHandler):
    """A conditional HTTP resource and a deliberately incomplete shell update."""

    def reply(self, status, body=b"", content_type="application/json", revision=None):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        if revision is not None:
            self.send_header("ETag", f'"{revision}"')
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/sketch.json":
            with self.server.lock:
                self.reply(200, json.dumps(self.server.document).encode(),
                           revision=self.server.revision)
        elif self.server.incomplete_update and self.path == "/offline.js":
            source = (Path(self.directory) / "offline.js").read_bytes()
            self.reply(200, source.replace(b'"v2"', b'"interrupted"'),
                       "application/javascript")
        elif self.server.incomplete_update and self.path == "/raster.mjs":
            self.reply(503)
        else:
            super().do_GET()

    def do_PUT(self):
        if self.path != "/sketch.json":
            self.reply(404)
            return
        document = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        with self.server.lock:
            if self.server.inject_conflict:
                self.server.inject_conflict = False
                self.server.document["cells"].append([95, 63])
                self.server.revision += 1
            if self.headers.get("If-Match") != f'"{self.server.revision}"':
                self.reply(412)
                return
            self.server.document = document
            self.server.revision += 1
            self.reply(204)


def point(canvas, x, y):
    bounds = canvas.bounding_box()
    return (bounds["x"] + (x + 0.5) * bounds["width"] / 96,
            bounds["y"] + (y + 0.5) * bounds["height"] / 64)


def verify(cli, channel):
    project = Path(__file__).resolve().parents[1] / "examples/drawing_editor"
    subprocess.run([cli, "build", "src/stroke.cor", "--target", "wasm"], cwd=project, check=True)
    handler = partial(SketchFixture, directory=project)
    with sync_playwright() as playwright, ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
        server.lock = Lock()
        server.document = {"version": 1, "cells": []}
        server.revision = 1
        server.inject_conflict = False
        server.incomplete_update = False
        Thread(target=server.serve_forever, daemon=True).start()
        browser = playwright.chromium.launch(channel=channel)
        context = browser.new_context()
        page = context.new_page()
        base = f"http://127.0.0.1:{server.server_port}/"
        old_tab = context.new_page()
        old_tab.goto(base + "editor.js")
        old_tab.evaluate("""() => new Promise(resolve => {
          const open = indexedDB.open('corvid-sketch:/', 1);
          open.onupgradeneeded = () => open.result.createObjectStore('cells').put(true, [0, 0]);
          open.onsuccess = () => { window.legacy = open.result; resolve(); };
        })""")
        page.goto(base, wait_until="commit")
        expect(page.locator("#storage")).to_contain_text("Close older sketchbook tabs")
        old_tab.evaluate("legacy.close()")
        expect(page.get_by_role("status")).to_have_text("Drag to draw.")
        expect(page.locator("#offline")).to_have_text("Ready for offline reload.")
        expect(page.locator("#sync-status")).to_have_text("Drawing synchronized.")
        assert old_tab.evaluate("""() => new Promise(resolve => {
          const open = indexedDB.open('corvid-sketch:/', 1);
          open.onerror = () => resolve(open.error.name);
        })""") == "VersionError"
        old_tab.goto(base)
        expect(old_tab.get_by_role("status")).to_have_text("Drag to draw.")
        context.set_offline(True)
        canvas = page.locator("canvas")
        page.mouse.move(*point(canvas, 10, 20))
        page.mouse.down()
        page.mouse.move(*point(canvas, 40, 20))
        page.mouse.up()
        expect(page.locator("#storage")).to_have_text("Drawing saved.")
        old_tab.mouse.click(*point(old_tab.locator("canvas"), 5, 3))
        expect(old_tab.locator("#storage")).to_have_text("Drawing saved.")
        expected = [0, 3 * 96 + 5, *range(20 * 96 + 10, 20 * 96 + 41)]
        pixels = """el => {
          const data = el.getContext('2d').getImageData(0, 0, el.width, el.height).data;
          return Array.from({length: el.width * el.height}, (_, i) => i)
            .filter(i => data[4 * i + 3] === 255);
        }"""
        assert page.reload().from_service_worker
        expect(page.get_by_role("status")).to_have_text("Drag to draw.")
        assert canvas.evaluate(pixels) == expected
        assert server.document["cells"] == [[0, 0]]
        server.inject_conflict = True
        context.set_offline(False)
        expected.append(63 * 96 + 95)
        page.wait_for_function("document.querySelector('canvas').getContext('2d').getImageData(95,63,1,1).data[3] === 255")
        assert canvas.evaluate(pixels) == expected
        assert sorted(y * 96 + x for x, y in server.document["cells"]) == expected
        server.incomplete_update = True
        state = page.evaluate("""async () => {
          const registration = await navigator.serviceWorker.ready;
          const installed = new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error('Update did not settle')), 15000);
            registration.addEventListener('updatefound', () => {
              const worker = registration.installing;
              worker.addEventListener('statechange', () => {
                if (['installed', 'redundant'].includes(worker.state)) {
                  clearTimeout(timeout);
                  resolve(worker.state);
                }
              });
            }, {once: true});
          });
          await registration.update();
          return installed;
        }""")
        assert state == "redundant"
        server.shutdown()
        context.set_offline(True)
        assert page.reload().from_service_worker
        expect(page.get_by_role("status")).to_have_text("Drag to draw.")
        assert canvas.evaluate(pixels) == expected
        page.mouse.click(*point(canvas, 1, 0))
        expect(page.locator("#storage")).to_have_text("Drawing saved.")
        assert page.reload().from_service_worker
        expect(page.get_by_role("status")).to_have_text("Drag to draw.")
        assert canvas.evaluate(pixels) == sorted([1, *expected])
        browser.close()
    print("Passed migration, offline edits across two tabs, reconnect, conflict merge and interrupted shell update.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", default="msedge")
    args = parser.parse_args()
    cli = str(Path(args.cli).resolve()) if Path(args.cli).is_file() else args.cli
    verify(cli, args.channel)
