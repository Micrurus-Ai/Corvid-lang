"""Exercise the storefront's shared component through the editor's stdio protocol."""
import io
import json
import subprocess


def frame(message):
    body = json.dumps({"jsonrpc": "2.0", **message}).encode()
    return f"Content-Length: {len(body)}\r\n\r\n".encode() + body


def responses(output):
    stream = io.BytesIO(output)
    while header := stream.readline():
        headers = {}
        while header.strip():
            name, value = header.split(b":", 1)
            headers[name.lower()] = value.strip()
            header = stream.readline()
        yield json.loads(stream.read(int(headers[b"content-length"])))


def check_editor(lsp, project):
    sources = {}
    messages = [{"id": 1, "method": "initialize", "params": {
        "rootUri": project.resolve().as_uri(), "capabilities": {}}},
        {"method": "initialized", "params": {}}]
    for name in ("notebook", "pencil", "color_picker"):
        path = (project / "src" / f"{name}.cor").resolve()
        uri, source = path.as_uri(), path.read_text(encoding="utf-8")
        sources[name] = uri, source
        messages.append({"method": "textDocument/didOpen", "params": {"textDocument": {
            "uri": uri, "text": source, "languageId": "corvid", "version": 1}}})

    picker_uri, original = sources["color_picker"]
    declaration = next(i for i, line in enumerate(original.splitlines()) if "fn ColorPicker(" in line)
    # Keep the component body well typed while breaking both calling screens.
    edited = original.replace("change_path: String", "change_path: Int").replace(
        "ui.post_form(change_path,", "ui.post_form(change_path.to_string(),")
    caller_lines = {}
    for request_id, name in enumerate(("notebook", "pencil"), start=2):
        uri, source = sources[name]
        line, text = next((i, s) for i, s in enumerate(source.splitlines()) if "ColorPicker(" in s)
        caller_lines[uri] = line
        messages.append({"id": request_id, "method": "textDocument/definition", "params": {
            "textDocument": {"uri": uri}, "position": {"line": line, "character": text.index("ColorPicker(")}}})

    for version, source in ((2, edited), (3, original)):
        messages.append({"method": "textDocument/didChange", "params": {
            "textDocument": {"uri": picker_uri, "version": version}, "contentChanges": [{"text": source}]}})
        # A response delimits diagnostics published for this edit in the stdio transcript.
        messages.append({"id": version + 2, "method": "workspace/symbol", "params": {"query": "ColorPicker"}})
    messages.extend([{"id": 6, "method": "shutdown"}, {"method": "exit"}])
    run = subprocess.run([lsp], cwd=project, input=b"".join(map(frame, messages)),
                         stdout=subprocess.PIPE, timeout=60, check=True)
    diagnostics = {}
    seen = []
    for reply in responses(run.stdout):
        assert "error" not in reply, reply
        if reply.get("method") == "textDocument/publishDiagnostics":
            params = reply["params"]
            diagnostics[params["uri"]] = params["diagnostics"]
            continue
        request_id = reply["id"]
        seen.append(request_id)
        if request_id in (2, 3):
            location = reply["result"]
            assert location["uri"].lower() == picker_uri.lower(), location
            assert location["range"]["start"]["line"] == declaration, location
        if request_id in (2, 4, 5):
            assert diagnostics[picker_uri] == [], diagnostics
            for uri, line in caller_lines.items():
                errors = diagnostics[uri]
                if request_id == 4:
                    assert any(e["severity"] == 1 and e["range"]["start"]["line"] == line
                               and "String" in e["message"] and "Int" in e["message"] for e in errors), errors
                else:
                    assert errors == [], errors
            diagnostics.clear()
    assert seen == list(range(1, 7)), seen
    print("Both editor bindings resolved to ColorPicker, reported the unsaved type change and cleared on repair")
