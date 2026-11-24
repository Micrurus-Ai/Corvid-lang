"""Compile the typed client and exercise the existing Corvid orders server."""
import argparse
from contextlib import nullcontext
import json
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory

from verify_reading_list import check_search, check_search_http, serve


def verify(args):
    root = Path(__file__).resolve().parents[1]
    project = root / "examples/orders_api"
    env = dict(os.environ, CORVID_HOME=str(root))
    cli = shutil.which(args.cli) or str(Path(args.cli).resolve())
    if args.tsc:
        command = ["node", Path(args.tsc).resolve()]
    elif compiler := shutil.which("tsc"):
        command = [compiler]
    else:
        # Hosted Windows has Node and npm but no global TypeScript compiler.
        npm = shutil.which("npm.cmd" if os.name == "nt" else "npm")
        if not npm:
            raise RuntimeError("Install npm or pass --tsc PATH_TO_TYPESCRIPT/bin/tsc")
        command = [npm, "exec", "--yes", "--package=typescript@5.9.3", "--", "tsc"]
    with (nullcontext(args.work_dir) if args.work_dir else TemporaryDirectory()) as temporary:
        build = Path(temporary).resolve()
        if args.work_dir:
            build.mkdir(parents=True, exist_ok=False)
        shutil.copytree(root / "sdk/typescript/client/src", build / "client")
        shutil.copy2(root / "scripts/fixtures/order_resources.ts", build)
        subprocess.run([cli, "contract", "ts-client", "src/main.cor", "--out", build / "generated"],
                       cwd=project, env=env, check=True)
        subprocess.run([*command, "--strict", "--target", "ES2022", "--module", "ES2022",
                        "--moduleResolution", "bundler", "--outDir", "dist", "order_resources.ts"],
                       cwd=build, check=True)
        dist = build / "dist"
        (dist / "package.json").write_text(json.dumps({"type": "module"}))
        subprocess.run([cli, "new", "reading-list"], cwd=build, env=env, check=True)
        reading_list = build / "reading-list"
        source = root / "examples/reading_list"
        shutil.copy2(source / "src/main.cor", reading_list / "src/main.cor")
        shutil.copytree(source / "migrations", reading_list / "migrations")
        subprocess.run([cli, "migrate", "up", "--dir", "migrations",
                        "--database", "reading.sqlite"], cwd=reading_list, env=env, check=True)
        with serve(cli, reading_list, env) as search_url, serve(cli, project, env, mode="mock") as url:
            check_search_http(search_url, reading_list / "reading.sqlite")
            subprocess.run(["node", "--input-type=module", "-e",
                            "import {checkOrders} from './order_resources.js'; await checkOrders(process.argv[1])",
                            url], cwd=dist, check=True)
            print("Typed search, submit, cursor requests, cancellation and order stream passed.", flush=True)
            if args.http_only:
                return
            from playwright.sync_api import sync_playwright
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(channel=args.channel)
                page = browser.new_page()
                # Serve only compiled test modules locally. All API requests reach Corvid.
                for module in dist.rglob("*.js"):
                    page.route(url + "/client-trial/" + module.relative_to(dist).as_posix(),
                               lambda route, path=module: route.fulfill(path=path, content_type="text/javascript"))
                page.route(url + "/client-trial/", lambda route: route.fulfill(
                    content_type="text/html", body="<!doctype html><title>Order client check</title>"))
                page.goto(url + "/client-trial/")
                page.evaluate("""async () => {
                    const checks = await import('/client-trial/order_resources.js');
                    await checks.checkOrders(location.origin);
                    await checks.checkOrderUpload(location.origin);
                }""")
                from verify_rgb_picker import check as check_rollback, package_picker
                picker = package_picker(cli, build / "picker")
                with serve(cli, picker, env) as picker_url:
                    check_rollback(browser.new_page(), picker_url)

                check_search(browser.new_page(), search_url, reading_list / "reading.sqlite")
                browser.close()
    print("Browser order requests, upload progress and retry, search cancellation and optimistic rollback passed.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel")
    parser.add_argument("--tsc", help="Path to the TypeScript package's bin/tsc script")
    parser.add_argument("--http-only", action="store_true")
    parser.add_argument("--work-dir", type=Path, help="Retain compiled files in a new directory")
    verify(parser.parse_args())
