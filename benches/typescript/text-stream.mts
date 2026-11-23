import {createServer} from "node:http";
import {readFileSync} from "node:fs";
import {Readable} from "node:stream";
import {createInterface} from "node:readline";

const [port, config] = process.argv.slice(2);
const {html, module, endpoint, payload} = JSON.parse(readFileSync(config, "utf8"));
createServer(async (req, res) => {
  if (req.url === "/") { res.writeHead(200, {"Content-Type":"text/html; charset=utf-8"}).end(html); return; }
  if (req.url === "/.well-known/corvid/stream.js") {
    res.writeHead(200, {"Content-Type":"text/javascript; charset=utf-8"}).end(module); return;
  }
  if (req.url !== "/answer") { res.writeHead(404).end(); return; }
  const abort = new AbortController();
  res.once("close", () => abort.abort());
  try {
    const upstream = await fetch(endpoint, {method:"POST", headers:{"Content-Type":"application/json"},
      body:JSON.stringify(payload), signal:abort.signal});
    if (!upstream.ok || !upstream.body) throw Error("provider unavailable");
    res.writeHead(200, {"Content-Type":"text/event-stream", "Cache-Control":"no-cache"});
    res.flushHeaders();
    for await (const line of createInterface({input:Readable.fromWeb(upstream.body)})) {
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") { res.write("event: done\ndata: complete\n\n"); break; }
      const text = JSON.parse(data).choices[0]?.delta?.content;
      if (typeof text === "string") res.write("data: " + JSON.stringify(text) + "\n\n");
    }
  } catch {
    if (!abort.signal.aborted) {
      if (res.headersSent) res.write("event: error\ndata: provider unavailable\n\n");
      else res.writeHead(502);
    }
  } finally { if (!res.destroyed) res.end(); }
}).listen(Number(port), "127.0.0.1");
