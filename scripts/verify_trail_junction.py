"""Check the extracted trail junction web package over HTTP and in Chromium."""
import argparse
from contextlib import nullcontext
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory
from urllib.request import urlopen

from verify_reading_list import serve


def check_assets(project, installed, url):
    for asset in (project / "assets").iterdir():
        assert (installed / "assets" / asset.name).read_bytes() == asset.read_bytes()
        expected_type = {".png": "image/png", ".ttf": "font/ttf", ".txt": "text/plain"}[asset.suffix]
        with urlopen(f"{url}/assets/{asset.name}", timeout=10) as response:
            assert response.headers.get_content_type() == expected_type, asset.name
            assert response.read() == asset.read_bytes(), asset.name


def check_junction(browser, url):
    from playwright.sync_api import expect

    widths = []
    for viewport, source in ((320, 320), (800, 640)):
        page = browser.new_page(viewport={"width": viewport, "height": 600},
                                device_scale_factor=1, java_script_enabled=False)
        page.goto(url)
        junction = page.get_by_role("navigation", name="Trail junction")
        bridge = junction.get_by_role("link", name="West to the bridge", exact=True)
        lookout = junction.get_by_role("link", name="East to the lookout", exact=True)
        arrow = bridge.get_by_role("img", name="West to the bridge")
        bounds = arrow.bounding_box()
        assert bounds and 0 < bounds["width"] < viewport
        assert bounds["width"] == 2 * bounds["height"]
        widths.append(bounds["width"])
        map_image = page.get_by_role("img", name="At the fork,", exact=False)
        assert map_image.evaluate("img => img.complete && img.naturalWidth > 0")
        expect(map_image).to_have_js_property("currentSrc", f"{url}/assets/junction-map-{source}.png")
        assert page.evaluate("""async () => {
            await document.fonts.ready;
            return [...document.fonts].some(face => face.family.includes('Barlow Condensed') &&
                                                    face.status === 'loaded');
        }""")
        page.keyboard.press("Tab")
        expect(bridge).to_be_focused()
        page.keyboard.press("Enter")
        expect(page.locator("h2:target")).to_have_text("Bridge")
        lookout.click()
        expect(page.locator("h2:target")).to_have_text("Lookout")
        page.close()
    assert widths[0] < widths[1]


def check_motion(browser, url):
    from playwright.sync_api import expect

    page = browser.new_page(reduced_motion="no-preference")
    page.goto(url)
    # Select the other destination on the first animated frame. Doing this in
    # the event listener avoids racing a short transition from the test driver.
    page.evaluate("""() => {
        window.cancelledMotion = false;
        const bridge = document.getElementById('bridge');
        bridge.addEventListener('transitioncancel', event => {
            if (event.propertyName === 'transform') window.cancelledMotion = true;
        });
        bridge.addEventListener('transitionstart', () => {
            document.querySelector('a[href="#lookout"]').click();
        }, {once: true});
    }""")
    page.get_by_role("link", name="West to the bridge").click()
    page.wait_for_function("window.cancelledMotion")
    expect(page.locator("h2:target")).to_have_text("Lookout")
    page.wait_for_function("""[...document.querySelectorAll('h2')].every(
        heading => heading.getAnimations().length === 0)""")
    expect(page.locator("#bridge")).to_have_css("transform", "none")
    expect(page.locator("#lookout")).to_have_css("transform", "matrix(1, 0, 0, 1, 8, 0)")

    page.emulate_media(reduced_motion="reduce")
    page.get_by_role("link", name="West to the bridge").click()
    expect(page.locator("h2:target")).to_have_text("Bridge")
    expect(page.locator("#bridge")).to_have_css("transition-duration", "0s")
    expect(page.locator("#bridge")).to_have_css("transform", "matrix(1, 0, 0, 1, 8, 0)")
    assert page.evaluate("document.getAnimations().length === 0")
    page.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="corvid")
    parser.add_argument("--channel", help="Installed Chromium channel, such as msedge")
    parser.add_argument("--http-only", action="store_true", help="Verify packaged assets without a browser")
    parser.add_argument("--work-dir", type=Path, help="Extract into this new directory and retain it")
    args = parser.parse_args()
    cli = shutil.which(args.cli) or args.cli
    project = Path(__file__).resolve().parents[1] / "examples/trail_junction"
    subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=False)
    with nullcontext(args.work_dir) if args.work_dir else TemporaryDirectory() as extracted:
        installed = Path(extracted).resolve()
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        with serve(cli, installed, os.environ) as url:
            with urlopen(url, timeout=10) as response:
                html = response.read().decode()
                assert "/assets/junction-map-640.png 640w" in html
                assert "/assets/BarlowCondensed-Regular.ttf" in html
                assert "prefers-reduced-motion: no-preference" in html
            check_assets(project, installed, url)
            print("Packaged trail map sources, font and license passed HTTP checks.", flush=True)
            if not args.http_only:
                from playwright.sync_api import sync_playwright

                with sync_playwright() as playwright:
                    browser = playwright.chromium.launch(channel=args.channel)
                    check_junction(browser, url)
                    check_motion(browser, url)
                    browser.close()
                print("Trail diagram, responsive sources, font loading and motion passed.")
