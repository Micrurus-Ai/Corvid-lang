# `std.mcp` — MCP client with governance

Slice 46f. Consume Model Context Protocol tool servers as governed
Corvid tools through ONE executing surface.

## The surface

```corvid-fragment
import "./std/mcp" use mcp_call

found: Result<String, String> = mcp_call("notes", "search", "{\"query\": \"corvid\"}")
```

`mcp_call(server_name, tool_name, args_json) -> Result<String, String>
uses mcp_egress` — calls `tool_name` on the configured server with
a JSON-object argument string and returns the tool's text content.

## Configuration

```toml
[mcp.servers.notes]
command = ["npx", "-y", "@modelcontextprotocol/server-filesystem", "."]

[mcp.servers.search]
url = "https://mcp.example.com/rpc"
trust = "autonomous"
```

Transports: `command` spawns a stdio server (newline-delimited
JSON-RPC; the connection is cached per server and respawned on
failure); `url` makes one JSON-RPC POST per call.

## The governance

- **Untrusted by default.** A server without `trust = "autonomous"`
  routes EVERY call through the runtime approver (interactive
  stdin in `corvid run`) BEFORE any transport I/O. Denial is an
  `Err` value naming the loosening path — a recoverable outcome,
  not a crash.
- **Traced + replay-quarantined.** `mcp_call` is a standard tool
  dispatch: `tool_call`/`tool_result` events record every call,
  and a replayed run substitutes the recorded result — it never
  contacts a server and never prompts.
- **Budget-visible.** The `mcp_egress` effect row rides the
  ordinary composition into `@budget` and `corvid effects`.
- **Honest failures.** Unknown server, transport errors, JSON-RPC
  errors, tool-side `isError` results, and approval denial are all
  `Err` values.

## Decoding structured output

MCP tools return text content; pair with `std/json`'s typed
accessors or the typed-decoder convention to decode structured
payloads.

## Non-scope (v1)

Client only — serving Corvid tools over MCP is post-v1.0. No
compile-time tool introspection (per-tool typed imports); no SSE
server-transport streaming; `tools/list` discovery is a CLI
follow-up.
