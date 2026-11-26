"""Tests for the tool registry the Rust runtime reads."""

from __future__ import annotations

import pytest

import corvid_runtime
from corvid_runtime import registry, tool


@pytest.fixture(autouse=True)
def _clean():
    registry._TOOL_IMPLS.clear()
    yield
    registry._TOOL_IMPLS.clear()


def test_the_package_offers_only_the_decorator():
    assert corvid_runtime.__all__ == ["tool"]


def test_tool_registers_the_function_under_its_name_and_returns_it():
    @tool("echo")
    async def echo(x):
        return x

    assert registry._TOOL_IMPLS == {"echo": echo}
    assert echo.__name__ == "echo", "the decorator leaves the function as written"


def test_a_later_registration_of_a_name_replaces_the_earlier():
    @tool("greet")
    def first():
        return "first"

    @tool("greet")
    def second():
        return "second"

    assert registry._TOOL_IMPLS["greet"] is second


async def test_invoke_tool_awaits_an_async_implementation():
    @tool("echo")
    async def echo(x):
        return x

    assert await registry.invoke_tool(echo, ["hello"]) == "hello"


async def test_invoke_tool_calls_a_sync_implementation():
    @tool("double")
    def double(value):
        return value * 2

    assert await registry.invoke_tool(double, [3]) == 6


async def test_invoke_tool_passes_an_implementation_error_on():
    @tool("broken")
    def broken():
        raise KeyError("no order")

    with pytest.raises(KeyError):
        await registry.invoke_tool(broken, [])
