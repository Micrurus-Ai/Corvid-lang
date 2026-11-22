"""Draw using the browser worker, stop the HTTP server and reload the sketch."""

import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread

from playwright.sync_api import expect, sync_playwright


def verify(channel):
    project = Path(__file__).resolve().parents[1] / "examples/drawing_editor"
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        page = browser.new_page()
        canvas = page.locator("canvas")
        handler = partial(SimpleHTTPRequestHandler, directory=project)
        with ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
            Thread(target=server.serve_forever, daemon=True).start()
            try:
                page.goto(f"http://127.0.0.1:{server.server_port}/")
                expect(page.get_by_role("status")).to_have_text("Drag to draw.")
                expect(page.locator("#offline")).to_have_text("Ready for offline reload.")
                bounds = canvas.bounding_box()
                def point(x, y):
                    return (bounds["x"] + (x + 0.5) * bounds["width"] / 96,
                            bounds["y"] + (y + 0.5) * bounds["height"] / 64)

                page.mouse.move(*point(10, 20))
                page.mouse.down()
                page.mouse.move(*point(40, 20))
                page.mouse.up()
                expect(page.locator("#storage")).to_have_text("Drawing saved.")
            finally:
                server.shutdown()

        pixels = """el => {
          const data = el.getContext('2d').getImageData(0, 0, el.width, el.height).data;
          return Array.from({ length: el.width * el.height }, (_, i) => i)
            .filter(i => data[4 * i + 3] === 255);
        }"""
        expected = [20 * 96 + x for x in range(10, 41)]
        assert canvas.evaluate(pixels) == expected
        assert page.reload().from_service_worker
        expect(page.get_by_role("status")).to_have_text("Drag to draw.")
        assert canvas.evaluate(pixels) == expected
        page.mouse.click(*point(5, 3))
        expect(page.locator("#storage")).to_have_text("Drawing saved.")
        assert page.reload().from_service_worker
        expect(page.get_by_role("status")).to_have_text("Drag to draw.")
        assert canvas.evaluate(pixels) == [3 * 96 + 5, *expected]
        browser.close()
    print("Passed pointer drawing, offline worker startup, offline edit and two reloads.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel", default="chrome")
    verify(parser.parse_args().channel)
