"""Exercise the server-rendered Corvid trail guide with browser scripting disabled."""
import argparse
from html import escape
import os
from pathlib import Path
from urllib.parse import parse_qs, urlencode, urlsplit

from verify_reading_list import read, serve


NOTE = '\nمطر & "</textarea></pre><em>wind</em>"\n  Bring water'


def check_response(url):
    initial = read(url)
    landmarks_and_content = [
        '<!doctype html><html lang="en">',
        "<main><h1>River trail guide</h1>",
        '<nav aria-label="Guide sections"><a href="#route" tabindex="0">Route, <abbr title="3 kilometres">3 km</abbr></a>'
        '<a href="#packing" tabindex="0">Packing note</a></nav>',
        '<section aria-labelledby="route"><h2 id="route">Route</h2><figure><img ',
        'alt="Green circle trail marker"><figcaption>Follow this marker to the lookout.</figcaption></figure>',
        '<p>Walking time: <time datetime="PT45M">45 minutes</time></p>',
        '<ol start="1"><li>Bridge</li><li value="3">Lookout</li></ol>',
        "<table><caption>Distances from the trailhead</caption>",
        '<th scope="col">Stop</th><th scope="col">Distance</th>',
        '<tr><th scope="row">Bridge</th><td><data value="1000"><abbr title="1 kilometre">1 km</abbr></data></td></tr>',
        '<tr><th scope="row">Lookout</th><td><data value="3000"><abbr title="3 kilometres">3 km</abbr></data></td></tr>'
        '</tbody></table></section>',
        '<section aria-labelledby="packing"><h2 id="packing">Packing note</h2>',
        '<div data-trail-note="Bring water"><aside aria-label="Suggested supplies">'
        '<details open><summary>Packing checklist</summary>'
        '<ul><li><label><input type="checkbox" name="water">Water bottle</label></li>'
        '<li><label><input type="checkbox" name="map">Trail map</label></li></ul></details></aside>',
        '<form method="get" action="/guide">',
        '<noscript><p>Select Preview to reload the guide with your note.</p></noscript>',
        '<fieldset><legend>Edit packing note</legend><label>Your note<textarea name="note" required>',
        '<button type="submit">Preview</button><button type="reset">Restore note</button></fieldset></form>',
    ]
    positions = [initial.index(fragment) for fragment in landmarks_and_content]
    assert positions == sorted(positions)
    response = read(url + "/guide?" + urlencode({"note": NOTE}))
    literal = escape(NOTE)
    assert response.startswith('<!doctype html><html lang="en">')
    assert f'data-trail-note="{literal}"' in response
    assert f'<h3>Current note</h3><pre dir="auto">\n{literal}</pre>' in response
    assert f'<textarea name="note" required>\n{literal}</textarea>' in response


def check_browser(url, channel):
    from playwright.sync_api import expect, sync_playwright

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        page = browser.new_page(java_script_enabled=False)
        response = page.goto(url)
        assert response.ok and "text/html" in response.headers["content-type"]
        main = page.get_by_role("main")
        assert main.locator(":scope > *").evaluate_all("nodes => nodes.map(n => n.localName)") == [
            "h1", "nav", "section", "section"
        ]
        expect(main.get_by_role("heading")).to_have_text(["River trail guide", "Route", "Packing note", "Current note"])
        route = main.get_by_role("region", name="Route", exact=True)
        assert route.locator(":scope > *").evaluate_all("nodes => nodes.map(n => n.localName)") == [
            "h2", "figure", "p", "ol", "table"
        ]
        expect(route.locator('time[datetime="PT45M"]')).to_have_text("45 minutes")
        figure = route.get_by_role("figure")
        expect(figure.locator("figcaption")).to_have_text("Follow this marker to the lookout.")
        image = figure.get_by_role("img", name="Green circle trail marker")
        assert image.evaluate("img => img.naturalWidth") == 24
        expect(main.locator("ol[start='1'] > li")).to_have_text(["Bridge", "Lookout"])
        expect(route.locator("ol > li").nth(1)).to_have_js_property("value", 3)
        table = main.get_by_role("table", name="Distances from the trailhead")
        expect(table.locator("th[scope=col]")).to_have_text(["Stop", "Distance"])
        expect(table.get_by_role("rowheader")).to_have_text(["Bridge", "Lookout"])
        expect(table.get_by_role("cell")).to_have_text(["1 km", "3 km"])
        for cell, metres in zip(table.get_by_role("cell").all(), ["1000", "3000"]):
            expect(cell.locator("data")).to_have_attribute("value", metres)
        expect(table.get_by_title("1 kilometre", exact=True)).to_have_text("1 km")
        expect(table.get_by_title("3 kilometres", exact=True)).to_have_text("3 km")
        navigation = main.get_by_role("navigation", name="Guide sections")
        for label, fragment, heading in [("Route, 3 km", "route", "Route"), ("Packing note", "packing", "Packing note")]:
            navigation.get_by_role("link", name=label, exact=True).click()
            expect(page).to_have_url(url + "/#" + fragment)
            expect(page.locator("h2:target")).to_have_text(heading)

        packing = main.get_by_role("region", name="Packing note")
        supplies_aside = packing.get_by_role("complementary", name="Suggested supplies")
        checklist = supplies_aside.locator("details")
        supplies = checklist.get_by_role("list", include_hidden=True)
        expect(supplies).to_be_visible()
        toggle = checklist.get_by_text("Packing checklist", exact=True)
        toggle.click()
        expect(supplies).to_be_hidden()
        toggle.click()
        expect(supplies).to_be_visible()
        expect(supplies.get_by_role("listitem")).to_have_text(["Water bottle", "Trail map"])
        water = supplies.get_by_role("checkbox", name="Water bottle")
        expect(water).not_to_be_checked()
        water.check()
        expect(water).to_be_checked()
        expect(supplies.get_by_role("checkbox", name="Trail map")).not_to_be_checked()
        toggle.click()
        expect(supplies).to_be_hidden()
        editor = packing.get_by_role("group", name="Edit packing note")
        expect(packing.locator("form > noscript > p")).to_be_visible()
        field = editor.get_by_label("Your note")
        field.fill("")
        editor.get_by_role("button", name="Preview").click()
        expect(field).to_be_focused()
        assert field.evaluate("field => field.validity.valueMissing")
        expect(page).to_have_url(url + "/#packing")
        field.fill(NOTE)
        editor.get_by_role("button", name="Preview").click()
        page.wait_for_url(url + "/guide?*")
        submitted = parse_qs(urlsplit(page.url).query)
        assert submitted == {"note": [NOTE.replace("\n", "\r\n")]}
        section = main.locator("[data-trail-note]")
        expect(section).to_have_attribute("data-trail-note", NOTE)
        expect(section.get_by_role("heading", level=3)).to_have_text("Current note")
        preview = section.locator("pre")
        assert preview.text_content() == NOTE
        expect(preview).to_have_css("direction", "rtl")
        expect(section.locator("em")).to_have_count(0)
        expect(section.get_by_label("Your note")).to_have_value(NOTE)
        preview_url = page.url
        field.fill("Unsaved edit")
        editor.get_by_role("button", name="Restore note").click()
        expect(field).to_have_value(NOTE)
        expect(page).to_have_url(preview_url)
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
