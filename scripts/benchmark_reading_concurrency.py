"""Time a real browser save during a burst of edits and a pending model request."""

import argparse
import json
import math
import os
import re
import shutil
import sqlite3
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack, closing
from http.server import ThreadingHTTPServer
from pathlib import Path
from queue import Queue
from statistics import median
from tempfile import TemporaryDirectory
from threading import Barrier, Event, Thread, Timer
from urllib.parse import urlencode

from benchmark_node import node_server
from benchmark_reading_save import PROBE, instrument, template
from playwright.sync_api import sync_playwright
from verify_browser_diagnostics import package
from verify_reading_list import LocalModel, action, read, serve
from verify_storefront import ROOT


def handler_ms(html):
    match = re.search(r"<!--bench \d+ \d+ (\d+)-->$", html)
    if not match:
        raise ValueError("Missing save handler timing")
    return int(match[1])


def peer_request(url, shelf, title, ready, start, read_only):
    ready.wait(timeout=10)
    if not start.wait(10):
        raise TimeoutError("Browser did not submit")
    begin = time.monotonic()
    duration = None
    try:
        if read_only:
            html = read(url + f"/edit?shelf={shelf}")
            error = None if "Save to shelf" in html else "Missing edit form"
        else:
            html = read(url + "/save", urlencode({"shelf": shelf, "title": title}).encode())
            error = None if "<p>Saved</p>" in html else "Missing save confirmation"
            if not error:
                duration = handler_ms(html)
    except OSError as exc:
        error = str(exc)
    return {
        "title": title,
        "shelf": shelf,
        "handler_ms": duration,
        "ms": round(1000 * (time.monotonic() - begin), 2),
        "error": error,
    }


def burst(pool, page, url, model, start, count, pending, label, delay, read_only):
    start.clear()
    title = label + "-browser"
    page.goto(url + "/edit?shelf=1")
    page.get_by_label("title", exact=True).fill(title)
    ready = Barrier(count)
    peers = [
        pool.submit(peer_request, url, 2 + i % 2, f"{label}-{i}", ready, start, read_only)
        for i in range(count - 1)
    ]
    ready.wait(timeout=10)
    release_times = []
    if pending:
        suggestion = pool.submit(read, url + "/suggest", b"shelf=1")
        model.requests.get(timeout=8)

        # Start the identical provider delay when the request reaches the fixture.
        def answer():
            release_times.append(time.time_ns() / 1e6)
            model.outputs.put(action("Suggest", "Concurrent suggestion"))

        release = Timer(delay, answer)
        release.start()
    with page.expect_response(
        lambda r: r.url == url + "/save" and r.request.method == "POST"
    ) as response:
        page.get_by_role("button", name="Save to shelf").press("Enter")
    page.wait_for_url(url + "/save")
    ok = response.value.ok and "<p>Saved</p>" in response.value.text()
    browser_ms = None
    if ok:
        page.wait_for_function("window.confirmMs > 0")
        browser_ms = round(page.evaluate("window.confirmMs"), 2)
    if pending:
        assert "Concurrent suggestion" in suggestion.result(timeout=12)
        release.join()
        keydown = page.evaluate("Number(sessionStorage.saveStart)")
        remaining_ms = round(release_times[0] - keydown, 2)
        assert remaining_ms > 0, "The model had already answered before Save"
    else:
        remaining_ms = None
    return {
        "editors": count,
        "pending": pending,
        "read_only_peers": read_only,
        "browser_handler_ms": handler_ms(response.value.text()) if ok else None,
        "browser_title": title,
        "browser_ms": browser_ms,
        "provider_ms_remaining_at_save": remaining_ms,
        "browser_status": response.value.status,
        "peers": [f.result() for f in peers],
    }


def check_database(url, path, samples):
    expected = {r["browser_title"]: 1 for r in samples if r["browser_ms"] is not None}
    expected.update(
        {
            p["title"]: p["shelf"]
            for r in samples
            if not r["read_only_peers"]
            for p in r["peers"]
            if not p["error"]
        }
    )
    with closing(sqlite3.connect(path)) as db:
        log = db.execute("SELECT title,shelf FROM audit ORDER BY rowid").fetchall()
        assert len(log) == len(expected) and dict(log) == expected
        html = read(url)
        for shelf, title, revision in db.execute("SELECT shelf,title,revision FROM shelves"):
            edits = [t for t, s in log if s == shelf]
            assert revision == len(edits) and (not edits or title == edits[-1])
            assert f"Shelf {shelf}: {title}" in html
    return len(log)


def report(samples, recovered):
    print("Editors | Model pending | Stack | Browser median / p95 (ms) | Failed requests")
    for count in (1, 10, 50):
        for pending in (False, True):
            for name in recovered:
                rows = [
                    row
                    for row in samples
                    if row["round"] > 0
                    and row["stack"] == name
                    and row["editors"] == count
                    and row["pending"] == pending
                ]
                values = sorted(row["browser_ms"] for row in rows if row["browser_ms"] is not None)
                failed = len(rows) - len(values)
                failed += sum(bool(peer["error"]) for row in rows for peer in row["peers"])
                times = "no confirmations"
                if values:
                    p95 = values[math.ceil(0.95 * len(values)) - 1]
                    times = f"{median(values):.1f} / {p95:.1f}"
                print(f"{count} | {pending} | {name} | {times} | {failed}")
    print("Edits recovered after restart, including warm-up:", recovered)


def run(args):
    cli = args.cli.resolve()
    with TemporaryDirectory() as tmp, ExitStack() as stack:
        root = Path(tmp)
        project = root / "app"
        subprocess.run([cli, "new", "app"], cwd=root, check=True)
        for folder in ["src", "migrations"]:
            shutil.copytree(
                ROOT / "examples/reading_list" / folder, project / folder, dirs_exist_ok=True
            )
        instrument(project / "src")
        corvid, node = root / "Corvid", root / "TypeScript"
        package(cli, project, corvid)
        subprocess.run(
            [cli, "migrate", "up", "--dir", "migrations", "--database", "reading.sqlite"],
            cwd=corvid,
            check=True,
        )
        with closing(sqlite3.connect(corvid / "reading.sqlite")) as db:
            db.executescript("""CREATE TABLE audit(title TEXT, shelf INTEGER);
                CREATE TRIGGER record_edit AFTER UPDATE ON shelves BEGIN
                INSERT INTO audit VALUES(new.title,new.shelf); END;""")
        node.mkdir()
        shutil.copyfile(corvid / "reading.sqlite", node / "reading.sqlite")
        model = stack.enter_context(ThreadingHTTPServer(("127.0.0.1", 0), LocalModel))
        model.requests, model.outputs = Queue(), Queue()
        Thread(target=model.serve_forever, daemon=True).start()
        stack.callback(model.shutdown)
        base = f"http://127.0.0.1:{model.server_port}/v1"
        env = dict(
            os.environ,
            CORVID_TEST_MOCK_LLM="0",
            NO_PROXY="127.0.0.1",
            CORVID_MODEL=f"openai-compat:{base}:local-model",
        )
        config = root / "templates.json"

        def server(name):
            if name == "Corvid":
                return serve(cli, corvid, env)
            return node_server(
                node, ROOT / "benches/typescript/reading-save.mts", config, "/settings"
            )

        samples = []
        with ExitStack() as live:
            urls = {"Corvid": live.enter_context(server("Corvid"))}
            with closing(sqlite3.connect(corvid / "reading.sqlite")) as db:
                layout = template(urls["Corvid"], db)
            layout.update(sharedConnection=True, modelUrl=base + "/chat/completions")
            config.write_text(json.dumps(layout), encoding="utf-8")
            urls["TypeScript"] = live.enter_context(server("TypeScript"))
            settings = {n: json.loads(read(u + "/settings")) for n, u in urls.items()}
            settings["Corvid"] = settings["Corvid"][0]
            assert settings["Corvid"]["sync"] == 2
            for key in ["journal", "sync", "fk", "page"]:
                assert settings["Corvid"][key] == settings["TypeScript"][key]
            browser = live.enter_context(sync_playwright()).chromium.launch(channel="chrome")
            live.callback(browser.close)
            pool = live.enter_context(ThreadPoolExecutor(max_workers=54))
            start = Event()
            page = browser.new_page()
            page.expose_function("releasePeers", start.set)
            page.add_init_script(
                f"({PROBE})(); document.addEventListener('keydown', e => {{if(e.isTrusted && e.key==='Enter') window.releasePeers() }},true)"
            )
            version = browser.version
            for round_no in range(args.rounds + 1):
                cases = [(count, pending) for count in (1, 10, 50) for pending in (False, True)]
                if round_no % 2:
                    cases.reverse()
                for count, pending in cases:
                    order = list(urls)
                    if (round_no + int(pending)) % 2:
                        order.reverse()
                    for name in order:
                        label = f"r{round_no}-n{count}-p{int(pending)}"
                        result = burst(
                            pool,
                            page,
                            urls[name],
                            model,
                            start,
                            count,
                            pending,
                            label,
                            args.delay_ms / 1000,
                            args.read_only_peers,
                        )
                        samples.append(dict(stack=name, round=round_no, **result))
        recovered = {}
        for name in urls:
            with server(name) as url:
                recovered[name] = check_database(
                    url, root / name / "reading.sqlite", [s for s in samples if s["stack"] == name]
                )
        args.output.write_text(
            json.dumps(
                {
                    "chrome": version,
                    "cli": subprocess.check_output([cli, "--version"], text=True).strip(),
                    "node": subprocess.check_output(["node", "--version"], text=True).strip(),
                    "sqlite": settings,
                    "delay_ms": args.delay_ms,
                    "recovered": recovered,
                    "samples": samples,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

        report(samples, recovered)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rounds", type=int, default=20)
    parser.add_argument("--delay-ms", type=int, default=500)
    parser.add_argument(
        "--read-only-peers",
        action="store_true",
        help="have the other editors load edit forms while the browser saves",
    )
    args = parser.parse_args()
    if args.rounds < 1 or not 1 <= args.delay_ms <= 4000:
        parser.error("need positive rounds and a model delay between 1 and 4000 ms")
    run(args)
