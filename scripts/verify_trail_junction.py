"""Exercise the trail junction diagram at two viewport widths without page scripts."""
import argparse
import os
from pathlib import Path

from playwright.sync_api import expect, sync_playwright

from verify_reading_list import serve


def check_junction(page, url):
    page.goto(url)
    junction = page.get_by_role("navigation", name="Trail junction")
    bridge = junction.get_by_role("link", name="West to the bridge", exact=True)
    lookout = junction.get_by_role("link", name="East to the lookout", exact=True)
    arrow = bridge.get_by_role("img", name="West to the bridge")
    widths = []
    for width in (320, 800):
        page.set_viewport_size({"width": width, "height": 600})
        bounds = arrow.bounding_box()
        assert bounds and 0 < bounds["width"] < width
        assert bounds["width"] == 2 * bounds["height"]
        widths.append(bounds["width"])
    assert widths[0] < widths[1]

    page.keyboard.press("Tab")
    expect(bridge).to_be_focused()
    page.keyboard.press("Enter")
    expect(page.locator("h2:target")).to_have_text("Bridge")
    lookout.click()
    expect(page.locator("h2:target")).to_have_text("Lookout")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", help="Installed Chromium channel, such as msedge")
    args = parser.parse_args()
    project = Path(__file__).resolve().parents[1] / "examples/trail_junction"
    with serve(args.cli, project, os.environ) as url, sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=args.channel)
        check_junction(browser.new_page(java_script_enabled=False), url)
        browser.close()
    print("Trail junction scaling, named links and navigation passed.")
