"""Exercise the server-rendered Corvid trail guide with browser scripting disabled."""
import argparse
from html import escape
import os
from pathlib import Path
from urllib.parse import parse_qs, urlencode, urlsplit

from verify_reading_list import read, serve


NOTE = '\nRain & "</textarea><em>wind</em>"'


def check_response(url):
    initial = read(url)
    landmarks_and_content = [
        '<!doctype html><html lang="en">',
        "<main><h1>River trail guide</h1>",
        '<nav aria-label="Guide sections"><a href="#packing"',
        "<h2>Route</h2>",
        'alt="Green circle trail marker">',
        '<ol start="1"><li>Bridge</li><li>Lookout</li></ol>',
        "<table><caption>Distances from the trailhead</caption>",
        '<th scope="col">Stop</th><th scope="col">Distance</th>',
        "<tr><td>Bridge</td><td>1 <abbr title='kilometres'>km</abbr></td></tr>",
        "<tr><td>Lookout</td><td>3 <abbr title='kilometres'>km</abbr></td></tr>",
        '<section aria-labelledby="packing"><h2 id="packing">Packing note</h2>',
        '<div data-trail-note="Bring water">',
        '<form method="get" action="/guide">',
    ]
    positions = [initial.index(fragment) for fragment in landmarks_and_content]
    assert positions == sorted(positions)
    response = read(url + "/guide?" + urlencode({"note": NOTE}))
    literal = escape(NOTE)
    assert response.startswith('<!doctype html><html lang="en">')
    assert f'data-trail-note="{literal}"' in response
    assert f"<p>{literal}</p>" in response
    assert f'<textarea name="note">\n{literal}</textarea>' in response


def check_browser(url, channel):
    from playwright.sync_api import expect, sync_playwright

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        page = browser.new_page(java_script_enabled=False)
        response = page.goto(url)
        assert response.ok and "text/html" in response.headers["content-type"]
        main = page.get_by_role("main")
        assert main.locator(":scope > *").evaluate_all("nodes => nodes.map(n => n.localName)") == [
            "h1", "nav", "h2", "img", "ol", "table", "section"
        ]
        expect(main.get_by_role("heading")).to_have_text(["River trail guide", "Route", "Packing note"])
        image = main.get_by_role("img", name="Green circle trail marker")
        assert image.evaluate("img => img.naturalWidth") == 24
        expect(main.locator("ol[start='1'] > li")).to_have_text(["Bridge", "Lookout"])
        table = main.get_by_role("table", name="Distances from the trailhead")
        expect(table.locator("th[scope=col]")).to_have_text(["Stop", "Distance"])
        expect(table.get_by_role("cell")).to_have_text(["Bridge", "1 km", "Lookout", "3 km"])
        expect(table.locator("td abbr[title=kilometres]")).to_have_text(["km", "km"])
        main.get_by_role("navigation", name="Guide sections").get_by_role("link").click()
        expect(page).to_have_url(url + "/#packing")
        expect(page.locator(":target")).to_have_text("Packing note")

        packing = main.get_by_role("region", name="Packing note")
        packing.get_by_label("Your note").fill(NOTE)
        packing.get_by_role("button", name="Preview").click()
        page.wait_for_url(url + "/guide?*")
        submitted = parse_qs(urlsplit(page.url).query)
        assert submitted == {"note": [NOTE.replace("\n", "\r\n")]}
        section = main.locator("[data-trail-note]")
        expect(section).to_have_attribute("data-trail-note", NOTE)
        expect(section.locator("p")).to_have_text(NOTE)
        expect(section.locator("em")).to_have_count(0)
        expect(section.get_by_label("Your note")).to_have_value(NOTE)
        browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", help="Installed Chromium channel, such as msedge")
    parser.add_argument("--http-only", action="store_true", help="Check just the server responses")
    args = parser.parse_args()
    project = Path(__file__).resolve().parents[1] / "examples/trail_guide"
    with serve(args.cli, project, os.environ) as url:
        check_response(url)
        print("Trail guide HTTP responses passed.")
        if not args.http_only:
            check_browser(url, args.channel)
            print("Trail guide passed with JavaScript disabled.")
