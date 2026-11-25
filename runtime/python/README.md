# corvid-runtime

The package for writing [Corvid](https://github.com/Micrurus-Ai/Corvid-lang) tools in Python. `corvid serve` and `corvid run` load a project's `tools.py`, and each `@tool` function becomes a tool the program can call, with tracing, approval and replay.

Corvid's Python build target, which generated `.py` modules that imported this package, was retired on 2026-11-25; build natively instead. To run Corvid standard-library tools such as HTTP, SQLite, or MCP, use the interpreter tier with `corvid run`. See the [build guide](https://github.com/Micrurus-Ai/Corvid-lang/blob/e42da8eae303010ca7b8f9cb15945e6a3a451a1e/docs/book/16-building.md).

## Installation

Corvid distributions include this package under `runtime-py`. To use generated agents in an existing Python environment, run the following from the extracted Corvid distribution directory with Python 3.11 or later.

```sh
python -m pip install ./runtime-py
```

Use `python -m pip install "./runtime-py[anthropic]"` to include the optional Anthropic adapter. From a Corvid source checkout, the package directory is `./runtime/python`.

## Usage

You generally don't import this package directly — the Corvid compiler emits code that imports it. What you *do* write is tool implementations:

```python
from corvid_runtime import tool

@tool("get_order")
async def get_order(order_id: str):
    ...
```

## Configuration

No default model. Set one via:

- Environment: `CORVID_MODEL=claude-opus-4-6`
- Config file: `corvid.toml` with `[llm] default_model = "..."`
- Per-call: `llm_call(..., model="...")`

Approvals are interactive by default. For CI, set `CORVID_APPROVE_ALL=1`.
