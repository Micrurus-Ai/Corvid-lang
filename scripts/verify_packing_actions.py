"""Check portal placement and ownership on the served packing list."""
import argparse
import os
from pathlib import Path

from playwright.sync_api import expect, sync_playwright
from verify_reading_list import serve


def verify(cli, channel):
    project = Path(__file__).resolve().parents[1] / "examples/packing_actions"
    with serve(cli, project, os.environ) as url, sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel=channel)
        for scripting in (False, True):
            page = browser.new_page(java_script_enabled=scripting)
            page.goto(url)
            owner = page.get_by_role("complementary", name="Map")
            target = page.get_by_role("region", name="Mark packed items")
            if not scripting:
                expect(owner.get_by_label("Map packed")).to_be_visible()
                expect(target.get_by_role("checkbox")).to_have_count(0)
            else:
                expect(target.get_by_role("checkbox")).to_have_count(2)
                expect(owner.get_by_role("checkbox")).to_have_count(0)
                checkbox = target.get_by_label("Map packed")
                checkbox.check()
                # Keep handles so the assertions can distinguish a move from a copy.
                original = checkbox.element_handle()
                detached = owner.evaluate_handle("node => { node.remove(); return node }")
                expect(target.get_by_role("checkbox")).to_have_count(1)
                expect(target.get_by_label("Water packed")).to_be_visible()
                assert detached.evaluate("(node, box) => node.contains(box)", original)
                page.get_by_role("region", name="Packing list").evaluate(
                    "(list, item) => list.append(item)", detached)
                expect(checkbox).to_be_checked()
                assert checkbox.evaluate("(box, original) => box === original", original)
                # A mount with a missing destination keeps its editable control inline.
                owner.evaluate("""item => {
                    const list = item.parentNode;
                    item.remove();
                    item.querySelector('corvid-portal').dataset.targetId = 'missing';
                    list.append(item);
                }""")
                expect(owner.get_by_label("Map packed")).to_be_checked()
                expect(target.get_by_role("checkbox")).to_have_count(1)
            page.close()
        browser.close()
        print("Packing actions passed with scripting disabled and enabled.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", help="Installed Chromium channel, such as msedge")
    args = parser.parse_args()
    verify(args.cli, args.channel)
