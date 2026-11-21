"""Run the book's streaming page against a controlled, local model endpoint."""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import re
import socket
import tarfile
import subprocess
from tempfile import TemporaryDirectory
from threading import Event, Thread
import time

from playwright.sync_api import expect, sync_playwright


def verify(cli, channel, bundle):
    calls = queue.Queue()
    shutdown = Event()

    class Provider(BaseHTTPRequestHandler):
        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert self.path == "/v1/chat/completions" and payload["stream"]
            commands, closed = queue.Queue(), Event()
            calls.put((commands, closed))
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            try:
                while not shutdown.is_set():
                    try:
                        command = commands.get(timeout=0.1)
                    except queue.Empty:
                        self.wfile.write(b": keepalive\n\n")
                        self.wfile.flush()
                        continue
                    if command is None:
                        return
                    if command == "DONE":
                        self.wfile.write(b"data: [DONE]\n\n")
                        return
                    event = {"choices": [{"delta": {"content": command}}]}
                    self.wfile.write(("data: " + json.dumps(event) + "\n\n").encode())
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                pass
            finally:
                closed.set()

    root = Path(__file__).resolve().parents[1]
    book = (root / "docs/book/19-browser-views.md").read_text(encoding="utf-8")
    source = re.search(r"## Displaying a streamed answer\s+```corvid-fragment\n(.*?)```", book, re.S)[1]
    with ThreadingHTTPServer(("127.0.0.1", 0), Provider) as provider, TemporaryDirectory() as tmp:
        Thread(target=provider.serve_forever, daemon=True).start()
        project = Path(tmp) / "streaming"
        env = dict(os.environ, CORVID_HOME=str(root), CORVID_TEST_MOCK_LLM="0",
                   OPENAI_COMPAT_API_KEY="", NO_PROXY="127.0.0.1",
                   CORVID_MODEL=f"openai-compat:http://127.0.0.1:{provider.server_port}/v1:local-model")
        subprocess.run([cli, "new", "streaming"], cwd=tmp, env=env, check=True)
        (project / "src/main.cor").write_text(source, encoding="utf-8")
        if bundle:
            chapter = book.split("## Packaging an interactive application")[1]
            app, events = re.findall(r"```corvid(?:-fragment)?\n(.*?)```", chapter, re.S)
            (project / "src/main.cor").write_text(app, encoding="utf-8")
            event_path = project / "src/events.cor"
            event_path.write_text(events, encoding="utf-8")
            build = [cli, "build", "src/main.cor", "--target", "web"]
            subprocess.run(build, cwd=project, env=env, check=True)
            archive = project / "target/web.tar"
            first = archive.read_bytes()
            subprocess.run(build, cwd=project, env=env, check=True)
            assert first == archive.read_bytes()
            event_path.write_text("agent invalid(\n", encoding="utf-8")
            assert subprocess.run(build, cwd=project, env=env).returncode != 0
            assert first == archive.read_bytes()
            installed = Path(tmp) / "installed"
            installed.mkdir()
            with tarfile.open(archive) as package:
                package.extractall(installed, filter="data")
            project.rename(Path(tmp) / "old-project")
            project = installed
            env["CORVID_HOME"] = str(installed)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        service = subprocess.Popen([cli, "serve", "src/main.cor", "--listen", f"127.0.0.1:{port}"],
                                   cwd=project, env=env)
        try:
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(channel=channel)
                page = browser.new_page()
                url = f"http://127.0.0.1:{port}"
                for _ in range(100):
                    if service.poll() is not None:
                        raise RuntimeError("Corvid service exited")
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                            break
                    except OSError:
                        time.sleep(0.1)
                page.goto(url)
                if bundle:
                    page.get_by_role("link", name="Open workspace").click()
                    expect(page).to_have_url(url + "/notes/workspace")
                    counter = page.get_by_role("group", name="Notes reviewed")
                    counter.get_by_role("button").click()
                    expect(counter.locator("output")).to_have_text("1")
                    assert page.request.get(url + "/target/wasm/events.wasm").headers["content-type"] == "application/wasm"
                    assert page.request.get(url + "/target/wasm/main.cor").status == 404
                status = page.locator("corvid-text-stream").get_by_role("status")
                output = page.locator("[data-output]")
                start = page.get_by_role("button", name=re.compile("^Start"))
                operation = page.request.get(url + "/openapi.json").json()["paths"]["/answer"]["get"]
                assert "text/event-stream" in operation["responses"]["200"]["content"]
                page.route("**/answer", lambda route: route.abort("connectionfailed"))
                start.click()
                expect(status).to_have_text("Connection interrupted")
                page.unroute("**/answer")
                fragment = "caf\u00e9 <milk>"
                for end in ("cancel", "broken", "done", "leave"):
                    start.click()
                    commands, closed = calls.get(timeout=6)
                    expect(status).to_have_text("Loading")
                    expect(output).to_be_empty()
                    commands.put(fragment)
                    expect(output).to_have_text(fragment)
                    expect(status).to_have_text("Streaming")
                    if end == "cancel":
                        page.get_by_role("button", name="Cancel").click()
                        expect(status).to_have_text("Cancelled")
                    elif end == "broken":
                        commands.put(None)
                        expect(status).to_contain_text("Stream failed")
                    elif end == "done":
                        commands.put("!")
                        commands.put("DONE")
                        expect(status).to_have_text("Complete")
                        expect(output).to_have_text(fragment + "!")
                    else:
                        page.goto(url + "/healthz")
                    assert closed.wait(6), "provider connection stayed open"
                page.go_back()
                expect(start).to_be_enabled()
                start.click()
                commands, closed = calls.get(timeout=6)
                commands.put("new answer")
                commands.put("DONE")
                expect(status).to_have_text("Complete")
                expect(output).to_have_text("new answer")
                assert closed.wait(6)
                browser.close()
        finally:
            shutdown.set()
            service.terminate()
            service.wait(timeout=10)
            provider.shutdown()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel")
    parser.add_argument("--bundle", action="store_true", help="exercise an extracted production bundle")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel, args.bundle)
