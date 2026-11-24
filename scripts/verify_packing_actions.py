"""Check component callbacks, resource cleanup and portals in the packaged packing list."""
import argparse
from contextlib import nullcontext
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory

from playwright.sync_api import expect, sync_playwright
from verify_browser_diagnostics import package
from verify_reading_list import serve


def verify(cli, channel, work_dir=None, callbacks_only=False):
    example = Path(__file__).resolve().parents[1] / "examples/packing_actions"
    if work_dir:
        work_dir.mkdir(parents=True)
    with nullcontext(work_dir) if work_dir else TemporaryDirectory() as tmp:
        project = Path(tmp) / "packing-list"
        shutil.copytree(example, project, ignore=shutil.ignore_patterns("target"))
        installed = Path(tmp) / "installed"
        package(cli, project, installed)
        if callbacks_only:
            check_compiled_callbacks(installed)
        else:
            exercise(cli, installed, channel)


def check_compiled_callbacks(project):
    subprocess.run(["node", "--input-type=module", "-e", """
        import assert from 'node:assert/strict';
        import {readFile} from 'node:fs/promises';
        import {instantiate} from './target/wasm/events.js';
        // The packaged loader fetches a sibling Wasm file. Node's fetch
        // supports HTTP, so supply its local file response for this check.
        globalThis.fetch = async url => {
            assert.equal(url.protocol, 'file:');
            url = new URL(url);
            url.search = '';
            return new Response(await readFile(url), {headers: {'Content-Type': 'application/wasm'}});
        };
        const calls = [];
        const callbacks = await instantiate({tools: {
            show: () => calls.push('show'),
            close: () => calls.push('close'),
        }});
        assert.equal(callbacks.add_spare(4n), 5n);
        assert.equal(callbacks.add_pair(4n), 6n);
        callbacks.open_instructions();
        callbacks.close_instructions();
        assert.deepEqual(calls, ['show', 'close']);
    """], cwd=project, check=True)
    print("Packaged integer and lifecycle callbacks passed in Node.")


def exercise(cli, project, channel):
    with serve(cli, project, os.environ) as url, sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        for scripting in (False, True):
            page = browser.new_page(java_script_enabled=scripting)
            page.goto(url)
            owner = page.get_by_role("complementary", name="Map")
            target = page.get_by_role("region", name="Mark packed items")
            if not scripting:
                expect(owner.get_by_label("Map packed")).to_be_visible()
                expect(target.get_by_role("checkbox")).to_have_count(0)
            else:
                water = page.get_by_role("complementary", name="Water")
                owner.get_by_role("button", name="Add spares").click()
                expect(owner.locator("output")).to_have_text("1")
                expect(water.locator("output")).to_have_text("0")
                water.get_by_role("button", name="Add spares").click()
                expect(water.locator("output")).to_have_text("2")
                dialog = page.get_by_role("dialog", name="Packing instructions")
                expect(dialog).to_be_visible()
                resource = dialog.evaluate_handle("dialog => dialog.parentElement")
                resource.evaluate("owner => owner.remove()")
                assert not resource.evaluate("owner => owner.querySelector('dialog').open")
                resource.evaluate("owner => document.body.append(owner)")
                expect(dialog).to_be_visible()
                resource.evaluate("""async original => {
                    const owner = original.cloneNode(true);
                    const tracker = document.createElement('packing-observer');
                    owner.firstElementChild.replaceWith(tracker);
                    document.body.append(owner);
                    await Promise.resolve();
                    owner.remove();
                    let changes = 0, releases = 0, mounted, fail = true;
                    const changed = () => changes++;
                    customElements.define('packing-observer', class extends HTMLElement {
                        show() {
                            window.addEventListener('packing-change', changed);
                            mounted();
                            if (fail) throw Error('Packing observer unavailable');
                        }
                        close() {
                            releases++;
                            window.removeEventListener('packing-change', changed);
                        }
                    });
                    await Promise.resolve();
                    const change = () => window.dispatchEvent(new Event('packing-change'));
                    change();
                    if (changes || releases) throw Error('Detached owner acquired a resource');
                    for (let mount = 0; mount <= 2; mount++) {
                        const ready = new Promise((resolve, reject) => {
                            const timeout = setTimeout(() => reject(Error('Observer did not mount')), 5000);
                            mounted = () => { clearTimeout(timeout); resolve(); };
                        });
                        document.body.append(owner);
                        await ready;
                        const alert = owner.querySelector('[role=alert]');
                        if (fail && !alert.textContent.includes('Packing observer unavailable'))
                            throw Error('Missing setup failure');
                        if (!fail && alert.textContent) throw Error('Remount did not recover');
                        change();
                        owner.remove();
                        change();
                        if (changes !== mount || releases !== mount + 1)
                            throw Error('Expected one subscription and release per mount');
                        fail = false;
                    }
                }""")
                expect(target.get_by_role("checkbox")).to_have_count(2)
                expect(owner.get_by_role("checkbox")).to_have_count(0)
                checkbox = target.get_by_label("Map packed")
                checkbox.check()
                # Keep handles so the assertions can distinguish a move from a copy.
                original = checkbox.element_handle()
                detached = owner.evaluate_handle("node => { node.remove(); return node }")
                expect(target.get_by_role("checkbox")).to_have_count(1)
                expect(target.get_by_label("Water packed")).to_be_visible()
                assert detached.evaluate("(node, box) => node.contains(box)", original)
                page.get_by_role("region", name="Packing list").evaluate(
                    "(list, item) => list.append(item)", detached)
                expect(checkbox).to_be_checked()
                assert checkbox.evaluate("(box, original) => box === original", original)
                # A mount with a missing destination keeps its editable control inline.
                owner.evaluate("""item => {
                    const list = item.parentNode;
                    item.remove();
                    item.querySelector('corvid-portal').dataset.targetId = 'missing';
                    list.append(item);
                }""")
                expect(owner.get_by_label("Map packed")).to_be_checked()
                expect(target.get_by_role("checkbox")).to_have_count(1)
            page.close()
        browser.close()
        print("Packing list passed callback, lifecycle and portal checks.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--work-dir", type=Path, help="create and retain the fixture in this new directory")
    parser.add_argument("--callbacks-only", action="store_true", help="run the packaged Wasm callbacks in Node")
    parser.add_argument("--channel", help="Installed Chromium channel, such as msedge")
    args = parser.parse_args()
    verify(args.cli, args.channel, args.work_dir, args.callbacks_only)
