"""Refresh the learner-facing index of builtin methods and std module calls."""

import argparse
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "docs/reference/functions.md"


def render():
    lines = [
        "# Corvid function reference",
        "",
        "Look up a method by receiver type or a standard-library call by module. "
        "Each signature gives the argument order and return type. For example, "
        "`List<T>.first() -> Option<T>` means that calling `names.first()` on a "
        "`List<String>` returns an `Option<String>`.",
        "",
        "For worked examples, see [Types and collections](../book/05-types.md), "
        "[Pattern matching](../book/13-pattern-matching.md), and the "
        "[standard library guide](stdlib/README.md).",
    ]
    source = (ROOT / "crates/corvid-types/src/builtin_methods.rs").read_text(encoding="utf-8")
    enum = source.split("pub enum BuiltinMethodKind {", 1)[1].split("\n}", 1)[0]
    groups = {}
    for variant in re.finditer(r"((?:^    ///[^\n]*\n)+)    (\w+),$", enum, re.M):
        signature = re.search(r"`([^`]+ -> [^`]+)`", variant[1])
        if not signature:
            raise ValueError(f"Missing documented signature for {variant[2]}")
        call = signature[1]
        group = call.split(".", 1)[0].split("<", 1)[0] if "." in call else "Built-in functions"
        groups.setdefault(group, []).append(call)
    if sum(map(len, groups.values())) != len(re.findall(r"^    \w+,$", enum, re.M)):
        raise ValueError("Every builtin method needs a documented signature")
    scope = (ROOT / "crates/corvid-resolve/src/scope.rs").read_text(encoding="utf-8")
    for signature in re.findall(r"^    /// Call: `([^`]+)`\.$", scope, re.M):
        groups["Built-in functions"].append(signature)
    for group, calls in groups.items():
        lines.extend(["", f"## {group}", ""])
        lines.extend(f"- `{call}`" for call in calls)
        if group == "Built-in functions":
            lines.extend(["", "Use `None` for an absent option. Pass a type name as the second "
                          "argument to `ask`, for example `ask(\"How many?\", Int)`. "
                          "`Weak::new` accepts strings, records and lists. "
                          "`resume` takes the name of a streaming prompt."])
    lines.extend(["", "## Grounded", "",
                  "- `Grounded<T>.unwrap_discarding_sources() -> T`", "",
                  "Returns the underlying value and discards its source information."])

    lines.extend(["", "## Standard library", "",
                  "Import the module that declares the call. A tool's `uses` clause "
                  "lists its declared effects. Each module heading links to its source."])
    for module in sorted((ROOT / "std").rglob("*.cor")):
        source = module.read_text(encoding="utf-8")
        calls = re.findall(r"^public ((?:agent|tool|prompt|fn)\s+\w+\([^)]*\)[^\n]*)", source, re.M)
        declarations = re.findall(r"^public (?:agent|tool|prompt|fn)\b", source, re.M)
        if len(declarations) != len(calls):
            raise ValueError(f"Unrecognized public callable declaration in {module}")
        if calls:
            path = module.relative_to(ROOT).as_posix()
            lines.extend(["", f"### [{path[:-4]}](../../{path})", ""])
            for call in calls:
                signature = " ".join(call.rstrip(":").split())
                lines.append(f"- `{signature}`")
            if path == "std/ui.cor":
                lines.extend(["", "`assistant_action` opts an integer-state button into WebMCP. "
                              "Use a unique tool name per page. `assistant_text_state` exposes a "
                              "single-line text control. Its tool requires one string field named "
                              "for the handler parameter and returns the handler's string result."])
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    expected = render()
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text(encoding="utf-8") != expected:
            raise SystemExit("Refresh docs/reference/functions.md with this script.")
    else:
        OUTPUT.write_text(expected, encoding="utf-8")
