"""Package a prototype named-state component with the ordinary Corvid web toolchain."""

import argparse
import json
from pathlib import Path
import re
import subprocess

NAME = r"[A-Za-z_][A-Za-z_0-9]*"
LABEL = r'"(?:[^"\\]|\\["\\])*"'


def lower(path):
    lines = iter((n, line) for n, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), 1) if line.strip())
    current = next(lines, None)

    def take(pattern, expectation):
        nonlocal current
        if current is None:
            raise ValueError(f"{path}: end of component: expected {expectation}")
        number, line = current
        match = re.fullmatch(pattern, line)
        if not match:
            raise ValueError(f"{path}:{number}: expected {expectation}")
        current = next(lines, None)
        return match.groups()

    (name,) = take(rf"component ({NAME}):", "component Name:")
    state, initial = take(rf"    state ({NAME}): Int = (0|[1-9][0-9]*)", "named Int state")
    if int(initial) > 2**63 - 1:
        raise ValueError(f"{path}: initial value exceeds Int range")
    (display,) = take(rf"    text\(({LABEL})\)", "text ending with the state placeholder")
    label, placeholder, tail = json.loads(display).rpartition("{" + state + "}")
    if not placeholder or tail or "{" in label or "}" in label:
        raise ValueError(f"{path}: text must end in one reference to {{{state}}}")

    actions = []
    handlers = []
    while current:
        caption, inline = take(rf"    button\(({LABEL})\)( on click)?:", "button with a click handler")
        indent = "        "
        if not inline:
            take("        on click:", "on click:")
            indent += "    "
        (step,) = take(rf"{indent}{state} = {state} \+ (0|[1-9][0-9]*)",
                       f"{state} = {state} + a nonnegative Int literal")
        if int(step) > 2**63 - 1:
            raise ValueError(f"{path}: increment exceeds Int range")
        handler = f"update_{state}_{len(actions) + 1}"
        actions.append(f'ui.action({caption}, "{handler}")')
        handlers.append(f"agent {handler}({state}: Int) -> Int:\n    return {state} + {step}\n")
    if not actions:
        raise ValueError(f"{path}: expected at least one button")
    view = f"""import "./std/ui" as ui
import "./std/ui" use View

public fn render_component() -> View:
    return ui.int_state({json.dumps(label.rstrip(), ensure_ascii=False)}, {initial}, [{", ".join(actions)}])
"""
    return name, view, "\n".join(handlers)


def build(path, project, cli):
    name, view, events = lower(path)
    if project.exists():
        raise ValueError(f"Choose a new directory: {project}")
    subprocess.run([cli, "new", project.name], cwd=project.parent, check=True)
    src = project / "src"
    (src / "component.cor").write_text(view, encoding="utf-8")
    (src / "events.cor").write_text(events, encoding="utf-8")
    # Mount twice to exercise independent component instances.
    main = f"""import "./component" use render_component
import "./std/ui" as ui

server app:
    route GET "/" -> html:
        return ui.interactive_page("{name}", [render_component(), render_component()])
"""
    (src / "main.cor").write_text(main, encoding="utf-8")
    subprocess.run([cli, "check"], cwd=project, check=True)
    subprocess.run([cli, "build", "src/main.cor", "--target", "web"], cwd=project, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("view", type=Path)
    parser.add_argument("project", type=Path)
    parser.add_argument("--cli", type=Path, required=True)
    args = parser.parse_args()
    build(args.view, args.project.resolve(), args.cli.resolve())
