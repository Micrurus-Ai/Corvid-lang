"""Install the storefront picker into a new app using a disposable file registry."""
import argparse
import hashlib
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from verify_reading_list import read, serve


ROOT = Path(__file__).resolve().parents[1]


def verify(cli, root):
    registry = root / "registry"
    source = ROOT / "examples/storefront/src/color_picker.cor"
    name = "@trial/color-picker"
    subprocess.run([cli, "package", "publish", source, "--name", name, "--version", "1.0.0",
                    "--out", registry, "--url-base", registry.as_uri(),
                    "--key", secrets.token_hex(32), "--key-id", "local-trial"], check=True)
    subprocess.run([cli, "new", "palette"], cwd=root, check=True)
    project = root / "palette"
    subprocess.run([cli, "add", "package", name + "@1.0.0", "--registry", registry / "index.json"],
                   cwd=project, check=True)
    subprocess.run([cli, "package", "verify-lock"], cwd=project, check=True)
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    cached = project / "target/cache/packages" / (digest + ".cor")
    assert cached.read_bytes() == source.read_bytes()
    registry.rename(root / "disconnected-registry")
    shutil.copyfile(ROOT / "examples/palette/src/main.cor", project / "src/main.cor")
    env = dict(os.environ, CORVID_OFFLINE="1")
    for args in (["check"], ["test", "src/main.cor"], ["build", "src/main.cor", "--target", "web"]):
        subprocess.run([cli, *args], cwd=project, env=env, check=True)
    installed = root / "installed"
    with tarfile.open(project / "target/web.tar") as archive:
        archive.extractall(installed, filter="data")
    with serve(cli, installed, env) as url:
        assert "Selected: Ocean" in read(url)
        assert "Selected: Moss" in read(url, b"shade=1")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--work-dir", type=Path, help="retain the trial in a new directory")
    args = parser.parse_args()
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=False)
        verify(args.cli.resolve(), args.work_dir.resolve())
    else:
        with TemporaryDirectory() as directory:
            verify(args.cli.resolve(), Path(directory).resolve())
