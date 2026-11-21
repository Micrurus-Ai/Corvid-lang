"""Check candidate diagnostics, build both grammars and exercise the packaged controls."""
import argparse
from contextlib import ExitStack, nullcontext
from html import escape
import os
from pathlib import Path
import statistics
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from build_component_trial import FIXTURES, build, compile_handler, parse_view
from urllib.request import urlopen

from verify_reading_list import check_http, read, serve

# Start the clock at navigation and stop when the DOM binding enables the button.
OBSERVE = """new MutationObserver((_, observer) => {
    if (document.querySelector('button[data-corvid-action]:enabled')) {
        performance.mark('binding-ready');
        observer.disconnect();
    }
}).observe(document, {subtree:true, childList:true, attributes:true});"""

MEASURE = """() => {
    const button = document.querySelector('[data-corvid-action]');
    const output = button.closest('fieldset').querySelector('output');
    const first = performance.now();
    button.click();
    const firstMs = performance.now() - first;
    if (output.value !== '1') throw Error('first click did not update output');
    const start = performance.now();
    for (let i = 0; i < 1000; i++) button.click();
    const elapsed = performance.now() - start;
    if (output.value !== '1001') throw Error('unexpected count: ' + output.value);
    const entries = [...performance.getEntriesByType('navigation'),
                     ...performance.getEntriesByType('resource')];
    return {bytes: entries.filter(e => e.name.startsWith(location.origin + '/'))
                          .reduce((sum, e) => sum + e.encodedBodySize, 0),
            ready: performance.getEntriesByName('binding-ready')[0].startTime,
            first: firstMs, batch: elapsed};
}"""


def packages(cli, root, stack):
    events = FIXTURES / "events.cor"
    invalid = root / "events.cor"
    invalid.write_text("agent read_page(current: String) -> String:\n    return current\n")
    urls = {}
    for grammar in ("indented", "functions"):
        view = FIXTURES / (grammar + ".view")
        control = parse_view(view)
        try:
            compile_handler(view, control, invalid)
        except ValueError as error:
            line = next(n for n, s in enumerate(view.read_text().splitlines(), 1) if "action(" in s)
            assert f"{view}:{line}: action requires read_page(Int) -> Int" in str(error), error
            print(error)
        else:
            raise AssertionError("bad binding passed")
        for target in ("js", "wasm"):
            project = root / (grammar + "-" + target)
            build(cli, project, view, events, target)
            installed = project / "installed"
            installed.mkdir()
            with tarfile.open(project / "target/web.tar") as archive:
                archive.extractall(installed, filter="data")
            subprocess.run([cli, "migrate", "up", "--dir", "migrations", "--database",
                            "reading.sqlite"], cwd=installed, check=True)
            env = os.environ.copy()
            with serve(cli, installed, env) as first_url:
                with urlopen(first_url + "/edit?shelf=2", timeout=10) as response:
                    page = response.read()
                saved = check_http(first_url)
            urls[project.name] = stack.enter_context(serve(cli, installed, env))
            url = urls[project.name]
            listing = read(url)
            for label in ("Shelf 1: The Odyssey", "Shelf 2: " + escape(saved), "Shelf 3: Frankenstein"):
                assert label in listing, f"{project.name}: missing {label} after restart"
            assert b'data-corvid-action="read_page"' in page
            assets = []
            if target == "wasm":
                for ext in ("js", "wasm"):
                    with urlopen(url + "/target/wasm/events." + ext) as response:
                        assets.append(len(response.read()))
            print(f"{project.name}: {len(page)} HTML bytes + {sum(assets)} asset bytes")
    return urls


def browser_trial(urls, channel):
    from playwright.sync_api import sync_playwright
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        print(browser.version)
        samples = {name: [] for name in urls}
        for run in range(5):
            order = list(urls) if run % 2 == 0 else list(reversed(urls))
            for name in order:
                page = browser.new_page()
                page.add_init_script(OBSERVE)
                page.goto(urls[name] + "/edit?shelf=2")
                button = page.get_by_role("button", name="Read one page")
                button.wait_for()
                page.wait_for_function("performance.getEntriesByName('binding-ready').length")
                page.wait_for_load_state("networkidle")
                sample = page.evaluate(MEASURE)
                samples[name].append(sample)
                button.press("Enter")
                assert page.locator("fieldset output").text_content() == "1002"
                page.reload()
                assert page.locator("fieldset output").text_content() == "0"
                assert page.get_by_role("button", name="Save to shelf").is_enabled()
                page.close()
        for name, rows in samples.items():
            print(name, rows, {key: statistics.median(r[key] for r in rows) for key in rows[0]})
        browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--packages-only", action="store_true", help="skip real-browser measurements")
    parser.add_argument("--work-dir", type=Path, help="retain packages in a new directory")
    args = parser.parse_args()
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=False)
    with (nullcontext(args.work_dir) if args.work_dir else TemporaryDirectory()) as tmp, ExitStack() as stack:
        urls = packages(args.cli.resolve(), Path(tmp).resolve(), stack)
        if not args.packages_only:
            browser_trial(urls, args.channel)
