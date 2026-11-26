"""Check authenticated RT-3 frames through serve on SQLite or two Postgres replicas."""
import argparse
from contextlib import ExitStack, contextmanager, nullcontext
import csv
import ctypes
from http.cookies import SimpleCookie
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
from tempfile import TemporaryDirectory
from urllib.request import Request, urlopen

from serve_test_session import seed_session, set_role
from verify_reading_list import serve

FIXTURE = Path(__file__).resolve().parents[1] / "crates/corvid-cli/tests/fixtures/live_cards.cor"
PROTOCOL = "corvid.live.v1"
PATH = "/.well-known/corvid/live"
WRITES = """
    live "/all-cards" -> Card requires authenticated:
        return Card.all()
    route POST "/add" -> json String requires permission("board.view"):
        return match Card.insert(Card(4, "home", "offline")):
            Ok(row) -> row.title
            Err(Taken(row)) -> "taken"
    route POST "/edit" -> json String requires permission("board.view"):
        return match Card.get(4):
            Some(row) -> saved(Card.save(Card { title: "online", ..row }))
            None -> "missing"

fn saved(result: Result<Card, Conflict<Card>>) -> String:
    return match result:
        Ok(row) -> row.title
        Err(_) -> "conflict"
"""


@contextmanager
def wire(url, headers):
    from websockets.sync.client import connect
    from websockets.exceptions import ConnectionClosed

    with connect(url.replace("http:", "ws:") + PATH, origin=url, subprotocols=[PROTOCOL],
                 additional_headers=headers, close_timeout=3, proxy=None) as ws:
        assert ws.subprotocol == PROTOCOL

        def receive():
            try:
                return json.loads(ws.recv(timeout=6))
            except ConnectionClosed:
                return {"close": ws.close_code}

        yield lambda msg: ws.send(json.dumps(msg)), receive


@contextmanager
def browser_wire(browser, url, headers):
    cookies = SimpleCookie(headers["Cookie"])
    with browser.new_context() as context:
        context.add_cookies([{"url": url, "name": key, "value": cookie.value,
                              "httpOnly": key == "corvid_session"} for key, cookie in cookies.items()])
        page = context.new_page()
        page.goto(url + "/healthz")
        page.evaluate("""([path, protocol]) => new Promise((resolve, reject) => {
            window.framesReceived = [];
            const ws = new WebSocket(location.origin.replace("http:", "ws:") + path, protocol);
            window.live = ws;
            ws.onopen = () => ws.protocol === protocol ? resolve() : reject("wrong subprotocol");
            ws.onerror = () => reject("upgrade failed");
            ws.onmessage = e => framesReceived.push(JSON.parse(e.data));
            ws.onclose = e => framesReceived.push({close: e.code});
        })""", [PATH, PROTOCOL])

        def receive():
            page.wait_for_function("framesReceived.length > 0", timeout=6000)
            return page.evaluate("framesReceived.shift()")

        yield lambda msg: page.evaluate("m => live.send(JSON.stringify(m))", msg), receive


def exercise(open_socket, reader, writer, auth, secret):
    headers = seed_session(auth, "member", "public", secret)
    set_role(auth, "member", "board_member", True)
    subscribe = {"type": "subscribe", "id": "cards", "path": "/boards/home/cards"}

    def post(path):
        request = Request(writer + path, method="POST", data=b"", headers=headers)
        # The logout redirect points to /, which the fixture does not serve.
        if path == "/auth/logout":
            from http.client import HTTPConnection
            from urllib.parse import urlsplit
            connection = HTTPConnection(urlsplit(writer).netloc, timeout=5)
            try:
                connection.request("POST", path, headers=headers)
                assert connection.getresponse().status == 302
            finally:
                connection.close()
            return
        with urlopen(request, timeout=6) as reply:
            return json.load(reply)

    with open_socket(reader, headers) as (send, receive):
        send(subscribe)
        assert receive() == {"type": "snapshot", "id": "cards", "rows": []}
    assert post("/add") == "offline"

    with open_socket(reader, headers) as (send, receive):
        send(subscribe)
        row = {"card": 4, "board": "home", "title": "offline", "revision": 1}
        assert receive() == {"type": "snapshot", "id": "cards", "rows": [{"row": row, "revision": "1"}]}
        assert post("/edit") == "online"
        row.update(title="online", revision=2)
        assert receive() == {"type": "change", "kind": "updated", "id": "cards",
                             "row": row, "revision": "2", "index": 0}
        send({"type": "subscribe", "id": "all", "path": "/all-cards"})
        all_cards = {"type": "snapshot", "id": "all", "rows": [{"row": row, "revision": "2"}]}
        assert receive() == all_cards
        set_role(auth, "member", "board_member", False)
        refreshed = {frame["id"]: frame for frame in (receive(), receive())}
        assert refreshed == {"cards": {"type": "error", "id": "cards", "code": "forbidden"},
                             "all": all_cards}
        post("/auth/logout")
        assert receive() == {"close": 1008}

    expiring = seed_session(auth, "temporary", "public", secret, lifetime_ms=4000)
    with open_socket(reader, expiring) as (send, receive):
        send({"type": "subscribe", "id": "all", "path": "/all-cards"})
        assert receive() == all_cards
        assert receive() == {"close": 1008}


@contextmanager
def postgres_environment(directory):
    """Use supplied disposable databases or start a loopback-only test cluster."""
    names = ("CORVID_TEST_POSTGRES_URL", "CORVID_TEST_POSTGRES_SCRATCH_URL")
    if any(os.getenv(name) for name in names):
        if not all(os.getenv(name) for name in names):
            raise RuntimeError("Provide both " + " and ".join(names))
        yield os.environ
        return
    initdb = shutil.which("initdb")
    if not initdb and os.name == "nt":
        installed = Path(os.environ["ProgramFiles"]) / "PostgreSQL"
        initdb = next(iter(sorted(installed.glob("*/bin/initdb.exe"), reverse=True)), None)
    if not initdb:
        raise RuntimeError("Install PostgreSQL with initdb on PATH or supply " + " and ".join(names))
    binaries = Path(initdb).parent
    if os.name == "nt" and ctypes.windll.shell32.IsUserAnAdmin():
        # initdb and pg_ctl drop the Administrators SID. Grant the runner's
        # user SID access to the cluster files before writing the password.
        directory = directory / "cluster"
        directory.mkdir(mode=0o700)
        identity = subprocess.check_output(["whoami", "/user", "/fo", "csv", "/nh"])
        _, sid = next(csv.reader(identity.decode("oem").splitlines()))
        subprocess.run(["icacls", directory, "/grant", f"*{sid}:(OI)(CI)F"], check=True)
    password = secrets.token_hex(24)
    password_file = directory / "test-password"
    password_file.write_text(password, encoding="ascii")
    data = directory / "pgdata"
    try:
        subprocess.run([initdb, "-D", data, "-U", "corvid", "-A", "scram-sha-256",
                        "--pwfile", password_file], check=True)
    finally:
        password_file.unlink()
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = str(sock.getsockname()[1])
    ctl = [binaries / "pg_ctl", "-D", data]
    subprocess.run([*ctl, "-l", directory / "postgres.log", "-o", f"-h 127.0.0.1 -p {port}",
                    "-w", "start"], check=True)
    try:
        env = dict(os.environ, PGPASSWORD=password)
        for name, database in zip(names, ("boards", "scratch")):
            subprocess.run([binaries / "createdb", "-h", "127.0.0.1", "-p", port,
                            "-U", "corvid", database], check=True, env=env)
            env[name] = f"postgresql://corvid:{password}@127.0.0.1:{port}/{database}"
        yield env
    finally:
        subprocess.run([*ctl, "-m", "fast", "-w", "stop"], check=True)


def run_mode(cli, root, mode, environment, open_socket):
    with ExitStack() as stack:
        secret = secrets.token_hex(32)
        env = dict(environment, CORVID_SERVE_DATA_DIR=str(root / "sessions"), CORVID_CSRF_SECRET=secret,
                   CORVID_OAUTH_GOOGLE_CLIENT_ID="fixture", CORVID_OAUTH_GOOGLE_CLIENT_SECRET="fixture")
        source = FIXTURE.read_text().replace("    board: String", "    board: String\n    title: String") + WRITES
        plan = ["migrate", "plan", "src/main.cor"]
        up = ["migrate", "up", "--database", "boards.sqlite"]
        if mode == "postgres":
            source = source.replace('engine: sqlite\n    location: "boards.sqlite"',
                                    'engine: postgres\n    location: env("CORVID_TEST_POSTGRES_URL")\n'
                                    '    tls: disabled\n    connections: 4')
            plan += ["--scratch", "CORVID_TEST_POSTGRES_SCRATCH_URL"]
            up = ["migrate", "up", "--program", "src/main.cor"]
        (root / "src").mkdir(parents=True)
        (root / "src/main.cor").write_text(source, encoding="utf-8")
        for command in [["check", "src/main.cor"], ["contract", "app", "src/main.cor", "--out", "contract.json"],
                        plan, up + ["--state", "target/state.json"]]:
            subprocess.run([cli, *command], cwd=root, env=env, check=True)
        reader = stack.enter_context(serve(cli, root, env))
        writer = stack.enter_context(serve(cli, root, env)) if mode == "postgres" else reader
        exercise(open_socket, reader, writer, root / "sessions/auth.sqlite", secret)
        print(f"Passed {mode}: reconnect, live edit, idle permission change, logout and expiry.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--channel", help="Playwright Chromium channel, e.g. msedge. Omit for a wire client.")
    parser.add_argument("--postgres", action="store_true",
                        help="Use two serve processes and fresh test and scratch databases.")
    parser.add_argument("--work-dir", type=Path, help="Retain the fixture in a new directory.")
    args = parser.parse_args()
    if args.work_dir:
        args.work_dir = args.work_dir.resolve()
        args.work_dir.mkdir(parents=True, exist_ok=False)
    cli = args.cli.resolve()
    workspace = nullcontext(args.work_dir) if args.work_dir else TemporaryDirectory()
    with workspace as temp, ExitStack() as stack:
        root = Path(temp)
        open_socket = wire
        if args.channel:
            from playwright.sync_api import sync_playwright
            playwright = stack.enter_context(sync_playwright())
            browser = playwright.chromium.launch(channel=args.channel)
            stack.callback(browser.close)
            open_socket = lambda url, headers: browser_wire(browser, url, headers)
        if not args.postgres:
            run_mode(cli, root / "sqlite", "sqlite", os.environ, open_socket)
        if args.channel or args.postgres:
            with postgres_environment(root) as env:
                run_mode(cli, root / "postgres", "postgres", env, open_socket)


if __name__ == "__main__":
    main()
