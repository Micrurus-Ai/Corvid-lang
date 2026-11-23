"""Measure a real notebook quantity edit on cold and repeat cart visits in Chrome."""
import argparse
from contextlib import ExitStack
import hashlib
from itertools import permutations
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
from tempfile import TemporaryDirectory

from playwright.sync_api import expect, sync_playwright
from verify_browser_diagnostics import package
from verify_reading_list import serve
from verify_storefront import ROOT, checkout


PROBE = """(() => {
  const row = '[data-corvid-key="notebook"]';
  const findOutput = () => document.querySelector(row + ' output');
  const ready = new MutationObserver(() => {
    const output = findOutput();
    if (output?.value === '450' && output.getClientRects().length) {
      window.cartReady = performance.now();
      ready.disconnect();
    }
  });
  ready.observe(document, {subtree:true, childList:true, attributes:true, characterData:true});
  document.addEventListener('keydown', event => {
    if (!event.isTrusted || event.key !== '2' || !event.target.closest(row)) return;
    const start = event.timeStamp;
    const output = findOutput();
    const observer = new MutationObserver(() => {
      if (output.value !== '5400') return;
      observer.disconnect();
      const dom = performance.now() - start;
      requestAnimationFrame(() => {
        const channel = new MessageChannel();
        channel.port1.onmessage = () => {
          window.cartSample = {dom_ms: dom, frame_ms: performance.now() - start};
          channel.port1.close();
          channel.port2.close();
        };
        channel.port2.postMessage(0);
      });
    });
    observer.observe(output, {subtree:true, childList:true, characterData:true});
  }, true);
})();"""


def visit(page, url):
    page.goto(url + "/cart")
    page.wait_for_function("window.cartReady > 0")
    quantity = page.get_by_label("Notebook quantity")
    expect(quantity).to_have_value("1")
    quantity.focus()
    quantity.press("End")
    quantity.press("2")
    page.wait_for_function("window.cartSample !== undefined")
    result = page.evaluate("""() => ({
        ...window.cartSample, ready_ms: window.cartReady,
        ...(window.startupStages ? {stages:window.startupStages} : {}),
        assets: performance.getEntriesByType('resource')
            .filter(e => e.name.includes('/target/wasm/'))
            .map(e => ({file:new URL(e.name).pathname.split('/').pop(), bytes:e.transferSize,
                        start_ms:e.startTime, end_ms:e.responseEnd, initiator:e.initiatorType}))
    })""")
    expect(page.get_by_text("5400", exact=True)).to_be_visible()
    checkout(page, 12, 1)
    return result


def instrument_loader(path):
    source = path.read_text(encoding="utf-8")
    markers = [
        ("handlers = await instantiate();", "init_start"),
        ('for (const field of root.querySelectorAll("[data-corvid-quantity-field]"))', "handlers_ready"),
        ("for (const panel of panels)", "quantities_bound"),
    ]
    if "WebAssembly.instantiateStreaming" in source:
        markers.extend([("const source = await WebAssembly.instantiateStreaming", "wasm_start"),
                        ("const exports = source.instance.exports;", "wasm_ready")])
    for anchor, name in markers:
        if source.count(anchor) != 1:
            raise RuntimeError(f"Cannot instrument {name} in {path}")
        source = source.replace(anchor, f"window.startupStages.{name} = performance.now();\n  " + anchor)
    path.write_text("window.startupStages = {module_start: performance.now()};\n" + source, encoding="utf-8")


def run(args):
    cli = args.cli.resolve()
    tsc = ROOT / "benches/typescript/node_modules/typescript/bin/tsc"
    with TemporaryDirectory() as temp, ExitStack() as stack:
        root = Path(temp)
        subprocess.run([cli, "new", "storefront"], cwd=root, check=True)
        project = root / "storefront"
        shutil.copytree(ROOT / "examples/storefront/src", project / "src", dirs_exist_ok=True)
        package(cli, project, root / "Corvid")
        shutil.copytree(root / "Corvid", root / "TypeScript")
        subprocess.run(["node", tsc, ROOT / "benches/typescript/storefront.ts", "--strict",
                        "--target", "ES2020", "--outDir", root], check=True)
        loader = root / "TypeScript/target/wasm/events.js"
        # Keep the exact generated DOM bindings, schemas and form behavior on both sides.
        original = loader.read_text(encoding="utf-8")
        shared_start = "const intStateHandlers = "
        if original.count(shared_start) != 1:
            raise RuntimeError("Cannot isolate the shared storefront controls in this CLI build")
        loader.write_text((root / "storefront.js").read_text(encoding="utf-8") +
                          "\n" + original[original.index(shared_start):], encoding="utf-8")
        variants = ["Corvid", "TypeScript"]
        if args.compare_preload:
            variants.append("Corvid preload")
            early = root / "Corvid preload"
            shutil.copytree(root / "Corvid", early)
            digest = hashlib.sha256((early / "target/wasm/events.wasm").read_bytes()).hexdigest()
            hint = f"<link rel='preload' as='fetch' crossorigin='anonymous' href='/target/wasm/events.wasm?v={digest}'>"
            main = early / "src/main.cor"
            source = main.read_text(encoding="utf-8")
            anchor = 'shop_page("Shopping cart", ['
            if source.count(anchor) != 1:
                raise RuntimeError("Cannot find the storefront cart to insert the preload hint")
            main.write_text(source.replace(anchor, anchor + f'View("{hint}"), '), encoding="utf-8")
        if args.profile_stages:
            for name in variants:
                instrument_loader(root / name / "target/wasm/events.js")
        urls = {name: stack.enter_context(serve(cli, root / name, os.environ.copy()))
                for name in variants}
        orders = list(permutations(variants))
        samples = []
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(channel=args.channel)
            try:
                for i in range(-1, args.pairs):  # Discard one warm-up pair for browser process startup.
                    order = orders[i % len(orders)]
                    for name in order:
                        with browser.new_context(viewport={"width": 960, "height": 720}) as context:
                            context.add_init_script(PROBE)
                            page = context.new_page()
                            for cache in ("cold", "warm"):
                                sample = visit(page, urls[name])
                                files = sorted(asset["file"] for asset in sample["assets"])
                                expected = ["events.js"] if name == "TypeScript" else ["events.js", "events.wasm"]
                                if files != expected:
                                    raise AssertionError((name, cache, files))
                                if i >= 0:
                                    samples.append(dict(pair=i+1, language=name, cache=cache, **sample))
                version = browser.version
            finally:
                browser.close()
        report = {
            "os": platform.platform(), "browser": version,
            "cli": subprocess.check_output([cli, "--version"], text=True).strip(),
            "wasm_sha256": hashlib.sha256((root / "Corvid/target/wasm/events.wasm").read_bytes()).hexdigest(),
            "typescript": subprocess.check_output(["node", tsc, "--version"], text=True).strip(),
        }
        with args.output.open("w", encoding="utf-8") as stream:
            stream.write(json.dumps(report) + "\n")
            for sample in samples:
                stream.write(json.dumps(sample) + "\n")
        for name in urls:
            for cache in ("cold", "warm"):
                rows = [s for s in samples if s["language"] == name and s["cache"] == cache]
                for metric in ("ready_ms", "dom_ms", "frame_ms"):
                    values = sorted(s[metric] for s in rows)
                    print(name, cache, metric, "median/p95",
                          round(statistics.median(values), 2), round(values[math.ceil(.95*len(values))-1], 2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pairs", type=int, default=40)
    parser.add_argument("--channel", default="chrome")
    parser.add_argument("--compare-preload", action="store_true", help="also time the same cart with an early Wasm fetch")
    parser.add_argument("--profile-stages", action="store_true", help="record loader and control startup timestamps in the benchmark copies")
    args = parser.parse_args()
    if args.pairs < 1:
        parser.error("--pairs must be positive")
    run(args)
