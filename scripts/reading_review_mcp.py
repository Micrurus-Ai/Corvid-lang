"""Optional loopback MCP endpoint for one reading-list proposal."""
import argparse
from html.parser import HTMLParser
from http.client import HTTPConnection, HTTPException
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from urllib.parse import urlencode


class ReviewTool(HTMLParser):
    def __init__(self, html, key):
        super().__init__()
        self.active = False
        self.tool = None
        self.key = key
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "form":
            self.active = (attrs.get("action") == "/proposals/apply" and
                           attrs.get("method", "").lower() == "post" and bool(attrs.get("toolname")))
            if self.active:
                self.name = attrs["toolname"]
                self.description = attrs["tooldescription"]
                self.fields = {}
                self.bound = False
        if tag == "input" and self.active:
            if attrs.get("type") == "hidden" and attrs.get("name") == "proposal_key":
                self.bound = attrs.get("value") == self.key
            elif attrs.get("type", "text") == "text":
                self.fields[attrs["name"]] = {"type": "string"}

    def handle_endtag(self, tag):
        if tag == "form" and self.active:
            if self.bound:
                self.tool = {"name": self.name, "description": self.description,
                             "inputSchema": {"type": "object", "properties": self.fields,
                                             "required": list(self.fields), "additionalProperties": False}}
            self.active = False


class ReviewEndpoint(BaseHTTPRequestHandler):
    def upstream(self, path, fields):
        headers = {"Content-Type": "application/x-www-form-urlencoded"}
        for name in ("Cookie", "X-CSRF-Token"):
            if name in self.headers:
                headers[name] = self.headers[name]
        app = HTTPConnection("127.0.0.1", self.server.app_port, timeout=10)
        try:
            app.request("POST", path, urlencode(fields), headers)
            response = app.getresponse()
            return response.status, response.read()
        finally:
            app.close()

    def reply(self, status, data=None):
        body = b"" if data is None else json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Cache-Control", "no-store")
        if data is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.reply(405)

    def do_POST(self):
        if self.path != "/mcp":
            return self.reply(404)
        host = f"127.0.0.1:{self.server.server_port}"
        if self.headers.get("Host") != host or self.headers.get("Origin", "http://" + host) != "http://" + host:
            return self.reply(403)
        if self.headers.get_content_type() != "application/json":
            return self.reply(415)
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 1024 * 1024:
                return self.reply(413)
            message = json.loads(self.rfile.read(length))
        except (ValueError, UnicodeError, RecursionError):
            return self.reply(400)
        if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
            return self.reply(400)
        method = message.get("method")
        if method == "notifications/initialized" and "id" not in message:
            return self.reply(202)
        if type(message.get("id")) not in (int, str):
            return self.reply(400)
        envelope = {"id": message["id"], "jsonrpc": "2.0"}
        try:
            result = self.dispatch(method, message.get("params", {}))
        except (HTTPException, OSError):
            return self.reply(502)
        if isinstance(result, int):
            return self.reply(result)
        self.reply(200, {**envelope, **result})

    def dispatch(self, method, params):
        if method == "initialize":
            return {"result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                               "serverInfo": {"name": "reading-list-review", "version": "1"}}}
        if method == "ping":
            return {"result": {}}
        if method not in ("tools/list", "tools/call"):
            return {"error": {"code": -32601, "message": "Unknown method"}}
        context = {"proposal_key": self.server.proposal_key}
        status, html = self.upstream("/proposals/context", context)
        if status != 200:
            return status
        tool = ReviewTool(html.decode(), self.server.proposal_key).tool
        if method == "tools/list":
            return {"result": {"tools": [tool] if tool else []}}
        if not tool or not isinstance(params, dict) or params.get("name") != tool["name"]:
            return {"error": {"code": -32602, "message": "Unknown tool"}}
        args = params.get("arguments")
        if (not isinstance(args, dict) or set(args) != set(tool["inputSchema"]["required"])
                or not all(isinstance(value, str) for value in args.values())):
            return {"error": {"code": -32602, "message": "Expected the reviewed title fields"}}
        status, receipt = self.upstream("/proposals/apply-result", {**args, **context})
        if status != 200:
            return status
        return {"result": {"content": [{"type": "text", "text": receipt.decode()}],
                           "isError": json.loads(receipt)["variant"] != "Reviewed"}}


def review_server(app_port, proposal_key, port=0):
    server = ThreadingHTTPServer(("127.0.0.1", port), ReviewEndpoint)
    server.app_port = app_port
    server.proposal_key = proposal_key
    return server


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app-port", required=True, type=int)
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()
    with review_server(args.app_port, input("Reviewed proposal key: ").strip(), args.port) as server:
        print(f"http://127.0.0.1:{server.server_port}/mcp", flush=True)
        server.serve_forever()
