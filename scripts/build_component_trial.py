"""Lower the single-control grammar experiment and package the existing reading list."""
import argparse
import base64
from dataclasses import dataclass
import json
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "examples/component_trial"
BINDER = ROOT / "crates/corvid-codegen-wasm/src/browser_controls.js"
NAME = r"[A-Za-z_][A-Za-z_0-9]*"
# The trial accepts plain quoted labels and one signed-integer state control.
LABEL = r'"[^"\\\r\n]*"'


@dataclass
class Control:
    legend: str
    initial: int
    label: str
    handler: str
    line: int


def parse_view(path):
    source = path.read_text().rstrip("\n")
    state = rf'int_state\((?P<legend>{LABEL}), (?P<initial>-?\d+)'
    button = rf'action\((?P<label>{LABEL}), (?P<handler>{NAME})\)'
    for pattern in (rf"view:\n    {state}\):\n        {button}",
                    rf"return {state}, \[\n    {button}\n\]\)"):
        match = re.fullmatch(pattern, source)
        if match:
            return Control(match["legend"], int(match["initial"]), match["label"],
                           match["handler"], source.count("\n", 0, match.start("handler")) + 1)
    raise ValueError(f"{path}: expected an integer state containing one action")


def compile_handler(view, control, events):
    # Deliberately small common subset. Wasm consumes the original events.cor.
    source = events.read_text().strip()
    signature = re.fullmatch(
        rf"agent ({NAME})\(({NAME}): ({NAME})\) -> ({NAME}):\n    return (.+)", source)
    if not signature:
        raise ValueError(f"{events}: expected one single-expression agent")
    name, param, argument, result, body = signature.groups()
    if (name, argument, result) != (control.handler, "Int", "Int"):
        raise ValueError(f"{view}:{control.line}: action requires {control.handler}(Int) -> Int, "
                         f"found {name}({argument}) -> {result} in {events.name}")
    addition = re.fullmatch(rf"{param} \+ (\d+)", body)
    if not addition or int(addition[1]) > 2**63 - 1:
        raise ValueError(f"{events}:2: trial supports adding a nonnegative Int literal")
    return (f"const intStateHandlers = new Set([{json.dumps(name)}]);\n"
            "const textStateHandlers = new Set();\n"
            f"function instantiate() {{ return {{[{json.dumps(name)}]: value => "
            f"BigInt.asIntN(64, value + {addition[1]}n)}}; }}\n" + BINDER.read_text())


def build(cli, project, view, events, target):
    control = parse_view(view)
    js = compile_handler(view, control, events)
    # 'new' supplies the same std/ui implementation to both packages.
    if project.exists():
        raise ValueError(f"choose a new project directory: {project}")
    subprocess.run([cli, "new", project.name], cwd=project.parent, check=True)
    example = ROOT / "examples/reading_list"
    shutil.copytree(example / "migrations", project / "migrations")
    main = (example / "src/main.cor").read_text(encoding="utf-8")
    anchor = "            content.append(suggestion_controls(shelf))"
    if main.count(anchor) != 1:
        raise ValueError("reading-list edit view changed")
    call = (f"ui.int_state({control.legend}, {control.initial}, "
            f"[ui.action({control.label}, {json.dumps(control.handler)})])")
    main = main.replace(anchor, anchor + f"\n            content.append({call})")
    page = 'return ui.page("Reading list", content)'
    if main.count(page) != 1:
        raise ValueError("reading-list page changed")
    if target == "wasm":
        shutil.copyfile(events, project / "src/events.cor")
        render = 'return ui.interactive_page("Reading list", content)'
    else:
        # Inline module bytes ride in the document. Count them in initial bytes.
        url = "data:text/javascript;base64," + base64.b64encode(js.encode()).decode()
        render = (f'content.append(View("<script type=\'module\' data-corvid-ui src=\'{url}\'></script>"))'
                  + "\n    " + page)
    (project / "src/main.cor").write_text(main.replace(page, render), encoding="utf-8")
    for args in (["check"], ["test", "src/main.cor"], ["build", "src/main.cor", "--target", "web"]):
        subprocess.run([cli, *args], cwd=project, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("project", type=Path)
    parser.add_argument("grammar", choices=["indented", "functions"])
    parser.add_argument("target", choices=["js", "wasm"])
    args = parser.parse_args()
    build(args.cli.resolve(), args.project.resolve(), FIXTURES / (args.grammar + ".view"),
          FIXTURES / "events.cor", args.target)
