"""Time a real browser save during a burst of edits and a pending model request."""

import argparse
import hashlib
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


def instrument_writer_transaction(source):
    # Only the disposable benchmark app splits SQLite's implicit transaction.
    path = source / "main.cor"
    code = path.read_text(encoding="utf-8")
    start = code.index("agent save_and_show(")
    end = code.index("agent database_settings()", start)
    handler = code[start:end]
    call = "    saved = update_shelf(db, reading)?"
    comment = 'html + "<!--bench "'
    assert handler.count(call) == handler.count(comment) == 1
    handler = handler.replace(
        call,
        """    before_begin = time_monotonic_ms()
    db_execute(db, "BEGIN IMMEDIATE", [])?
    acquired = time_monotonic_ms()
    saved = update_shelf(db, reading)?
    updated = time_monotonic_ms()
    db_execute(db, "COMMIT", [])?
    committed = time_monotonic_ms()
    stages = [(acquired - before_begin).to_string(), (updated - acquired).to_string(), (committed - updated).to_string()]""",
    )
    handler = handler.replace(comment, 'html + "<!--writer " + stages.join(" ") + "--><!--bench "')
    path.write_text(code[:start] + handler + code[end:], encoding="utf-8")


def instrument_write_order(source):
    path = source / "main.cor"
    code = path.read_text(encoding="utf-8")
    update = "    saved = update_shelf(db, reading)?"
    comment = 'html + "<!--bench "'
    assert code.count(update) == code.count(comment) == 1
    code = code.replace(update, "    ready_to_write = time_monotonic_ms()\n" + update)
    code = code.replace(
        comment,
        'html + "<!--order " + started.to_string() + " " + ready_to_write.to_string() + "--><!--bench "',
    )
    path.write_text(code, encoding="utf-8")


def writer_stages(html):
    order = re.search(r"<!--order (\d+) (\d+)--><!--bench ", html)
    if order:
        return {"entry_ms": int(order[1]), "before_update_ms": int(order[2])}
    match = re.search(r"<!--writer ([0-9]+) ([0-9]+) ([0-9]+)--><!--bench ", html)
    return (
        dict(zip(("begin_ms", "update_ms", "commit_ms"), map(int, match.groups()))) if match else {}
    )


def peer_request(url, shelf, title, ready, start, read_only):
    ready.wait(timeout=10)
    if not start.wait(10):
        raise TimeoutError("Browser did not submit")
    begin = time.monotonic()
    duration = None
    stages = {}
    try:
        if read_only:
            html = read(url + f"/edit?shelf={shelf}")
            error = None if "Save to shelf" in html else "Missing edit form"
        else:
            html = read(url + "/save", urlencode({"shelf": shelf, "title": title}).encode())
            error = None if "<p>Saved</p>" in html else "Missing save confirmation"
            if not error:
                duration = handler_ms(html)
                stages = writer_stages(html)
    except OSError as exc:
        error = str(exc)
    return {
        "title": title,
        "read_only": read_only,
        "shelf": shelf,
        "handler_ms": duration,
        **stages,
        "ms": round(1000 * (time.monotonic() - begin), 2),
        "error": error,
    }


def burst(pool, page, url, model, start, count, pending, label, delay, read_only, mixed):
    start.clear()
    title = label + "-browser"
    page.goto(url + "/edit?shelf=1")
    page.get_by_label("title", exact=True).fill(title)
    ready = Barrier(count)
    peers = [
        pool.submit(
            peer_request,
            url,
            2 + i % 2,
            f"{label}-{i}",
            ready,
            start,
            read_only or (mixed and i % 2 == 0),
        )
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
        **(writer_stages(response.value.text()) if ok else {}),
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
            if not p["read_only"] and not p["error"]
        }
    )
    with closing(sqlite3.connect(path)) as db:
        log = db.execute("SELECT title,shelf FROM audit ORDER BY rowid").fetchall()
        assert len(log) == len(expected) and dict(log) == expected
        # The audit trigger runs in the save transaction. SQLite has one
        # writer, so the persisted rows also establish durable write order.
        positions = {title: i for i, (title, _) in enumerate(log)}
        for burst_result in samples:
            title = burst_result["browser_title"]
            if title in positions:
                burst_result["browser_commit_rank"] = 1 + sum(
                    positions[peer["title"]] < positions[title]
                    for peer in burst_result["peers"]
                    if peer["title"] in positions
                )
        html = read(url)
        for shelf, title, revision in db.execute("SELECT shelf,title,revision FROM shelves"):
            edits = [t for t, s in log if s == shelf]
            assert revision == len(edits) and (not edits or title == edits[-1])
            assert f"Shelf {shelf}: {title}" in html
    return len(log)


def report(samples, recovered):
    print(
        "Editors | Model pending | Stack | Browser median / p95 (ms) | Median commit position | Failed requests"
    )
    for count in sorted({sample["editors"] for sample in samples}):
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
                ranks = [row["browser_commit_rank"] for row in rows if "browser_commit_rank" in row]
                position = f"{median(ranks):g}" if ranks else "unavailable"
                print(f"{count} | {pending} | {name} | {times} | {position} | {failed}")
    if any(peer["read_only"] for row in samples for peer in row["peers"]):
        print("50-client edit-form reads | Model pending | Stack | HTTP median / p95 (ms)")
        for pending in (False, True):
            for name in recovered:
                times = sorted(
                    peer["ms"]
                    for row in samples
                    if row["round"] > 0
                    and row["stack"] == name
                    and row["editors"] == 50
                    and row["pending"] == pending
                    for peer in row["peers"]
                    if peer["read_only"] and not peer["error"]
                )
                if times:
                    print(
                        f"50 | {pending} | {name} | {median(times):.1f} / {times[math.ceil(0.95 * len(times)) - 1]:.1f}"
                    )
    print("Edits recovered after restart, including warm-up:", recovered)


def binary_digest(path):
    with path.open("rb") as binary:
        return hashlib.file_digest(binary, "sha256").hexdigest()


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
        if args.profile_writer:
            instrument_writer_transaction(project / "src")
        if args.profile_order:
            instrument_write_order(project / "src")
        reference = "Corvid baseline" if args.compare_cli else "TypeScript"
        corvid, other = root / "Corvid", root / reference
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
        if args.compare_cli:
            shutil.copytree(corvid, other)
        else:
            other.mkdir()
            shutil.copyfile(corvid / "reading.sqlite", other / "reading.sqlite")
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
            if args.compare_cli:
                return serve(args.compare_cli.resolve(), other, env)
            return node_server(
                other, ROOT / "benches/typescript/reading-save.mts", config, "/settings"
            )

        samples = []
        with ExitStack() as live:
            urls = {"Corvid": live.enter_context(server("Corvid"))}
            with closing(sqlite3.connect(corvid / "reading.sqlite")) as db:
                layout = template(urls["Corvid"], db)
            layout.update(sharedConnection=True, modelUrl=base + "/chat/completions")
            config.write_text(json.dumps(layout), encoding="utf-8")
            urls[reference] = live.enter_context(server(reference))
            settings = {n: json.loads(read(u + "/settings")) for n, u in urls.items()}
            settings["Corvid"] = settings["Corvid"][0]
            if args.compare_cli:
                settings[reference] = settings[reference][0]
            assert settings["Corvid"]["sync"] == 2
            for key in ["journal", "sync", "fk", "page"]:
                assert settings["Corvid"][key] == settings[reference][key]
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
                cases = [(count, pending) for count in args.editors for pending in (False, True)]
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
                            args.mixed_peers,
                        )
                        if args.profile_writer and name == "Corvid":
                            if result["browser_ms"] is not None:
                                assert "begin_ms" in result
                            if not args.read_only_peers:
                                assert all(
                                    peer["read_only"] or "begin_ms" in peer or peer["error"]
                                    for peer in result["peers"]
                                )
                        if args.profile_order:
                            if result["browser_ms"] is not None:
                                assert "before_update_ms" in result
                            assert all(
                                p["read_only"] or p["error"] or "before_update_ms" in p
                                for p in result["peers"]
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
                    "comparison": reference,
                    "binaries_sha256": {
                        name: binary_digest(path)
                        for name, path in (("Corvid", cli), (reference, args.compare_cli))
                        if path
                    },
                    "node": None
                    if args.compare_cli
                    else subprocess.check_output(["node", "--version"], text=True).strip(),
                    "mixed_peers": args.mixed_peers,
                    "sqlite": settings,
                    "delay_ms": args.delay_ms,
                    "profile_writer": args.profile_writer,
                    "profile_order": args.profile_order,
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
    parser.add_argument(
        "--editors",
        nargs="+",
        type=int,
        choices=(1, 10, 50),
        default=(1, 10, 50),
        help="burst sizes to measure",
    )
    parser.add_argument("--rounds", type=int, default=20)
    parser.add_argument("--delay-ms", type=int, default=500)
    peer_modes = parser.add_mutually_exclusive_group()
    peer_modes.add_argument(
        "--mixed-peers",
        action="store_true",
        help="alternate edit-form reads and saves in each burst",
    )
    parser.add_argument(
        "--compare-cli",
        type=Path,
        help="compare a second Corvid binary using the same packaged app",
    )
    peer_modes.add_argument(
        "--read-only-peers",
        action="store_true",
        help="have the other editors load edit forms while the browser saves",
    )
    probes = parser.add_mutually_exclusive_group()
    probes.add_argument(
        "--profile-order",
        action="store_true",
        help="record Corvid handler entry and pre-update timestamps",
    )
    probes.add_argument(
        "--profile-writer",
        action="store_true",
        help="time BEGIN IMMEDIATE, the update and COMMIT on Corvid",
    )
    args = parser.parse_args()
    if args.rounds < 1 or not 1 <= args.delay_ms <= 4000:
        parser.error("need positive rounds and a model delay between 1 and 4000 ms")
    if len(set(args.editors)) != len(args.editors):
        parser.error("each editor count must appear once")
    if args.profile_order and not args.compare_cli:
        parser.error(
            "--profile-order requires --compare-cli for matching Corvid clocks within each burst"
        )
    run(args)
