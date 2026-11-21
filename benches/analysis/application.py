"""Measure developer-facing Corvid workloads using only local dependencies."""
import argparse
import http.client
import hashlib
import json
import os
import platform
import socket
import statistics
import subprocess
import tempfile
import time
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread


SLICE = """agent main() -> Int:
    items = range(0, 16384)
    total = 0
    for i in range(0, 1024):
        part = items.slice(i, i + 4)
        total += part[0]
    return total
"""

SERVICE = """prompt answer(topic: String) -> Stream<String>:
    "{topic}"

server app:
    route GET "/ping" -> html:
        return "ok"
    route GET "/answer" -> json Stream<String>:
        return answer("hello")
"""
CHUNKS = ["word" + str(i) for i in range(16)]
PROVIDER_BODY = b"".join(
    ("data: " + json.dumps({"choices": [{"delta": {"content": word}}]}) + "\n\n").encode()
    for word in CHUNKS
) + b"data: [DONE]\n\n"


def sample(actions, warmups, trials, batch_size=1):
    samples = {name: [] for name in actions}
    for i in range(warmups + trials):
        elapsed = dict.fromkeys(actions, 0)
        for j in range(batch_size):
            # Interleave direct and proxied requests inside each measured batch.
            names = list(actions) if (i + j) % 2 == 0 else reversed(actions)
            for name in names:
                start = time.perf_counter_ns()
                actions[name]()
                elapsed[name] += time.perf_counter_ns() - start
        if i >= warmups:
            for name, duration in elapsed.items():
                samples[name].append(round(duration / (batch_size * 1_000_000), 3))
    return {name: {"median_ms": round(statistics.median(values), 3), "samples_ms": values}
            for name, values in samples.items()}


class Provider(BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path != "/v1/chat/completions" or body.get("stream") is not True:
            self.send_error(400)
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(PROVIDER_BODY)))
        self.end_headers()
        self.wfile.write(PROVIDER_BODY)

    def log_message(self, *args):
        pass


def request(port, path, payload=None):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
    try:
        conn.request("GET" if payload is None else "POST", path, body=payload,
                     headers={"Content-Type": "application/json"} if payload else {})
        response = conn.getresponse()
        data = response.read()
        if response.status != 200:
            raise RuntimeError(f"{path} returned {response.status}: {data!r}")
        return data
    finally:
        conn.close()


@contextmanager
def local_service(cli, root, provider):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env = dict(os.environ, CORVID_MODEL=f"openai-compat:http://127.0.0.1:{provider}/v1:local-model",
               CORVID_TEST_MOCK_LLM="0", OPENAI_COMPAT_API_KEY="", NO_PROXY="127.0.0.1")
    with (root / "serve.log").open("w+b") as log:
        server = subprocess.Popen([cli, "serve", "service.cor", "--listen", f"127.0.0.1:{port}"],
                                  cwd=root, env=env, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline and server.poll() is None:
                try:
                    if request(port, "/ping") == b"ok":
                        break
                except OSError:
                    pass
                time.sleep(.05)
            else:
                log.seek(0)
                raise RuntimeError(log.read().decode(errors="replace"))
            yield port
        finally:
            if server.poll() is None:
                server.terminate()
            server.wait(timeout=15)


def benchmark(cli, root, workloads, warmups, trials):
    metrics = {}
    stream_delta = None

    def command(args, expected=None):
        result = subprocess.run([cli, *args], cwd=root, capture_output=True, timeout=90)
        if result.returncode or (expected is not None and result.stdout.strip() != expected):
            detail = (result.stdout + result.stderr).decode(errors="replace")
            if len(detail) > 3000:
                detail = detail[:1500] + "\n...\n" + detail[-1500:]
            raise RuntimeError(f"{args} exited {result.returncode}\n{detail}")
        return result.stdout

    def measure(name, action, batch_size=1):
        metrics.update(sample({name: action}, warmups, trials, batch_size))

    version = command(["--version"]).strip()
    if "startup" in workloads:
        measure("startup_version", lambda: command(["--version"], version))
    if "compiler" in workloads:
        program = "\n".join(f"fn add_{i}(n: Int) -> Int:\n    return n + {i}\n" for i in range(100))
        (root / "compile.cor").write_text(program + "\nagent main() -> Int:\n    return add_99(1)\n")
        measure("check_100_functions", lambda: command(["check", "compile.cor"]))
        path = root / "target" / "bin" / ("compile.exe" if os.name == "nt" else "compile")

        def build():
            path.unlink(missing_ok=True)
            command(["build", "compile.cor", "--target", "native"])
            if not path.is_file():
                raise RuntimeError(f"missing native build output {path}")
        measure("native_build_100_functions", build)
        execution = subprocess.run([path], cwd=root, capture_output=True, check=True, timeout=15)
        if execution.stdout.strip() != b"100":
            raise RuntimeError(f"unexpected compiled result {execution.stdout!r}")
    if "collections" in workloads:
        (root / "slice.cor").write_text(SLICE)
        measure("list_slice_interp", lambda: command(["run", "slice.cor", "--target", "interp"], b"523776"))
    if "service" in workloads:
        (root / "service.cor").write_text(SERVICE)
        with ThreadingHTTPServer(("127.0.0.1", 0), Provider) as provider:
            thread = Thread(target=provider.serve_forever, daemon=True)
            thread.start()
            try:
                with local_service(cli, root, provider.server_port) as port:
                    def ping():
                        if request(port, "/ping") != b"ok":
                            raise RuntimeError("unexpected HTTP response")
                    def direct():
                        data = request(provider.server_port, "/v1/chat/completions", '{"stream":true}')
                        events = [line[5:].strip() for line in data.splitlines() if line.startswith(b"data:")]
                        if events[-1:] != [b"[DONE]"]:
                            raise RuntimeError("incomplete provider stream")
                        chunks = [json.loads(event)["choices"][0]["delta"]["content"] for event in events[:-1]]
                        if chunks != CHUNKS:
                            raise RuntimeError("unexpected provider chunks")
                    def stream():
                        data = request(port, "/answer").decode()
                        received = [line[5:].strip() for line in data.splitlines() if line.startswith("data:")]
                        if received[:len(CHUNKS)] != [json.dumps(word) for word in CHUNKS] or "event: done" not in data:
                            raise RuntimeError(f"unexpected Corvid stream: {data!r}")
                    measure("http_ping", ping, batch_size=16)
                    paired = sample({"provider_stream_direct": direct, "corvid_stream_16_chunks": stream},
                                    warmups, trials, batch_size=16)
                    metrics.update(paired)
                    differences = [round(via_service - direct_ms, 3) for via_service, direct_ms in zip(
                        paired["corvid_stream_16_chunks"]["samples_ms"],
                        paired["provider_stream_direct"]["samples_ms"],
                    )]
                    stream_delta = {"median_ms": round(statistics.median(differences), 3),
                                    "paired_differences_ms": differences}
            finally:
                provider.shutdown()
                thread.join()
    with cli.open("rb") as binary:
        digest = hashlib.file_digest(binary, "sha256").hexdigest()
    return {"protocol": 2, "service_requests_per_sample": 16, "local_stream_overhead": stream_delta, "cli_version": version.decode(), "cli_sha256": digest, "platform": platform.platform(),
            "machine": platform.machine(), "processor": platform.processor(),
            "python": platform.python_version(), "warmups": warmups, "trials": trials, "metrics": metrics}


# Initial same-machine investigation thresholds, in milliseconds. A change
# must clear both its workload's absolute noise floor and a relative limit.
NOISE_FLOOR_MS = {
    "startup_version": 3, "check_100_functions": 5, "native_build_100_functions": 50,
    "list_slice_interp": 5, "http_ping": 0.5, "provider_stream_direct": 0.5,
    "corvid_stream_16_chunks": 1,
}


def regressions(result, baseline):
    if any(result[key] != baseline[key] for key in ("protocol", "platform", "machine", "processor")):
        raise ValueError("compare measurements on the same machine and OS build")
    failures = []
    for name, current in result["metrics"].items():
        if name not in baseline["metrics"]:
            raise ValueError(f"baseline has no measurement for {name}")
        reference = baseline["metrics"][name]
        previous = reference["median_ms"]
        mad = statistics.median(abs(x - previous) for x in reference["samples_ms"])
        # Three robust standard deviations of the reference trials.
        limit = previous + max(previous * 0.30, NOISE_FLOOR_MS[name], 3 * 1.4826 * mad)
        if current["median_ms"] > limit:
            failures.append(f"{name}: {current['median_ms']:.3f} ms exceeds {limit:.3f} ms")
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--workload", choices=["startup", "compiler", "collections", "service"], action="append")
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--trials", type=int, default=15)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--baseline", type=Path, help="compare selected medians with a same-host report")
    args = parser.parse_args()
    if args.trials < 1 or args.warmups < 0:
        parser.error("trials must be positive and warmups nonnegative")
    baseline = json.loads(args.baseline.read_text(encoding="utf-8")) if args.baseline else None
    with tempfile.TemporaryDirectory() as tmp:
        result = benchmark(args.cli.resolve(), Path(tmp), args.workload or
                           ["startup", "compiler", "collections", "service"], args.warmups, args.trials)
    output = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(output, encoding="utf-8")
    print(output, end="")
    if baseline is not None:
        failures = regressions(result, baseline)
        if failures:
            raise SystemExit("\n".join(failures))


if __name__ == "__main__":
    main()
