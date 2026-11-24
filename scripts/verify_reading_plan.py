"""Package the reactive reading plan and check its projections through the public CLI."""
import argparse
from contextlib import nullcontext
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from verify_reading_list import read, serve


def check_browser(page, url):
    from playwright.sync_api import expect

    page.goto(url)
    first, second = page.locator("[data-local-selection]").all()
    rows = first.get_by_role("listitem")
    minutes = first.locator('[data-handler="reading_minutes"] > output')
    sessions = first.locator('[data-handler="review_sessions"] > output')
    expect(rows).to_have_count(3)
    expect(minutes).to_have_text("9")
    expect(sessions).to_have_text("1")
    note = rows.nth(1).get_by_label("Note")
    original = note.element_handle()
    note.fill("Remember this page")
    note.evaluate("node => node.setSelectionRange(2, 7, 'backward')")

    def check_rows(keys):
        expect(rows.locator("[data-projected-key]")).to_have_text(keys)
        assert original.evaluate("""node => node.isConnected && document.activeElement === node &&
            node.closest('li').dataset.corvidKey === '2' && node.value === 'Remember this page' &&
            node.selectionStart === 2 && node.selectionEnd === 7 && node.selectionDirection === 'backward'""")
        assert first.evaluate("""plan => [...plan.querySelectorAll('li')].every(row =>
            !window.previousRows.has(row.dataset.corvidKey) || window.previousRows.get(row.dataset.corvidKey) === row)""")
        remember_rows()

    def remember_rows():
        first.evaluate("""plan => { window.previousRows = new Map([...plan.querySelectorAll('li')]
            .map(row => [row.dataset.corvidKey, row])); }""")

    remember_rows()
    # Deliver model changes without transferring focus away from the selected note.
    pages = first.get_by_label("Pages", exact=True)
    pages.evaluate("""input => {
        const output = input.closest('[data-local-selection]').querySelector('output');
        window.writes = 0;
        const observer = new MutationObserver(records => window.writes += records.length);
        observer.observe(output, {childList: true, subtree: true, characterData: true});
        for (const pages of [4, 5, 6]) {
            input.value = pages;
            input.dispatchEvent(new Event('input', {bubbles: true}));
            if (output.value !== '9') throw Error('Projection rendered inside the batch');
        }
    }""")
    expect(minutes).to_have_text("18")
    assert page.evaluate("window.writes") == 1
    expect(sessions).to_have_text("2")
    expect(first.get_by_text("Schedule a break")).to_be_visible()
    check_rows(["1", "2", "3", "4", "5", "6"])
    pages.evaluate("input => { input.value = 5; input.dispatchEvent(new Event('input')); }")
    check_rows(["1", "2", "3", "4", "5"])
    first.get_by_label("Review even pages only").evaluate("box => box.click()")
    check_rows(["2", "4"])
    expect(first.get_by_text("Schedule a break")).to_be_hidden()
    first.get_by_label("Newest first").evaluate("box => box.click()")
    check_rows(["4", "2"])
    pages.evaluate("input => { input.value = 6; input.dispatchEvent(new Event('input')); }")
    check_rows(["6", "4", "2"])
    pages.evaluate("input => { input.value = 5; input.dispatchEvent(new Event('input')); }")
    check_rows(["4", "2"])
    expect(second.locator("output").first).to_have_text("9")
    expect(second.get_by_role("listitem")).to_have_count(3)

    # Disconnect after scheduling a write, then change the model while detached.
    projection = minutes.evaluate_handle("output => output.parentElement")
    pages.evaluate("""(input, view) => {
        input.value = 4;
        input.dispatchEvent(new Event('input'));
        view.remove();
        input.value = 0;
        input.dispatchEvent(new Event('input'));
    }""", projection)
    expect(rows).to_have_count(0)
    expect(sessions).to_have_text("0")
    assert projection.evaluate("view => view.textContent") == "15"
    first.evaluate("(plan, view) => plan.lastElementChild.before(view)", projection)
    expect(minutes).to_have_text("0")
    expect(first.get_by_role("alert")).to_be_empty()


def verify(cli, channel, package_only, work_dir):
    example = Path(__file__).resolve().parents[1] / "examples/reading_plan"
    if work_dir:
        work_dir.mkdir(parents=True)
    with nullcontext(work_dir) if work_dir else TemporaryDirectory() as tmp:
        project = Path(tmp) / "reading_plan"
        shutil.copytree(example, project, ignore=shutil.ignore_patterns("target"))
        for command in (["check", "src/main.cor"], ["check", "src/events.cor"],
                        ["build", "src/main.cor", "--target", "web"]):
            subprocess.run([cli, *command], cwd=project, check=True)
        installed = Path(tmp) / "installed"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        with serve(cli, installed, os.environ) as url:
            assert read(url).count('data-key-at="page_number"') == 2
            subprocess.run(["node", "--input-type=module", "-e", """
                import assert from 'node:assert/strict';
                import {readFile} from 'node:fs/promises';
                import {instantiate} from './target/wasm/events.js';
                globalThis.fetch = async url => new Response(await readFile(url), {
                    headers: {'Content-Type': 'application/wasm'}
                });
                const handlers = await instantiate();
                for (let pages = 0n; pages <= 12n; pages++) {
                    assert.equal(handlers.reading_minutes(pages), pages * 3n);
                    assert.equal(handlers.review_sessions(pages), pages === 0n ? 0n : pages <= 3n ? 1n : 2n);
                    for (const even of [false, true]) for (const reverse of [false, true]) {
                        assert.equal(handlers.needs_break(pages, even), pages > 3n && !even);
                        const keys = Array.from({length: Number(pages)}, (_, i) => BigInt(i + 1))
                            .filter(key => !even || key % 2n === 0n);
                        if (reverse) keys.reverse();
                        assert.equal(handlers.visible_pages(pages, even, reverse), BigInt(keys.length));
                        keys.forEach((key, i) =>
                            assert.equal(handlers.page_number(pages, even, reverse, BigInt(i)), key));
                    }
                }
            """], cwd=installed, check=True)
            print("Packaged reading plan, served markup and all 52 model combinations passed.", flush=True)
            if package_only:
                return
            from playwright.sync_api import sync_playwright
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(channel=channel)
                check_browser(browser.new_page(), url)
                browser.close()
                print("Reading plan browser checks passed.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--package-only", action="store_true")
    parser.add_argument("--work-dir", type=Path, help="retain the checked package in a new directory")
    args = parser.parse_args()
    verify(shutil.which(args.cli) or str(Path(args.cli).resolve()), args.channel, args.package_only, args.work_dir)
