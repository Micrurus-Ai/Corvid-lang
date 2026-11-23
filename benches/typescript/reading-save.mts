import {createServer} from "node:http";
import {readFileSync} from "node:fs";
import {DatabaseSync} from "node:sqlite";

const [port, templatePath] = process.argv.slice(2);
const template = JSON.parse(readFileSync(templatePath, "utf8"));
const settings = template.settings;
const now = () => Math.floor(performance.now());
function open() {
  const db = new DatabaseSync("reading.sqlite", {enableForeignKeyConstraints: Boolean(settings.fk)});
  return db;
}
function shelves(db: DatabaseSync) {
  return db.prepare("SELECT shelf, title FROM shelves ORDER BY shelf").all();
}
function list(stamps?: number[]) {
  const db = open();
  if (stamps) stamps.push(now());
  try {
    const rows = shelves(db);
    if (stamps) stamps.push(now());
    return rows;
  }
  finally { db.close(); }
}
function escape(value: string) {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
}
function screen(rows: ReturnType<typeof list>, selected: number, saved: boolean) {
  const items = rows.map(({shelf, title}) => {
    const text = escape(String(title));
    const form = shelf === selected ? template.forms[selected].replaceAll("__TITLE__", () => text) : "";
    return `<li><a href="/edit?shelf=${shelf}">Shelf ${shelf}: ${text}</a>${form}</li>`;
  });
  return template.prefix + (saved ? "<p>Saved</p>" : "") + "<ul>" + items.join("") + "</ul>" + template.suffix;
}
createServer(async (req, res) => {
  try {
    const url = new URL(req.url!, "http://localhost");
    res.setHeader("content-type", "text/html; charset=utf-8");
    if (url.pathname === "/settings") {
      const db = open();
      try { res.end(JSON.stringify(db.prepare(template.settingsSql).get())); }
      finally { db.close(); }
      return;
    }
    if (req.method === "GET" && ["/", "/edit"].includes(url.pathname)) {
      res.end(screen(list(), Number(url.searchParams.get("shelf")), false));
      return;
    }
    const suggest = url.pathname === "/suggest" && template.modelUrl;
    if (req.method !== "POST" || (url.pathname !== "/save" && !suggest)) {
      res.writeHead(404).end(); return;
    }
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const form = new URLSearchParams(Buffer.concat(chunks).toString());
    const shelf = Number(form.get("shelf"));
    if (suggest) {
      const current = list().find(row => row.shelf === shelf);
      if (!current) { res.writeHead(400).end(); return; }
      const answer = await fetch(template.modelUrl, {
        method: "POST", headers: {"Content-Type": "application/json"},
        body: JSON.stringify({model: "local-model", messages: [{role: "user", content: String(current.title)}]}),
      });
      if (!answer.ok) throw Error("Model request failed");
      const message = (await answer.json()).choices[0].message;
      res.end(escape(JSON.parse(message.content).action.fields[0]));
      return;
    }
    const title = form.get("title");
    if (!Number.isInteger(shelf) || shelf < 1 || shelf > 3 || title === null) {
      res.writeHead(400).end(); return;
    }
    const begin = now();
    const db = open();
    const opened = template.profile ? now() : 0;
    let updated = 0;
    let sharedRows: ReturnType<typeof shelves> | undefined;
    try {
      const result = db.prepare("UPDATE shelves SET title = ?, revision = revision + 1 WHERE shelf = ?").run(title, shelf);
      if (result.changes !== 1) throw Error("Shelf was not found");
      if (template.profile) updated = now();
      if (template.sharedConnection) sharedRows = shelves(db);
    } finally { db.close(); }
    const written = now();
    const stamps = template.profile ? [] as number[] : undefined;
    const rows = sharedRows ?? list(stamps);
    const loaded = now();
    const html = screen(rows, shelf, true);
    const end = now();
    const timings = [written-begin, loaded-written, end-begin];
    if (stamps) {
      const [readerOpened, queried] = stamps;
      // SQLite's all() returns JavaScript rows directly; no separate decode loop.
      timings.push(opened-begin, updated-opened, readerOpened-written, queried-readerOpened, 0, end-loaded);
    }
    res.end(html + "<!--bench " + timings.join(" ") + "-->");
  } catch (error) { res.writeHead(500).end(String(error)); }
}).listen(Number(port), "127.0.0.1");
