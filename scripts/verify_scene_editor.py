"""Build the landscape editor and exercise graphics, input and capture in Edge."""

import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
from threading import Thread


PROJECT = Path(__file__).resolve().parents[1] / "examples/scene_editor"


def build(cli):
    for command in [
        [cli, "check", "src/viewport.cor"],
        [cli, "build", "src/viewport.cor", "--target", "wasm"],
        ["node", "test/scene.test.mjs"],
    ]:
        subprocess.run(command, cwd=PROJECT, check=True)


def exercise(page, origin):
    from playwright.sync_api import expect

    # Observe the actual WebGL framebuffer during a frame, before compositing
    # can discard it. Sample a point in the lake at the initial viewport.
    page.add_init_script("""{
      let inFrame = false;
      const schedule = window.requestAnimationFrame.bind(window);
      window.requestAnimationFrame = callback => schedule(time => {
        inFrame = true;
        try { callback(time); } finally { inFrame = false; }
      });
      const clear = WebGLRenderingContext.prototype.clear;
      WebGLRenderingContext.prototype.clear = function(mask) {
        clear.call(this, mask);
        if (!inFrame) throw new Error("Scene painted outside an animation frame");
        const pixel = new Uint8Array(4);
        this.readPixels(60, this.drawingBufferHeight - 61, 1, 1, this.RGBA, this.UNSIGNED_BYTE, pixel);
        window.sampledColor = [...pixel];
      };
    }""")
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(origin)
    canvas = page.locator("canvas")
    summary = page.locator("#description")
    button = lambda name: page.get_by_role("button", name=name, exact=True)
    expect(summary).to_contain_text("Selected: Lake")
    assert any(worker.url.endswith("/scene-worker.js") for worker in page.workers)
    page.wait_for_function("""() => {
      const el = document.querySelector('canvas'), bounds = el.getBoundingClientRect();
      return el.width === Math.round(bounds.width * 2) && el.height === Math.round(bounds.height * 2)
        && window.sampledColor?.[2] > 190 && sampledColor[0] < 40;
    }""")
    bounds = canvas.bounding_box()
    page.mouse.move(bounds["x"] + 100, bounds["y"] + 70)
    page.mouse.down()
    page.mouse.move(bounds["x"] + 116, bounds["y"] + 78)
    page.mouse.up()
    expect(summary).to_contain_text("Pan (16, 8)")
    button("Zoom in").click()
    expect(summary).to_contain_text("Zoom 2")
    page.wait_for_function("sampledColor[0] > 220")
    canvas.click(position={"x": 200, "y": 100})
    expect(summary).to_contain_text("Selected: Forest")
    button("Move right").click()
    expect(page.get_by_role("listitem").nth(1)).to_have_text("Forest at (96, 40)")
    button("Undo move").click()
    expect(page.get_by_role("listitem").nth(1)).to_have_text("Forest at (88, 40)")

    canvas.evaluate("""el => {
      window.loss = el.getContext('webgl').getExtension('WEBGL_lose_context');
      loss.loseContext();
    }""")
    expect(page.locator("#graphics")).to_contain_text("Graphics lost")
    page.get_by_label("Selected tile").focus()
    page.keyboard.press("ArrowUp")
    expect(summary).to_contain_text("Selected: Lake")
    button("Move down").focus()
    page.keyboard.press("Enter")
    expect(page.get_by_role("listitem").first).to_have_text("Lake at (24, 32)")
    button("Zoom out").click()
    button("Undo move").click()
    expect(page.get_by_role("listitem").first).to_have_text("Lake at (24, 24)")
    # Reset the sample before restoration to require a fresh GPU frame.
    page.evaluate("window.sampledColor = null; loss.restoreContext()")
    expect(page.locator("#graphics")).to_have_text("Graphics ready.")
    page.wait_for_function("window.sampledColor?.[0] > 220")
    button("Pan left").click()
    button("Pan up").click()
    page.wait_for_function("sampledColor[2] > 190 && sampledColor[0] < 40")

    session = page.context.new_cdp_session(page)
    context_id = session.send("Target.getTargetInfo")["targetInfo"]["browserContextId"]
    session.send("Browser.setPermission", {
        "permission": {"name": "camera"}, "setting": "denied",
        "origin": origin, "browserContextId": context_id,
    })
    button("Start camera").click()
    expect(page.locator("#camera-status")).to_contain_text("permission denied")
    button("Move right").click()
    expect(page.get_by_role("listitem").first).to_have_text("Lake at (32, 24)")
    page.context.grant_permissions(["camera"], origin=origin)
    button("Start camera").click()
    expect(page.locator("#camera-status")).to_have_text("Camera live.")
    page.wait_for_function("document.querySelector('video').videoWidth > 0")
    track = page.locator("video").evaluate_handle("video => video.srcObject.getVideoTracks()[0]")
    button("Stop camera").click()
    assert track.evaluate("track => track.readyState") == "ended"
    expect(page.locator("video")).to_be_hidden()
    assert not errors, errors


def verify(channel):
    from playwright.sync_api import sync_playwright

    handler = partial(SimpleHTTPRequestHandler, directory=PROJECT)
    with sync_playwright() as playwright, ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
        Thread(target=server.serve_forever, daemon=True).start()
        try:
            browser = playwright.chromium.launch(channel=channel, args=[
                "--use-fake-device-for-media-stream", "--enable-unsafe-swiftshader",
            ])
            try:
                exercise(browser.new_page(device_scale_factor=2), f"http://127.0.0.1:{server.server_port}")
            finally:
                browser.close()
        finally:
            server.shutdown()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--build-only", action="store_true")
    args = parser.parse_args()
    build(args.cli.resolve())
    if not args.build_only:
        verify(args.channel)
        print("Passed scene editing, frame rendering, context recovery and camera checks.")
