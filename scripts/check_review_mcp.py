"""Compare an MCP SDK client's scoped tool with the packaged review form."""
import asyncio
import json
from threading import Thread
from urllib.parse import urlsplit

import httpx
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client

from reading_review_mcp import review_server


def exercise(url, form, reviewer, peer, expired, reader, grant=None, receipt=None):
    key = form.fields["proposal_key"]
    with review_server(urlsplit(url).port, key) as server:
        thread = Thread(target=server.serve_forever)
        thread.start()
        try:
            asyncio.run(check(f"http://127.0.0.1:{server.server_port}/mcp", form, reviewer,
                              peer, expired, reader, grant, receipt))
        finally:
            server.shutdown()
            thread.join()


async def check(endpoint, form, reviewer, peer, expired, reader, grant, receipt):
    args = {k: v for k, v in form.fields.items() if k != "proposal_key"}
    async with httpx.AsyncClient(headers=reviewer, trust_env=False) as client:
        async with streamable_http_client(endpoint, http_client=client) as (receive, send, _):
            async with ClientSession(receive, send) as session:
                init = await session.initialize()
                assert init.protocolVersion == "2025-06-18"
                tool, = (await session.list_tools()).tools
                assert tool.name == form.tool
                assert set(tool.inputSchema["required"]) == set(args)
                assert tool.inputSchema["properties"] == {k: {"type": "string"} for k in args}
                async with httpx.AsyncClient(trust_env=False) as probe:
                    msg = {"jsonrpc": "2.0", "id": 1, "method": "tools/list"}
                    for headers, status in ((expired, 401), (reader, 403), ({}, 401)):
                        assert (await probe.post(endpoint, headers=headers, json=msg)).status_code == status
                    response = await probe.post(endpoint, headers=peer, json=msg)
                    assert response.json()["result"]["tools"] == []
                    msg.update(method="tools/call", params={"name": tool.name, "arguments": args})
                    assert "error" in (await probe.post(endpoint, headers=peer, json=msg)).json()
                    if grant:
                        grant(False)
                        try:
                            assert (await probe.post(endpoint, headers=reviewer, json=msg)).status_code == 403
                        finally:
                            grant(True)
                    for name, inputs in (("save", args), (tool.name, dict(args, proposal_key="other"))):
                        msg["params"] = {"name": name, "arguments": inputs}
                        assert "error" in (await probe.post(endpoint, headers=reviewer, json=msg)).json()
                applied = await session.call_tool(tool.name, args)
                assert not applied.isError, applied
                result = json.loads(applied.content[0].text)
                assert result["variant"] == "Reviewed"
                if receipt is not None:
                    assert result == receipt
                    changed = await session.call_tool(tool.name, dict(args, third="Different"))
                    assert changed.isError and json.loads(changed.content[0].text)["variant"] == "KeyConflict"
