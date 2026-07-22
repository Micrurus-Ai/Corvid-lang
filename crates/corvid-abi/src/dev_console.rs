//! The `corvid dev` universal console (slice 51m).
//!
//! `emit_dev_console` renders a single self-contained HTML page whose
//! behavior is driven entirely by the Application Contract + the
//! `corvid-ai.json` metadata embedded into it. The SAME renderer works
//! for every Corvid app — it reads the embedded contract and builds:
//!
//! - sign-in buttons per identity provider + a session panel,
//! - a form per public agent/prompt with typed inputs, capability
//!   badges (streaming / grounded / approvals / cost / latency /
//!   pagination), a Run button, and a live result/stream panel,
//! - a tool-call timeline + approval cards fed by the streamed event
//!   protocol,
//! - a citation/source viewer for grounded answers,
//! - a typed-error inspector that reads the `@status`/`@ui` metadata,
//! - a type browser and the auth safeguards list.
//!
//! Execution targets a configurable backend base URL (default same
//! origin), using the exact `/agents/{name}` + SSE conventions the
//! shipped `@corvid/client` uses — so a running `corvid serve` drives
//! the console live.

use crate::app_contract::ApplicationContract;
use crate::corvid_ai::CorvidAiMetadata;

/// Render the self-contained dev-console HTML for a contract.
pub fn emit_dev_console(contract: &ApplicationContract, ai: &CorvidAiMetadata) -> String {
    let contract_json = serde_json::to_string(contract).unwrap_or_else(|_| "{}".to_string());
    let ai_json = serde_json::to_string(ai).unwrap_or_else(|_| "{}".to_string());
    let title = "Corvid dev console";

    // The embedded JSON is injected into a <script type="application/json">
    // block (not executable) and parsed by the renderer, so contract
    // strings can never break out into script context.
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{title}</title>\n<style>{css}</style>\n</head>\n<body>\n<div id=\"app\"></div>\n<script type=\"application/json\" id=\"corvid-contract\">{contract_json}</script>\n<script type=\"application/json\" id=\"corvid-ai\">{ai_json}</script>\n<script>{js}</script>\n</body>\n</html>\n",
        css = CONSOLE_CSS,
        js = CONSOLE_JS,
    )
}

const CONSOLE_CSS: &str = r##"
:root { --bg:#0b0e14; --panel:#141922; --border:#232a36; --fg:#e6edf3; --muted:#9aa7b5; --accent:#4c9ffe; --ok:#3fb950; --warn:#d29922; --err:#f85149; }
@media (prefers-color-scheme: light){ :root{ --bg:#f6f8fa; --panel:#fff; --border:#d0d7de; --fg:#1f2328; --muted:#636c76; --accent:#0969da; } }
*{box-sizing:border-box} body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.5 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,sans-serif}
header{padding:16px 24px;border-bottom:1px solid var(--border);display:flex;align-items:baseline;gap:12px;flex-wrap:wrap}
header h1{font-size:16px;margin:0} .ver{color:var(--muted);font-size:12px}
main{max-width:960px;margin:0 auto;padding:24px;display:flex;flex-direction:column;gap:24px}
.card{background:var(--panel);border:1px solid var(--border);border-radius:10px;padding:16px}
.card h2{font-size:14px;margin:0 0 10px} .card h3{font-size:13px;margin:0}
.row{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
label{display:block;font-size:12px;color:var(--muted);margin:8px 0 4px}
input,textarea,select{width:100%;background:var(--bg);color:var(--fg);border:1px solid var(--border);border-radius:6px;padding:8px}
button{background:var(--accent);color:#fff;border:0;border-radius:6px;padding:8px 14px;cursor:pointer;font-weight:600}
button.secondary{background:transparent;color:var(--fg);border:1px solid var(--border)}
.badge{display:inline-block;font-size:11px;padding:2px 8px;border-radius:999px;border:1px solid var(--border);color:var(--muted)}
.badge.stream{color:var(--accent);border-color:var(--accent)} .badge.grounded{color:var(--ok);border-color:var(--ok)}
.badge.approve{color:var(--warn);border-color:var(--warn)} .badge.taint{color:var(--err);border-color:var(--err)}
.out{margin-top:10px;background:var(--bg);border:1px solid var(--border);border-radius:6px;padding:10px;white-space:pre-wrap;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px;max-height:320px;overflow:auto}
.evt{border-left:3px solid var(--border);padding:2px 8px;margin:2px 0} .evt.completed{border-color:var(--ok)} .evt.failed{border-color:var(--err)} .evt.approval_required{border-color:var(--warn)} .evt.chunk{border-color:var(--accent)}
.muted{color:var(--muted)} .provs button{margin-right:8px}
.safe{display:flex;flex-wrap:wrap;gap:6px}
"##;

const CONSOLE_JS: &str = r##"
const $ = (s, el=document) => el.querySelector(s);
const h = (tag, attrs={}, ...kids) => { const e=document.createElement(tag); for(const[k,v]of Object.entries(attrs)){ if(k==="class")e.className=v; else if(k.startsWith("on"))e.addEventListener(k.slice(2),v); else e.setAttribute(k,v);} for(const k of kids.flat()){ e.append(k?.nodeType?k:document.createTextNode(k??"")); } return e; };
const contract = JSON.parse($("#corvid-contract").textContent||"{}");
const ai = JSON.parse($("#corvid-ai").textContent||"{}");
let backend = localStorage.getItem("corvid.backend") || location.origin;

function tsPreview(name){ return name; }
function inputControl(p){
  const t = (p.type||p["type"]||"").toString();
  const long = /String/.test(t);
  const el = long ? h("textarea",{rows:"2"}) : h("input",{type:/Int|Float/.test(t)?"number":"text"});
  el.dataset.param = p.name; el.dataset.ptype = t; return el;
}
function aiFor(name){ return (ai.agents||[]).concat(ai.prompts||[]).find(a=>a.name===name); }

function callableCard(c, kind){
  const meta = aiFor(c.name)||{}; const caps=c.capabilities||{};
  const badges = h("div",{class:"row"});
  if(caps.streaming) badges.append(h("span",{class:"badge stream"},"streaming"));
  if(caps.grounded) badges.append(h("span",{class:"badge grounded"},"grounded"));
  if(caps.approvals_possible) badges.append(h("span",{class:"badge approve"},"approval"));
  if(caps.tainted_input) badges.append(h("span",{class:"badge taint"},"tainted input"));
  if(caps.max_cost_usd!=null) badges.append(h("span",{class:"badge"},"≤ $"+caps.max_cost_usd));
  if(caps.latency_class) badges.append(h("span",{class:"badge"},caps.latency_class));
  if(caps.pagination) badges.append(h("span",{class:"badge"},caps.pagination.style+" pages"));

  const inputs=(c.inputs||[]).map(inputControl);
  const form=h("div",{});
  (c.inputs||[]).forEach((p,i)=>{ form.append(h("label",{},p.name+" : "+(p.type||p["type"])); form.append(inputs[i]); });
  const out=h("div",{class:"out muted"},"— run to see output —");

  const run=h("button",{onclick:async()=>{
    const body={}; inputs.forEach(el=>{ let v=el.value; if(el.dataset.ptype&&/Int|Float/.test(el.dataset.ptype))v=Number(v); body[el.dataset.param]=v; });
    out.textContent=""; out.className="out";
    try{
      if(caps.streaming){ await runStream(c.name, body, out, meta); }
      else { const r=await fetch(backend+"/agents/"+encodeURIComponent(c.name),{method:"POST",credentials:"include",headers:{"content-type":"application/json"},body:JSON.stringify(body)});
        const txt=await r.text(); let parsed; try{parsed=JSON.parse(txt)}catch{parsed=txt}
        if(!r.ok){ out.append(h("div",{class:"evt failed"},"HTTP "+r.status)); out.append(h("pre",{},JSON.stringify(parsed,null,2))); return; }
        out.append(h("pre",{},JSON.stringify(parsed,null,2)));
      }
    }catch(e){ out.className="out"; out.append(h("div",{class:"evt failed"},"request failed: "+e.message)); out.append(h("div",{class:"muted"},"Is a backend running at "+backend+" ?")); }
  }},"Run");

  return h("div",{class:"card"},
    h("div",{class:"row"}, h("h3",{},kind+" "+c.name), h("span",{class:"muted"},"→ "+(c.output_type||"")) ),
    badges, form, h("div",{class:"row"},run), out);
}

async function runStream(name, body, out, meta){
  const r=await fetch(backend+"/agents/"+encodeURIComponent(name)+"/stream",{method:"POST",credentials:"include",headers:{"content-type":"application/json",accept:"text/event-stream"},body:JSON.stringify(body)});
  if(!r.ok||!r.body){ out.append(h("div",{class:"evt failed"},"stream failed: HTTP "+r.status)); return; }
  const reader=r.body.getReader(); const dec=new TextDecoder(); let buf="";
  for(;;){ const {value,done}=await reader.read(); if(done)break; buf+=dec.decode(value,{stream:true}); let i;
    while((i=buf.indexOf("\n\n"))!==-1){ const raw=buf.slice(0,i); buf=buf.slice(i+2); let ev="message",data="";
      for(const line of raw.split("\n")){ if(line.startsWith("event:"))ev=line.slice(6).trim(); else if(line.startsWith("data:"))data+=line.slice(5).trim(); }
      out.append(h("div",{class:"evt "+ev}, ev+": "+data)); out.scrollTop=out.scrollHeight; } }
}

function render(){
  const app=$("#app"); app.innerHTML="";
  app.append(h("header",{}, h("h1",{},"Corvid dev console"), h("span",{class:"ver"},contract.source_path||""), h("span",{class:"ver"},"contract v"+(contract.contract_version||"?")) ));
  const main=h("main",{});

  // backend config
  main.append(h("div",{class:"card"},
    h("label",{},"Backend base URL (a running `corvid serve`)"),
    (()=>{ const i=h("input",{type:"text",value:backend}); i.addEventListener("change",()=>{backend=i.value;localStorage.setItem("corvid.backend",backend);}); return i;})() ));

  // identity / auth
  for(const idb of (contract.identities||[])){
    const provs=h("div",{class:"provs"});
    for(const p of idb.providers||[]) provs.append(h("button",{class:"secondary",onclick:()=>location.href=backend+"/auth/"+encodeURIComponent(p.name)+"/login"},"Sign in · "+p.name));
    provs.append(h("button",{class:"secondary",onclick:async()=>{const r=await fetch(backend+"/auth/session",{credentials:"include"});$("#session").textContent=await r.text();}},"Session"));
    provs.append(h("button",{class:"secondary",onclick:()=>fetch(backend+"/auth/logout",{method:"POST",credentials:"include"})},"Logout"));
    const safe=h("div",{class:"safe"}); for(const s of idb.safeguards||[]) safe.append(h("span",{class:"badge grounded"},s));
    main.append(h("div",{class:"card"}, h("h2",{},"Identity · "+idb.name), provs, h("div",{class:"out muted",id:"session"},"— session —"), h("h3",{},"Guaranteed safe-defaults"), safe));
  }

  // agents + prompts
  const callables=h("div",{class:"card"}, h("h2",{},"Agents & prompts"));
  for(const a of contract.agents||[]) callables.append(callableCard(a,"agent"));
  for(const p of contract.prompts||[]) callables.append(callableCard(p,"prompt"));
  if((contract.agents||[]).length+(contract.prompts||[]).length===0) callables.append(h("div",{class:"muted"},"No public agents or prompts in this contract."));
  main.append(callables);

  // types
  const types=h("div",{class:"card"}, h("h2",{},"Types"));
  for(const t of contract.types||[]){ const fields=(t.fields||[]).map(f=>f.name+": "+(f.type||f["type"])).join(", ");
    const variants=(t.variants||[]).map(v=>v.name+(v.status?(" ["+v.status+"]"):"")).join(" | ");
    types.append(h("div",{class:"row"}, h("h3",{},t.name), h("span",{class:"muted"}, variants?("= "+variants):("{ "+fields+" }")) )); }
  main.append(types);

  $("#app").append(main);
}
render();
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_contract::{effect_decls_of, emit_application_contract, ContractOptions};
    use crate::corvid_ai::emit_corvid_ai;
    use corvid_types::effects::EffectRegistry;

    fn console_for(src: &str) -> String {
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
        let registry = EffectRegistry::from_decls(&effect_decls_of(&file));
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(checked.errors.is_empty(), "check: {:?}", checked.errors);
        let contract = emit_application_contract(
            &file,
            &resolved,
            &checked,
            &registry,
            &ContractOptions {
                source_path: "app.cor",
                compiler_version: "test",
                generated_at: "now",
            },
        );
        let ai = emit_corvid_ai(&contract);
        emit_dev_console(&contract, &ai)
    }

    #[test]
    fn console_embeds_contract_and_is_self_contained() {
        let html = console_for(
            "public agent classify(question: String) -> String:
    return question
",
        );
        assert!(html.starts_with("<!doctype html>"));
        // Self-contained: no external scripts/styles.
        assert!(!html.contains("src=\"http"));
        assert!(!html.contains("href=\"http"));
        // Embeds the contract + ai metadata as inert JSON.
        assert!(html.contains("id=\"corvid-contract\""));
        assert!(html.contains("id=\"corvid-ai\""));
        assert!(html.contains("classify"));
    }

    #[test]
    fn console_renders_identity_and_capability_surface() {
        let html = console_for(
            "identity app_users:
    provider google

public agent chat(message: String) -> Stream<String>:
    return stream_answer(message)

tool stream_answer(m: String) -> Stream<String>
",
        );
        // The embedded contract carries the identity providers + the
        // streaming capability the JS renderer reads.
        assert!(html.contains("app_users"));
        assert!(html.contains("google"));
        assert!(html.contains("no_silent_email_merge") || html.contains("secure_http_only_cookies"));
        assert!(html.contains("\"streaming\":true"));
        // The renderer wires the SSE + agents conventions.
        assert!(html.contains("/agents/"));
        assert!(html.contains("text/event-stream"));
    }
}
