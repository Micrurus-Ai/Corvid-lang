"""Exercise the browser views chapter with a built CLI and Playwright Chromium."""

import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import re
import socket
import time
import subprocess
from tempfile import TemporaryDirectory
from threading import Thread

from playwright.sync_api import expect, sync_playwright


def verify(cli, channel):
    root = Path(__file__).resolve().parents[1]
    chapter = (root / "docs/book/19-browser-views.md").read_text(encoding="utf-8")
    examples = chapter.split("## Updating state with a Corvid handler")[1].split("## Serving a form and its result", 1)[0]
    handlers, page_source = re.findall(r"```corvid(?:-fragment)?\n(.*?)```", examples, re.S)
    with TemporaryDirectory() as tmp:
        project = Path(tmp) / "tallies"
        env = dict(os.environ, CORVID_HOME=str(root), CORVID_IO_ROOT=str(project))

        def run(*args, cwd=project):
            subprocess.run([cli, *args], cwd=cwd, env=env, check=True)

        run("new", "tallies", cwd=tmp)
        (project / "src/events.cor").write_text(handlers, encoding="utf-8")
        main = project / "src/main.cor"
        main.write_text(page_source, encoding="utf-8")
        run("build", "src/events.cor", "--target", "wasm")
        run("run", "--target", "interp", "src/main.cor")
        with ThreadingHTTPServer(("127.0.0.1", 0), partial(SimpleHTTPRequestHandler, directory=project)) as server:
            thread = Thread(target=server.serve_forever)
            thread.start()
            try:
                with sync_playwright() as playwright:
                    browser = playwright.chromium.launch(channel=channel)
                    tab = browser.new_page()
                    tab.goto(f"http://127.0.0.1:{server.server_port}/index.html")
                    apples = tab.get_by_role("group", name="Apples")
                    pears = tab.get_by_role("group", name="Pears")
                    button = apples.get_by_role("button")
                    button.click()
                    expect(apples.locator("output")).to_have_text("2")
                    button.press("Space")
                    expect(apples.locator("output")).to_have_text("4")
                    expect(button).to_be_focused()
                    expect(pears.locator("output")).to_have_text("10")
                    pears.get_by_role("button").press("Enter")
                    expect(pears.locator("output")).to_have_text("12")
                    expect(apples.locator("output")).to_have_text("4")

                    note = tab.get_by_role("textbox", name="Note")
                    preview = tab.locator("fieldset").filter(has=note).locator("output")
                    expect(preview).to_have_text("Write a note")
                    note.fill("Pr\u00eat <&>")
                    expect(preview).to_have_text("Pr\u00eat <&>")
                    expect(note).to_be_focused()
                    note.fill("done")
                    expect(preview).to_have_text("Ready to save")
                    note.fill("dune")
                    expect(preview).to_have_text("dune")
                    note.fill("")
                    expect(preview).to_have_text("Write a note")

                    main.write_text(page_source.replace('"add_pair"', '"absent"').replace('"preview"', '"add_pair"'), encoding="utf-8")
                    run("run", "--target", "interp", "src/main.cor")
                    tab.reload()
                    expect(apples.get_by_role("alert")).to_contain_text("absent(Int) -> Int")
                    expect(button).to_be_disabled()
                    expect(note).to_be_disabled()
                    expect(tab.get_by_text("Expected a Corvid handler add_pair(String) -> String.")).to_be_visible()
                    browser.close()
            finally:
                server.shutdown()
                thread.join()

        records, app = re.findall(r"```corvid-fragment\n(.*?)```", chapter.split("## Serving a form and its result")[1].split("\n## ", 1)[0], re.S)
        model = project / "src/search.cor"
        model.write_text(records, encoding="utf-8")
        main.write_text(app, encoding="utf-8")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            address = "127.0.0.1:" + str(sock.getsockname()[1])
        service = subprocess.Popen([cli, "serve", "src/main.cor", "--listen", address], cwd=project, env=env)
        try:
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(channel=channel)
                page = browser.new_page()
                url = "http://" + address
                for _ in range(100):
                    if service.poll() is not None:
                        raise RuntimeError("Corvid service stopped during startup")
                    try:
                        with socket.create_connection(("127.0.0.1", int(address.split(":")[1])), timeout=0.2):
                            break
                    except OSError:
                        time.sleep(0.1)
                response = page.goto(url)
                assert response.status == 200 and response.headers["content-type"].startswith("text/html")
                phrase = "tea & <milk>"
                page.locator("form").first.get_by_label("term", exact=True).fill(phrase)
                page.get_by_role("button", name="Search", exact=True).click()
                expect(page.locator("p")).to_have_text(phrase)
                page.reload()
                expect(page.locator("form").first.get_by_label("term", exact=True)).to_have_value(phrase)
                save = page.locator('form[action="/save"]')
                save.get_by_label("term", exact=True).fill("saved " + phrase)
                save.get_by_role("button").click()
                expect(page.locator("p")).to_have_text("saved " + phrase)
                assert (project / "saved-search.txt").read_text() == "saved " + phrase
                malformed = page.request.post(url + "/save", form={"term":"changed", "limit":"wrong"})
                assert malformed.status == 400 and malformed.json()["error"] == "invalid_form"
                outside = page.request.post(url + "/save", form={"term": "changed", "limit": 51})
                assert outside.status == 400 and "between" in outside.json()["detail"]
                assert page.request.post(url + "/save", data={"term":"changed","limit":1}).status == 415
                assert (project / "saved-search.txt").read_text() == "saved " + phrase
                invalid = page.request.get(url + "/search?term=x&limit=wrong")
                assert invalid.status == 400 and invalid.json()["error"] == "invalid_query"
                posted = page.request.post(url + "/preview", data={"term": "meeting", "limit": 2})
                assert posted.status == 200 and "<p>meeting</p>" in posted.text()
                bad_body = page.request.post(url + "/preview", data={"term": "meeting", "limit": "two"})
                assert bad_body.status == 400 and bad_body.json()["error"] == "invalid_body"
                spec = page.request.get(url + "/openapi.json").json()
                assert "application/x-www-form-urlencoded" in spec["paths"]["/save"]["post"]["requestBody"]["content"]
                properties = spec["components"]["schemas"]["Search"]["properties"]
                assert properties["term"]["type"] == "string"
                assert properties["limit"]["type"] == "integer"
                assert "text/html" in spec["paths"]["/search"]["get"]["responses"]["200"]["content"]
                browser.close()
        finally:
            service.terminate()
            service.wait(timeout=10)

        def check_fails(expected):
            result = subprocess.run([cli, "check", "src/main.cor"], cwd=project, env=env, capture_output=True)
            assert result.returncode and expected.encode() in result.stderr, result.stderr

        main.write_text(app.replace('"GET /search"', '"GET /missing"'), encoding="utf-8")
        check_fails("no declared route")
        main.write_text(app.replace('"POST /save", request', '"POST /save", request.term'), encoding="utf-8")
        check_fails("initial values")
        main.write_text(app, encoding="utf-8")
        model.write_text(records.replace("term: String", "phrase: String"), encoding="utf-8")
        check_fails("term")
        model.write_text(records, encoding="utf-8")
        main.write_text(app.replace("return search_page(query)", "return query.limit"), encoding="utf-8")
        check_fails("String")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", help="installed Chromium channel, for example msedge")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
