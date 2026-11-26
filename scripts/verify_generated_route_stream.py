"""Exercise the generated TypeScript client against a served stream route.

Generates the client for sdk/typescript/client/test/route_stream.cor,
type-checks it with the client's sources and the typed test program
(sdk/typescript/client/test/route-stream.ts), serves the program with
`corvid serve`, and runs the test under Node. A local proxy forwards the
first event of a stream and then drops the connection, so the lost-
connection case goes over a real socket rather than a stubbed fetch.

    python scripts/verify_generated_route_stream.py --cli target/debug/corvid \
        --tsc path/to/node_modules/typescript/bin/tsc
"""
import argparse
from contextlib import ExitStack
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
from tempfile import TemporaryDirectory
from threading import Thread
import time

ROOT = Path(__file__).resolve().parents[1]
SDK = ROOT / "sdk/typescript/client"


def build(cli, root, tsc):
    modules = root / "modules"
    shutil.copytree(SDK / "src", modules)
    subprocess.run([cli, "contract", "ts-client", SDK / "test/route_stream.cor", "--out", modules], check=True)
    shutil.copyfile(SDK / "test/route-stream.ts", modules / "route-stream.ts")
    # Resolve the package to the local sources for this unbundled test, and
    # name the generated types module the way Node's ESM loader needs.
    api = modules / "api.ts"
    api.write_text(api.read_text(encoding="utf-8")
                   .replace('"@corvid/client"', '"./index.js"')
                   .replace('from "./types"', 'from "./types.js"'), encoding="utf-8")
    (root / "package.json").write_text(json.dumps({"type": "module"}))
    (modules / "tsconfig.json").write_text(json.dumps({"compilerOptions": {
        "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler",
        "lib": ["ES2022", "DOM", "DOM.AsyncIterable"], "strict": True, "outDir": "../js",
    }}))
    compiler = ["node", str(tsc)] if tsc else [shutil.which("tsc") or "tsc"]
    subprocess.run([*compiler, "-p", modules], check=True)


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def serve(cli, port, log):
    env = dict(os.environ, CORVID_HTTP_ALLOW="api.example.com", NO_PROXY="127.0.0.1")
    process = subprocess.Popen(
        [cli, "serve", SDK / "test/route_stream.cor", "--mode", "real", "--listen", f"127.0.0.1:{port}"],
        cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=log)
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("corvid serve exited before it was ready")
        try:
            connection = HTTPConnection("127.0.0.1", port, timeout=1)
            connection.request("GET", "/healthz")
            if connection.getresponse().status == 200:
                return process
        except OSError:
            time.sleep(0.2)
    process.kill()
    raise RuntimeError("corvid serve did not become ready")


def dropping_proxy(upstream_port):
    """Forward a stream's first event, then drop the connection mid-body."""

    class Proxy(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_GET(self):
            upstream = HTTPConnection("127.0.0.1", upstream_port, timeout=10)
            upstream.request("GET", self.path, headers={"Accept": "text/event-stream"})
            reply = upstream.getresponse()
            first = b""
            while not first.endswith(b"\n\n"):
                line = reply.fp.readline()
                if not line:
                    break
                first += line if not line.strip() or b":" in line else b""
            self.send_response(reply.status)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            self.wfile.write(b"%x\r\n%s\r\n" % (len(first), first))
            self.wfile.flush()
            # No terminating chunk: the socket closes mid-body.
            self.connection.shutdown(socket.SHUT_RDWR)
            self.close_connection = True
            upstream.close()

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Proxy)
    Thread(target=server.serve_forever, daemon=True).start()
    return server


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--tsc", type=Path, help="TypeScript bin/tsc JavaScript entrypoint")
    args = parser.parse_args()
    cli = args.cli.resolve()
    with TemporaryDirectory() as tmp, ExitStack() as stack:
        root = Path(tmp)
        build(cli, root, args.tsc.resolve() if args.tsc else None)
        port = free_port()
        log = open(root / "serve.log", "w+")
        stack.callback(log.close)
        process = serve(cli, port, log)
        stack.callback(process.wait)
        stack.callback(process.kill)
        proxy = dropping_proxy(port)
        stack.callback(proxy.shutdown)
        subprocess.run(["node", root / "js/route-stream.js", f"http://127.0.0.1:{port}",
                        f"http://127.0.0.1:{proxy.server_port}"], check=True)
        log.flush()
        log.seek(0)
        server_log = log.read()
        assert "http://127.0.0.1:9/Users/ops/vault/notes/7" in server_log, \
            f"the operator's log keeps the full error: {server_log}"


if __name__ == "__main__":
    main()
