"""Exercise named state through the packaged page's public browser interface."""

import argparse
import os
from pathlib import Path
import tarfile
from tempfile import TemporaryDirectory

from playwright.sync_api import expect, sync_playwright

from prototype_named_state import build, lower
from verify_reading_list import serve

VIEW = Path(__file__).resolve().parents[1] / "examples/named_state/reading_counter.view"


def verify(cli, channel):
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        nested = root / "nested.view"
        nested.write_text(VIEW.read_text().replace(
            ") on click:\n        ", "):\n        on click:\n            "))
        assert lower(nested) == lower(VIEW)
        invalid = root / "invalid.view"
        invalid.write_text(VIEW.read_text().replace("pages_read + 2", "unknown + 2"))
        try:
            lower(invalid)
        except ValueError as error:
            assert "invalid.view:10:" in str(error), error
        else:
            raise AssertionError("An undeclared state reference passed")
        project = root / "counter"
        build(VIEW, project, cli)
        package = root / "installed"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(package, filter="data")
        with serve(cli, package, os.environ.copy()) as url, sync_playwright() as playwright:
            browser = playwright.chromium.launch(channel=channel)
            page = browser.new_page()
            page.goto(url)
            counters = page.get_by_role("group", name="Pages read this session:")
            expect(counters).to_have_count(2)
            first, second = counters.nth(0), counters.nth(1)
            first.get_by_role("button", name="Read one page").click()
            first.get_by_role("button", name="Read two pages").press("Enter")
            expect(first.locator("output")).to_have_text("3")
            expect(second.locator("output")).to_have_text("0")
            second.get_by_role("button", name="Read two pages").click()
            expect(second.locator("output")).to_have_text("2")
            expect(first.locator("output")).to_have_text("3")
            page.reload()
            expect(first.locator("output")).to_have_text("0")
            expect(second.locator("output")).to_have_text("0")
            browser.close()
            print("Passed shared button state, keyboard, instance isolation and reload.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="msedge")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel)
