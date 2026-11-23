"""Visit three packaged storefront releases through one origin with a warm browser cache."""
import argparse
from contextlib import ExitStack, contextmanager
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory
from threading import Thread
from urllib.parse import urlsplit

from playwright.sync_api import expect, sync_playwright
from verify_browser_diagnostics import package
from verify_reading_list import serve
from verify_storefront import ROOT


@contextmanager
def origin():
    # Count actual network transfers without intercepting browser requests
    # through Playwright, which disables its HTTP cache.
    class Proxy(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def forward(self):
            upstream = HTTPConnection(self.server.upstream, timeout=15)
            try:
                body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                upstream.request(self.command, self.path, body=body, headers=dict(self.headers))
                response = upstream.getresponse()
                data = response.read()
                self.server.requests.append((self.path, response.status, len(data)))
                self.send_response_only(response.status)
                for key, value in response.getheaders():
                    if key.lower() not in {"connection", "transfer-encoding", "content-length"}:
                        self.send_header(key, value)
                if response.status != 304:
                    self.send_header("Content-Length", str(len(data)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(data)
            finally:
                upstream.close()

        do_GET = do_POST = forward

        def log_message(self, *_):
            pass

    with ThreadingHTTPServer(("127.0.0.1", 0), Proxy) as proxy:
        proxy.requests = []
        thread = Thread(target=proxy.serve_forever, daemon=True)
        thread.start()
        try:
            yield proxy
        finally:
            proxy.shutdown()
            thread.join()


def visit(page, url, price, heading):
    page.goto(url + "/cart")
    page.get_by_label("Notebook quantity").fill("2")
    expect(page.locator('[data-corvid-key="notebook"]').get_by_text(str(price * 2), exact=True)).to_be_visible()
    page.get_by_role("button", name="Checkout").press("Enter")
    expect(page.get_by_role("heading", name=heading)).to_be_visible()
    expect(page.get_by_text(f"Total: {price * 2 + 125} cents", exact=True)).to_be_visible()


def trial(context, servers):
    with origin() as proxy:
        url = f"http://127.0.0.1:{proxy.server_port}"
        page = context.new_page()
        proxy.upstream = servers[0]
        visit(page, url, 450, "Checkout summary")
        js = "/target/wasm/events.js"
        wasm = next(path for path, status, _ in proxy.requests if ".wasm?v=" in path and status == 200)
        proxy.requests.clear()
        proxy.upstream = servers[1]
        visit(page, url, 450, "Order summary")
        assert (js, 304, 0) in proxy.requests, proxy.requests
        assert all(path != wasm for path, _, _ in proxy.requests), proxy.requests

        proxy.upstream = servers[2]
        proxy.requests.clear()
        visit(page, url, 475, "Order summary")
        assert any(path == js and status == 200 for path, status, _ in proxy.requests), proxy.requests
        assert any(path != wasm and ".wasm?v=" in path and status == 200
                   for path, status, _ in proxy.requests), proxy.requests
        stale = context.request.get(url + wasm)
        assert stale.status == 404 and stale.headers["cache-control"] == "no-store"


def verify(cli, channel):
    with TemporaryDirectory() as tmp, ExitStack() as stack:
        root = Path(tmp)
        subprocess.run([cli, "new", "storefront"], cwd=root, check=True)
        project = root / "storefront"
        src = project / "src"
        shutil.copytree(ROOT / "examples/storefront/src", src, dirs_exist_ok=True)
        servers = []
        for release, price in (("initial", "450"), ("heading", "450"), ("price", "475")):
            for filename in ("events.cor", "main.cor"):
                path = src / filename
                source = path.read_text(encoding="utf-8").replace("450", price)
                if release == "heading" and filename == "main.cor":
                    source = source.replace("Checkout summary", "Order summary")
                path.write_text(source, encoding="utf-8")
            package(cli, project, root / release)
            url = stack.enter_context(serve(cli, root / release, os.environ.copy()))
            servers.append(urlsplit(url).netloc)
        with sync_playwright() as playwright:
            for engine in ("chromium", "firefox", "webkit"):
                context = getattr(playwright, engine).launch_persistent_context(
                    str(root / (engine + "-profile")), channel=channel if engine == "chromium" else None)
                try:
                    trial(context, servers)
                    print(engine, "warm cache and updated checkout passed", flush=True)
                finally:
                    context.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", help="Installed Chromium distribution")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
