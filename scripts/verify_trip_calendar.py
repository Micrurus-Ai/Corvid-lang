"""Build and serve the bilingual trip calendar and check both locales in one browser run."""
import argparse
from contextlib import nullcontext
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from verify_reading_list import read, serve


def check_locale(browser, url, language, direction, title, label, prefix, city):
    from playwright.sync_api import expect

    with browser.new_context(locale=language, timezone_id="America/Los_Angeles",
                             viewport={"width": 1280, "height": 800},
                             forced_colors="active", reduced_motion="reduce") as context:
        page = context.new_page()
        page.goto(url)
        root = page.locator("html")
        expect(root).to_have_attribute("lang", language)
        expect(root).to_have_css("direction", direction)
        expect(page.get_by_role("main").get_by_role("heading", level=1)).to_have_text(title)
        expected = page.evaluate("""language => {
            const units = new Intl.NumberFormat(language, {style:'unit', unit:'day', unitDisplay:'long'});
            const dates = new Intl.DateTimeFormat(language, {dateStyle:'long', timeZone:'UTC'});
            return [dates.format(new Date('2028-06-14')), units.format(2), units.format(1234)];
        }""", language)
        expect(page.locator("time")).to_have_text(expected[0])
        expect(page.locator("data")).to_have_text(expected[1:])
        if language == "ar-EG":
            assert "يومان" in expected[1] and "١٬٢٣٤" in expected[2]
        else:
            assert expected[1:] == ["2 days", "1,234 days"]

        root.evaluate("el => el.style.zoom = '4'")
        field = page.get_by_role("combobox", name=label)
        page.keyboard.press("Tab")
        expect(field).to_be_focused()
        session = context.new_cdp_session(page)
        session.send("Input.imeSetComposition", {"text": prefix, "selectionStart": len(prefix),
                                                  "selectionEnd": len(prefix)})
        expect(field).to_have_value(prefix)
        expect(field).to_have_attribute("aria-expanded", "false")
        session.send("Input.insertText", {"text": prefix})
        choices = page.get_by_role("listbox", name=label)
        expect(choices).to_be_visible()
        expect(choices).to_have_css("animation-name", "none")
        assert page.evaluate("matchMedia('(forced-colors: active)').matches")
        field.press("ArrowDown")
        selected = choices.get_by_role("option", name=city, selected=True)
        expect(selected).to_have_css("outline-style", "solid")
        expect(field).to_have_attribute("aria-activedescendant", selected.get_attribute("id"))
        assert root.evaluate("el => el.scrollWidth <= el.clientWidth")
        expect(selected).to_be_in_viewport()
        field.press("Enter")
        expect(field).to_have_value(city)
        expect(field).to_be_focused()
        expect(choices).to_be_hidden()


def verify(cli, channel, http_only, work_dir):
    root = Path(__file__).resolve().parents[1]
    if work_dir:
        work_dir.mkdir(parents=True, exist_ok=False)
    with nullcontext(work_dir) if work_dir else TemporaryDirectory() as tmp:
        project = Path(tmp) / "trip_calendar"
        shutil.copytree(root / "examples/trip_calendar", project, ignore=shutil.ignore_patterns("target"))
        for args in (["check", "src/main.cor"], ["test", "src/main.cor"],
                     ["build", "src/main.cor", "--target", "web"]):
            subprocess.run([cli, *args], cwd=project, check=True)
        installed = project / "installed"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        with serve(cli, installed, os.environ) as url:
            for path, language, direction, fallback in [("/", "en-US", "ltr", "Two days"),
                                                       ("/ar", "ar-EG", "rtl", "مدة يومين")]:
                html = read(url + path)
                assert f'<html lang="{language}" dir="{direction}">' in html
                assert f'value="2">{fallback}</data>' in html
                assert 'datetime="2028-06-14"' in html
            assert "Intl.DateTimeFormat" in read(url + "/target/wasm/events.js")
            print("Both packaged locale pages and their browser module passed HTTP checks.", flush=True)
            if http_only:
                return
            from playwright.sync_api import sync_playwright

            with sync_playwright() as engines:
                browser = engines.chromium.launch(channel=channel)
                check_locale(browser, url, "en-US", "ltr", "Trip calendar", "Destination", "Cai", "Cairo")
                check_locale(browser, url + "/ar", "ar-EG", "rtl", "تقويم الرحلة", "الوجهة", "الق", "القاهرة")
                browser.close()
            print("Both locales passed the browser checks.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--http-only", action="store_true")
    parser.add_argument("--work-dir", type=Path, help="Keep the build in a new directory")
    args = parser.parse_args()
    verify(Path(shutil.which(args.cli) or args.cli).resolve(), args.channel, args.http_only, args.work_dir)
