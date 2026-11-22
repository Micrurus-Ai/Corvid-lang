"""Check native authoring syntax through the CLI and the packaged applications."""
import argparse
from http.server import ThreadingHTTPServer
import os
from pathlib import Path
from queue import Queue
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory
from threading import Event, Thread
import time
from urllib.error import HTTPError

from playwright.sync_api import expect, sync_playwright
from verify_reading_list import LocalModel, action, read, serve

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"


def package(cli, root, name, mistake, correction):
    project = root / name
    shutil.copytree(EXAMPLES / name, project, ignore=shutil.ignore_patterns("target"))
    entry = project / "src/main.cor"
    source = entry.read_text(encoding="utf-8")
    line = source[:source.index(mistake)].count("\n") + 1
    entry.write_text(source.replace(mistake, correction, 1), encoding="utf-8")
    for command in (["check"], ["build", "src/main.cor", "--target", "web"]):
        invalid = subprocess.run([cli, *command], cwd=project, capture_output=True, encoding="utf-8")
        assert invalid.returncode and f"main.cor:{line}:" in invalid.stderr, invalid.stderr
    entry.write_text(source, encoding="utf-8")
    subprocess.run([cli, "check"], cwd=project, check=True)
    subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
    installed = project / "installed"
    with tarfile.open(project / "target/web.tar") as archive:
        archive.extractall(installed, filter="data")
    return installed


def verify(cli, channel):
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        counter = package(cli, root, "reading_counter", "pages_read += 1", "page_read += 1")
        with serve(cli, counter, os.environ.copy()) as url, sync_playwright() as playwright:
            browser = playwright.chromium.launch(channel=channel)
            page = browser.new_page()
            page.goto(url)
            page.get_by_role("button", name="Read one page").press("Enter")
            page.get_by_role("button", name="Read two pages").click()
            expect(page.locator("output")).to_have_text("3")
            page.reload()
            expect(page.locator("output")).to_have_text("0")
            browser.close()
        books = package(cli, root, "book_suggestions", "allow lookup_shelf once", "allow missing_lookup once")
        with ThreadingHTTPServer(("127.0.0.1", 0), LocalModel) as provider:
            provider.requests, provider.outputs = Queue(), Queue()
            provider.closed, provider.waiting = Event(), Event()
            Thread(target=provider.serve_forever, daemon=True).start()
            env = dict(os.environ, CORVID_TEST_MOCK_LLM="0", NO_PROXY="127.0.0.1",
                       CORVID_MODEL=f"openai-compat:http://127.0.0.1:{provider.server_port}/v1:local-model")
            try:
                with serve(cli, books, env) as url:
                    for responses, receipt in [
                        ([action("Suggest", "Dune")], "Dune"),
                        ([action("Lookup", 2), action("Suggest", "Dracula")], "Dracula"),
                        ([action("Lookup", 1), action("Lookup", 2)], "lookup limit"),
                        ([action("Lookup", 99)], "Unknown shelf"),
                    ]:
                        for response in responses:
                            provider.outputs.put(response)
                        assert receipt in read(url + "/suggest", b"title=Emma")
                        requests = [provider.requests.get(timeout=2) for _ in responses]
                        if receipt == "Dracula":
                            assert "Frankenstein" in requests[-1]["messages"][-1]["content"]
                # Shorten only the deadline in this disposable app.
                source = books / "src/main.cor"
                source.write_text(source.read_text().replace("within 30 seconds", "within 2 seconds"))
                with serve(cli, books, env) as url:
                    def slow_lookup():
                        provider.requests.get(timeout=5)
                        time.sleep(1.3)
                        provider.outputs.put(action("Lookup", 1))
                        provider.outputs.put(None)

                    responder = Thread(target=slow_lookup, daemon=True)
                    responder.start()
                    start = time.monotonic()
                    try:
                        read(url + "/suggest", b"title=Emma")
                    except HTTPError as error:
                        assert error.code == 504, error
                    else:
                        raise AssertionError("The bounded ask outlived its deadline.")
                    elapsed = time.monotonic() - start
                    responder.join(timeout=3)
                    assert provider.waiting.is_set() and provider.closed.wait(2)
                    assert elapsed < 3.2, elapsed
                    print(f"The second call was cancelled after {elapsed:.2f}s total.")
            finally:
                provider.shutdown()
    print("Native counter, bounded suggestion, original-source diagnostics and web packages passed.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="chrome")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
