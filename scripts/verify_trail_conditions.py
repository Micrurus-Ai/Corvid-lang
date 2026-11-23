"""Check trail card layout and inherited theme colors on a Corvid-served page."""
import argparse
import os
from pathlib import Path

from playwright.sync_api import expect, sync_playwright

from verify_reading_list import serve


def check_cards(page, url):
    page.emulate_media(media="screen", color_scheme="light")
    page.goto(url)
    forest = page.locator(".trail-card.forest")
    river = page.locator(".trail-card.river")
    expect(forest.get_by_role("heading")).to_have_css("color", "rgb(40, 94, 56)")
    expect(river.get_by_role("heading")).to_have_css("color", "rgb(34, 87, 130)")
    assert page.locator("h1").evaluate(
        "heading => getComputedStyle(heading).getPropertyValue('--trail-accent')"
    ) == ""
    for width in (360, 960):
        page.set_viewport_size({"width": width, "height": 720})
        first = forest.bounding_box()
        second = river.bounding_box()
        if width == 360:
            assert first["x"] == second["x"]
            assert first["y"] + first["height"] < second["y"]
        else:
            assert first["y"] == second["y"]
            assert first["x"] + first["width"] < second["x"]
        for card in (forest, river):
            summary = card.locator(".trail-summary")
            expect(summary).to_have_css("display", "flex")
            expect(summary).to_have_css("flex-direction", "column" if width == 360 else "row")

    # Keep the wide viewport and constrain only the forest card.
    forest.evaluate("card => card.style.maxInlineSize = '18rem'")
    expect(forest.locator(".trail-summary")).to_have_css("flex-direction", "column")
    expect(river.locator(".trail-summary")).to_have_css("flex-direction", "row")

    page.emulate_media(media="screen", color_scheme="dark")
    body = page.locator("body")
    expect(body).to_have_css("background-color", "rgb(24, 35, 29)")
    expect(body).to_have_css("color", "rgb(242, 245, 243)")
    expect(forest.get_by_role("heading")).to_have_css("color", "rgb(154, 221, 171)")
    expect(river.get_by_role("heading")).to_have_css("color", "rgb(152, 207, 255)")

    page.emulate_media(media="print", color_scheme="dark")
    expect(body).to_have_css("background-color", "rgb(255, 255, 255)")
    expect(body).to_have_css("color", "rgb(0, 0, 0)")
    for card in (forest, river):
        expect(card.get_by_role("heading")).to_have_css("color", "rgb(0, 0, 0)")
        expect(card).to_have_css("break-inside", "avoid")
    assert river.bounding_box()["y"] >= forest.bounding_box()["y"] + forest.bounding_box()["height"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", help="Installed Chromium channel, such as msedge")
    args = parser.parse_args()
    project = Path(__file__).resolve().parents[1] / "examples/trail_conditions"
    with serve(args.cli, project, os.environ) as url, sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=args.channel)
        check_cards(browser.new_page(), url)
        browser.close()
    print("Trail card layout and theme checks passed.")
