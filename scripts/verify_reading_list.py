"""Exercise a newly packaged reading list and measure its loopback interactions."""
import argparse
from promote_reading_suggestion import SYSTEM, promote, user_message
from contextlib import closing, contextmanager, nullcontext
from html import escape
from html.parser import HTMLParser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib
import hmac
import json
import secrets
import os
import re
from pathlib import Path
import shutil
import socket
import sqlite3
import statistics
import subprocess
from queue import Queue
import tarfile
from tempfile import TemporaryDirectory
import time
from threading import Event, Thread
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import Request, urlopen


@contextmanager
def serve(cli, directory, env, port=None, source="src/main.cor"):
    if port is None:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
    address = f"127.0.0.1:{port}"
    process = subprocess.Popen([cli, "serve", source, "--listen", address],
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


def check_search(page, url, database):
    from playwright.sync_api import expect

    page.goto(url + "/search?term=Odyssey")
    field = page.get_by_label("Find a title")
    results = page.locator("#matches")
    expect(results.get_by_role("link")).to_have_text("The Odyssey")
    # Hold the real SQLite reader so that the second edit overtakes its fetch.
    with closing(sqlite3.connect(database)) as writer, writer:
        writer.execute("BEGIN EXCLUSIVE")
        with page.expect_request("**/search/results?term=Od"):
            field.fill("Od")
        expect(page.get_by_role("status")).to_have_text("Searching…")
        with page.expect_event("requestfailed", predicate=lambda r: r.url.endswith("term=Od")) as cancelled:
            field.fill("frank")
        assert "ABORTED" in cancelled.value.failure
    expect(results.get_by_role("link")).to_have_text("Frankenstein")
    field.press("Enter")
    expect(page).to_have_url(url + "/search?term=frank")
    page.go_back()
    expect(field).to_have_value("Odyssey")
    expect(field).to_be_focused()
    expect(results.get_by_role("link")).to_have_text("The Odyssey")
    page.go_forward()
    expect(field).to_have_value("frank")
    expect(results.get_by_role("link")).to_have_text("Frankenstein")
    page.reload()
    expect(results.get_by_role("link")).to_have_text("Frankenstein")


def check_stream_cleanup(cli, installed, env, channel, http_only):
    fixture = Path(__file__).resolve().parents[1] / "crates/corvid-cli/tests/fixtures/sqlite_service.cor"
    source = installed / "src/stream_trial.cor"
    source.write_text('import "std:ui" as ui\n' + fixture.read_text(encoding="utf-8") + """
    route GET "/" -> html:
        return ui.page("Stream trial", [ui.stream_text("Notes", "/events"), ui.link("Leave", "/left")])
    route GET "/left" -> html:
        return ui.page("Left", [ui.text("Left stream")])
""", encoding="utf-8")
    with serve(cli, installed, env, source="src/stream_trial.cor") as url:
        with closing(sqlite3.connect(installed / "notes.sqlite", timeout=0)) as db, db:
            def check_locked():
                try:
                    db.execute("BEGIN IMMEDIATE")
                except sqlite3.OperationalError as exc:
                    assert exc.sqlite_errorcode == sqlite3.SQLITE_BUSY
                else:
                    raise AssertionError("Stream did not hold its transaction")

            if http_only:
                with urlopen(url + "/events", timeout=5) as stream:
                    assert b"tick" in stream.readline()
                    check_locked()
            else:
                from playwright.sync_api import expect, sync_playwright
                with sync_playwright() as playwright:
                    browser = playwright.chromium.launch(channel=channel)
                    page = browser.new_page()
                    page.goto(url)
                    page.get_by_role("button", name="Start").click()
                    expect(page.get_by_role("status")).to_have_text("Streaming")
                    check_locked()
                    # Activate the native link without waiting for the streaming layout to settle.
                    page.get_by_role("link", name="Leave").press("Enter")
                    expect(page.get_by_text("Left stream")).to_be_visible()
                    wait_for_writer(db)
                    browser.close()
                    return
            wait_for_writer(db)


def wait_for_writer(db):
    deadline = time.monotonic() + 5
    while True:
        try:
            db.execute("BEGIN IMMEDIATE")
        except sqlite3.OperationalError as exc:
            if exc.sqlite_errorcode != sqlite3.SQLITE_BUSY or time.monotonic() >= deadline:
                raise
            time.sleep(0.05)
        else:
            db.rollback()
            return


def check_contract_change(cli, installed, env, page=None):
    def save_fields(url):
        schema = json.loads(read(url + "/openapi.json"))
        body = schema["paths"]["/save"]["post"]["requestBody"]
        ref = body["content"]["application/x-www-form-urlencoded"]["schema"]["$ref"]
        return schema["components"]["schemas"][ref.split("/")[-1]]["required"]

    # This extracted package is disposable. Keep the tab and migrated database
    # alive across the replacement of the service on the same origin.
    with serve(cli, installed, env) as url:
        port = int(url.rsplit(":", 1)[1])
        before = read(url)
        assert set(save_fields(url)) == {"shelf", "title"}
        if page:
            page.goto(url + "/edit?shelf=1")
            page.get_by_label("title", exact=True).fill("Stale edit")
    source = installed / "src/main.cor"
    original = source.read_text(encoding="utf-8")
    changed = original.replace("    title: String\n", "    book_title: String\n", 1)
    try:
        source.write_text(changed.replace("reading.title", "reading.book_title"), encoding="utf-8")
        subprocess.run([cli, "check"], cwd=installed, env=env, check=True)
        with serve(cli, installed, env, port) as url:
            assert set(save_fields(url)) == {"shelf", "book_title"}
            if page:
                with page.expect_response(url + "/save") as submitted:
                    page.get_by_role("button", name="Save to shelf").click()
                assert submitted.value.status == 400
                error = submitted.value.json()
                page.goto(url + "/edit?shelf=1")
                assert page.locator('[name="book_title"]').input_value() == "The Odyssey"
            else:
                try:
                    read(url + "/save", b"shelf=1&title=Stale+edit")
                except HTTPError as rejected:
                    assert rejected.code == 400
                    error = json.load(rejected)
                else:
                    raise AssertionError("Replacement server accepted the old form")
            assert error["error"] == "invalid_form" and "book_title" in error["detail"]
            assert read(url) == before
    finally:
        source.write_text(original, encoding="utf-8")


class LocalModel(BaseHTTPRequestHandler):
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.server.requests.put(request)
        output = self.server.outputs.get(timeout=getattr(self.server, "response_timeout", 5))
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


def check_ask_deadline(cli, installed, env, provider):
    from prototype_reading_list_ask import EXAMPLE, expand

    author = (EXAMPLE / "suggest_title.cor.in").read_text(encoding="utf-8")
    original = (installed / "src/main.cor").read_text(encoding="utf-8")
    assert expand(author) in original
    shorter = expand(author.replace("within 30 seconds", "within 2 seconds"))
    probe = installed / "src/deadline.cor"
    probe.write_text(original.replace(expand(author), shorter), encoding="utf-8")
    provider.waiting.clear()
    provider.closed.clear()

    def delay_first_answer():
        provider.requests.get(timeout=5)
        time.sleep(1.3)
        provider.outputs.put(action("Lookup", 1))
        provider.outputs.put(None)

    with serve(cli, installed, env, source="src/deadline.cor") as url:
        answer = Thread(target=delay_first_answer)
        answer.start()
        start = time.monotonic()
        try:
            read(url + "/suggest", b"shelf=2")
        except HTTPError as error:
            assert error.code == 504, (error.code, error.read())
        else:
            raise AssertionError("Suggestion exceeded its shared deadline")
        elapsed = time.monotonic() - start
        answer.join()
        assert provider.waiting.is_set() and provider.closed.wait(2)
        provider.requests.get(timeout=2)
        provider.waiting.clear()
        provider.closed.clear()
        assert elapsed < 3.2, f"Second model call received a fresh deadline: {elapsed:.2f}s"
        print(f"Two model calls shared the deadline and the second was cancelled after {elapsed:.2f}s.")


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
            self.tool = attrs.get("toolname")
        elif tag == "input":
            self.fields[attrs["name"]] = attrs["value"]


def check_suggestions(url, provider, original, channel, http_only, regression_dir):
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
    for request, context in [(first, "No lookup yet"), (second, "The Odyssey")]:
        assert request["messages"] == [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": user_message(original, context)}]
    assert "Call 1 returned Lookup(1). Lookup returned: The Odyssey" in preview
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
            title = 'Browser selection <private> & friends'
            provider.outputs.put(action("Lookup", 1))
            provider.outputs.put(action("Suggest", title))
            with page.expect_response(lambda response: response.url == url + "/suggest"
                                      and response.request.method == "POST") as captured:
                start.click()
            response = captured.value
            assert response.ok, response.status
            preview = response.text()
            assert PreviewForm(preview).fields == {"shelf": "2", "title": title}
            draft = page.locator("#preview").get_by_label("title", exact=True)
            expect(draft).to_have_value(title)
            page.get_by_text("Suggestion trace", exact=True).click()
            expect(page.get_by_text("Call 1 returned Lookup(1). Lookup returned: The Odyssey",
                                    exact=True)).to_be_visible()
            expect(page.get_by_text("recommend returned Suggest: " + title, exact=True)).to_be_visible()
            assert "Shelf 2: " + escape(original) in read(url)
            title += " edited"
            draft.fill(title)
            page.get_by_role("button", name="Apply suggestion").click()
            expect(page.get_by_text("Saved", exact=True)).to_be_visible()
            browser.close()
    promote(preview, regression_dir)
    private_title = PreviewForm(preview).fields["title"]
    for file in regression_dir.iterdir():
        sanitized = file.read_text(encoding="utf-8")
        assert all(secret not in sanitized for secret in [original, "The Odyssey", private_title])
    return title


def check_batch(cli, installed, env, provider, channel, http_only, webmcp=False, remote_mcp=False):
    proposed = {"first": "North & South", "second": "Emma", "third": "The Moonstone"}
    database = installed / "reading.sqlite"

    def shelves():
        with closing(sqlite3.connect(database)) as connection, connection:
            return connection.execute("SELECT title, revision FROM shelves ORDER BY shelf").fetchall()

    state = installed / "sessions"
    state.mkdir()
    csrf_secret = secrets.token_hex(32)
    env = dict(env, CORVID_SERVE_DATA_DIR=str(state.resolve()), CORVID_CSRF_SECRET=csrf_secret,
               CORVID_OAUTH_GOOGLE_CLIENT_ID="fixture", CORVID_OAUTH_GOOGLE_CLIENT_SECRET="fixture")

    def session(auth, actor):
        # Seed the durable state a completed login produces. No provider is contacted.
        token = secrets.token_hex(32)
        hashed = "sha256:" + hashlib.sha256(("corvid-auth-session-v1:" + token).encode()).hexdigest()
        binding = secrets.token_hex(16)
        mac = hmac.new(csrf_secret.encode(), ("corvid-csrf-v1:" + binding).encode(), "sha256")
        csrf = binding + "." + mac.hexdigest()
        with closing(sqlite3.connect(auth)) as db, db:
            db.execute("INSERT INTO auth_actors (id, tenant_id, display_name, actor_kind, auth_method, "
                       "assurance_level, role_fingerprint, permission_fingerprint, created_ms, updated_ms) "
                       "VALUES (?, 'library', ?, 'user', 'oauth', 'aal1', '', '', 0, 0)", (actor, actor))
            db.execute("INSERT INTO auth_sessions (id, actor_id, tenant_id, token_hash, issued_ms, "
                       "expires_ms, rotation_counter, csrf_binding_id, created_ms, updated_ms) "
                       "VALUES (?, ?, 'library', ?, 1, ?, 0, ?, 1, 1)",
                       (actor, actor, hashed, int(time.time() * 1000) + 600000, binding))
        return {"Cookie": f"corvid_session={token}; corvid_csrf={csrf}", "X-CSRF-Token": csrf}

    def set_role(auth, actor, role, granted):
        with closing(sqlite3.connect(auth)) as db, db:
            if granted:
                db.execute("INSERT INTO auth_actor_roles VALUES (?, ?, 1)", (actor, role))
            else:
                db.execute("DELETE FROM auth_actor_roles WHERE actor_id=? AND role=?", (actor, role))

    def post(endpoint, payload, headers):
        return read(Request(endpoint, data=payload, headers=headers))

    def refused(endpoint, payload, headers, status):
        try:
            post(endpoint, payload, headers)
        except HTTPError as error:
            assert error.code == status, (error.code, error.read())
        else:
            raise AssertionError(f"Expected HTTP {status} at {endpoint}")

    with serve(cli, installed, env, source="src/reviews.cor") as url:
        auth = state / "auth.sqlite"
        reviewer = session(auth, "reviewer")
        peer = session(auth, "other-reviewer")
        editor = session(auth, "editor")
        set_role(auth, "reviewer", "reviewer", True)
        set_role(auth, "other-reviewer", "reviewer", True)
        set_role(auth, "editor", "editor", True)
        before = shelves()
        check_search_http(url, database)
        assert 'action="/save"' in read(url + "/edit?shelf=2")
        refused(url + "/proposals/review", b"", editor, 403)
        provider.outputs.put(json.dumps(proposed))
        review = post(url + "/proposals/review", b"", reviewer)
        provider.requests.get(timeout=2)
        form = PreviewForm(review)
        assert form.action == "/proposals/apply" and shelves() == before
        assert form.tool == "apply_reviewed_titles"
        for name, title in proposed.items():
            assert form.fields[name] == title
        for current, _ in before:
            assert escape(current) in review
        endpoint = url + "/proposals/apply-result"
        # Discovery must be read-only and must not consume another model response.
        context_url = url + "/proposals/context"
        context_body = urlencode({"proposal_key": form.fields["proposal_key"]}).encode()
        scoped = post(context_url, context_body, reviewer)
        discovered = PreviewForm(scoped)
        assert (discovered.tool, discovered.action) == (form.tool, form.action)
        assert discovered.fields == {name: value if name == "proposal_key" else ""
                                     for name, value in form.fields.items()}
        for title in [*proposed.values(), *(current for current, _ in before)]:
            assert escape(title) not in scoped
        for context, identity in [(b"proposal_key=missing", reviewer), (context_body, peer)]:
            assert PreviewForm(post(context_url, context, identity)).action is None
        reader = session(auth, "read-only")
        expired = session(auth, "expired")
        set_role(auth, "expired", "reviewer", True)
        with closing(sqlite3.connect(auth)) as db, db:
            db.execute("UPDATE auth_sessions SET expires_ms = 2 WHERE actor_id = 'expired'")
        refused(context_url, context_body, expired, 401)
        refused(context_url, context_body, reader, 403)
        fields = form.fields
        assert json.loads(post(endpoint, urlencode(dict(fields, proposal_key="missing")).encode(), reviewer))["variant"] == "InvalidProposal"
        assert json.loads(post(endpoint, urlencode(dict(fields, third=" ")).encode(), reviewer))["variant"] == "InvalidProposal"
        assert shelves() == before
        fields["first"] += " (edited)"
        payload = urlencode(fields).encode()
        assert json.loads(post(endpoint, payload, peer))["variant"] == "InvalidProposal"
        set_role(auth, "reviewer", "reviewer", False)
        refused(context_url, context_body, reviewer, 403)
        for path in (form.action, "/proposals/apply-result"):
            refused(url + path, payload, {}, 401)
            refused(url + path, payload, reviewer, 403)
        assert shelves() == before
        set_role(auth, "reviewer", "reviewer", True)
        refused(endpoint, payload, {"Cookie": reviewer["Cookie"]}, 403)
        refused(url + "/save", b"shelf=2&title=Manual", reviewer, 403)
        post(url + "/save", b"shelf=2&title=Manual", editor)
        # Commit the operation, then discard the response before consuming its outcome.
        if remote_mcp:
            from check_review_mcp import exercise
            exercise(url, form, reviewer, peer, expired, reader,
                     lambda enabled: set_role(auth, "reviewer", "reviewer", enabled))
        else:
            with urlopen(Request(url + form.action, data=payload, headers=reviewer), timeout=10) as response:
                assert response.status == 200
        committed = [(fields["first"], before[0][1] + 1),
                     ("Manual", before[1][1] + 1), (proposed["third"], before[2][1] + 1)]
        assert shelves() == committed
    with serve(cli, installed, env, source="src/reviews.cor") as url:
        endpoint = url + "/proposals/apply-result"
        receipt = json.loads(post(endpoint, payload, reviewer))
        assert receipt["variant"] == "Reviewed"
        if remote_mcp:
            exercise(url, form, reviewer, peer, expired, reader, receipt=receipt)
        outcomes, = receipt["fields"]
        assert [(item["variant"], item["fields"]) for item in outcomes] == [
            ("SavedShelf", [1]), ("ChangedShelf", [2]), ("SavedShelf", [3])]
        assert json.loads(post(endpoint, payload, reviewer)) == receipt
        assert json.loads(post(endpoint, urlencode(dict(fields, third="Different")).encode(), reviewer))["variant"] == "KeyConflict"
        summary = post(url + form.action, payload, reviewer)
        assert "Shelf 2 changed since review" in summary
        assert all(f"Shelf {i} saved." in summary for i in (1, 3))
        assert shelves() == committed
        set_role(auth, "reviewer", "reviewer", False)
        refused(endpoint, payload, reviewer, 403)
        assert shelves() == committed
        if not http_only:
            from playwright.sync_api import expect, sync_playwright
            with sync_playwright() as playwright:
                flags = ["--enable-experimental-web-platform-features"] if webmcp else []
                browser = playwright.chromium.launch(channel=channel, args=flags)
                context = browser.new_context()
                for pair in reviewer["Cookie"].split("; "):
                    name, value = pair.split("=", 1)
                    context.add_cookies([{"url": url, "name": name, "value": value,
                                          "httpOnly": name == "corvid_session"}])
                page = context.new_page()
                page.goto(url)
                set_role(auth, "reviewer", "reviewer", True)
                provider.outputs.put(json.dumps(proposed))
                page.get_by_role("button", name="Suggest three titles").click()
                draft = page.locator('[name="first"]')
                expect(draft).to_have_value(proposed["first"])
                assert page.locator('[name="proposal_key"]').input_value() != fields["proposal_key"]
                if webmcp:
                    page.wait_for_function("""async () => (await document.modelContext.getTools())
                        .some(tool => tool.name === 'apply_reviewed_titles')""")
                    page.evaluate("""async () => {
                        window.reviewTool = (await document.modelContext.getTools())
                            .find(tool => tool.name === 'apply_reviewed_titles');
                    }""")
                else:
                    draft.fill("Edited in the browser")
                set_role(auth, "reviewer", "reviewer", False)
                apply = page.get_by_role("button", name="Apply reviewed titles")

                def prepare_call(first):
                    page.evaluate("""args => {
                        window.reviewResult = null;
                        document.modelContext.executeTool(window.reviewTool, args)
                            .then(value => { window.reviewResult = {value}; },
                                  error => { window.reviewResult = {error: String(error)}; });
                    }""", dict(proposed, first=first))
                    expect(draft).to_have_value(first)

                def check_reply(message):
                    page.wait_for_function("() => window.reviewResult !== null")
                    reply = page.evaluate("() => window.reviewResult")
                    assert message in reply.get("value", ""), reply

                if webmcp:
                    prepare_call("Attempt after revocation")
                    assert shelves() == committed
                apply.click()
                expect(page.get_by_role("status")).to_contain_text("Permission denied")
                if webmcp:
                    check_reply("Permission denied")
                assert shelves() == committed
                set_role(auth, "reviewer", "reviewer", True)
                post(url + "/save", b"shelf=2&title=New+manual+edit", editor)
                if webmcp:
                    prepare_call("Edited in the browser")
                apply.click()
                expect(page.get_by_text("Shelf 2 changed since review.", exact=False)).to_be_visible()
                if webmcp:
                    check_reply("Shelf 2 changed since review")
                assert shelves() == [("Edited in the browser", committed[0][1] + 1),
                                     ("New manual edit", committed[1][1] + 1),
                                     (proposed["third"], committed[2][1] + 1)]
                from check_reading_handoff import handoff_trial
                handoff_trial(cli, installed, env, url, reviewer, peer, editor, context.new_page())
                browser.close()
        else:
            from check_reading_handoff import handoff_trial
            set_role(auth, "reviewer", "reviewer", True)
            handoff_trial(cli, installed, env, url, reviewer, peer, editor)
    print("Replayed two saves and one conflict after restart with unchanged revisions.")


def check_search_http(url, database):
    html = read(url + "/search?term=ODYS")
    assert re.search(r'<a\b[^>]*\shref="/edit\?shelf=1"[^>]*>The Odyssey</a>', html)
    result = json.loads(read(url + "/search/results?term=ODYs"))
    assert result["variant"] == "Matches" and result["fields"] == [[{"shelf": 1, "title": "The Odyssey"}]]
    assert json.loads(read(url + "/search/results?term="))["fields"] == [[]]
    # Make the storage dependency unavailable in this disposable installation.
    with closing(sqlite3.connect(database)) as fixture, fixture:
        fixture.execute("ALTER TABLE shelves RENAME TO shelves_offline")
        try:
            assert json.loads(read(url + "/search/results?term=Od"))["variant"] == "Unavailable"
            assert "<p>Search unavailable. Try again.</p>" in read(url + "/search?term=Od")
        finally:
            fixture.execute("ALTER TABLE shelves_offline RENAME TO shelves")
    assert json.loads(read(url + "/search/results?term=Od")) == result


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


def check_browser(url, channel, database):
    from playwright.sync_api import expect, sync_playwright

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        page = browser.new_page()
        loads, saves = [], []
        check_search(page, url, database)
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
    cli = "corvid" if args.cli == Path("corvid") else args.cli.resolve()
    example = Path(__file__).resolve().parents[1] / "examples/reading_list"
    system_keys = {"systemroot", "windir", "temp", "tmp", "path", "userprofile",
                   "localappdata", "appdata", "comspec", "pathext", "corvid_home"}
    env = {k: v for k, v in os.environ.items() if k.lower() in system_keys}
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=False)
    workspace = nullcontext(args.work_dir) if args.work_dir else TemporaryDirectory()
    with workspace as tmp:
        root = Path(tmp).resolve()
        project = root / "reading-list"
        subprocess.run([cli, "new", project.name], cwd=root, env=env, check=True)
        shutil.copyfile(example / "src/main.cor", project / "src/main.cor")
        for name in ("reviews.cor", "handoff.cor", "extract.cor"):
            shutil.copyfile(example / "src" / name, project / "src" / name)
        subprocess.run([cli, "check", "src/extract.cor"], cwd=project, env=env, check=True)
        subprocess.run([cli, "check", "src/reviews.cor"], cwd=project, env=env, check=True)
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
            check_search_http(url, installed / "reading.sqlite")
            title = check_http(url)
            if not args.http_only:
                title = check_browser(url, args.channel, installed / "reading.sqlite")
        check_stream_cleanup(cli, installed, env, args.channel, args.http_only)
        with ThreadingHTTPServer(("127.0.0.1", 0), LocalModel) as provider:
            provider.requests, provider.outputs = Queue(), Queue()
            provider.waiting, provider.closed = Event(), Event()
            Thread(target=provider.serve_forever, daemon=True).start()
            local_env = dict(env, CORVID_TEST_MOCK_LLM="0", NO_PROXY="127.0.0.1",
                             CORVID_MODEL=f"openai-compat:http://127.0.0.1:{provider.server_port}/v1:local-model")
            try:
                check_ask_deadline(cli, installed, local_env, provider)
                with serve(cli, installed, local_env) as url:
                    title = check_suggestions(url, provider, title, args.channel, args.http_only, root / "regression")
                with serve(cli, installed, env) as url:
                    result = read(url)
                    for label in ["Shelf 1: The Odyssey", "Shelf 2: " + escape(title),
                                  "Shelf 3: Frankenstein"]:
                        assert label in result
                    print("Replacement server returned all three expected shelves.")
                if args.http_only:
                    check_contract_change(cli, installed, env)
                else:
                    from playwright.sync_api import sync_playwright
                    with sync_playwright() as playwright:
                        browser = playwright.chromium.launch(channel=args.channel)
                        check_contract_change(cli, installed, env, browser.new_page())
                        browser.close()
                check_batch(cli, installed, local_env, provider, args.channel, args.http_only,
                            args.webmcp, args.remote_mcp)
            finally:
                provider.shutdown()
        check_replay(cli, project, installed, root / "regression", env)


def check_replay(cli, project, installed, regression, env):
    source = (project / "src/main.cor").read_text(encoding="utf-8")
    test = (regression / "suggestion-test.cor").read_text(encoding="utf-8")
    suite = project / "src/suggestion-test.cor"
    shutil.copyfile(regression / "suggestion.jsonl", suite.with_name("suggestion.jsonl"))
    suite.write_text(source + test, encoding="utf-8")
    subprocess.run([cli, "test", suite], cwd=installed, env=env, check=True)

    with closing(sqlite3.connect(installed / "reading.sqlite")) as db:
        def shelves():
            return db.execute("SELECT * FROM shelves ORDER BY shelf").fetchall()

        before = shelves()
        probe = suite.with_name("replay-probe.cor")
        try:
            for changed in [
                source.replace('recommend(shelf.title, context)',
                               'recommend(shelf.title, "different context")'),
                source.replace('    first_choice = recommend',
                               '    saved = save(Reading(shelf.number, "unexpected write"))?\n'
                               '    first_choice = recommend'),
            ]:
                probe.write_text(changed + test, encoding="utf-8")
                result = subprocess.run([cli, "test", probe], cwd=installed, env=env,
                                        capture_output=True, text=True)
                assert result.returncode and "FAIL recorded_suggestion" in result.stdout, result
                assert shelves() == before
        finally:
            probe.unlink(missing_ok=True)
    print("Changed model context and an unexpected save failed replay. All shelf revisions matched.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--work-dir", type=Path, help="create and retain the trial in this new directory")
    parser.add_argument("--channel", help="Playwright Chromium channel, e.g. msedge")
    parser.add_argument("--remote-mcp", action="store_true", help="exercise the scoped HTTP adapter with the MCP SDK")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--http-only", action="store_true", help="skip the keyboard and browser timings")
    mode.add_argument("--webmcp", action="store_true", help="also discover and invoke the reviewed apply form")
    verify(parser.parse_args())
