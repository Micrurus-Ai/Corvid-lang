"""Export a Corvid view and its compiled event handlers as a browser module."""

import argparse
import json
from pathlib import Path
import subprocess


def build(cli, project):
    rendered = subprocess.run(
        [cli, "run", "src/component.cor", "--target", "interp"], cwd=project,
        check=True, stdout=subprocess.PIPE, text=True, encoding="utf-8",
    )
    html = json.loads(rendered.stdout)
    if not isinstance(html, str):
        raise ValueError("The component entry agent must return a String containing the view HTML.")
    subprocess.run([cli, "build", "src/events.cor", "--target", "wasm"], cwd=project, check=True)
    output = project / "target/wasm"
    (output / "index.js").write_text(
        'import { componentElement } from "./events.js";\n'
        f'export default componentElement({json.dumps(html)});\n', encoding="utf-8",
    )
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", type=Path)
    parser.add_argument("--cli", type=Path, required=True)
    args = parser.parse_args()
    print(build(args.cli.resolve(), args.project.resolve()))
