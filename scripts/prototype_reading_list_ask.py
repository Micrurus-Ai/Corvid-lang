"""Expand the reading-list ask prototype into ordinary Corvid and check for drift."""

import argparse
from pathlib import Path
import re

EXAMPLE = Path(__file__).resolve().parents[1] / "examples/reading_list"
START = "# Generated from suggest_title.cor.in by scripts/prototype_reading_list_ask.py"
END = "# End of generated suggestion agent"


def expand(source):
    # The prototype adapter binds lookup_shelf to lookup(readings, shelf).
    form = re.compile(
        r"^    title = ask recommend\(current_title\)\n"
        r"        allow lookup_shelf once\n"
        r"        timeout ([1-9][0-9]*) seconds$", re.MULTILINE
    )
    matches = list(form.finditer(source))
    if len(matches) != 1:
        raise ValueError("Expected one reading-list ask with one lookup and a deadline.")
    match = matches[0]
    milliseconds = int(match[1]) * 1000
    if milliseconds > 2**64 - 1:
        raise ValueError("Deadline exceeds Corvid's millisecond range.")
    invocation = f"    title = try ask_recommend(current_title, readings) timeout {milliseconds}"
    agent = source[:match.start()] + invocation + source[match.end():]
    helpers = """
agent ask_recommend(current_title: String, readings: List<Reading>) -> Result<String, String>:
    decision = recommend(current_title, "No lookup yet")
    return match decision.action:
        Suggest(title) -> Ok(title)
        Lookup(shelf) -> finish_recommendation(current_title, lookup(readings, shelf)?)

agent finish_recommendation(current_title: String, shelf_title: String) -> Result<String, String>:
    decision = recommend(current_title, shelf_title)
    return match decision.action:
        Suggest(title) -> Ok(title)
        Lookup(_) -> Err("The model reached its lookup limit.")
"""
    return START + "\n" + agent.rstrip() + "\n" + helpers + END


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="update the generated agent in main.cor")
    args = parser.parse_args()
    main_path = EXAMPLE / "src/main.cor"
    source = main_path.read_text(encoding="utf-8")
    before, region = source.split(START)
    _, after = region.split(END)
    expected = before + expand((EXAMPLE / "suggest_title.cor.in").read_text(encoding="utf-8")) + after
    if source != expected:
        if not args.write:
            raise SystemExit("Generated suggestion agent differs. Run with --write.")
        main_path.write_text(expected, encoding="utf-8")


if __name__ == "__main__":
    main()
