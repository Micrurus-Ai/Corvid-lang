"""Compare the packaged inventory models in the same browser workflow."""
import argparse
from contextlib import ExitStack, contextmanager
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import math
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
from threading import Thread
import time
from urllib.parse import urlsplit
from urllib.request import urlopen
from uuid import uuid4

ROOT = Path(__file__).resolve().parents[1]
BUDGET = dict(startup_bytes=65536, cold_ms=2000, warm_ms=1000, input_ms=100,
              frame_ms=50, navigation_ms=2000, growth_mib=128, mounted_rows=16)


class InventoryHost(SimpleHTTPRequestHandler):
    extensions_map = {**SimpleHTTPRequestHandler.extensions_map,
                      ".wasm": "application/wasm", ".js": "text/javascript"}

    def end_headers(self):
        if urlsplit(self.path).path.endswith((".wasm", ".js")):
            self.send_header("Cache-Control", "public, max-age=3600")
        super().end_headers()

    def do_GET(self):
        if urlsplit(self.path).path != "/shipment":
            return super().do_GET()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        try:
            for sequence in range(1, 33):
                self.wfile.write(f"data: {sequence}\n\n".encode())
                self.wfile.flush()
                time.sleep(0.02)
        except (ConnectionResetError, BrokenPipeError):
            pass

    def log_message(self, *_):
        pass


@contextmanager
def host(directory):
    with ThreadingHTTPServer(("127.0.0.1", 0), partial(InventoryHost, directory=directory)) as server:
        Thread(target=server.serve_forever, daemon=True).start()
        try:
            yield f"http://127.0.0.1:{server.server_port}"
        finally:
            server.shutdown()


def prepare(cli, root):
    project = root / "Corvid"
    shutil.copytree(ROOT / "examples/windowed_inventory", project,
                    ignore=shutil.ignore_patterns("target"))
    for args in (["test", "src/inventory.cor"], ["build", "src/inventory.cor", "--target", "wasm"]):
        subprocess.run([cli, *args], cwd=project, check=True)
    baseline = root / "TypeScript"
    baseline.mkdir()
    for asset in ("inventory.js", "index.html"):
        shutil.copy(project / asset, baseline)
    module = baseline / "target/wasm/inventory.js"
    module.parent.mkdir(parents=True)
    compile_ts = """
        import {stripTypeScriptTypes} from 'node:module';
        import {readFileSync, writeFileSync} from 'node:fs';
        writeFileSync(process.argv[2], stripTypeScriptTypes(readFileSync(process.argv[1], 'utf8')));
    """
    subprocess.run(["node", "--input-type=module", "-e", compile_ts,
                    ROOT / "benches/typescript/windowed-inventory.ts", module], check=True)
    parity = """
        import assert from 'node:assert/strict';
        import {readFileSync} from 'node:fs';
        import {instantiate} from './TypeScript/target/wasm/inventory.js';
        const ts = await instantiate();
        const {instance} = await WebAssembly.instantiate(readFileSync('./Corvid/target/wasm/inventory.wasm'));
        for (let index = 0n; index < 10032n; index++) {
            const scroll = index * 32n + index % 32n;
            for (const [name, args] of [['stock', [index + 1n]], ['first_row', [scroll, 32n]],
                    ['end_row', [scroll, 320n, 32n, 10032n]]]) {
                assert.equal(instance.exports[name](...args), ts[name](...args));
            }
        }
    """
    subprocess.run(["node", "--input-type=module", "-e", parity], cwd=root, check=True)
    return {"Corvid": project, "TypeScript": baseline}


PROBE = """(() => {
    let last;
    window.framesMs = [];
    window.inputsMs = [];
    window.shipmentSamples = [];
    let requested;
    document.addEventListener('click', event => {
        if (event.isTrusted && event.target.id === 'receive') requested = event.timeStamp;
    }, true);
    window.maxRows = 0;
    new MutationObserver(() => {
        maxRows = Math.max(maxRows, document.querySelectorAll('[data-row]').length);
        const bins = document.querySelector('#status')?.value;
        if (requested !== undefined && bins && bins !== shipmentSamples.at(-1)?.bins) {
            shipmentSamples.push({bins, elapsed: performance.now() - requested});
        }
    }).observe(document, {subtree: true, childList: true, characterData: true});
    window.addEventListener('pageshow', () => { last = undefined; });
    function frame(now) {
        if (last !== undefined) framesMs.push(now - last);
        last = now;
        requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);
    document.addEventListener('keydown', event => {
        if (!event.isTrusted || !['PageDown','End','ArrowUp'].includes(event.key)) return;
        const before = document.activeElement;
        requestAnimationFrame(() => {
            if (before !== document.activeElement) inputsMs.push(performance.now() - event.timeStamp);
        });
    }, true);
})();"""


def p95(values):
    return sorted(values)[math.ceil(len(values) * .95) - 1]


def working_set():
    # This verifier's only long-lived children during a trial are Playwright and its browser.
    import psutil
    total = 0
    for process in psutil.Process().children(recursive=True):
        try:
            total += process.memory_info().rss
        except psutil.NoSuchProcess:
            pass
    if not total:
        raise RuntimeError("Could not measure the browser process tree")
    return total / 2**20


def trial(browser, url, view, body_sizes):
    from playwright.sync_api import expect
    with browser.new_context(viewport={"width": 960, "height": 720}, device_scale_factor=1) as context:
        context.add_init_script(PROBE)
        page = context.new_page()
        errors = []
        page.on("pageerror", lambda error: errors.append(str(error)))
        memory = {"blank": working_set()}
        metrics = {}
        transfer = {}
        for visit in ("cold", "warm"):
            page.goto(url + "/?view=" + view)
            page.wait_for_function("performance.getEntriesByName('inventory-ready').length === 1")
            metrics[visit + "_ms"] = page.evaluate("performance.getEntriesByName('inventory-ready')[0].startTime")
            transfer[visit] = page.evaluate("""[...performance.getEntriesByType('navigation'),
                ...performance.getEntriesByType('resource')].map(e => ({
                    path: new URL(e.name).pathname, transfer_bytes: e.transferSize ?? null
                }))""")
            paths = page.evaluate("""performance.getEntriesByType('resource')
                .map(entry => new URL(entry.name).pathname).filter(path => path !== '/favicon.ico')""")
            # Uncompressed response bodies required for startup, including the entry document.
            assert sorted(paths) == sorted(path for path in body_sizes if path != "/"), paths
            metrics["startup_bytes"] = body_sizes["/"] + sum(body_sizes[path] for path in paths)
        memory["loaded"] = working_set()
        viewport = page.locator("#viewport")
        links = viewport.get_by_role("link")
        page.get_by_role("link", name="Stock table", exact=True).focus()
        page.keyboard.press("Tab")
        expect(links.first).to_be_focused()
        frames = []
        for i in range(1, 21):
            page.keyboard.press("PageDown")
            expect(viewport.get_by_role("link", name=f"Bin {10*i+1}", exact=True)).to_be_focused()
            page.wait_for_function(f"inputsMs.length === {i}")
        page.keyboard.press("End")
        page.wait_for_function("inputsMs.length === 21")
        last = viewport.get_by_role("link", name="Bin 10000", exact=True)
        expect(last).to_be_focused()
        expect(last).to_be_in_viewport()
        # Wheel-like scrolling with focus held on the last bin must leave it reachable.
        viewport.evaluate("el => { el.scrollTop = 3200; }")
        expect(viewport.get_by_role("link", name="Bin 101", exact=True)).to_be_in_viewport()
        expect(last).to_be_focused()
        page.keyboard.press("ArrowUp")
        expect(viewport.get_by_role("link", name="Bin 9999", exact=True)).to_be_in_viewport()
        page.wait_for_function("inputsMs.length === 22")
        page.keyboard.press("End")
        page.wait_for_function("inputsMs.length === 23")
        expect(last).to_be_focused()
        row = page.locator('[data-row="9999"]')
        expect(row).to_contain_text(f"{10000 * 17 % 101} parts")
        expect(row).to_have_attribute("aria-rowindex" if view == "table" else "aria-posinset", "10000")
        metrics["input_ms"] = p95(page.evaluate("inputsMs"))
        frames.extend(page.evaluate("framesMs.splice(0)"))
        memory["keyboard"] = working_set()
        mounting = [page.evaluate("maxRows")]
        position = viewport.evaluate("el => el.scrollTop")
        assert position > 0
        returns = []
        for _ in range(5):
            start = time.perf_counter()
            last.press("Enter")
            expect(page.get_by_role("heading", name="Bin 10000")).to_be_visible()
            expect(viewport).to_have_text(f"{10000 * 17 % 101} parts")
            page.go_back()
            expect(last).to_be_focused()
            assert viewport.evaluate("el => el.scrollTop") == position
            returns.append(1000 * (time.perf_counter() - start))
            mounting.append(page.evaluate("maxRows"))
        metrics["navigation_ms"] = p95(returns)
        memory["navigation"] = working_set()
        page.evaluate("framesMs.length = 0")
        page.get_by_role("button", name="Receive shipment").press("Enter")
        expect(page.locator("#status")).to_have_text("10032 bins")
        stream_frames = page.evaluate("framesMs")
        assert len(stream_frames) >= 2, stream_frames
        shipment = page.evaluate("shipmentSamples")
        assert [sample["bins"] for sample in shipment] == [f"{i} bins" for i in range(10001, 10033)], shipment
        metrics["input_ms"] = max(metrics["input_ms"], shipment[0]["elapsed"])
        metrics["frame_ms"] = max(p95(frames), p95(stream_frames))
        assert viewport.evaluate("el => el.scrollTop") == position
        last.focus()
        page.keyboard.press("End")
        final = viewport.get_by_role("link", name="Bin 10032", exact=True)
        expect(final).to_be_focused()
        expect(final).to_be_in_viewport()
        page.keyboard.press("Home")
        expect(links.first).to_be_focused()
        expect(links.first).to_have_text("Bin 1")
        page.keyboard.press("Shift+Tab")
        expect(page.get_by_role("link", name="Stock table", exact=True)).to_be_focused()
        metrics["mounted_rows"] = max(*mounting, page.evaluate("maxRows"))
        assert metrics["mounted_rows"] <= BUDGET["mounted_rows"], metrics
        memory["shipment"] = working_set()
        metrics["growth_mib"] = max(memory.values()) - memory["blank"]
        assert not errors, errors
        return {"metrics": metrics, "working_set_mib": memory, "returns_ms": returns,
                "transfers": transfer, "shipment_elapsed_ms": [sample["elapsed"] for sample in shipment]}


def run(args):
    with ExitStack() as stack:
        cli = args.cli.resolve()
        print(subprocess.check_output([cli, "--version"], text=True).strip(),
              "Node", subprocess.check_output(["node", "--version"], text=True).strip(), flush=True)
        build = ROOT / "target" / ("inventory-" + uuid4().hex)
        build.mkdir(parents=True)
        stack.callback(shutil.rmtree, build)
        projects = prepare(cli, build)
        urls = {name: stack.enter_context(host(project)) for name, project in projects.items()}
        sizes = {}
        for name, url in urls.items():
            paths = ["/", "/inventory.js", "/target/wasm/inventory.js"]
            if name == "Corvid":
                paths.append("/target/wasm/inventory.wasm")
            sizes[name] = {}
            for path in paths:
                with urlopen(url + path) as response:
                    sizes[name][path] = len(response.read())
            with urlopen(url + "/shipment") as response:
                assert response.read().decode().splitlines()[::2] == [f"data: {i}" for i in range(1, 33)]
            assert sum(sizes[name].values()) <= BUDGET["startup_bytes"], sizes[name]
        print("Checked model parity, shipment and startup body bytes:", sizes, flush=True)
        if args.prepare_only:
            return
        from playwright.sync_api import sync_playwright
        failures = []
        with sync_playwright() as pw:
            for engine in ("chromium", "firefox", "webkit"):
                browser = getattr(pw, engine).launch(channel=args.channel if engine == "chromium" else None)
                try:
                    for view in ("list", "table"):
                        samples = {name: [] for name in projects}
                        for turn in range(4):
                            for name in (list(projects) if turn % 2 == 0 else list(reversed(projects))):
                                result = trial(browser, urls[name], view, sizes[name])
                                print(json.dumps(dict(engine=engine, version=browser.version, os=platform.platform(),
                                                      view=view, language=name, turn=turn, **result)), flush=True)
                                samples[name].append(result["metrics"])
                        for metric, limit in BUDGET.items():
                            medians = {name: statistics.median(row[metric] for row in trials)
                                       for name, trials in samples.items()}
                            print(engine, view, metric, medians, "budget", limit, flush=True)
                            for name, value in medians.items():
                                if value > limit:
                                    failures.append((engine, view, name, metric, value, limit))
                finally:
                    browser.close()
        if failures:
            raise AssertionError(failures)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--channel", help="Chromium distribution, e.g. msedge")
    parser.add_argument("--prepare-only", action="store_true", help="Check builds and HTTP fixtures locally")
    run(parser.parse_args())
