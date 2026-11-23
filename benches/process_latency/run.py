"""Measure fresh-process latency for the same batch scorer in four languages."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "analysis"))
from aggregate import bootstrap_ratio


def output(command, cwd):
    return subprocess.check_output(command, cwd=cwd, text=True, timeout=120).strip()


def expected(count):
    # Each block of 257 IDs has exactly the same sequence of scores.
    period = [sum(w * (i * k % 257) for w, k in ((3, 1), (5, 17), (7, 31)))
              for i in range(257)]
    cycles, tail = divmod(count, len(period))
    return (cycles * sum(s for s in period if s >= 2000)
            + sum(s for s in period[:tail] if s >= 2000))


def measure(commands, count, work):
    target = str(expected(count))
    samples = {name: [] for name in commands}
    for trial in range(-3, 30):
        for name, command in commands.items():
            start = time.perf_counter_ns()
            result = output([*command, str(count)], work)
            elapsed = time.perf_counter_ns() - start
            if result != target:
                raise ValueError(f"{name}({count}) returned {result!r}, expected {target}")
            if trial >= 0:
                samples[name].append(elapsed)

    corvid = samples["Corvid"]
    ratios = {}
    for name, values in samples.items():
        ratios[name] = {
            "corvid_over_language": statistics.median(corvid) / statistics.median(values),
            "paired_bootstrap_95pct": bootstrap_ratio(corvid, values),
        }
    return {"records": count, "checksum": target, "samples_ns": samples, "ratios": ratios}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corvid", type=Path, help="path to the release Corvid CLI")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    cli = args.corvid.resolve(strict=True)
    with tempfile.TemporaryDirectory() as directory:
        work = Path(directory)
        for ext in ("cor", "rs", "py", "js"):
            shutil.copyfile(HERE / f"score.{ext}", work / f"score.{ext}")
        suffix = ".exe" if os.name == "nt" else ""
        output([cli, "build", "score.cor", "--target", "native"], work)
        rust = work / ("rust-score" + suffix)
        output(["rustc", "-O", "score.rs", "-o", rust], work)
        commands = {
            "Corvid": [work / "target" / "bin" / ("score" + suffix)],
            "Python": [sys.executable, "score.py"],
            "JavaScript": ["node", "score.js"],
            "Rust": [rust],
        }
        report = {
            "timestamp_utc": datetime.now(timezone.utc).isoformat(),
            "platform": platform.platform(), "cpu": platform.processor() or platform.machine(),
            "versions": {name: output([exe, "--version"], work) for name, exe in
                         (("Corvid", cli), ("Python", sys.executable),
                          ("JavaScript", "node"), ("Rust", "rustc"))},
            "corvid_cli_sha256": hashlib.sha256(cli.read_bytes()).hexdigest(),
            "warmups": 3, "trials": 30, "order": list(commands),
            "cases": [measure(commands, count, work) for count in (0, 1_000_000)],
        }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
