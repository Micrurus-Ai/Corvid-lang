"""Compare two Corvid builds under sustained, independently scheduled reads and writes."""

import argparse
import json
import math
import os
import shutil
import subprocess
import time
from contextlib import ExitStack
from pathlib import Path
from tempfile import TemporaryDirectory

from benchmark_reading_concurrency import (
    binary_digest,
    package_audited_app,
    verify_audit,
)
from playwright.sync_api import Error as BrowserError
from playwright.sync_api import sync_playwright
from verify_reading_list import serve
from verify_storefront import ROOT


def p95(values):
    return sorted(values)[math.ceil(0.95 * len(values)) - 1]


def measure(url, page, label, seconds, writes, reads):
    # Node drives HTTP arrivals on its own clock while Chrome opens the edit form.
    command = [
        "node",
        ROOT / "benches/typescript/reading-arrivals.mts",
        url,
        label,
        str(seconds),
        str(writes),
        str(reads),
    ]
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True) as driver:
        try:
            assert driver.stdout.readline().strip() == "started", "load generator failed to start"
            start = time.perf_counter()
            opens = []
            for offset in range(1, seconds, 2):
                delay = start + offset - time.perf_counter()
                if delay > 0:
                    time.sleep(delay)
                launched = time.perf_counter() - start
                if launched >= seconds:
                    break
                try:
                    page.goto(url + "/edit?shelf=3", wait_until="domcontentloaded")
                    page.get_by_role("button", name="Save to shelf").wait_for(state="visible")
                    opens.append(
                        {
                            "at_ms": round(launched * 1000, 2),
                            "visible_ms": round((time.perf_counter() - start - launched) * 1000, 2),
                        }
                    )
                except BrowserError as exc:
                    opens.append(
                        {"at_ms": round(launched * 1000, 2), "error": str(exc).splitlines()[0]}
                    )
            out, _ = driver.communicate(timeout=45)
            assert driver.returncode == 0
        finally:
            if driver.poll() is None:
                driver.kill()
        samples = json.loads(out)
    assert sum(s[0] == "POST" for s in samples) == seconds * writes
    assert sum(s[0] == "GET" for s in samples) == seconds * reads
    cutoff = seconds * 1000
    checkpoints = [sum(s[3] <= t < s[4] for s in samples) for t in range(1000, cutoff + 1, 1000)]
    metrics = {
        "outstanding_each_second": checkpoints,
        "drain_ms": round(max(0, max(s[4] for s in samples) - cutoff), 2),
        "dispatch_lag_p95_ms": p95([round(s[3] - s[2], 2) for s in samples]),
        "errors": sum(s[5] is not None for s in samples),
    }
    for method, key in [("POST", "save"), ("GET", "read")]:
        times = [round(s[4] - s[3], 2) for s in samples if s[0] == method and s[5] is None]
        metrics[key + "_p95_ms"] = p95(times) if times else None
    return {
        "label": label,
        "writes_per_second": writes,
        "seconds": seconds,
        "browser_opens": opens,
        "requests": samples,
        "metrics": metrics,
    }


def verify_restart(url, directory, runs):
    expected = {
        f"{run['label']}-{i}": 1 + i % 2
        for run in runs
        for method, i, _, _, _, error in run["requests"]
        if method == "POST" and error is None
    }
    unconfirmed = {
        f"{run['label']}-{i}": 1 + i % 2
        for run in runs
        for method, i, _, _, _, error in run["requests"]
        if method == "POST" and error is not None
    }
    log = verify_audit(url, directory / "reading.sqlite", expected, unconfirmed)
    return len(expected), len(log) - len(expected)


def run(args):
    builds = {"queued": args.cli.resolve(), "baseline": args.baseline.resolve()}
    env = os.environ.copy()
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        project = root / "app"
        subprocess.run([builds["queued"], "new", "app"], cwd=root, check=True)
        for folder in ["src", "migrations"]:
            shutil.copytree(
                ROOT / "examples/reading_list" / folder, project / folder, dirs_exist_ok=True
            )
        package_audited_app(builds["queued"], project, root / "queued")
        shutil.copytree(root / "queued", root / "baseline")
        runs = []
        with ExitStack() as stack:
            urls = {
                name: stack.enter_context(serve(cli, root / name, env))
                for name, cli in builds.items()
            }
            browser = stack.enter_context(sync_playwright()).chromium.launch(channel="chrome")
            stack.callback(browser.close)
            pages = {name: browser.new_page() for name in builds}
            version = browser.version
            for name, url in urls.items():
                pages[name].goto(url)
                runs.append(
                    dict(
                        build=name, round=0, **measure(url, pages[name], "warm", 2, 10, args.reads)
                    )
                )
            for round_no in range(1, args.rounds + 1):
                rates = args.rates if round_no % 2 else reversed(args.rates)
                for rate in rates:
                    order = list(builds)
                    if (round_no + args.rates.index(rate)) % 2:
                        order.reverse()
                    for name in order:
                        result = measure(
                            urls[name],
                            pages[name],
                            f"r{round_no}-w{rate}",
                            args.seconds,
                            rate,
                            args.reads,
                        )
                        runs.append(dict(build=name, round=round_no, **result))
                        print(name, result["label"], result["metrics"], flush=True)
        recovered = {}
        unconfirmed_commits = {}
        for name, cli in builds.items():
            with serve(cli, root / name, env) as url:
                recovered[name], unconfirmed_commits[name] = verify_restart(
                    url, root / name, [r for r in runs if r["build"] == name]
                )
        args.output.write_text(
            json.dumps(
                {
                    "chrome": version,
                    "node": subprocess.check_output(["node", "--version"], text=True).strip(),
                    "binary_sha256": {n: binary_digest(p) for n, p in builds.items()},
                    "reads_per_second": args.reads,
                    "request_columns": [
                        "method",
                        "index",
                        "scheduled_ms",
                        "sent_ms",
                        "completed_ms",
                        "error",
                    ],
                    "recovered": recovered,
                    "committed_without_confirmation": unconfirmed_commits,
                    "runs": runs,
                },
                separators=(",", ":"),
            )
            + "\n",
            encoding="utf-8",
        )
        print("Recovered edits", recovered)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rates", nargs="+", type=int, default=[20, 80, 160])
    parser.add_argument("--seconds", type=int, default=12)
    parser.add_argument("--reads", type=int, default=20)
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args()
    if (
        args.seconds < 2
        or min(args.rounds, args.reads, *args.rates) < 1
        or len(set(args.rates)) != len(args.rates)
    ):
        parser.error("use positive counts, distinct write rates and at least two seconds")
    run(args)
