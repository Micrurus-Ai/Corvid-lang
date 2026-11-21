"""Exercise a newly packaged reading list and measure its loopback interactions."""
import argparse
from contextlib import contextmanager, nullcontext
from html import escape
from html.parser import HTMLParser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import socket
import statistics
import subprocess
from queue import Queue
import tarfile
from tempfile import TemporaryDirectory
import time
from threading import Event, Thread
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import urlopen


@contextmanager
def serve(cli, directory, env):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    address = f"127.0.0.1:{port}"
    process = subprocess.Popen([cli, "serve", "src/main.cor", "--listen", address],
                               cwd=directory, env=env)
    url = "http://" + address
    try:
        for _ in range(150):
            if process.poll() is not None:
                raise RuntimeError("Corvid exited before serving the archive")
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    break
            except OSError:
                time.sleep(0.1)
        else:
            raise TimeoutError("Corvid did not start listening")
        yield url
    finally:
        process.terminate()
        process.wait(timeout=10)


def read(url, data=None):
    with urlopen(url, data, timeout=10) as response:
        return response.read().decode()


def report(label, samples):
    print(f"{label} (ms): {samples}; median {statistics.median(samples):.2f}")


class LocalModel(BaseHTTPRequestHandler):
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.server.requests.put(request)
        output = self.server.outputs.get(timeout=5)
        if output is None:
            self.server.waiting.set()
            self.connection.settimeout(8)
            try:
                closed = self.connection.recv(1) == b""
            except ConnectionResetError:
                closed = True
            if closed:
                self.server.closed.set()
            return
        payload = json.dumps({"choices": [{"message": {"role": "assistant", "content": output}}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def action(name, value):
    return json.dumps({"action": {"variant": name, "fields": [value]}})


class PreviewForm(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.fields = {}
        self.action = None
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "form":
            assert attrs["method"].upper() == "POST"
            self.action = attrs["action"]
        elif tag == "input":
            self.fields[attrs["name"]] = attrs["value"]


def check_suggestions(url, provider, original, channel, http_only):
    def suggest():
        return read(url + "/suggest", b"shelf=2")

    title = 'Books & "birds" <2026>'
    for output in ["{", action("Suggest", 17)]:
        provider.outputs.put(output)
        try:
            suggest()
        except HTTPError as error:
            assert error.code == 500
        else:
            raise AssertionError("Malformed proposal was accepted")
        provider.requests.get(timeout=2)
    provider.outputs.put(action("Lookup", 4))
    assert "shelf was not found" in suggest()
    provider.requests.get(timeout=2)
    provider.outputs.put(action("Suggest", "  "))
    assert "suggested title was empty" in suggest()
    provider.requests.get(timeout=2)
    for _ in range(2):
        provider.outputs.put(action("Lookup", 1))
    assert "lookup limit" in suggest()
    for _ in range(2):
        provider.requests.get(timeout=2)
    provider.outputs.put(action("Lookup", 1))
    provider.outputs.put(action("Suggest", title))
    preview = suggest()
    assert 'action="/save"' in preview and escape(title) in preview
    form = PreviewForm(preview)
    assert form.action == "/save" and form.fields == {"shelf": "2", "title": title}
    first = provider.requests.get(timeout=2)
    second = provider.requests.get(timeout=2)
    variants = first["response_format"]["json_schema"]["schema"]["properties"]["action"]["oneOf"]
    assert [v["properties"]["variant"]["const"] for v in variants] == ["Lookup", "Suggest"]
    assert "The Odyssey" in second["messages"][-1]["content"]
    assert "Shelf 2: " + escape(original) in read(url)

    provider.outputs.put(None)
    if http_only:
        address = url.removeprefix("http://").split(":")
        with socket.create_connection((address[0], int(address[1])), timeout=3) as client:
            client.sendall(b"POST /suggest HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 7\r\n\r\nshelf=2")
            assert provider.waiting.wait(3)
        assert provider.closed.wait(5), "Model request survived client disconnect"
        assert "Shelf 2: " + escape(original) in read(url)
        result = read(url + form.action, urlencode(form.fields).encode())
        assert "Shelf 2: " + escape(title) in result
    else:
        from playwright.sync_api import expect, sync_playwright
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(channel=channel)
            page = browser.new_page()
            page.goto(url + "/edit?shelf=2")
            start = page.get_by_role("button", name="Suggest a title")
            start.click()
            assert provider.waiting.wait(3)
            page.get_by_role("button", name="Cancel suggestion").click()
            expect(page.get_by_role("status")).to_have_text("Suggestion cancelled.")
            assert provider.closed.wait(5), "Model request survived Cancel"
            provider.outputs.put("{")
            start.click()
            expect(page.get_by_role("status")).to_contain_text("Suggestion failed")
            provider.outputs.put(action("Suggest", title))
            start.click()
            draft = page.locator("#preview").get_by_label("title", exact=True)
            expect(draft).to_have_value(title)
            assert "Shelf 2: " + escape(original) in read(url)
            title += " edited"
            draft.fill(title)
            page.get_by_role("button", name="Apply suggestion").click()
            expect(page.get_by_text("Saved", exact=True)).to_be_visible()
            browser.close()
    return title


def check_http(url):
    assert "Shelf 2: Persuasion" in read(url)
    assert 'action="/save"' in read(url + "/edit?shelf=2")
    samples = []
    for i in range(5):
        title = f"Birds & books <{i}>"
        form = urlencode({"shelf": 2, "title": title}).encode()
        start = time.perf_counter()
        result = read(url + "/save", form)
        samples.append(round(1000 * (time.perf_counter() - start), 2))
        assert "<p>Saved</p>" in result and escape(title) in result
    try:
        read(url + "/save", b"shelf=wrong&title=invalid")
    except HTTPError as error:
        assert error.code == 400
    else:
        raise AssertionError("Invalid shelf was accepted")
    report("POST through complete HTML response", samples)
    return title


def check_browser(url, channel):
    from playwright.sync_api import expect, sync_playwright

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        page = browser.new_page()
        loads, saves = [], []
        for i in range(5):
            page.goto(url)
            loads.append(page.evaluate("performance.getEntriesByType('navigation')[0].duration"))
            page.keyboard.press("Tab")
            page.keyboard.press("Tab")
            expect(page.locator(":focus")).to_contain_text("Shelf 2:")
            page.keyboard.press("Enter")
            page.wait_for_url("**/edit?shelf=2")
            # Each shelf has a link. The selected shelf's form follows its link.
            for _ in range(3):
                page.keyboard.press("Tab")
            expect(page.get_by_label("shelf", exact=True)).to_be_focused()
            page.keyboard.press("Tab")
            expect(page.get_by_label("title", exact=True)).to_be_focused()
            title = f"Keyboard & paper <{i}>"
            page.keyboard.press("ControlOrMeta+a")
            page.keyboard.type(title)
            page.keyboard.press("Tab")
            expect(page.get_by_role("button", name="Save to shelf")).to_be_focused()
            page.keyboard.press("Enter")
            page.wait_for_url("**/save", wait_until="load")
            expect(page.get_by_text("Saved", exact=True)).to_be_visible()
            expect(page.get_by_role("link", name="Shelf 2: " + title)).to_be_visible()
            saves.append(page.evaluate("performance.getEntriesByType('navigation')[0].duration"))
        print("Chromium", browser.version)
        report("List navigation through load", loads)
        report("POST navigation through load", saves)
        browser.close()
        return title


def verify(args):
    cli = args.cli.resolve()
    example = Path(__file__).resolve().parents[1] / "examples/reading_list"
    system_keys = {"systemroot", "windir", "temp", "tmp", "path", "userprofile",
                   "localappdata", "appdata", "comspec", "pathext"}
    env = {k: v for k, v in os.environ.items() if k.lower() in system_keys}
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=False)
    workspace = nullcontext(args.work_dir) if args.work_dir else TemporaryDirectory()
    with workspace as tmp:
        root = Path(tmp)
        project = root / "reading-list"
        subprocess.run([cli, "new", project.name], cwd=root, env=env, check=True)
        shutil.copyfile(example / "src/main.cor", project / "src/main.cor")
        shutil.copytree(example / "migrations", project / "migrations")
        subprocess.run([cli, "check"], cwd=project, env=env, check=True)
        subprocess.run([cli, "test", "src/main.cor"], cwd=project, env=env, check=True)
        subprocess.run([cli, "build", "src/main.cor", "--target", "web"],
                       cwd=project, env=env, check=True)
        installed = root / "installed"
        installed.mkdir()
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        subprocess.run([cli, "migrate", "up", "--dir", "migrations",
                        "--database", "reading.sqlite"], cwd=installed, env=env, check=True)
        with serve(cli, installed, env) as url:
            title = check_http(url)
            if not args.http_only:
                title = check_browser(url, args.channel)
        with ThreadingHTTPServer(("127.0.0.1", 0), LocalModel) as provider:
            provider.requests, provider.outputs = Queue(), Queue()
            provider.waiting, provider.closed = Event(), Event()
            Thread(target=provider.serve_forever, daemon=True).start()
            local_env = dict(env, CORVID_TEST_MOCK_LLM="0", NO_PROXY="127.0.0.1",
                             CORVID_MODEL=f"openai-compat:http://127.0.0.1:{provider.server_port}/v1:local-model")
            try:
                with serve(cli, installed, local_env) as url:
                    title = check_suggestions(url, provider, title, args.channel, args.http_only)
            finally:
                provider.shutdown()
        with serve(cli, installed, env) as url:
            result = read(url)
            for label in ["Shelf 1: The Odyssey", "Shelf 2: " + escape(title),
                          "Shelf 3: Frankenstein"]:
                assert label in result
            print("Replacement server returned all three expected shelves.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--work-dir", type=Path, help="create and retain the trial in this new directory")
    parser.add_argument("--channel", help="Playwright Chromium channel, e.g. msedge")
    parser.add_argument("--http-only", action="store_true", help="skip the keyboard and browser timings")
    verify(parser.parse_args())
