"""Exercise the reading-list handoff with real Corvid worker processes and a loopback model."""
from contextlib import closing
from html import escape
from http.server import ThreadingHTTPServer
from queue import Queue
from threading import Thread
import json
import sqlite3
import subprocess
from urllib.parse import urlencode
from urllib.request import Request


def handoff_trial(cli, installed, env, url, reviewer, peer, editor, page=None):
    from verify_reading_list import LocalModel
    with ThreadingHTTPServer(("127.0.0.1", 0), LocalModel) as model:
        model.response_timeout = 20
        model.requests, model.outputs = Queue(), Queue()
        Thread(target=model.serve_forever, daemon=True).start()
        env = dict(env, CORVID_MODEL=f"openai-compat:http://127.0.0.1:{model.server_port}/v1:local-model")
        try:
            check_handoff(cli, installed, env, model, url, reviewer, peer, editor, page)
        finally:
            model.shutdown()


def check_handoff(cli, installed, env, provider, url, reviewer, peer, editor, page=None):
    from verify_reading_list import read

    def post(path, fields, actor=reviewer):
        return read(Request(url + path, data=urlencode(fields).encode(), headers=actor))

    def progress(actor=reviewer):
        return read(Request(url + "/work", headers=actor))

    def click(label):
        with page.expect_navigation():
            page.get_by_role("button", name=label, exact=True).click(timeout=5000)

    def enqueue(count=1):
        for _ in range(count):
            subprocess.run([cli, "jobs", "enqueue", "--task", "extract_next", "--payload", "[]"],
                           cwd=installed, env=env, check=True, stdout=subprocess.DEVNULL)

    def worker(concurrency=1):
        return subprocess.Popen([cli, "jobs", "run", "--source", "src/extract.cor", "--workers",
                                 str(concurrency), "--max-runtime-ms", "2000"], cwd=installed, env=env)

    def finish(process):
        try:
            assert process.wait(timeout=30) == 0
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()

    def version():
        # The revision the progress page's forms carry.
        with closing(sqlite3.connect(installed / "reading.sqlite")) as db:
            return db.execute('SELECT revision FROM "ExtractionJob" WHERE job=?', (job,)).fetchone()[0]

    def request(expected):
        body = provider.requests.get(timeout=8)
        assert expected in json.dumps(body["messages"]), body

    with closing(sqlite3.connect(installed / "reading.sqlite")) as db:
        title, revision = db.execute("SELECT title,revision FROM StoredShelf WHERE shelf=1").fetchone()
    first = "Extract the first word"
    revised = "Extract the last word"
    if page:
        page.goto(url + "/search?term=")
        # Hold a real search open, then leave its owning document.
        with closing(sqlite3.connect(installed / "reading.sqlite")) as lock, lock:
            lock.execute("BEGIN EXCLUSIVE")
            with page.expect_request("**/search/results?term=close"):
                page.get_by_label("Find a title").fill("close")
            with page.expect_event("requestfailed", predicate=lambda r: r.url.endswith("term=close")) as cancelled:
                page.goto("about:blank")
            assert "ABORTED" in cancelled.value.failure
        page.goto(url + "/work")
        page.get_by_label("Model operation budget").fill("2")
        page.get_by_label("Estimated USD microdollars", exact=False).fill("7")
        click("Start extraction")
    else:
        started = post("/work/start", {"instruction": first, "budget": 2, "estimated_microusd": 7})
        assert "Started" in started, started
    with closing(sqlite3.connect(installed / "reading.sqlite")) as db:
        job, = db.execute("""SELECT job FROM "ExtractionJob" WHERE owner='reviewer' ORDER BY job DESC""").fetchone()
    assert 'Source revision ' + str(revision) in progress()
    assert "Shelf 1 | Pending" not in progress(peer)
    enqueue()
    running = worker()
    try:
        request(first)
        if page:
            page.reload()
            click("Pause")
            assert page.get_by_text("Pause requested. Waiting for in-flight work.").is_visible()
        else:
            assert "Pause" in post("/work/checkpoint", {"job": job, "version": version(), "command": "Pause"})
        assert "Shelf 1 | In flight" in progress()
        fields = dict(job=job, version=version(), shelf=1, correction="Reader field", instruction=revised)
        assert "unavailable" in post("/work/revise", fields)
        provider.outputs.put("Model field")
    finally:
        finish(running)
    assert "Paused at shelf checkpoint" in progress()
    assert "unavailable" in post("/work/revise", fields, peer)
    if page:
        page.reload()
        page.get_by_label("Correction for shelf 1").fill(fields["correction"])
        page.get_by_label("Revised instruction").fill(revised)
        click("Save revision")
    else:
        assert "Updated" in post("/work/revise", fields)
    assert "unavailable" in post("/work/revise", fields)
    # A replacement worker must observe the pause and leave the model budget alone.
    enqueue()
    finish(worker())
    post("/save", {"shelf": 1, "title": "Later source"}, editor)
    html = progress()
    assert f"Source revision {revision}: {escape(title)}" in html and "Later source" not in html
    assert "Human correction: Reader field" in html and "Model extraction: Model field" in html
    assert "1 of 2" in html and provider.requests.empty()
    if page:
        page.reload()
        assert page.get_by_text("Human correction: Reader field").is_visible()
        click("Resume")
    else:
        assert "Resume" in post("/work/checkpoint", dict(job=job, version=version(), command="Resume"))
    enqueue(2)
    running = worker(2)
    try:
        request(revised)
        if page:
            page.reload()
            assert page.get_by_text("Shelf 2 | In flight", exact=False).is_visible()
            context = page.context
            page.close()
        provider.outputs.put("Revised model field")
    finally:
        finish(running)
    html = progress()
    for value in ("Model extraction: Model field", "Human correction: Reader field",
                  "Model extraction: Revised model field", "Instruction used: " + revised,
                  "Shelf 3 | Skipped", "Operation budget exhausted", "2 of 2", "Estimated cost of reserved operations: 14 USD microdollars"):
        assert value in html, value
    assert provider.requests.empty()
    if page:
        page = context.new_page()
        page.goto(url + "/work")
        assert page.get_by_text("Shelf 3 | Skipped", exact=False).is_visible()
        assert page.get_by_text("Model extraction: Revised model field", exact=False).is_visible()
        page.close()
    # A failed model operation spends one reservation. Run its retry through the
    # ordinary CLI entry point, then ensure a worker cannot take a third attempt.
    started = post("/work/start", dict(instruction="Retry extraction", budget=2, estimated_microusd=0))
    assert "Started" in started, started
    provider.outputs.put(" ")
    enqueue()
    finish(worker())
    request("Retry extraction")
    assert "1 of 2" in progress() and "Shelf 1 | Pending" in progress()
    provider.outputs.put("Recovered field")
    subprocess.run([cli, "run", "--target", "interp", "src/extract.cor"],
                   cwd=installed, env=env, check=True, timeout=30)
    request("Retry extraction")
    enqueue()
    finish(worker())
    html = progress()
    assert "2 of 2" in html and "Model extraction: Recovered field" in html
    assert html.count("Operation budget exhausted.") == 2 and provider.requests.empty()
    print("Shelf checkpoint, correction, replacement worker, revised extraction, snapshot and budget passed.")
