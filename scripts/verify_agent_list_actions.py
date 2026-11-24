"""Verify the packaged agent list with the native WebMCP client and keyboard."""
import argparse
from contextlib import nullcontext
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from verify_reading_list import read, serve


def exercise(browser, url, assistant):
    from playwright.sync_api import expect

    page = browser.new_page()
    if not assistant:
        page.add_init_script("Object.defineProperty(document, 'modelContext', {value: undefined})")
    page.goto(url)
    earlier = page.get_by_role("button", name="Move writer earlier")
    later = page.get_by_role("button", name="Move writer later")
    rows = page.get_by_role("listitem")
    earlier.press("Enter")
    expect(rows.first).to_have_attribute("data-corvid-key", "writer")
    later.press("Enter")
    expect(rows.first).to_have_attribute("data-corvid-key", "reader")
    if not assistant:
        assert page.evaluate("document.modelContext === undefined")
        page.close()
        return

    page.wait_for_function("""async () => (await document.modelContext.getTools())
        .some(tool => tool.name === 'move_agent')""")
    schema = page.evaluate("""async () => {
        const context = document.modelContext;
        const tools = await context.getTools();
        if (tools.length !== 1) throw Error('Unexpected exposed actions');
        const [tool] = tools;
        window.invokeMove = args => context.executeTool(tool, args);
        return tool.inputSchema;
    }""")
    assert schema["properties"] == {"key": {"type": "string"},
                                    "direction": {"type": "string", "enum": ["up", "down"]}}
    context = page.evaluate("() => invokeMove({})")
    assert (json.loads(context) if isinstance(context, str) else context) == ["reader", "writer"]
    move = {"key": "writer", "direction": "up"}
    page.evaluate("args => invokeMove(args)", move)
    expect(rows.first).to_have_attribute("data-corvid-key", "writer")
    later.press("Enter")

    def expect_refusal(args):
        assert page.evaluate("""async args => {
            try { await invokeMove(args); } catch { return true; }
            return false;
        }""", args), args

    earlier.evaluate("button => button.disabled = true")
    rows.last.get_by_role("textbox").press("Alt+ArrowUp")
    expect(rows.first).to_have_attribute("data-corvid-key", "reader")
    expect_refusal(move)
    expect(rows.first).to_have_attribute("data-corvid-key", "reader")
    earlier.evaluate("button => button.disabled = false")
    page.get_by_role("button", name="Remove agent writer").click()
    expect_refusal(move)
    expect(rows).to_have_count(1)
    page.locator("[data-list-editor]").evaluate("editor => document.body.append(editor)")
    expect_refusal({})
    expect_refusal({"key": "reader", "direction": "down"})
    page.close()


def verify(cli, channel, work_dir):
    example = Path(__file__).resolve().parents[1] / "examples/agent_settings"
    if work_dir:
        work_dir.mkdir(parents=True)
    with nullcontext(work_dir) if work_dir else TemporaryDirectory() as directory:
        project = Path(directory) / "agent-settings"
        shutil.copytree(example, project, ignore=shutil.ignore_patterns("target"))
        subprocess.run([cli, "check"], cwd=project, check=True)
        subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)
        installed = Path(directory) / "installed"
        with tarfile.open(project / "target/web.tar") as archive:
            archive.extractall(installed, filter="data")
        with serve(cli, installed, os.environ) as url:
            agents = url + "/agents"
            assert 'data-corvid-tool="move_agent"' in read(agents)
            print("Packaged agent list served with its declared action.", flush=True)
            from playwright.sync_api import sync_playwright
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(channel=channel, args=[
                    "--enable-blink-features=WebMCP,WebMCPTesting"])
                for assistant in (False, True):
                    exercise(browser, agents, assistant)
                print(f"{channel} {browser.version} passed keyboard and WebMCP checks.")
                browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--work-dir", type=Path, help="retain the package in a new directory")
    args = parser.parse_args()
    verify(args.cli.resolve(), args.channel, args.work_dir)
