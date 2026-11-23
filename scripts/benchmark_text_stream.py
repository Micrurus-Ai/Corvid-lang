"""Time first visible stream text and cancellation at a controlled local provider."""
import argparse
from contextlib import ExitStack
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import math
import os
from pathlib import Path
from queue import Queue
import re
import select
import statistics
import subprocess
from tempfile import TemporaryDirectory
from threading import Event, Thread
import time

from playwright.sync_api import sync_playwright
from benchmark_node import node_server
from verify_browser_diagnostics import package
from verify_reading_list import read, serve
from verify_storefront import ROOT


def milliseconds():
    return time.perf_counter_ns() / 1e6


class Provider(BaseHTTPRequestHandler):
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        assert self.path == "/v1/chat/completions" and request["stream"]
        call = {"request": request, "closed": Event(), "count": 0, "received": milliseconds()}
        self.server.calls.put(call)
        try:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            # Waiting on the socket lets cancellation interrupt the simulated token delay.
            while milliseconds() - call["received"] < 5000:
                delay = .03 if call["count"] == 0 else .01
                if select.select([self.connection], [], [], delay)[0]:
                    if not self.connection.recv(1):
                        break
                    raise RuntimeError("Unexpected data after provider request body")
                text = "First" if not call["count"] else " next"
                event = {"choices": [{"delta": {"content": text}}]}
                self.wfile.write(b"data: " + json.dumps(event).encode() + b"\n\n")
                self.wfile.flush()
                call["count"] += 1
                if call["count"] == 1:
                    call["first"] = milliseconds()
            else:
                call["expired"] = True
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass
        finally:
            call["stopped"] = milliseconds()
            call["closed"].set()

    def log_message(self, *_):
        pass


PROBE = """() => {
  const output = document.querySelector('[data-output]');
  const status = document.querySelector('corvid-text-stream [role=status]');
  document.addEventListener('click', e => {
    if (!e.isTrusted) return;
    if (e.target.closest('[data-start]')) window.measure = {start: e.timeStamp};
    if (e.target.closest('[data-cancel]')) window.measure.cancel = e.timeStamp;
  }, true);
  new MutationObserver(() => {
    const m = window.measure;
    if (m && m.first === undefined && status.textContent === 'Streaming' &&
        output.textContent.startsWith('First') && output.getClientRects().length) m.first = performance.now();
    if (m?.cancel !== undefined && m.cancelled === undefined && status.textContent === 'Cancelled') {
      m.cancelled = performance.now();
    }
  }).observe(document.querySelector('corvid-text-stream'), {childList:true, characterData:true, subtree:true});
}"""


def clock_offset(page):
    # Choose the shortest driver round trip. Half of it bounds midpoint uncertainty.
    probes = []
    for _ in range(7):
        before = milliseconds()
        browser = page.evaluate("performance.now()")
        after = milliseconds()
        probes.append((after-before, (after+before)/2 - browser))
    return min(probes)


def trial(page, provider):
    rtt, offset = clock_offset(page)
    page.locator("[data-start]").press("Enter")
    call = provider.calls.get(timeout=6)
    page.wait_for_function("window.measure?.first !== undefined")
    page.locator("[data-cancel]").press("Enter")
    assert call["closed"].wait(4) and not call.get("expired"), "generation continued after cancellation"
    m = page.evaluate("window.measure")
    assert m["cancelled"] >= m["cancel"] and call["count"] > 0
    first = call["first"] - call["received"]
    stopped = call["stopped"] - (m["cancel"] + offset)
    assert stopped >= -rtt/2 - 1, (stopped, rtt)
    return call["request"], dict(first_dom_ms=m["first"]-m["start"], cancel_dom_ms=m["cancelled"]-m["cancel"],
                                 provider_stop_ms=stopped, clock_half_rtt_ms=rtt/2,
                                 provider_first_ms=first, chunks=call["count"])


def verify(args):
    cli = args.cli.resolve()
    with TemporaryDirectory() as tmp, ExitStack() as stack:
        root = Path(tmp)
        provider = stack.enter_context(ThreadingHTTPServer(("127.0.0.1", 0), Provider))
        provider.calls = Queue()
        Thread(target=provider.serve_forever, daemon=True).start()
        stack.callback(provider.shutdown)
        endpoint = f"http://127.0.0.1:{provider.server_port}/v1"
        env = dict(os.environ, CORVID_MODEL=f"openai-compat:{endpoint}:local-model",
                   CORVID_TEST_MOCK_LLM="0", OPENAI_COMPAT_API_KEY="", NO_PROXY="127.0.0.1")
        subprocess.run([cli, "new", "stream"], cwd=root, env=env, check=True)
        book = (ROOT / "docs/book/19-browser-views.md").read_text(encoding="utf-8")
        source = re.search(r"## Displaying a streamed answer\s+```corvid-fragment\n(.*?)```", book, re.S)[1]
        (root / "stream/src/main.cor").write_text(source, encoding="utf-8")
        package(cli, root / "stream", root / "served")
        url = stack.enter_context(serve(cli, root / "served", env))
        with sync_playwright() as pw:
            browser = pw.chromium.launch(channel=args.channel)
            try:
                def open_page(url):
                    page = browser.new_page()
                    page.goto(url)
                    page.evaluate(PROBE)
                    return page
                pages = {"Corvid": open_page(url)}
                payload, _ = trial(pages["Corvid"], provider)
                config = root / "stream.json"
                config.write_text(json.dumps(dict(html=read(url), module=read(url+"/.well-known/corvid/stream.js"),
                                      payload=payload, endpoint=endpoint+"/chat/completions")), encoding="utf-8")
                ts = stack.enter_context(node_server(root, ROOT/"benches/typescript/text-stream.mts", config, "/"))
                pages["TypeScript"] = open_page(ts)
                samples = []
                for i in range(-2, args.rounds):
                    for name in (list(pages) if i % 2 == 0 else list(reversed(pages))):
                        request, sample = trial(pages[name], provider)
                        assert request == payload
                        if i >= 0:
                            samples.append(dict(round=i+1, language=name, **sample))
                version = browser.version
            finally:
                browser.close()
        report = dict(cli=subprocess.check_output([cli, "--version"], text=True).strip(), browser=version,
                      node=subprocess.check_output(["node", "--version"], text=True).strip(),
                      first_delay_ms=30, subsequent_delay_ms=10, samples=samples)
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        for name in pages:
            for key in ("first_dom_ms", "cancel_dom_ms", "provider_stop_ms", "clock_half_rtt_ms"):
                values = sorted(s[key] for s in samples if s["language"] == name)
                print(name, key, "median / p95:", round(statistics.median(values), 2),
                      round(values[math.ceil(.95*len(values))-1], 2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="chrome")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rounds", type=int, default=40)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    verify(args)
