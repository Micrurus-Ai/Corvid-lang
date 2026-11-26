"""Exercise the generated client against an authenticated Corvid WebSocket."""
import argparse
from contextlib import ExitStack
from http.cookies import SimpleCookie
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
from uuid import uuid4
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

from verify_reading_list import serve

SDK = Path(__file__).resolve().parents[1] / "sdk/typescript/client"


def build(cli, root, tsc):
    source = root / "src/main.cor"
    source.parent.mkdir()
    shutil.copyfile(SDK / "test/boards.cor", source)
    modules = root / "modules"
    shutil.copytree(SDK / "src", modules)
    for args in (["check", source], ["contract", "ts-client", source, "--out", modules],
                 ["migrate", "plan", source], ["migrate", "up", "--database", "boards.sqlite",
                                              "--state", "target/state.json"]):
        subprocess.run([cli, *args], cwd=root, check=True)
    (modules / "tsconfig.json").write_text(json.dumps({"compilerOptions": {
        "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler",
        "strict": True, "paths": {"@corvid/client": ["./index.ts"]}, "outDir": "../js",
    }}))
    compiler = ["node", tsc] if tsc else [shutil.which("tsc") or "tsc"]
    subprocess.run([*compiler, "-p", modules], cwd=root, check=True)


def exercise(page, context, url, headers, root):
    cookies = SimpleCookie(headers["Cookie"])
    context.add_cookies([{"url": url, "name": k, "value": v.value,
                          "httpOnly": k == "corvid_session"} for k, v in cookies.items()])
    # Supply the test page and compiled modules. Every subscription and write
    # goes through the actual service, including the reconnect handshake.
    assets = {p.name: p for p in (root / "js").glob("*.js")}
    page.route(url + "/test/*.js", lambda route: route.fulfill(
        path=assets[Path(urlsplit(route.request.url).path).name],
        content_type="application/javascript"))
    page.route(url + "/test/page", lambda route: route.fulfill(
        content_type="text/html", body="<title>Live client test</title>"))
    page.goto(url + "/test/page")

    def write(card, title):
        request = Request(f"{url}/boards/7/cards/{card}/{title}", data=b"", headers=headers)
        with urlopen(request, timeout=10) as reply:
            assert json.load(reply) == title

    write(1, "first")
    page.evaluate("""async () => {
        const { Api } = await import('/test/api.js');
        const { CorvidClient } = await import('/test/index.js');
        window.sockets = [];
        window.WebSocket = class extends WebSocket {
            constructor(...args) { super(...args); sockets.push(this); }
        };
        const api = new Api(new CorvidClient({baseUrl: location.origin}));
        window.updates = [];
        window.failures = [];
        window.stop = api.live.board_app.cards(7, {
            rows: rows => updates.push(rows), error: e => failures.push(e.message)
        });
        window.otherUpdates = [];
        window.stopOther = api.live.board_app.cards(9, {
            rows: rows => otherUpdates.push(rows), error: e => failures.push(e.message)
        });
    }""")
    page.wait_for_function("updates.at(-1)?.[0]?.title === 'first' && otherUpdates.length > 0")
    assert page.evaluate("sockets.length === 1 && otherUpdates.at(-1).length === 0")
    page.evaluate("() => { stopOther(); stopOther(); }")
    context.set_offline(True)
    page.evaluate("""() => new Promise(resolve => {
        const ws = sockets[0];
        if (ws.readyState === WebSocket.CLOSED) return resolve();
        ws.addEventListener('close', resolve, {once:true});
        ws.close(1000);
    })""")
    write(2, "offline")
    context.set_offline(False)
    page.wait_for_function("updates.at(-1)?.[1]?.title === 'offline'")
    assert page.evaluate("updates.at(-1)[0] === updates[0][0]")
    page.evaluate("() => { window.resumed = sockets.at(-1); }")
    write(2, "online")
    page.wait_for_function("updates.at(-1)?.[1]?.title === 'online'")
    assert page.evaluate("""updates.at(-1)[0] === updates[0][0]
        && updates.at(-1)[1] !== updates.at(-2)[1] && failures.length === 0
        && sockets.at(-1) === resumed && otherUpdates.length === 1""")
    page.evaluate("() => { stop(); stop(); }")
    page.wait_for_function("sockets.every(s => s.readyState === WebSocket.CLOSED)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--tsc", type=Path, help="TypeScript bin/tsc JavaScript entrypoint")
    args = parser.parse_args()
    cli = args.cli.resolve()
    with ExitStack() as stack:
        root = SDK / "target" / f"live-browser-{uuid4()}"
        root.mkdir(parents=True)
        stack.callback(shutil.rmtree, root)
        build(cli, root, args.tsc.resolve() if args.tsc else None)
        secret = secrets.token_hex(32)
        env = dict(os.environ, CORVID_SERVE_DATA_DIR=str(root / "sessions"),
                   CORVID_CSRF_SECRET=secret, CORVID_OAUTH_GOOGLE_CLIENT_ID="test",
                   CORVID_OAUTH_GOOGLE_CLIENT_SECRET="test")
        url = stack.enter_context(serve(cli, root, env))
        from serve_test_session import seed_session, set_role  # RT-3 fixture
        from playwright.sync_api import sync_playwright

        auth = root / "sessions/auth.sqlite"
        headers = seed_session(auth, "reader", "public", secret)
        set_role(auth, "reader", "board_member", True)
        playwright = stack.enter_context(sync_playwright())
        browser = playwright.chromium.launch(channel=args.channel)
        stack.callback(browser.close)
        context = browser.new_context()
        stack.callback(context.close)
        exercise(context.new_page(), context, url, headers, root)
        print("Generated client received the offline commit and live edit, retained row identity and closed.")


if __name__ == "__main__":
    main()
