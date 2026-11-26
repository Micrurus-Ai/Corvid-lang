"""Run the F16 installed-picker and source-mapped repair trials on one browser channel."""
import argparse
from pathlib import Path
from tempfile import TemporaryDirectory

from verify_browser_diagnostics import verify as verify_diagnostics
from verify_picker_package import verify as verify_picker


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--channel", required=True)
    args = parser.parse_args()
    cli = args.cli.resolve()
    with TemporaryDirectory() as directory:
        verify_picker(cli, Path(directory).resolve(), args.channel)
    verify_diagnostics(cli, args.channel, ("chromium",))
