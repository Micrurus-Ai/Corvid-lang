"""Compare a browser save through SQLite commit and verify all shelves after restart."""
import argparse
from contextlib import ExitStack, closing
from html import escape
import hashlib
from itertools import permutations
import json
import math
import os
from pathlib import Path
import re
import shutil
import sqlite3
import statistics
import subprocess
from tempfile import TemporaryDirectory

from playwright.sync_api import sync_playwright
from benchmark_node import node_server
from verify_browser_diagnostics import package
from verify_reading_list import read, serve
from verify_storefront import ROOT


SQL = ("SELECT sqlite_version() AS version, journal_mode AS journal, synchronous AS sync, "
       "foreign_keys AS fk, page_size AS page FROM pragma_journal_mode, pragma_synchronous, "
       "pragma_foreign_keys, pragma_page_size")
PROBE = """() => {
  document.addEventListener('keydown', e => {
    if (e.isTrusted && e.key === 'Enter' && e.target.matches('button[type=submit]')) {
      sessionStorage.setItem('saveStart', performance.timeOrigin + e.timeStamp);
      sessionStorage.setItem('savedLink', 'Shelf '+e.target.form.elements.shelf.value+': '+e.target.form.elements.title.value);
    }
  }, true);
  if (location.pathname !== '/save') return;
  const observer = new MutationObserver(() => {
    const saved = [...document.querySelectorAll('p')].find(p => p.textContent === 'Saved');
    const link = [...document.links].find(a => a.textContent === sessionStorage.savedLink);
    if (saved?.getClientRects().length && link?.getClientRects().length) {
      observer.disconnect();
      window.confirmMs = performance.timeOrigin + performance.now() - Number(sessionStorage.saveStart);
    }
  });
  observer.observe(document, {childList:true, subtree:true});
}"""


def instrument(source):
    path = source / "main.cor"
    code = path.read_text(encoding="utf-8")
    # Place clocks around the actual save handler's operations.
    start = code.index("agent save_and_show(")
    end = code.index("server library:", start)
    handler = code[start:end]
    for line, stamp in [
        ('    saved = save(reading)', 'started'),
        ('    readings = list_readings()', 'written'),
        ('    return Ok(screen', 'loaded'),
    ]:
        assert handler.count(line) == 1
        handler = handler.replace(line, f"    {stamp} = time_monotonic_ms()\n{line}")
    result = '    return Ok(screen(readings, saved.shelf, "Saved"))'
    assert handler.count(result) == 1
    handler = handler.replace(result, '''    html = screen(readings, saved.shelf, "Saved")
    ended = time_monotonic_ms()
    timings = [(written - started).to_string(), (loaded - written).to_string(), (ended - started).to_string()]
    return Ok(html + "<!--bench " + timings.join(" ") + "-->")''')
    code = code[:start] + handler + code[end:]
    probe = '''
agent database_settings() -> Result<List<JsonValue>, String>:
    db = db_open("reading.sqlite")?
    return db_query_rows(db, "SETTINGS_SQL", [])

server library:
    route GET "/settings" -> json List<JsonValue>:
        return match database_settings():
            Ok(settings) -> settings
            Err(_) -> []
'''
    code = code.replace("server library:", probe.replace("SETTINGS_SQL", SQL))
    path.write_text('import "std:db" use db_open, db_query_rows\n'
                    'import "std:time" use time_monotonic_ms\n' + code, encoding="utf-8")



def template(url, db):
    data = {"settings": json.loads(read(url + "/settings"))[0], "settingsSql": SQL, "forms": {}}
    for shelf, title in db.execute("SELECT shelf, title FROM StoredShelf ORDER BY shelf"):
        page = read(url + f"/edit?shelf={shelf}")
        data["prefix"], content = page.split("<ul>", 1)
        _, data["suffix"] = content.split("</ul>", 1)
        form = re.search(r"(<form\b.*?</script>)</li>", page, re.S)[1]
        assert title in form
        data["forms"][shelf] = form.replace(title, "__TITLE__")
    return data


def verify(args):
    cli = args.cli.resolve()
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        subprocess.run([cli, "new", "app"], cwd=root, check=True)
        project = root / "app"
        shutil.copytree(ROOT / "examples/reading_list/src", project / "src", dirs_exist_ok=True)
        shutil.copytree(ROOT / "examples/reading_list/migrations", project / "migrations")
        instrument(project / "src")
        corvid = root / "Corvid"
        package(cli, project, corvid)
        subprocess.run([cli, "migrate", "up", "--dir", "migrations", "--database", "reading.sqlite"], cwd=corvid, check=True)
        node = root / "TypeScript"
        node.mkdir()
        shutil.copyfile(corvid / "reading.sqlite", node / "reading.sqlite")
        templates = root / "templates.json"
        env = os.environ.copy()
        samples = []
        expected = {}
        metric = re.compile(rb"<!--bench ([0-9 ]+)-->$")
        with ExitStack() as services, sync_playwright() as pw:
            urls = {"Corvid": services.enter_context(serve(cli, corvid, env))}
            with closing(sqlite3.connect(corvid / "reading.sqlite")) as db:
                layout = template(urls["Corvid"], db)
            templates.write_text(json.dumps(layout), encoding="utf-8")
            urls["TypeScript"] = services.enter_context(node_server(node, ROOT / "benches/typescript/reading-save.mts", templates, "/settings"))
            node_settings = json.loads(read(urls["TypeScript"] + "/settings"))
            for key in ("journal", "sync", "fk", "page"):
                assert node_settings[key] == layout["settings"][key], (node_settings, layout["settings"])
            assert layout["settings"]["sync"] >= 2
            orders = list(permutations(urls))
            browser = pw.chromium.launch(channel=args.channel)
            try:
                pages = {name: browser.new_page() for name in urls}
                for page in pages.values():
                    page.add_init_script(f"({PROBE})()")
                for i in range(args.rounds + 3):
                    shelf = i % 3 + 1
                    title = f'Birds & books <{i}> "revised"'
                    receipts = []
                    for name in orders[(i-3) % len(orders)]:
                        page, url = pages[name], urls[name]
                        page.goto(url + f"/edit?shelf={shelf}")
                        page.get_by_label("title", exact=True).fill(title)
                        with page.expect_response(lambda r: r.url == url + "/save" and r.request.method == "POST") as response:
                            page.get_by_role("button", name="Save to shelf").press("Enter")
                        page.wait_for_url(url + "/save")
                        page.wait_for_function("window.confirmMs > 0")
                        assert response.value.ok
                        body = response.value.body()
                        timing = metric.search(body)
                        assert timing, body[-250:]
                        receipts.append(hashlib.sha256(body[:timing.start()]).hexdigest())
                        browser_ms, ttfb = page.evaluate("""[window.confirmMs, (() => {
                            const n = performance.getEntriesByType('navigation')[0];
                            return n.responseStart - n.requestStart;
                        })()]""")
                        write, query, handler = map(int, timing[1].split())
                        if i >= 3:
                            samples.append(dict(round=i-2, language=name, browser_ms=browser_ms,
                                                request_to_first_byte_ms=ttfb, write_ms=write, read_ms=query,
                                                handler_ms=handler, receipt_sha256=receipts[-1]))
                    assert len(set(receipts)) == 1, (i, receipts)
                    expected[shelf] = (title, expected.get(shelf, ("", 0))[1] + 1)
                version = browser.version
            finally:
                browser.close()
        for name in urls:
            directory = root / name
            service = (node_server(node, ROOT / "benches/typescript/reading-save.mts", templates, "/settings")
                       if name == "TypeScript" else serve(cli, directory, env))
            with service as url, closing(sqlite3.connect(directory / "reading.sqlite")) as db:
                rows = {s: (t, rev) for s, t, rev in db.execute("SELECT shelf, title, revision FROM StoredShelf")}
                assert rows == expected
                html = read(url)
                for shelf, (title, _) in rows.items():
                    assert f"Shelf {shelf}: {escape(title)}" in html
        meta = dict(browser=version, cli=subprocess.check_output([cli, "--version"], text=True).strip(),
                    node=subprocess.check_output(["node", "--version"], text=True).strip(),
                    sqlite={"Corvid": layout["settings"], "TypeScript": node_settings},
                    restart_revisions={s:t[1] for s,t in expected.items()}, samples=samples)
        args.output.write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
        for name in urls:
            for key in ("browser_ms", "request_to_first_byte_ms", "handler_ms", "write_ms", "read_ms"):
                values = sorted(s[key] for s in samples if s["language"] == name)
                print(name, key, "median / p95:", round(statistics.median(values), 2), round(values[math.ceil(.95*len(values))-1], 2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="chrome")
    parser.add_argument("--rounds", type=int, default=40)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    verify(args)
