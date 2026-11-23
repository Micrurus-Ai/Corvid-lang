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
function list() {
  const db = open();
  try { return db.prepare("SELECT shelf, title FROM shelves ORDER BY shelf").all(); }
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
    if (req.method !== "POST" || url.pathname !== "/save") { res.writeHead(404).end(); return; }
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const form = new URLSearchParams(Buffer.concat(chunks).toString());
    const shelf = Number(form.get("shelf"));
    const title = form.get("title");
    if (!Number.isInteger(shelf) || shelf < 1 || shelf > 3 || title === null) {
      res.writeHead(400).end(); return;
    }
    const begin = now();
    const db = open();
    try {
      const result = db.prepare("UPDATE shelves SET title = ?, revision = revision + 1 WHERE shelf = ?").run(title, shelf);
      if (result.changes !== 1) throw Error("Shelf was not found");
    } finally { db.close(); }
    const written = now();
    const rows = list();
    const loaded = now();
    const html = screen(rows, shelf, true);
    res.end(html + `<!--bench ${written-begin} ${loaded-written} ${now()-begin}-->`);
  } catch (error) { res.writeHead(500).end(String(error)); }
}).listen(Number(port), "127.0.0.1");
