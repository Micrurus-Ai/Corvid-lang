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
  return db.prepare("SELECT shelf, title FROM StoredShelf ORDER BY shelf").all();
}
function list() {
  const db = open();
  try { return shelves(db); }
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
    let sharedRows: ReturnType<typeof shelves> | undefined;
    try {
      // The same work as the Corvid store's atomic rename: read the row and
      // write it back only if its revision is unchanged, in one transaction.
      db.exec("BEGIN IMMEDIATE");
      try {
        const current = db.prepare('SELECT revision FROM "StoredShelf" WHERE shelf = ?').get(shelf);
        if (!current) throw Error("Shelf was not found");
        const result = db.prepare('UPDATE "StoredShelf" SET title = ?, revision = revision + 1 WHERE shelf = ? AND revision = ?')
          .run(title, shelf, current.revision);
        if (result.changes !== 1) throw Error("The shelf changed while saving. Try again.");
        db.exec("COMMIT");
      } catch (error) { db.exec("ROLLBACK"); throw error; }
      if (template.sharedConnection) sharedRows = shelves(db);
    } finally { db.close(); }
    const written = now();
    const rows = sharedRows ?? list();
    const loaded = now();
    const html = screen(rows, shelf, true);
    const end = now();
    const timings = [written-begin, loaded-written, end-begin];
    res.end(html + "<!--bench " + timings.join(" ") + "-->");
  } catch (error) { res.writeHead(500).end(String(error)); }
}).listen(Number(port), "127.0.0.1");
