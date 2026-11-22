"""Expand the reading-list ask prototype into ordinary Corvid and check for drift."""

import argparse
from pathlib import Path
import re

EXAMPLE = Path(__file__).resolve().parents[1] / "examples/reading_list"
START = "# Generated from suggest_title.cor.in by scripts/prototype_reading_list_ask.py"
END = "# End of generated suggestion agent"


def expand(source):
    # This application binds one prompt and one read-only lookup over a shelf snapshot.
    form = re.fullmatch(
        r"agent suggest_title\(shelf: Shelf\) -> BookSuggestion:\n"
        r"    ([A-Za-z_][A-Za-z_0-9]*) = ask recommend\(shelf\.title\)\n"
        r"        allow lookup_shelf once\n"
        r"        (?:within|timeout) ([1-9][0-9]*) seconds\n+"
        r"    return BookSuggestion\(shelf, \1\)\n?", source)
    if not form:
        raise ValueError("Expected a shelf suggestion with one permitted lookup and a deadline.")
    milliseconds = int(form[2]) * 1000
    if milliseconds > 2**64 - 1:
        raise ValueError("Deadline exceeds Corvid's millisecond range.")
    return START + f"""
type Shelf:
    number: Int
    title: String
    readings: List<Reading>

type BookSuggestion:
    shelf: Shelf
    title: String

type Recommendation:
    suggestion: BookSuggestion
    lookup_shelf: Int
    context: String

agent suggest_title(shelf: Shelf) -> Result<BookSuggestion, String>:
    result = recommend_with_trace(shelf)?
    return Ok(result.suggestion)

agent recommend_with_trace(shelf: Shelf) -> Result<Recommendation, String>:
    return try ask_recommend(shelf) timeout {milliseconds}

agent ask_recommend(shelf: Shelf) -> Result<Recommendation, String>:
    first_choice = recommend(shelf.title, "No lookup yet")
    return match first_choice.action:
        Suggest(title) -> suggestion_result(shelf, title, 0, "")
        Lookup(number) -> finish_recommendation(shelf, number)

agent finish_recommendation(shelf: Shelf, number: Int) -> Result<Recommendation, String>:
    context = lookup(shelf.readings, number)?
    final_choice = recommend(shelf.title, context)
    return match final_choice.action:
        Suggest(title) -> suggestion_result(shelf, title, number, context)
        Lookup(_) -> Err("The model reached its lookup limit.")

fn suggestion_result(shelf: Shelf, title: String, number: Int, context: String) -> Result<Recommendation, String>:
    if title.trim() == "":
        return Err("The suggested title was empty.")
    return Ok(Recommendation(BookSuggestion(shelf, title), number, context))
""" + END


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
