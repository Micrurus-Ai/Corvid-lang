"""Collect upstream notices for the Rust components in a Corvid distribution."""

import argparse
import json
from pathlib import Path
import subprocess


PRODUCTS = {"corvid-cli", "corvid-lsp", "corvid-runtime"}
NOTICE_PREFIXES = ("license", "licence", "copying", "copyright", "notice")
# Some crates omit the license files from their published Cargo package.
# These copies come from the repository revision recorded by Cargo packaging.
UPSTREAM_NOTICES = {
    "7b1abfd750a2cacaea71d6a56ecfb83cb7de560b": ("madsmtm/objc2", ["LICENSE.md"]),
    "3b27cf2bfd1adcfa6e0264eb51c1025ddaf0f342": ("DoumanAsh/clipboard-win", ["LICENSE"]),
    "58282df898d79a787a726d829b166272dde155b9": ("bytecodealliance/wasmtime", ["LICENSE"]),
    "d4e317f22c3bace76cb3205003bcc34b4929037d": (
        "bytecodealliance/wasm-tools",
        ["LICENSE-APACHE", "LICENSE-Apache-2.0_WITH_LLVM-exception", "LICENSE-MIT"],
    ),
}


def upstream_notices(commit, repository):
    if commit not in UPSTREAM_NOTICES:
        return []
    repo, names = UPSTREAM_NOTICES[commit]
    url = f"https://github.com/{repo}"
    if repository != url and not (repository or "").startswith(url + "/"):
        return []
    local = Path(__file__).with_name("upstream-notices") / repo.split("/")[-1]
    return [(f"{url}/blob/{commit}/{name}", local / name) for name in names]


def dependencies(metadata):
    packages = {p["id"]: p for p in metadata["packages"]}
    members = set(metadata["workspace_members"])
    pending = [i for i in members if packages[i]["name"] in PRODUCTS]
    if {packages[i]["name"] for i in pending} != PRODUCTS:
        raise ValueError("Missing Corvid distribution component in Cargo workspace")
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    visited = set()
    while pending:
        current = pending.pop()
        if current in visited:
            continue
        visited.add(current)
        for dep in nodes[current]["deps"]:
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                pending.append(dep["pkg"])
    return sorted((packages[i] for i in visited - members), key=lambda p: p["id"])


def collect(target, output):
    data = subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target,
    ])
    packages = dependencies(json.loads(data))
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("wb") as out:
        def line(value):
            out.write((value + "\n").encode("utf-8"))

        line(f"Corvid Rust dependency notices for {target}")
        line("Includes normal and build dependencies in Cargo's resolved workspace graph.")
        for pkg in packages:
            root = Path(pkg["manifest_path"]).parent
            files = {p for p in root.rglob("*")
                     if p.name.lower().startswith(NOTICE_PREFIXES) and p.is_file()}
            if pkg["license_file"]:
                files.add(root / pkg["license_file"])
            line(f"\n=== {pkg['name']} {pkg['version']} ===")
            line(f"Declared license: {pkg['license'] or 'See upstream license file'}")
            line(f"Source: {pkg['repository'] or pkg['id']}")
            notices = [(path.relative_to(root).as_posix(), path) for path in sorted(files)]
            commit = None
            if not notices:
                info = root / ".cargo_vcs_info.json"
                vcs = json.loads(info.read_bytes()) if info.is_file() else {}
                commit = vcs.get("git", {}).get("sha1")
                if commit:
                    line(f"Packaged source revision: {commit}")
                notices = upstream_notices(commit, pkg["repository"])
            if not notices:
                line("No separate notice file found in the Cargo package.")
                if pkg["authors"]:
                    line(f"Package authors (Cargo metadata): {', '.join(pkg['authors'])}")
                if pkg["license"] == "MIT":
                    notices = [(
                        "Standard MIT template (SPDX license-list-data v3.25.0)",
                        Path(__file__).with_name("upstream-notices") / "spdx/MIT.txt",
                    )]
            if commit == "8ea76f95eaed2de45c9337830f3fdd5c0a862292":
                local = Path(__file__).with_name("upstream-notices")
                pdfbox = "https://github.com/apache/pdfbox/blob/636c4ec790749eb8bc18496ba1be602b3e8a95a3"
                adobe = "https://github.com/adobe-type-tools/agl-aglfn/blob/4036a9ca80a62f64f9de4f7321a9a045ad0ecfd6"
                fonts = "https://github.com/matplotlib/matplotlib/tree/be56634d682bed257cb941369d8d3600635ddadf/lib/matplotlib/mpl-data/fonts/pdfcorefonts"
                line("Glyph mappings source: https://github.com/michal-h21/htfgen/blob/0d9e796ebe9d6f8978f7c069d4922278e8716a75/glyphlist-extended.txt")
                line("Glyph mappings source: https://github.com/kohler/lcdf-typetools/blob/46fa0a9f2fe86fbd0eb11ac1fc83a1ad43a20615/texglyphlist.txt")
                line(f"Glyph mappings source: {pdfbox}/pdfbox/src/main/resources/org/apache/pdfbox/resources/glyphlist/additional.txt")
                line("Core font metrics are compiled into Rust tables of character codes, widths and glyph names.")
                notices.extend([
                    (f"Shared notice from {adobe}/glyphlist.txt and zapfdingbats.txt", local / "agl-aglfn/glyph-list-header.txt"),
                    (f"{fonts}/readme.txt", local / "pdfcorefonts/readme.txt"),
                    (f"Copyright and trademark lines from the 14 AFM files at {fonts}", local / "pdfcorefonts/font-notices.txt"),
                ])
                for name in ("NOTICE.txt", "LICENSE.txt"):
                    notices.append((f"{pdfbox}/{name}", local / "pdfbox" / name))
            for source, path in notices:
                line(f"\n--- {source} ---")
                out.write(path.read_bytes())
                out.write(b"\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    collect(args.target, args.output)
