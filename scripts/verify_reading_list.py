"""Exercise a newly packaged reading list and measure its loopback interactions."""
import argparse
from contextlib import contextmanager, nullcontext
from html import escape
import os
from pathlib import Path
import shutil
import socket
import statistics
import subprocess
import tarfile
from tempfile import TemporaryDirectory
import time
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
