# corvid-runtime

The package for writing [Corvid](https://github.com/Micrurus-Ai/Corvid-lang) tools in Python. `corvid serve` and `corvid run` load a project's `tools.py`, and each `@tool` function becomes a tool the program can call, with tracing, approval and replay.

```python
from corvid_runtime import tool

@tool("get_order")
async def get_order(order_id: str):
    ...
```

A tool may be `async` or a plain function. The Corvid program declares the tool, its types and its effects; `tools.py` only supplies the implementation.

## Installation

`corvid serve` and `corvid run` find the copy bundled with a Corvid distribution under `runtime-py`, so there is nothing to install. To import it in your own Python environment, for example to test your tools with pytest, run the following from the extracted distribution with Python 3.11 or later:

```sh
python -m pip install ./runtime-py
```

From a Corvid source checkout, the package directory is `./runtime/python`.

## What it no longer contains

Corvid's Python build target, which generated `.py` modules that called `tool_call`, `approve_gate` and `llm_call` from this package, was retired on 2026-11-25. Those functions went with it. Build natively with `corvid build`, or run with `corvid run`; model calls, approvals and traces are the Corvid runtime's job.
