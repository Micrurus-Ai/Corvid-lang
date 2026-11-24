"""Check document navigation against a running Corvid itinerary."""
import argparse
import os
from pathlib import Path
import shutil
from urllib.error import HTTPError
from uuid import uuid4

from verify_reading_list import check_stream_cleanup, read, serve


ROOT = Path(__file__).resolve().parents[1]
LOOKOUT = "/trails/river/stops/lookout?days=2"
BRIDGE = "/trails/ridge/stops/bridge?days=3"
MISSING = "/trails/river/missing"


def check_http(url):
    assert f'href="{LOOKOUT}"' in read(url)
    assert f'content="0;url={LOOKOUT}"' in read(url + "/lookout")
    page = read(url + "/trails/north/stops/A%26B?days=7")
    assert "<title>A&amp;B on the north trail</title>" in page
    assert '<meta name="description" content="Visit A&amp;B during a 7-day walk.">' in page
    assert f'href="{BRIDGE}"' in page
    for path, status in [(MISSING, 404), (LOOKOUT.replace("days=2", "days=two"), 400)]:
        try:
            read(url + path)
        except HTTPError as error:
            assert error.code == status, (path, error.code)
        else:
            raise AssertionError(f"Expected HTTP {status} for {path}")


def check_browser(url, channel):
    from playwright.sync_api import expect, sync_playwright

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        page = browser.new_page(viewport={"width": 320, "height": 180})

        def expect_stop(tab, path, title, description):
            expect(tab).to_have_url(url + path)
            expect(tab).to_have_title(title)
            expect(tab.get_by_role("heading", level=1)).to_have_text(title)
            expect(tab.locator('meta[name="description"]')).to_have_attribute("content", description)

        lookout = (LOOKOUT, "lookout on the river trail", "Visit lookout during a 2-day walk.")
        bridge = (BRIDGE, "bridge on the ridge trail", "Visit bridge during a 3-day walk.")
        assert page.goto(url + LOOKOUT).ok
        expect_stop(page, *lookout)
        assert page.reload().ok
        expect_stop(page, *lookout)
        next_stop = page.get_by_role("link", name="Ridge bridge")
        next_stop.focus()
        page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
        scroll = page.evaluate("window.scrollY")
        assert scroll > 0
        page.keyboard.press("Enter")
        expect_stop(page, *bridge)
        shared = browser.new_page()
        assert shared.goto(page.url).ok
        expect_stop(shared, *bridge)
        shared.close()
        previous_stop = page.get_by_role("link", name="River lookout")
        previous_stop.focus()
        page.evaluate("window.scrollTo(0, 42)")
        assert page.evaluate("window.scrollY") == 42
        page.go_back()
        expect_stop(page, *lookout)
        expect(next_stop).to_be_focused()
        page.wait_for_function("saved => window.scrollY === saved", arg=scroll)
        page.go_forward()
        expect_stop(page, *bridge)
        expect(previous_stop).to_be_focused()
        page.wait_for_function("window.scrollY === 42")
        assert page.request.get(url + MISSING).status == 404

        plain = browser.new_page(java_script_enabled=False)
        plain.goto(url)
        plain.get_by_role("link", name="River lookout").click()
        expect_stop(plain, *lookout)
        plain.get_by_role("link", name="Ridge bridge").click()
        expect_stop(plain, *bridge)
        plain.goto(url + "/lookout")
        expect_stop(plain, *lookout)
        browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", help="Installed Chromium channel, such as msedge")
    parser.add_argument("--http-only", action="store_true")
    args = parser.parse_args()
    cli = shutil.which(args.cli)
    if cli is None:
        parser.error(f"Cannot find Corvid executable: {args.cli}")
    cli = str(Path(cli).resolve())
    env = dict(os.environ, CORVID_HOME=str(ROOT))
    with serve(cli, ROOT / "examples/trail_itinerary", env) as url:
        check_http(url)
        print("Itinerary HTTP checks passed.", flush=True)
        if not args.http_only:
            check_browser(url, args.channel)
    project = ROOT / "target" / f"itinerary-stream-{uuid4().hex}"
    project.mkdir(parents=True)
    try:
        (project / "src").mkdir()
        stream_env = dict(env, CORVID_IO_ROOT=str(project))
        check_stream_cleanup(cli, project, stream_env, args.channel, args.http_only)
    finally:
        shutil.rmtree(project)
    print("HTTP stream cleanup passed." if args.http_only else "Document navigation and departure checks passed.")
