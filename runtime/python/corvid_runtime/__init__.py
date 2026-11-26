"""Write Corvid tools in Python.

A project's `tools.py` imports `tool` and decorates each implementation:

    from corvid_runtime import tool

    @tool("get_order")
    async def get_order(order_id: str):
        ...

`corvid serve` and `corvid run` load the file and call the functions as
the program's tools, with tracing, approval and replay.
"""

from __future__ import annotations

from .registry import tool

__all__ = ["tool"]
