"""JSON-lines Playwright driver embedded in the local Corvid interpreter."""
import json
import os
from pathlib import Path
import sys
import tempfile
from urllib.parse import urlsplit


def reply(value=None, error=None):
    print(json.dumps({"value": value, "error": error}), flush=True)


def origin(address):
    url = urlsplit(address)
    return url.scheme, url.hostname, url.port or {"http": 80, "https": 443}.get(url.scheme)


def main():
    from playwright.sync_api import sync_playwright

    page = browser = captures = None
    scope = None
    blocked_navigation = False
    count = 0
    with sync_playwright() as playwright:
        try:
            for line in sys.stdin:
                try:
                    request = json.loads(line)
                    tool, args = request["tool"], request["args"]
                    result = None
                    blocked_navigation = False
                    if scope and tool != "browser_close" and origin(page.url) != scope:
                        raise ValueError("page is outside the configured browser origin")
                    if tool == "browser_open_scoped":
                        width, height = args
                        if any(type(n) is not int or not 100 <= n <= 4096 for n in (width, height)):
                            raise ValueError("viewport dimensions must be integers between 100 and 4096")
                        start = os.environ["CORVID_BROWSER_URL"]
                        url = urlsplit(start)
                        if url.scheme not in ("http", "https") or not url.hostname or url.username or url.password:
                            raise ValueError("CORVID_BROWSER_URL must be an HTTP(S) URL without credentials")
                        channel = os.getenv("CORVID_BROWSER_CHANNEL", "msedge" if os.name == "nt" else "") or None
                        browser = playwright.chromium.launch(channel=channel)
                        context = browser.new_context(viewport={"width": width, "height": height},
                                                      accept_downloads=False, service_workers="block")
                        page = context.new_page()
                        page.set_default_timeout(10000)
                        scope = origin(start)

                        def route_request(route):
                            nonlocal blocked_navigation
                            if origin(route.request.url) != scope:
                                blocked_navigation |= route.request.is_navigation_request()
                                route.abort()
                            else:
                                # Inspect one HTTP response at a time. Browser redirects
                                # otherwise continue outside Playwright's route handler.
                                response = route.fetch(max_redirects=0)
                                try:
                                    if response.status in (301, 302, 303, 307, 308):
                                        blocked_navigation |= route.request.is_navigation_request()
                                        route.abort()
                                    else:
                                        route.fulfill(response=response)
                                finally:
                                    response.dispose()

                        context.route("**/*", route_request)
                        context.route_web_socket("**/*", lambda socket: socket.close())
                        response = page.goto(start, wait_until="domcontentloaded")
                        if response is None or not response.ok:
                            raise ValueError("could not open the configured page")
                    elif tool == "browser_read_text":
                        selector, = args
                        element = page.locator(selector)
                        element.wait_for(state="visible")
                        result = element.inner_text().strip()
                    elif tool == "browser_click":
                        name, = args
                        page.get_by_role("button", name=name, exact=True).click()
                    elif tool == "browser_capture":
                        if captures is None:
                            root = Path(os.getenv("CORVID_BROWSER_EVIDENCE_DIR", "target/browser-evidence"))
                            root.mkdir(parents=True, exist_ok=True)
                            captures = Path(tempfile.mkdtemp(prefix="run-", dir=root)).resolve()
                        count += 1
                        target = captures / f"capture-{count:02}.png"
                        page.screenshot(path=target, type="png")
                        result = str(target)
                    elif tool == "browser_close":
                        browser.close()
                        browser = None
                        reply()
                        return
                    else:
                        raise ValueError("unknown browser operation")
                    if blocked_navigation or (scope and origin(page.url) != scope):
                        raise ValueError("page is outside the configured browser origin")
                    reply(result)
                except Exception as exc:
                    reply(error=str(exc))
        finally:
            if browser is not None:
                browser.close()


if __name__ == "__main__":
    try:
        main()
    except ImportError as exc:
        reply(error=f"{exc}. Install Playwright in CORVID_BROWSER_PYTHON.")
