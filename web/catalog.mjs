const revision = "1022d50b9e11fc4c10c982715929245ad9d9f402";
const repository = "Micrurus-Ai/Corvid-lang";
const packages = [{
  name: "@corvid/ui", version: "0.0.1", alias: "ui", testedWith: "0.0.1",
  description: "Build browser views with native buttons, forms and expandable details.",
  keywords: "disclosure accordion expand collapse show hide details text input submit stack row grid",
  source: `https://github.com/${repository}/blob/${revision}/std/ui.cor`,
  url: `https://raw.githubusercontent.com/${repository}/${revision}/std/ui.cor`,
  sha256: "63d02561610ab9750fd90fdb6c4a32cc1092e9be8b29cb69957ac134bcc3c703",
  example: 'agent main() -> String:\n    return ui.disclosure("Details", [ui.text("Hello")]).html',
  preview: "<details><summary>Details</summary><p>Hello</p></details>",
}];

function escape(value) {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
}

function entry(pkg) {
  const {name, version, alias, description, keywords, source, testedWith} = pkg;
  return {name, version, description, keywords, source, testedWith, path: "/packages/" + name,
    importLine: `import "corvid://${name}/v${version}" as ${alias}`};
}

function page(title, body, status = 200) {
  return new Response(`<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>${title} ? Corvid</title>
<style>*{box-sizing:border-box}body{margin:auto;padding:32px 20px 64px;max-width:800px;font:17px/1.55 system-ui;color:#18291e;background:#f7faf6}nav{margin-bottom:44px}h1{font-size:clamp(2rem,6vw,3.4rem);line-height:1.08}a{color:#185638}form{display:flex;gap:8px;margin:10px 0 24px}input,button{font:inherit;border:1px solid #74917e;border-radius:6px;padding:8px 12px}input{min-width:0;flex:1}button{background:#173f2a;color:white;cursor:pointer}article,pre,details{padding:16px;background:white;border:1px solid #cbd7cf;border-radius:8px}pre{overflow:auto}summary{cursor:pointer}</style>
<nav><a href="/">Corvid</a> / <a href="/packages">Packages</a></nav><main>${body}</main></html>`,
    {status, headers: {"content-type":"text/html; charset=utf-8"}});
}

export function catalog(url) {
  const path = url.pathname.replace(/\/+$/, "");
  const query = (url.searchParams.get("q") || "").trim();
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  const entries = packages.map(entry).filter(p => terms.every(term =>
    [p.name, p.description, p.keywords].join(" ").toLowerCase().includes(term)));
  if (path === "/packages/catalog.json") return Response.json(entries);
  if (path === "/packages/index.json") {
    const index = {};
    for (const {name, version, url, sha256} of packages) {
      (index[name] ??= {versions: {}}).versions[version] = {name, version, url, sha256};
    }
    return Response.json({packages:index});
  }
  if (path === "/packages") {
    const results = entries.map(p => `<article><a href="${p.path}"><strong>${escape(p.name)}</strong></a> ? ${p.version}<p>${escape(p.description)}</p></article>`).join("");
    return page("Packages", `<h1>Find a package</h1><label for="q">Package name or task</label><form action="/packages"><input id="q" name="q" type="search" value="${escape(query)}" placeholder="expand details"><button>Search</button></form>${results || "<p>No matching packages. Try another name or task.</p>"}`);
  }
  const pkg = packages.find(p => entry(p).path === path);
  if (!pkg) return page("Package not found", "<h1>Package not found</h1>", 404);
  const p = entry(pkg);
  return page(p.name, `<h1>${p.name}</h1><p>${p.description}</p><p>Publisher: ${p.name.split("/")[0]} ? Version ${p.version} ? Tested with Corvid ${p.testedWith}</p><a href="${p.source}">View source</a>
<h2>Try it</h2>${pkg.preview}<h2>Use in a project</h2><p>Add the package to the project lockfile once.</p><pre>corvid add package ${p.name}@${p.version} --registry ${escape(url.origin)}/packages/index.json</pre>
<pre><code id="snippet">${escape(p.importLine)}</code>\n\n${escape(pkg.example)}</pre>
<button id="copy">Copy import</button> <span role="status" id="status"></span>
<script>document.querySelector("#copy").onclick=async()=>{const status=document.querySelector("#status");try{await navigator.clipboard.writeText(document.querySelector("#snippet").textContent);status.textContent="Copied."}catch{status.textContent="Select the import line above to copy it."}}</script>`);
}
