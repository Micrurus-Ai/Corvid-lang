"""Build both booking screens and exercise their shared form through Chromium WebMCP."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory
from urllib.parse import urlencode
from urllib.request import urlopen

from playwright.sync_api import expect, sync_playwright
from verify_reading_list import serve


EXAMPLE = Path(__file__).resolve().parents[1] / "examples/booking"
MODULE = "https://cdn.jsdelivr.net/npm/cally@0.9.2/dist/cally.js"


def verify_field_lifecycle(cli, channel):
    with TemporaryDirectory() as tmp:
        project = Path(tmp) / "booking"
        shutil.copytree(EXAMPLE, project)
        subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
        with serve(cli, project, dict(os.environ)) as url, sync_playwright() as playwright:
            browser = playwright.chromium.launch(channel=channel)
            page = browser.new_page()
            page.route(MODULE, lambda route: route.abort())
            page.goto(url)
            page.evaluate("""async () => {
                const field = document.querySelector('input[name=date]');
                const calendar = document.querySelector('calendar-date');
                const binding = field.closest('[data-corvid-bind-field]');
                const form = field.form;
                await customElements.whenDefined('corvid-field-binding');
                binding.remove();
                customElements.define('calendar-date', class extends HTMLElement { value = ''; });
                field.value = '2028-06-14';
                await Promise.resolve();
                if (calendar.value) throw Error('Detached picker synchronized during upgrade');
                form.prepend(binding);
                await Promise.resolve();
                if (calendar.value !== field.value) throw Error('Mount did not synchronize the picker');
                form.reset();
                binding.remove();
                field.value = '2028-06-15';
                field.dispatchEvent(new Event('input'));
                await Promise.resolve();
                if (calendar.value !== '2028-06-14') throw Error('Detached picker received an update');
                calendar.value = '2028-06-16';
                calendar.dispatchEvent(new Event('change'));
                if (field.value !== '2028-06-15') throw Error('Detached picker changed the field');
                const nextForm = document.createElement('form');
                document.body.append(nextForm);
                nextForm.append(binding);
                await Promise.resolve();
                if (calendar.value !== field.value) throw Error('Remount lost the current field value');
                field.value = '2028-06-17';
                form.reset();
                await Promise.resolve();
                if (calendar.value !== '2028-06-15') throw Error('Previous form still controls the picker');
                nextForm.reset();
                await Promise.resolve();
                if (calendar.value !== field.defaultValue) throw Error('New form reset did not synchronize');
            }""")
            browser.close()
    print("Bound field released its listeners and synchronized after remount.")


def trial(cli, channel):
    with TemporaryDirectory() as tmp, sync_playwright() as playwright:
        project = Path(tmp) / "booking"
        shutil.copytree(EXAMPLE, project)
        main = project / "src/main.cor"
        source = main.read_text(encoding="utf-8")
        browser = playwright.chromium.launch(channel=channel, args=["--enable-experimental-web-platform-features"])
        first_schema = None
        for picker in ("calendar_picker", "native_picker"):
            main.write_text(source.replace("calendar_picker as picker", picker + " as picker"), encoding="utf-8")
            subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
            installed = Path(tmp) / picker
            with tarfile.open(project / "target/web.tar") as archive:
                archive.extractall(installed, filter="data")
            with serve(cli, installed, dict(os.environ)) as url:
                page = browser.new_page()
                page.goto(url)
                field = page.get_by_label("Booking date", exact=True)
                field.fill("2028-06-14")
                if picker == "calendar_picker":
                    calendar = page.locator("calendar-date")
                    expect(calendar).to_be_visible()
                    day = calendar.locator("button[tabindex='0']")
                    expect(day).to_have_attribute("aria-label", "June 14")
                    day.press("ArrowRight")
                    next_day = calendar.get_by_role("button", name="June 15", exact=True)
                    expect(next_day).to_be_focused()
                    next_day.press("Enter")
                    expect(field).to_have_value("2028-06-15")
                else:
                    field.press("ArrowUp")
                selected = field.input_value()
                assert selected and selected != "2028-06-14"
                page.get_by_role("button", name="Book date").press("Enter")
                expect(page.get_by_text("Booked for " + selected, exact=True)).to_be_visible()
                page.goto(url)
                expect(field).to_have_value(selected)
                schema = page.evaluate("""async () => (await document.modelContext.getTools())
                    .map(({name, description, inputSchema}) => ({name, description, inputSchema}))""")
                assert len(schema) == 1 and schema[0]["name"] == "book_date", schema
                assert set(schema[0]["inputSchema"]["properties"]) == {"date"}, schema
                if first_schema is None:
                    first_schema = schema
                assert schema == first_schema
                page.evaluate("""async () => {
                    const ctx = document.modelContext;
                    const [tool] = await ctx.getTools();
                    window.bookingReply = ctx.executeTool(tool, {date: "2028-02-29"});
                }""")
                expect(field).to_have_value("2028-02-29")
                with urlopen(url) as response:
                    assert selected in response.read().decode()
                page.get_by_role("button", name="Book date").press("Enter")
                assert "Booked for 2028-02-29" in page.evaluate("() => window.bookingReply")
                with urlopen(url + "/book", urlencode({"date": "2027-02-29"}).encode()) as response:
                    assert "Enter a valid booking date" in response.read().decode()
                page.close()
            with serve(cli, installed, dict(os.environ)) as url:
                page = browser.new_page()
                page.route(MODULE, lambda route: route.abort())
                page.goto(url)
                expect(page.get_by_label("Booking date")).to_have_value("2028-02-29")
                page.get_by_label("Booking date").fill("2028-03-01")
                page.get_by_role("button", name="Book date").press("Enter")
                expect(page.get_by_text("Booked for 2028-03-01", exact=True)).to_be_visible()
                page.close()
            print(f"{picker}: keyboard, confirmed WebMCP action, validation, restart and fallback passed.", flush=True)
        browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--channel", default="chrome")
    parser.add_argument("--field-lifecycle-only", action="store_true")
    args = parser.parse_args()
    verify_field_lifecycle(args.cli.resolve(), args.channel)
    if not args.field_lifecycle_only:
        trial(args.cli.resolve(), args.channel)
