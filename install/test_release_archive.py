"""Exercise a candidate CLI archive outside the checkout, before publication."""

import argparse
from http.client import parse_headers
from io import BytesIO
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import URLError
from urllib.request import urlopen


def verify(archive, expected_version, work):
    # Use the production installer with only its release download supplied
    # locally. This checks target selection as well as the installed layout.
    root = work / "Corvid installation"
    scripts = Path(__file__).resolve().parent
    env = dict(os.environ, CORVID_HOME=str(root), CORVID_VERSION="v" + expected_version,
               CORVID_NO_MODIFY_PROFILE="1", CORVID_SKIP_EDITOR_EXTENSION="1",
               CORVID_TEST_ARCHIVE=str(archive))
    for key in ("GITHUB_PATH", "GITHUB_ENV", "CORVID_STD_PATH", "CORVID_REPO"):
        env.pop(key, None)
    if os.name == "nt":
        wrapper = work / "candidate.ps1"
        wrapper.write_text("""$ErrorActionPreference = 'Stop'
function Invoke-WebRequest($Uri, $OutFile, [switch]$UseBasicParsing, $ErrorAction) {
    if (([uri]$Uri).Segments[-1] -ne [IO.Path]::GetFileName($env:CORVID_TEST_ARCHIVE)) {
        throw "Unexpected archive request: $Uri"
    }
    Copy-Item -LiteralPath $env:CORVID_TEST_ARCHIVE -Destination $OutFile
}
function git { throw 'Candidate archive installation attempted a source fallback' }
& $env:CORVID_TEST_INSTALLER @args
""", encoding="utf-8")
        env["CORVID_TEST_INSTALLER"] = str(scripts / "install.ps1")
        command = ["powershell", "-NoProfile", "-File", wrapper]
        uninstall = "-Uninstall"
    else:
        commands = work / "commands"
        commands.mkdir()
        curl = commands / "curl"
        curl.write_text('''#!/bin/sh
set -eu
while [ "$1" != -o ]; do shift; done
[ "${3##*/}" = "${CORVID_TEST_ARCHIVE##*/}" ]
cp "$CORVID_TEST_ARCHIVE" "$2"
''', encoding="utf-8")
        curl.chmod(0o755)
        git = commands / "git"
        git.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
        git.chmod(0o755)
        env["PATH"] = str(commands) + os.pathsep + env["PATH"]
        command = ["sh", scripts / "install.sh"]
        uninstall = "--uninstall"
    subprocess.run(command, cwd=work, env=env, check=True)
    exercise_installation(root, expected_version, work)
    user_file = root / "user-notes.txt"
    user_file.write_text("keep", encoding="utf-8")
    subprocess.run([*command, uninstall], cwd=work, env=env, check=True)
    if list(root.iterdir()) != [user_file] or user_file.read_text(encoding="utf-8") != "keep":
        raise RuntimeError("Uninstall did not preserve exactly the user-owned file")


def check_editor_diagnostics(lsp, work, env):
    uri = (work / "editor.cor").as_uri()
    valid = "agent main() -> Int:\n    return 42\n"
    document = {"uri": uri, "languageId": "corvid", "version": 1,
                "text": 'agent main() -> Int:\n    return "wrong type"\n'}
    messages = [
        {"id": 1, "method": "initialize", "params": {"capabilities": {}}},
        {"method": "initialized", "params": {}},
        {"method": "textDocument/didOpen", "params": {"textDocument": document}},
        {"method": "textDocument/didChange", "params": {
            "textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": valid}]}},
        {"id": 2, "method": "shutdown", "params": None},
        {"method": "exit"},
    ]
    frames = bytearray()
    for message in messages:
        body = json.dumps({"jsonrpc": "2.0", **message}).encode()
        frames.extend(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    output = subprocess.check_output([lsp], input=frames, cwd=work, env=env, timeout=60)
    stream = BytesIO(output)
    diagnostics = []
    results = {}
    while stream.tell() < len(output):
        headers = parse_headers(stream)
        reply = json.loads(stream.read(int(headers["Content-Length"])))
        if "error" in reply:
            raise RuntimeError(f"Language server request failed: {reply['error']}")
        if "id" in reply:
            results[reply["id"]] = reply["result"]
        if reply.get("method") == "textDocument/publishDiagnostics" and reply["params"]["uri"] == uri:
            diagnostics.append(reply["params"]["diagnostics"])
    if not results[1]["capabilities"].get("hoverProvider") or results[2] is not None:
        raise RuntimeError(f"Unexpected editor initialization or shutdown result: {results}")
    if len(diagnostics) != 2 or not diagnostics[0] or diagnostics[1]:
        raise RuntimeError(f"Expected diagnostics for the error and none after correction: {diagnostics}")


def exercise_installation(root, expected_version, work):
    windows = os.name == "nt"
    cli = root / "bin" / ("corvid.exe" if windows else "corvid")
    lsp = root / "bin" / ("corvid-lsp.exe" if windows else "corvid-lsp")
    runtime = root / "lib" / ("corvid_runtime.lib" if windows else "libcorvid_runtime.a")
    for required in (lsp, runtime, root / "lib/corvid-rust-notices.txt",
                     root / "std/ui.cor", root / "runtime-py/corvid_runtime/__init__.py",
                     root / "extensions/corvid.vsix", root / "LICENSE"):
        if not required.is_file() or not required.stat().st_size:
            raise RuntimeError(f"Missing release component: {required}")

    env = dict(os.environ, CORVID_HOME=str(root))
    env.pop("CORVID_STD_PATH", None)
    env.pop("PYTHONPATH", None)
    check_editor_diagnostics(lsp, work, env)

    def run(*args, cwd=work):
        return subprocess.check_output([cli, *args], cwd=cwd, env=env, text=True).strip()

    run("doctor")
    version = run("--version")
    if version.split()[:2] != ["corvid", expected_version]:
        raise RuntimeError(f"Expected {expected_version}, got {version}")
    run("new", "--with-python-tools", "python-notes")
    python_project = work / "python-notes"
    message = "Hello from Corvid"
    echoed = run("run", "src/python_tools_example.cor", message, cwd=python_project)
    if json.loads(echoed) != message:
        raise RuntimeError(f"The generated Python tool returned {echoed!r}")
    (python_project / "src/numbers_example.cor").write_text(
        "agent twice(value: Int) -> Int:\n    return value * 2\n\n"
        "agent remainder(value: Float) -> Float:\n    return value % 2.0\n",
        encoding="utf-8",
    )
    run("build", "src/numbers_example.cor", "--target", "python", cwd=python_project)
    # Isolated mode ensures a checkout or a user-installed runtime cannot satisfy the import.
    host = """import sys
sys.path[:0] = sys.argv[1:]
from corvid_runtime import run_sync
from numbers_example import twice, remainder
print(run_sync(twice, 21), run_sync(remainder, -7.5))
"""
    values = subprocess.check_output(
        [sys.executable, "-I", "-c", host, str(root / "runtime-py"), str(python_project / "target/py")],
        cwd=python_project, env=env, text=True,
    ).strip()
    if values != "42 -1.5":
        raise RuntimeError(f"Python host received unexpected agent results: {values!r}")
    run("new", "notes")
    project = work / "notes"
    run("check", cwd=project)
    greeting = json.loads(run("run", cwd=project))
    if not greeting.startswith("Hello, Corvid! It is "):
        raise RuntimeError(f"Unexpected first-project output: {greeting!r}")
    traces = list((project / "target/trace").glob("*.jsonl"))
    if len(traces) != 1:
        raise RuntimeError(f"Expected one starter trace, found {len(traces)}")
    replay = run("replay", traces[0], "--source", "src/main.cor", cwd=project)
    result_prefix = "replay completed: agent `main` -> "
    results = [line[len(result_prefix):] for line in replay.splitlines() if line.startswith(result_prefix)]
    if len(results) != 1 or json.loads(results[0]) != greeting:
        raise RuntimeError(f"Starter replay did not reproduce {greeting!r}: {replay}")
    if (project / "src/std/ui.cor").read_bytes() != (root / "std/ui.cor").read_bytes():
        raise RuntimeError("The project scaffold did not copy the archive's UI library")
    (project / "src/main.cor").write_text(
        'import "./std/ui" as ui\n\n'
        'server notes:\n'
        '    route GET "/" -> html:\n'
        '        return ui.page("Notes", [ui.link("Open workspace", "/workspace")])\n'
        '    route GET "/workspace" -> html:\n'
        '        count = ui.int_state("Notes reviewed", 0, [ui.action("Review", "increment")])\n'
        '        return ui.interactive_page("Workspace", [count])\n'
        '    route GET "/api/status" -> json String:\n'
        '        return "ready"\n',
        encoding="utf-8",
    )
    (project / "src/events.cor").write_text(
        "agent increment(current: Int) -> Int:\n    return current + 1\n", encoding="utf-8"
    )
    run("check", "src/main.cor", cwd=project)
    run("build", "src/main.cor", "--target", "web", cwd=project)
    deployed = work / "deployed"
    shutil.unpack_archive(project / "target/web.tar", deployed)
    project.rename(work / "original-notes")
    if not any(deployed.rglob("*.wasm")):
        raise RuntimeError("Web bundle is missing the compiled browser handler")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    address = f"127.0.0.1:{port}"
    server = subprocess.Popen([cli, "serve", "src/main.cor", "--listen", address],
                              cwd=deployed, env=env)
    try:
        deadline = time.monotonic() + 30
        while True:
            try:
                with urlopen(f"http://{address}/", timeout=2) as response:
                    if response.headers.get_content_type() != "text/html":
                        raise RuntimeError("The home route did not serve HTML")
                    page = response.read()
                break
            except URLError:
                if server.poll() is not None or time.monotonic() >= deadline:
                    raise
                time.sleep(0.1)
        if b"Open workspace" not in page:
            raise RuntimeError("The packaged home page is missing its navigation")
        with urlopen(f"http://{address}/workspace", timeout=5) as response:
            workspace = response.read()
            if b"Notes reviewed" not in workspace or b"/target/wasm/events.js" not in workspace:
                raise RuntimeError("The packaged workspace is missing its control")
        with urlopen(f"http://{address}/api/status", timeout=5) as response:
            if response.headers.get_content_type() != "application/json" or json.load(response) != "ready":
                raise RuntimeError("The packaged API did not return its declared JSON response")
        for name, mime in (("events.js", "text/javascript"), ("events.wasm", "application/wasm")):
            with urlopen(f"http://{address}/target/wasm/{name}", timeout=5) as response:
                expected = (deployed / "target/wasm" / name).read_bytes()
                if response.headers.get_content_type() != mime or response.read() != expected:
                    raise RuntimeError(f"Server returned the wrong content for {name}")
    finally:
        server.terminate()
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
    print(f"{version} built and served the relocated web application.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("expected_version")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        verify(args.archive.resolve(), args.expected_version, Path(directory))
