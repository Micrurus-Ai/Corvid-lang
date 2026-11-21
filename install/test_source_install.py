"""Test the installer's stable source fallback using local Git and Cargo."""

import json
import os
from pathlib import Path
import subprocess
import tempfile


def verify(work):
    repo = work / "repository"
    repo.mkdir()

    def git(*args):
        subprocess.run(["git", *args], check=True, cwd=repo, stdout=subprocess.DEVNULL)

    git("init", "-q")
    (repo / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n')
    for package in ("corvid-cli", "corvid-lsp", "corvid-runtime"):
        crate = repo / "crates" / package
        (crate / "src").mkdir(parents=True)
        manifest = f'[package]\nname="{package}"\nversion="1.0.0"\nedition="2021"\n'
        if package == "corvid-runtime":
            manifest += '[lib]\ncrate-type=["staticlib"]\n'
            (crate / "src/lib.rs").write_text("pub fn value() -> i32 { 1 }\n")
        else:
            if package == "corvid-cli":
                manifest += '[[bin]]\nname="corvid"\npath="src/main.rs"\n'
            (crate / "src/main.rs").write_text('fn main() { println!("release one"); }\n')
        (crate / "Cargo.toml").write_text(manifest)
    for directory in ("std", "runtime/python"):
        (repo / directory).mkdir(parents=True)
        (repo / directory / "version").write_text("release one")
    (repo / "LICENSE").write_text("fixture notice")
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=repo, check=True)
    git("add", ".")
    git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
        "commit", "-qm", "release")
    git("tag", "v1.0.0")
    (repo / "std/version").write_text("development")
    git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
        "commit", "-qam", "development after release")

    root = work / "corvid install"
    env = dict(os.environ, CORVID_HOME=str(root), CORVID_NO_MODIFY_PROFILE="1",
               CORVID_SKIP_EDITOR_EXTENSION="1", CORVID_VERSION="latest",
               CORVID_REPO="fixture/corvid", CARGO_NET_OFFLINE="true",
               CARGO_TARGET_DIR=str(work / "build cache"), GIT_CONFIG_COUNT="1",
               GIT_CONFIG_KEY_0=f"url.{repo.as_uri()}.insteadOf",
               GIT_CONFIG_VALUE_0="https://github.com/fixture/corvid.git")
    for key in ("GITHUB_PATH", "GITHUB_ENV"):
        env.pop(key, None)
    scripts = Path(__file__).resolve().parent
    release = work / "release.json"
    release.write_text(json.dumps({"tag_name": "v1.0.0"}))
    env["RELEASE_FIXTURE"] = str(release)
    if os.name == "nt":
        wrapper = work / "fallback.ps1"
        wrapper.write_text("""
function Invoke-WebRequest { throw 'release archive unavailable' }
function Invoke-RestMethod { Get-Content $env:RELEASE_FIXTURE -Raw | ConvertFrom-Json }
& $env:INSTALL_SCRIPT
""")
        env["INSTALL_SCRIPT"] = str(scripts / "install.ps1")
        command = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", wrapper]
        suffix = ".exe"
        library = "corvid_runtime.lib"
    else:
        mock = work / "mock"
        mock.mkdir()
        curl = mock / "curl"
        curl.write_text("""#!/bin/sh
while [ "$1" != "-o" ]; do shift; done
case "$3" in */releases/latest) cp "$RELEASE_FIXTURE" "$2";; *) exit 1;; esac
""")
        curl.chmod(0o755)
        env["PATH"] = str(mock) + os.pathsep + env["PATH"]
        command = ["sh", scripts / "install.sh"]
        suffix = ""
        library = "libcorvid_runtime.a"
    subprocess.run(command, cwd=work, env=env, check=True)
    for tool in ("corvid", "corvid-lsp"):
        output = subprocess.check_output([root / "bin" / (tool + suffix)], text=True)
        assert output.strip() == "release one", output
    assert (root / "lib" / library).stat().st_size > 0
    assert (root / "runtime-py/version").read_text() == "release one"
    assert (root / "std/version").read_text() == "release one"
    assert (root / "LICENSE").read_text() == "fixture notice"
    tag = subprocess.check_output(["git", "describe", "--tags", "--exact-match"],
                                  cwd=root / "src", text=True).strip()
    assert tag == "v1.0.0", tag
    print("Stable source fallback installed both tools and the runtime from the release tag.")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as directory:
        verify(Path(directory))
