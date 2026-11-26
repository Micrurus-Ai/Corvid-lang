"""The tool registry `corvid serve` and `corvid run` read.

A project's `tools.py` decorates its functions with `@tool("name")`.
Loading the file fills `_TOOL_IMPLS`; the Rust runtime reads that dict
and registers each function as the tool of that name, and calls it
through `invoke_tool`, so tracing, approval and replay apply to it like
to any other tool.
"""

from __future__ import annotations

from inspect import isawaitable
from typing import Any, Callable, Iterable, TypeVar

ToolFunction = TypeVar("ToolFunction", bound=Callable[..., Any])

# Read by `corvid_runtime::python_tools` in the Rust runtime.
_TOOL_IMPLS: dict[str, Callable[..., Any]] = {}


def tool(name: str) -> Callable[[ToolFunction], ToolFunction]:
    """Decorator: register `fn` as the implementation of tool `name`."""

    def wrap(fn: ToolFunction) -> ToolFunction:
        _TOOL_IMPLS[name] = fn
        return fn

    return wrap


async def invoke_tool(impl: Callable[..., Any], args: Iterable[Any]) -> Any:
    """Call a tool with its arguments, awaiting it if it is async."""
    result = impl(*args)
    if isawaitable(result):
        return await result
    return result
