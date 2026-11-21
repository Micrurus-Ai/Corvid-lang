"""Write a CycloneDX inventory of a packaged Corvid distribution (Python 3.11+)."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile
import tomllib
import zipfile
from urllib.parse import quote


def members(path):
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            for entry in archive.infolist():
                if not entry.is_dir():
                    with archive.open(entry) as content:
                        yield entry.filename, content
    else:
        with tarfile.open(path, "r|gz") as archive:
            for entry in archive:
                if entry.isfile():
                    with archive.extractfile(entry) as content:
                        yield entry.name, content
                elif not entry.isdir():
                    raise ValueError(f"Unexpected archive entry: {entry.name}")


def component(name, version, ecosystem):
    result = {"type": "library", "name": name, "version": version, "group": ecosystem}
    if ecosystem in ("cargo", "npm", "pypi"):
        result["purl"] = f"pkg:{ecosystem}/{quote(name)}@{quote(version, safe='')}"
    return result


def editor_components(data):
    result = []
    with zipfile.ZipFile(io.BytesIO(data)) as vsix:
        for name in vsix.namelist():
            extension = name == "extension/package.json"
            if extension or re.search(r"/node_modules/(?:@[^/]+/)?[^/]+/package\.json$", name):
                package = json.loads(vsix.read(name))
                result.append(component(package["name"], package["version"],
                                        "vscode" if extension else "npm"))
    if not any(c["group"] == "vscode" for c in result):
        raise ValueError("Missing editor extension manifest")
    return result


def hashes(digest):
    return [{"alg": "SHA-256", "content": digest}]


def inventory(path):
    root = path.name.removesuffix(".tar.gz").removesuffix(".zip")
    notices = "lib/corvid-rust-notices.txt"
    runtime = "runtime-py/pyproject.toml"
    editor = "extensions/corvid.vsix"
    files = {}
    packages = []
    for name, content in members(path):
        prefix, slash, relative = name.partition("/")
        if prefix != root or not slash or relative in files:
            raise ValueError(f"Unexpected or duplicate archive member: {name}")
        if relative in (notices, runtime, editor):
            data = content.read()
            digest = hashlib.sha256(data).hexdigest()
            if relative == notices:
                rows = re.findall(rb"^=== ([\w-]+) (\S+) ===\r?\nDeclared license:", data, re.M)
                if not rows:
                    raise ValueError("Empty Rust dependency inventory")
                packages.extend(component(n.decode(), v.decode(), "cargo") for n, v in rows)
            elif relative == runtime:
                project = tomllib.loads(data.decode())["project"]
                packages.append(component(project["name"], project["version"], "pypi"))
            else:
                packages.extend(editor_components(data))
        else:
            digest = hashlib.file_digest(content, "sha256").hexdigest()
        files[relative] = {"type": "file", "name": relative, "hashes": hashes(digest)}
    if not {notices, runtime, editor}.issubset(files):
        raise ValueError("Archive is missing component metadata")
    unique = {(c["group"], c["name"], c["version"]): c for c in packages}
    with path.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    return {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1,
            "metadata": {"component": {"type": "application", "name": root,
                                       "hashes": hashes(digest)}},
            "components": [unique[k] for k in sorted(unique)] + [files[k] for k in sorted(files)]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.write_bytes((json.dumps(inventory(args.archive), indent=2) + "\n").encode())
