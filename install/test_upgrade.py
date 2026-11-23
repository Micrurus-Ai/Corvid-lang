"""Offline regression test for the production installer. Run with Python 3."""
import io
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

INSTALL = Path(__file__).resolve().parent
WINDOWS = os.name == "nt" and "--unix-fixture" not in sys.argv


def main(work):
    root = work / "developer's installation with spaces"
    mocks = work / "commands"
    mocks.mkdir()
    archive = work / ("fixture.zip" if WINDOWS else "fixture.tar.gz")
    env = dict(os.environ, CORVID_HOME=str(root), CORVID_VERSION="v1",
               CORVID_SKIP_EDITOR_EXTENSION="1", CORVID_NO_MODIFY_PROFILE="1",
               FIXTURE=str(archive), PATH=str(mocks) + os.pathsep + os.environ["PATH"])
    for key in ("GITHUB_PATH", "GITHUB_ENV", "OFFLINE", "BUILD_LOG", "FAIL_SWITCH"):
        env.pop(key, None)
    if WINDOWS:
        wrapper = work / "run.ps1"
        wrapper.write_text(r"""$ErrorActionPreference = 'Stop'
function Invoke-WebRequest($Uri, $OutFile, [switch]$UseBasicParsing, $ErrorAction) {
    if ($env:OFFLINE) { throw 'offline fixture' }
    Copy-Item -LiteralPath $env:FIXTURE -Destination $OutFile
}
function Move-Item($LiteralPath, $Destination) {
    if ($env:FAIL_SWITCH -and $Destination -eq $env:CORVID_HOME -and
        $LiteralPath.EndsWith('new' + [IO.Path]::DirectorySeparatorChar + 'std')) {
        throw 'simulated package activation failure'
    }
    Microsoft.PowerShell.Management\Move-Item -LiteralPath $LiteralPath -Destination $Destination
}
if (-not $env:TEST_USER_PROFILE) {
    & $env:INSTALLER @args
    return
}
# Redirect this process's HKCU to a temporary key before changing user settings.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class TestHive {
    [DllImport("advapi32.dll")]
    public static extern int RegOverridePredefKey(IntPtr key, IntPtr replacement);
}
'@
$testKey = 'Software\CorvidInstallerTest-' + [guid]::NewGuid().ToString('N')
$hive = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($testKey)
$environmentKey = $hive.CreateSubKey('Environment')
$environmentKey.SetValue('Path', 'C:\existing')
$environmentKey.SetValue('CORVID_HOME', 'C:\previous')
$environmentKey.Dispose()
$hkcu = [IntPtr](-2147483647)
try {
    if ([TestHive]::RegOverridePredefKey($hkcu, $hive.Handle.DangerousGetHandle()) -ne 0) {
        throw 'Cannot isolate test registry'
    }
    if ([Environment]::GetEnvironmentVariable('CORVID_HOME', 'User') -ne 'C:\previous') {
        throw 'Registry isolation check failed'
    }
    & $env:INSTALLER
    $installedPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($installedPath -ne ((Join-Path $env:CORVID_HOME 'bin') + ';C:\existing')) {
        throw 'Installer did not persist the expected path'
    }
    # Simulate a previous installer that appended its own directory.
    $bin = Join-Path $env:CORVID_HOME 'bin'
    [Environment]::SetEnvironmentVariable('Path', "C:\existing;$bin", 'User')
    & $env:INSTALLER
    if ([Environment]::GetEnvironmentVariable('Path', 'User') -ne $installedPath) {
        throw 'Upgrade did not restore precedence for its managed PATH entry'
    }
    [Environment]::SetEnvironmentVariable('Path', "$installedPath;C:\later", 'User')
    & $env:INSTALLER -Uninstall
    if ([Environment]::GetEnvironmentVariable('Path', 'User') -ne 'C:\existing;C:\later' -or
        [Environment]::GetEnvironmentVariable('CORVID_HOME', 'User') -ne 'C:\previous') {
        throw 'Uninstall failed to restore user environment'
    }
} finally {
    [void][TestHive]::RegOverridePredefKey($hkcu, [IntPtr]::Zero)
    $hive.Dispose()
    [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($testKey)
}
""")
        env["INSTALLER"] = str(INSTALL / "install.ps1")
        command = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(wrapper)]
        (mocks / "git.cmd").write_text('@echo %* > "%CLONE_LOG%"\n@if not defined BUILD_LOG exit /b 3\n@mkdir "%~7"\n@exit /b 0\n')
        (mocks / "cargo.cmd").write_text('@echo failed>"%BUILD_LOG%"\n@exit /b 7\n')
        binary = "bin/corvid.exe"
    else:
        shell = shutil.which("sh") if os.name != "nt" else "C:/Program Files/Git/bin/bash.exe"
        command = [shell, str(INSTALL / "install.sh")]
        binary = "bin/corvid"
        scripts = {
            "mv": '''if [ -n "${FAIL_SWITCH:-}" ]; then
    case "$1" in */.corvid-installing/new/std) exit 1;; esac
fi
exec /bin/mv "$@"''',
            "curl": '''test -z "${OFFLINE:-}" || exit 1
while [ "$1" != "-o" ]; do shift; done
cp "$FIXTURE" "$2"''',
            "git": '''echo "$*" > "$CLONE_LOG"
test -n "${BUILD_LOG:-}" || exit 3
mkdir "$7"''',
            "cargo": 'echo failed > "$BUILD_LOG"; exit 7',
        }
        if os.name == "nt":
            setup = work / "environment.sh"
            setup.write_bytes(b'export PATH="$(cygpath -u "$MOCK_COMMANDS"):$PATH"\n')
            env.update(BASH_ENV=setup.as_posix(), MOCK_COMMANDS=str(mocks))
            scripts["uname"] = 'case "$1" in -s) echo Linux;; -m) echo x86_64;; esac'
        for name, body in scripts.items():
            path = mocks / name
            path.write_bytes(("#!/bin/sh\n" + body + "\n").encode())
            path.chmod(0o755)

    def package(version):
        if WINDOWS:
            source = work / "cli.rs"
            source.write_text('fn main() { println!("corvid ' + version + '"); }')
            exe = work / "corvid.exe"
            subprocess.run(["rustc", str(source), "-o", str(exe)], check=True, cwd=work)
            with zipfile.ZipFile(archive, "w") as out:
                out.write(exe, binary)
                out.writestr("std/version", version)
        else:
            data = ('#!/bin/sh\necho "corvid ' + version + '"\n').encode()
            with tarfile.open(archive, "w:gz") as out:
                info = tarfile.TarInfo(binary)
                info.mode, info.size = 0o755, len(data)
                out.addfile(info, io.BytesIO(data))
                info = tarfile.TarInfo("std/version")
                info.size = len(version)
                out.addfile(info, io.BytesIO(version.encode()))

    def run(success=True, extra=()):
        attempt = subprocess.run([*command, *extra], env=env, cwd=work, capture_output=True, text=True)
        assert (attempt.returncode == 0) == success, attempt.stdout + attempt.stderr
        assert not (root / ".corvid-installing").exists()
        return attempt.stdout

    def check_version(expected):
        argv = [str(root / binary), "--version"]
        if os.name == "nt" and not WINDOWS:
            argv.insert(0, shell)
        actual = subprocess.check_output(argv, text=True).strip()
        assert actual == "corvid " + expected, actual
        assert (root / "std/version").read_text() == expected
        assert (root / "settings.txt").read_text() == "keep"

    if not WINDOWS:
        profile_dir = work / "profile"
        profile_dir.mkdir()
        profile = profile_dir / ".profile"
        profile.write_bytes(b"# CORVID_HOME notes\n")
        env.update(HOME=profile_dir.as_posix(), CORVID_NO_MODIFY_PROFILE="0")
    root.mkdir()
    (root / "settings.txt").write_text("keep")
    for index, tag in enumerate(("v1", "v2", "v1")):
        env["CORVID_VERSION"] = tag
        package(tag)
        output = run()
        check_version(tag)
        if WINDOWS and index == 0:
            exports = [line.strip() for line in output.splitlines() if line.startswith("  $env:")]
            assert len(exports) == 2, output
            probe = subprocess.check_output(
                ["powershell", "-NoProfile", "-Command", "\n".join([*exports, "corvid --version"])],
                cwd=work, text=True,
            )
            assert probe.strip() == "corvid v1", probe
        # A package-shaped directory the installer did not supply stays unmanaged.
        if index == 0:
            (root / "lib").mkdir()
            (root / "lib/custom").write_text("keep")
        assert (root / "lib/custom").read_text() == "keep"

    package("v2")
    # A failed activation must restore the original files with their Unix metadata.
    # In particular, copying file contents back can sever a developer's hard link.
    if os.name == "posix":
        original = root / "std/version"
        original.chmod(0o640)
        alias = work / "installed-version"
        os.link(original, alias)
        shortcut = root / "std/current"
        shortcut.symlink_to("version")
    env["FAIL_SWITCH"] = "1"
    run(False)
    check_version("v1")
    del env["FAIL_SWITCH"]
    if os.name == "posix":
        assert original.stat().st_mode & 0o7777 == 0o640
        assert original.samefile(alias)
        assert shortcut.readlink() == Path("version")
        assert shortcut.samefile(original)

    if WINDOWS:
        invalid = b"invalid executable"
        with zipfile.ZipFile(archive, "w") as out:
            out.writestr(binary, invalid)
        attempt = subprocess.run(command, env=env, cwd=work, capture_output=True, text=True)
        assert attempt.returncode != 0
        # PowerShell can wrap the diagnostic to the console width.
        assert hashlib.sha256(invalid).hexdigest() in "".join(attempt.stderr.split()).lower(), attempt.stderr
        check_version("v1")

    archive.write_bytes(b"incomplete download")
    run(False)
    check_version("v1")
    env.update(OFFLINE="1", CLONE_LOG=str(work / "clone.log"))
    run(False)
    assert "--branch v1" in (work / "clone.log").read_text()
    check_version("v1")
    env["BUILD_LOG"] = str(work / "build.log")
    run(False)
    assert (work / "build.log").is_file()
    check_version("v1")
    if not WINDOWS:
        assert profile.read_text().count("export CORVID_HOME=") == 1
        with profile.open("ab") as output:
            output.write(b"export MY_SETTING=1\n")
    external = work / "project"
    external.mkdir()
    (external / "source.cor").write_text("# user source")
    link = root / "std/project"
    if WINDOWS:
        subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(external)], check=True)
    elif os.name != "nt":
        link.symlink_to(external, target_is_directory=True)
    run(extra=["-Uninstall" if WINDOWS else "--uninstall"])
    assert (external / "source.cor").read_text() == "# user source"
    assert not (root / binary).exists()
    assert not (root / "std").exists()
    assert (root / "lib/custom").read_text() == "keep"
    assert (root / "settings.txt").read_text() == "keep"
    assert not (root / ".corvid-package-files").exists()
    if not WINDOWS:
        assert profile.read_text().splitlines() == ["# CORVID_HOME notes", "export MY_SETTING=1"]
        env.pop("OFFLINE")
        root = work / "developer's manual PATH installation"
        env["CORVID_HOME"] = str(root)
        env["CORVID_NO_MODIFY_PROFILE"] = "1"
        package("v1")
        output = run()
        exports = [line.strip() for line in output.splitlines() if line.startswith("  export ")]
        assert len(exports) == 2, output
        probe = subprocess.check_output([shell, "-c", "\n".join([*exports, "corvid --version"])],
                                        cwd=work, text=True)
        assert probe.strip() == "corvid v1", probe
        run(extra=["--uninstall"])
    else:
        package("v1")
        env.pop("OFFLINE")
        env.update(TEST_USER_PROFILE="1", CORVID_NO_MODIFY_PROFILE="0",
                   CORVID_HOME=str(work / "profile install"))
        run()
        assert not Path(env["CORVID_HOME"]).exists()
    print("Install, upgrade, rollback, failed replacement and uninstall passed.")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as temp:
        main(Path(temp))
