use crate::error::WasmCodegenError;
use crate::WasmHostImport;
use corvid_ir::IrAgent;
use corvid_types::Type;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct WasmArtifacts {
    pub wasm: Vec<u8>,
    pub js_loader: String,
    pub quantity_loader: Option<String>,
    pub ts_types: String,
    pub manifest_json: String,
}

#[derive(Debug, Serialize)]
struct WasmManifest {
    schema_version: u32,
    module_name: String,
    target: &'static str,
    exports: Vec<WasmExport>,
    imports: Vec<WasmImport>,
    host_capability_abi: &'static str,
    non_scope: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct WasmExport {
    name: String,
    params: Vec<WasmParam>,
    return_type: String,
    /// WASM-level kind discriminator for the return type:
    /// `"i64"` (Int), `"f64"` (Float), `"i32"` (Bool), `"void"`
    /// (Nothing), or `"string"` (multi-value `(ptr, len)`).
    /// Downstream tooling reads this to dispatch on the boundary
    /// shape without re-parsing `return_type`.
    return_kind: &'static str,
}

#[derive(Debug, Serialize)]
struct WasmParam {
    name: String,
    /// Human-readable Corvid type name (e.g. `"Int"`, `"String"`).
    /// Stable for display; downstream consumers should switch on
    /// `kind` for ABI shape rather than parsing `ty`.
    ty: String,
    /// WASM-level boundary kind. Slice 20n-B-3 added this uniformly
    /// across all params (decision 3): callers can tell a String's
    /// `(ptr, len)` pair from a scalar `i32`/`i64`/`f64` without
    /// inspecting `ty`. Values: `"i64"`, `"f64"`, `"i32"`,
    /// `"string"`. Host imports stay scalar in v1, so their params
    /// never have `kind: "string"`.
    kind: &'static str,
}

#[derive(Debug, Serialize)]
struct WasmImport {
    kind: &'static str,
    source_name: String,
    import_module: &'static str,
    import_name: String,
    params: Vec<WasmParam>,
    return_type: String,
    /// See `WasmExport::return_kind`. Always one of the scalar /
    /// `void` kinds for v1 because host imports do not carry
    /// String params or returns.
    return_kind: &'static str,
}

pub(crate) fn build_artifacts(
    module_name: &str,
    agents: &[&IrAgent],
    imports: &[WasmHostImport],
    wasm: Vec<u8>,
    trap_sites: &[String],
) -> Result<WasmArtifacts, WasmCodegenError> {
    use sha2::{Digest, Sha256};
    let wasm_hash = format!("{:x}", Sha256::digest(&wasm));
    let methods: Vec<_> = if module_name == "events" {
        agents.iter().filter_map(|agent| {
            crate::integer_handlers::method(agent).map(|method| (*agent, method))
        }).collect()
    } else {
        Vec::new()
    };
    let integer_exports = |unary_only: bool| {
        let body = methods.iter()
            .filter(|(agent, _)| !unary_only || agent.params.len() == 1)
            .map(|(_, method)| method.as_str())
            .collect::<Vec<_>>().join(",\n");
        format!("const integerHandlers = {{\n{body}\n}};\n")
    };
    let quantity_loader = methods.iter().any(|(agent, _)| agent.params.len() == 1).then(|| {
        let names: Vec<_> = agents.iter()
            .filter(|agent| agent.params.len() == 1 &&
                agent.params[0].ty == Type::Int && agent.return_ty == Type::Int)
            .map(|agent| &agent.name).collect();
        format!("{}const intStateHandlers = new Set({});\n{}\n{}\n{}",
            integer_exports(true),
            serde_json::to_string(&names).expect("serialize handler names"),
            include_str!("quantity_field.js"),
            include_str!("keyed_list.js"),
            include_str!("quantity_page.js"))
    });
    let direct = (!methods.is_empty() && methods.len() == agents.len() && imports.is_empty())
        .then(|| integer_exports(false));
    Ok(WasmArtifacts {
        quantity_loader,
        wasm,
        js_loader: emit_js_loader(module_name, agents, imports, trap_sites, &wasm_hash, direct.as_deref())?,
        ts_types: {
            let mut types = emit_ts_types(agents, imports)?;
            if module_name == "events" {
                types.push_str("export function componentElement(html: string): CustomElementConstructor;\n");
            }
            types
        },
        manifest_json: emit_manifest(module_name, agents, imports)?,
    })
}

/// WASM-level boundary kind for a Corvid `Type`. Used by the
/// manifest's `kind` discriminator so downstream tooling can
/// dispatch on the boundary shape uniformly. See
/// `WasmParam::kind` and `WasmExport::return_kind`.
fn wasm_boundary_kind(ty: &Type) -> &'static str {
    match ty {
        Type::Int => "i64",
        Type::Float => "f64",
        Type::Bool => "i32",
        Type::Nothing => "void",
        Type::String => "string",
        // Anything else is rejected at validate time before reaching
        // the manifest emitter; surface it as an explicit unknown
        // so a manifest containing this kind is recognisable as a
        // reachability bug rather than silently mistyped.
        _ => "unknown",
    }
}

fn ts_type(ty: &Type) -> Result<&'static str, WasmCodegenError> {
    match ty {
        Type::Int => Ok("bigint"),
        Type::Float => Ok("number"),
        Type::Bool => Ok("boolean"),
        Type::Nothing => Ok("void"),
        // Slice 20n-B-2a: surface a `String` agent param/return as
        // `string` in the generated `.d.ts`. The JS loader still
        // emits the legacy `<agent>(<scalar args>)` glue today —
        // the loader's String-aware packing/unpacking is shipped
        // by commit 3 of this slice. Until then, calling a String
        // agent through the loader will see two raw `i32` arguments
        // instead of a string. The `.d.ts` mapping is deliberately
        // shipped early so the binary-shape regression tests in
        // 2a have something to assert against.
        Type::String => Ok("string"),
        _ => Err(WasmCodegenError::unsupported(format!(
            "unsupported TypeScript wasm surface type `{}`",
            ty.display_name()
        ))),
    }
}

fn js_return_expr(name: &str, ty: &Type) -> String {
    match ty {
        Type::Bool => "Boolean(result)".into(),
        Type::Nothing => "undefined".into(),
        _ => name.into(),
    }
}

fn emit_js_loader(
    module_name: &str,
    agents: &[&IrAgent],
    imports: &[WasmHostImport],
    trap_sites: &[String],
    wasm_hash: &str,
    direct: Option<&str>,
) -> Result<String, WasmCodegenError> {
    let mut out = String::new();
    out.push_str("// Auto-generated by Corvid. Do not edit.\n");
    out.push_str(&format!(
        "const WASM_URL = new URL('./{module_name}.wasm?v={wasm_hash}', import.meta.url);\n\n"
    ));
    if !trap_sites.is_empty() {
        let labels = serde_json::to_string(trap_sites).expect("serialize trap sites");
        out.push_str(&format!("const trapSites = {labels};\n"));
    }
    out.push_str("function missing(kind, name) {\n");
    out.push_str("  throw new Error(`Corvid WASM host ${kind} '${name}' is not provided`);\n");
    out.push_str("}\n\n");
    out.push_str("function traceValue(value) {\n");
    out.push_str("  if (typeof value === 'bigint') {\n");
    out.push_str("    const asNumber = Number(value);\n");
    out.push_str("    return Number.isSafeInteger(asNumber) ? asNumber : value.toString();\n");
    out.push_str("  }\n");
    out.push_str("  return value;\n");
    out.push_str("}\n\n");
    out.push_str("function freshRunId() {\n");
    out.push_str("  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();\n");
    out.push_str("  return `wasm-${Date.now()}-${Math.random().toString(16).slice(2)}`;\n");
    out.push_str("}\n\n");
    out.push_str("function makeTraceContext(trace) {\n");
    out.push_str("  const sink = trace ?? null;\n");
    out.push_str("  return {\n");
    out.push_str("    activeRunId: null,\n");
    out.push_str("    emit(event) {\n");
    out.push_str("      if (!sink) return;\n");
    out.push_str("      if (Array.isArray(sink)) sink.push(event);\n");
    out.push_str("      else if (typeof sink === 'function') sink(event);\n");
    out.push_str("      else if (Array.isArray(sink.events)) sink.events.push(event);\n");
    out.push_str("    },\n");
    out.push_str("    beginRun(agent, args) {\n");
    out.push_str("      const runId = freshRunId();\n");
    out.push_str("      this.activeRunId = runId;\n");
    out.push_str("      this.emit({ kind: 'schema_header', version: 2, writer: 'corvid-codegen-wasm', commit_sha: null, source_path: null, ts_ms: Date.now(), run_id: runId });\n");
    out.push_str("      this.emit({ kind: 'run_started', ts_ms: Date.now(), run_id: runId, agent, args: args.map(traceValue) });\n");
    out.push_str("      return runId;\n");
    out.push_str("    },\n");
    out.push_str("    completeRun(runId, ok, result, error) {\n");
    out.push_str("      this.emit({ kind: 'run_completed', ts_ms: Date.now(), run_id: runId, ok, result: result === undefined ? null : traceValue(result), error: error ?? null });\n");
    out.push_str("      this.activeRunId = null;\n");
    out.push_str("    },\n");
    out.push_str("  };\n");
    out.push_str("}\n\n");
    out.push_str("function adaptImportsWithContext(host = {}, traceContext) {\n");
    out.push_str("  return {\n");
    out.push_str("    'corvid:host': {\n");
    for import in imports {
        let call_line = format!(
            "(host.{}?.['{}'] ?? (() => missing('{}', '{}')))(...args)",
            import.kind.namespace(),
            import.source_name,
            import.kind.label(),
            import.source_name
        );
        let run_id = "const runId = traceContext.activeRunId ?? freshRunId();\n";
        let args_event = "args: args.map(traceValue)";
        out.push_str(&format!(
            "      '{}': (...args) => {{\n",
            import.import_name
        ));
        out.push_str("        ");
        out.push_str(run_id);
        match import.kind.label() {
            "prompt" => {
                out.push_str(&format!(
                    "        traceContext.emit({{ kind: 'llm_call', ts_ms: Date.now(), run_id: runId, prompt: '{}', model: null, model_version: null, rendered: null, {} }});\n",
                    import.source_name, args_event
                ));
                out.push_str(&format!("        const result = {};\n", call_line));
                out.push_str(&format!(
                    "        traceContext.emit({{ kind: 'llm_result', ts_ms: Date.now(), run_id: runId, prompt: '{}', model: null, model_version: null, result: traceValue(result) }});\n",
                    import.source_name
                ));
            }
            "tool" => {
                out.push_str(&format!(
                    "        traceContext.emit({{ kind: 'tool_call', ts_ms: Date.now(), run_id: runId, tool: '{}', {} }});\n",
                    import.source_name, args_event
                ));
                out.push_str(&format!("        const result = {};\n", call_line));
                out.push_str(&format!(
                    "        traceContext.emit({{ kind: 'tool_result', ts_ms: Date.now(), run_id: runId, tool: '{}', result: traceValue(result) }});\n",
                    import.source_name
                ));
            }
            "approval" => {
                out.push_str(&format!(
                    "        traceContext.emit({{ kind: 'approval_request', ts_ms: Date.now(), run_id: runId, label: '{}', {} }});\n",
                    import.source_name, args_event
                ));
                out.push_str(&format!("        const result = Boolean({});\n", call_line));
                out.push_str(&format!(
                    "        traceContext.emit({{ kind: 'approval_decision', ts_ms: Date.now(), run_id: runId, site: '{}', args: args.map(traceValue), accepted: result, decider: 'wasm-host', rationale: null }});\n",
                    import.source_name
                ));
                out.push_str(&format!(
                    "        traceContext.emit({{ kind: 'approval_response', ts_ms: Date.now(), run_id: runId, label: '{}', approved: result }});\n",
                    import.source_name
                ));
            }
            _ => {}
        }
        out.push_str("        return result;\n");
        out.push_str("      },\n");
    }
    out.push_str("    },\n");
    out.push_str("  };\n");
    out.push_str("}\n\n");
    out.push_str("export function adaptImports(host = {}, trace = undefined) {\n");
    out.push_str("  return adaptImportsWithContext(host, makeTraceContext(trace));\n");
    out.push_str("}\n\n");
    out.push_str("function indexedDbRequest(request) {\n");
    out.push_str("  return new Promise((resolve, reject) => {\n");
    out.push_str("    request.onsuccess = () => resolve(request.result);\n");
    out.push_str("    request.onerror = () => reject(request.error ?? new Error('IndexedDB request failed'));\n");
    out.push_str("  });\n");
    out.push_str("}\n\n");
    out.push_str("export async function createIndexedDbStoreHost(options = {}) {\n");
    out.push_str("  const dbName = options.dbName ?? 'corvid-wasm-store';\n");
    out.push_str("  const storeName = options.storeName ?? 'records';\n");
    out.push_str("  const db = await new Promise((resolve, reject) => {\n");
    out.push_str("    if (!globalThis.indexedDB) {\n");
    out.push_str("      reject(new Error('IndexedDB is not available'));\n");
    out.push_str("      return;\n");
    out.push_str("    }\n");
    out.push_str("    const request = globalThis.indexedDB.open(dbName, 1);\n");
    out.push_str("    request.onupgradeneeded = () => {\n");
    out.push_str("      if (!request.result.objectStoreNames.contains(storeName)) request.result.createObjectStore(storeName);\n");
    out.push_str("    };\n");
    out.push_str("    request.onsuccess = () => resolve(request.result);\n");
    out.push_str("    request.onerror = () => reject(request.error ?? new Error('open IndexedDB store failed'));\n");
    out.push_str("  });\n");
    out.push_str("  function objectStore(mode) {\n");
    out.push_str("    return db.transaction(storeName, mode).objectStore(storeName);\n");
    out.push_str("  }\n");
    out.push_str("  return {\n");
    out.push_str("    store: {\n");
    out.push_str("      async get(key) {\n");
    out.push_str("        const value = await indexedDbRequest(objectStore('readonly').get(String(key)));\n");
    out.push_str("        return value === undefined ? null : value;\n");
    out.push_str("      },\n");
    out.push_str("      async put(key, value) {\n");
    out.push_str("        await indexedDbRequest(objectStore('readwrite').put(value, String(key)));\n");
    out.push_str("        return value;\n");
    out.push_str("      },\n");
    out.push_str("      async delete(key) {\n");
    out.push_str("        await indexedDbRequest(objectStore('readwrite').delete(String(key)));\n");
    out.push_str("      },\n");
    out.push_str("    },\n");
    out.push_str("    close() {\n");
    out.push_str("      db.close();\n");
    out.push_str("    },\n");
    out.push_str("  };\n");
    out.push_str("}\n\n");
    if let Some(methods) = direct {
        out.push_str(methods);
    }
    out.push_str("async function wasmExports(imports) {\n");
    out.push_str("  const source = await WebAssembly.instantiateStreaming(fetch(WASM_URL), imports).catch(async () => {\n");
    out.push_str("    const bytes = await fetch(WASM_URL).then((r) => r.arrayBuffer());\n");
    out.push_str("    return WebAssembly.instantiate(bytes, imports);\n");
    out.push_str("  });\n");
    out.push_str("  return source.instance.exports;\n}\n");
    out.push_str("export async function instantiate(hostOrImports = {}, options = {}) {\n");
    out.push_str("  const traceContext = makeTraceContext(options.trace);\n");
    out.push_str("  const imports = hostOrImports['corvid:host'] ? hostOrImports : adaptImportsWithContext(hostOrImports, traceContext);\n");
    if direct.is_some() {
        out.push_str("  const exports = options.backend === 'wasm' ? await wasmExports(imports) : integerHandlers;\n");
    } else {
        out.push_str("  const exports = await wasmExports(imports);\n");
    }
    out.push_str("  return {\n");
    for agent in agents {
        emit_js_agent_wrapper(&mut out, agent, !trap_sites.is_empty());
    }
    out.push_str("  };\n");
    out.push_str("}\n");
    if module_name == "events" {
        let signatures: Vec<_> = agents.iter().map(|agent| {
            let params: Vec<_> = agent.params.iter().map(|param| wasm_boundary_kind(&param.ty)).collect();
            (&agent.name, (params, wasm_boundary_kind(&agent.return_ty)))
        }).collect();
        let signatures = serde_json::to_string(&signatures).expect("serialize projection signatures");
        out.push_str(&format!("const projectionSignatures = new Map({signatures});\n"));
        for (name, ty) in [("intStateHandlers", Type::Int), ("textStateHandlers", Type::String)] {
            let bindings: Vec<_> = agents.iter()
                .filter(|agent| agent.params.len() == 1 && agent.params[0].ty == ty && agent.return_ty == ty)
                .map(|agent| {
                    let param = agent.params[0].name.as_str();
                    // Counter buttons own their argument. Text controls accept a named field.
                    let properties = if ty == Type::String {
                        serde_json::json!({(param): {"type": "string", "pattern": "^[^\r\n]*$"}})
                    } else {
                        serde_json::json!({})
                    };
                    let mut input = serde_json::json!({
                        "type": "object", "properties": properties, "additionalProperties": false
                    });
                    if ty == Type::String {
                        input["required"] = serde_json::json!([param]);
                    }
                    // The JS loader preserves i64 precision by returning decimal text.
                    let output = if ty == Type::Int {
                        serde_json::json!({"type": "string", "pattern": "^-?(0|[1-9][0-9]*)$"})
                    } else {
                        serde_json::json!({"type": "string"})
                    };
                    (&agent.name, serde_json::json!({"inputSchema": input, "outputSchema": output}))
                })
                .collect();
            let bindings = serde_json::to_string(&bindings).expect("serialize control bindings");
            out.push_str(&format!("const {name} = new Map({bindings});\n"));
        }
        out.push_str("if (typeof document !== 'undefined' && [...document.querySelectorAll('script[type=module][data-corvid-ui]')].some(script => script.src === import.meta.url)) {\n");
        out.push_str(include_str!("record_projections.js"));
        out.push_str(include_str!("form_controls.js"));
        out.push_str("}\n");
        out.push_str(include_str!("browser_controls.js"));
        if agents.iter().any(|agent| agent.name.starts_with("__drag_")) {
            out.push_str(include_str!("drag_controls.js"));
        }
        out.push_str(include_str!("quantity_field.js"));
        out.push_str(include_str!("keyed_list.js"));
    }
    Ok(out)
}

/// Emit the JS wrapper for a single agent inside the
/// `instantiate()` return object. Handles four shapes:
///
/// 1. Scalar params + scalar/Nothing return — calls `exports.<agent>`
///    directly with the JS args, returns the WASM result (with a
///    Boolean coercion for Bool returns to keep the user-visible
///    type from being i32).
/// 2. String params + scalar return — encodes each String input
///    via TextEncoder, allocates linear-memory space via
///    `exports.corvid_alloc`, writes bytes, calls with `(ptr, len)`
///    pairs in place of the original String args, returns the
///    scalar result. Frees inputs in `finally`.
/// 3. Scalar params + String return — calls with scalar args,
///    destructures the multi-value `(ptr, len)` return, decodes via
///    TextDecoder.
/// 4. String params + String return — combines (2) and (3).
///
/// Per the v1 ownership convention (slice 20n-B), JS frees only
/// the inputs it allocated. The agent's returned `(ptr, len)` may
/// alias an input or a const-memory literal from the data section
/// — the loader copies the bytes out via `TextDecoder` (which makes
/// a JS-heap copy) before freeing inputs, so neither input nor
/// return ever needs an explicit free on the return side.
fn emit_js_agent_wrapper(out: &mut String, agent: &IrAgent, has_trap_sites: bool) {
    let js_params = agent
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let trace_args_array = if js_params.is_empty() {
        "[]".to_string()
    } else {
        format!("[{js_params}]")
    };
    let returns_string = matches!(agent.return_ty, Type::String);
    let returns_nothing = matches!(agent.return_ty, Type::Nothing);
    let returns_bool = matches!(agent.return_ty, Type::Bool);
    let has_string_param = agent
        .params
        .iter()
        .any(|param| matches!(param.ty, Type::String));

    out.push_str(&format!("    {}({}) {{\n", agent.name, js_params));
    out.push_str(&format!(
        "      const runId = traceContext.beginRun('{}', {});\n",
        agent.name, trace_args_array
    ));

    // Pre-amble: encode + allocate + write bytes for each String
    // parameter. Locals named `<param>_bytes` and `<param>_ptr`
    // survive into the `finally` block so the free runs even if the
    // agent throws.
    if has_string_param {
        out.push_str("      const __corvid_enc = new TextEncoder();\n");
        for param in &agent.params {
            if matches!(param.ty, Type::String) {
                out.push_str(&format!(
                    "      const __corvid_{name}_bytes = __corvid_enc.encode({name});\n",
                    name = param.name
                ));
                out.push_str(&format!(
                    "      const __corvid_{name}_ptr = exports.corvid_alloc(__corvid_{name}_bytes.length);\n",
                    name = param.name
                ));
                out.push_str(&format!(
                    "      new Uint8Array(exports.memory.buffer, __corvid_{name}_ptr, __corvid_{name}_bytes.length).set(__corvid_{name}_bytes);\n",
                    name = param.name
                ));
            }
        }
    }

    // WASM-side argument list: scalar params pass straight through;
    // String params expand to `<name>_ptr, <name>_bytes.length`.
    let wasm_call_args = agent
        .params
        .iter()
        .map(|param| {
            if matches!(param.ty, Type::String) {
                format!(
                    "__corvid_{name}_ptr, __corvid_{name}_bytes.length",
                    name = param.name
                )
            } else {
                param.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str("      try {\n");
    if returns_nothing {
        out.push_str(&format!(
            "        exports.{}({});\n",
            agent.name, wasm_call_args
        ));
        out.push_str("        traceContext.completeRun(runId, true, undefined, null);\n");
        out.push_str("        return undefined;\n");
    } else if returns_string {
        // Destructure the multi-value `(ptr, len)` return into JS
        // variables, then decode.
        out.push_str(&format!(
            "        const __corvid_result = exports.{}({});\n",
            agent.name, wasm_call_args
        ));
        out.push_str("        const __corvid_dec = new TextDecoder();\n");
        out.push_str("        const result = __corvid_dec.decode(\n");
        out.push_str("          new Uint8Array(exports.memory.buffer, __corvid_result[0], __corvid_result[1]),\n");
        out.push_str("        );\n");
        out.push_str("        traceContext.completeRun(runId, true, result, null);\n");
        out.push_str("        return result;\n");
    } else if returns_bool {
        out.push_str(&format!(
            "        const result = exports.{}({});\n",
            agent.name, wasm_call_args
        ));
        out.push_str("        const value = Boolean(result);\n");
        out.push_str("        traceContext.completeRun(runId, true, value, null);\n");
        out.push_str(&format!(
            "        return {};\n",
            js_return_expr("result", &agent.return_ty)
        ));
    } else {
        out.push_str(&format!(
            "        const result = exports.{}({});\n",
            agent.name, wasm_call_args
        ));
        out.push_str("        traceContext.completeRun(runId, true, result, null);\n");
        out.push_str("        return result;\n");
    }
    out.push_str("      } catch (error) {\n");
    if has_trap_sites {
        out.push_str("        const site = exports['corvid:trap-site'];\n");
        out.push_str("        const location = trapSites[site.value - 1];\n");
        out.push_str("        site.value = 0;\n");
        out.push_str("        if (location && error instanceof WebAssembly.RuntimeError) error.message += ' at ' + location;\n");
    }
    out.push_str("        traceContext.completeRun(runId, false, undefined, String(error?.message ?? error));\n");
    out.push_str("        throw error;\n");
    out.push_str("      }");

    // Free the inputs. The agent's return may alias an input span
    // (pass-through) — that's fine because the TextDecoder above
    // already copied the bytes into a fresh JS string before we
    // get here, so `corvid_free` reclaims the linear-memory space
    // safely.
    if has_string_param {
        out.push_str(" finally {\n");
        for param in &agent.params {
            if matches!(param.ty, Type::String) {
                out.push_str(&format!(
                    "        exports.corvid_free(__corvid_{name}_ptr, __corvid_{name}_bytes.length);\n",
                    name = param.name
                ));
            }
        }
        out.push_str("      }");
    }
    out.push_str("\n    },\n");
}

fn emit_ts_types(
    agents: &[&IrAgent],
    imports: &[WasmHostImport],
) -> Result<String, WasmCodegenError> {
    let mut out = String::new();
    out.push_str("// Auto-generated by Corvid. Do not edit.\n");
    out.push_str("export interface CorvidWasmHost {\n");
    out.push_str("  prompts?: {\n");
    for import in imports.iter().filter(|import| import.kind.namespace() == "prompts") {
        out.push_str(&format!("    '{}': {};\n", import.source_name, host_fn_type(import)?));
    }
    out.push_str("  };\n");
    out.push_str("  tools?: {\n");
    for import in imports.iter().filter(|import| import.kind.namespace() == "tools") {
        out.push_str(&format!("    '{}': {};\n", import.source_name, host_fn_type(import)?));
    }
    out.push_str("  };\n");
    out.push_str("  approvals?: {\n");
    for import in imports
        .iter()
        .filter(|import| import.kind.namespace() == "approvals")
    {
        out.push_str(&format!("    '{}': {};\n", import.source_name, host_fn_type(import)?));
    }
    out.push_str("  };\n");
    out.push_str("  store?: CorvidWasmStoreHost;\n");
    out.push_str("}\n\n");
    out.push_str("export interface CorvidWasmStoreHost {\n");
    out.push_str("  get(key: string): Promise<unknown | null>;\n");
    out.push_str("  put(key: string, value: unknown): Promise<unknown>;\n");
    out.push_str("  delete(key: string): Promise<void>;\n");
    out.push_str("}\n\n");
    out.push_str("export interface CorvidIndexedDbStoreOptions {\n");
    out.push_str("  dbName?: string;\n");
    out.push_str("  storeName?: string;\n");
    out.push_str("}\n\n");
    out.push_str("export type CorvidIndexedDbStoreHost = CorvidWasmHost & { close(): void };\n\n");
    out.push_str("export interface CorvidWasmModule {\n");
    for agent in agents {
        let params = agent
            .params
            .iter()
            .map(|param| Ok(format!("{}: {}", param.name, ts_type(&param.ty)?)))
            .collect::<Result<Vec<_>, WasmCodegenError>>()?
            .join(", ");
        out.push_str(&format!(
            "  {}({}): {};\n",
            agent.name,
            params,
            ts_type(&agent.return_ty)?
        ));
    }
    out.push_str("}\n\n");
    out.push_str(
        "export type CorvidWasmTraceSink = Array<Record<string, unknown>> | ((event: Record<string, unknown>) => void) | { events: Array<Record<string, unknown>> };\n",
    );
    out.push_str(
        "export interface CorvidWasmInstantiateOptions { trace?: CorvidWasmTraceSink; backend?: 'wasm'; }\n",
    );
    out.push_str(
        "export function adaptImports(host?: CorvidWasmHost, trace?: CorvidWasmTraceSink): WebAssembly.Imports;\n",
    );
    out.push_str(
        "export function createIndexedDbStoreHost(options?: CorvidIndexedDbStoreOptions): Promise<CorvidIndexedDbStoreHost>;\n",
    );
    out.push_str(
        "export function instantiate(hostOrImports?: WebAssembly.Imports | CorvidWasmHost, options?: CorvidWasmInstantiateOptions): Promise<CorvidWasmModule>;\n",
    );
    Ok(out)
}

fn host_fn_type(import: &WasmHostImport) -> Result<String, WasmCodegenError> {
    let params = import
        .params
        .iter()
        .map(|(name, ty)| Ok(format!("{name}: {}", ts_type(ty)?)))
        .collect::<Result<Vec<_>, WasmCodegenError>>()?
        .join(", ");
    Ok(format!("({params}) => {}", ts_type(&import.return_ty)?))
}

fn emit_manifest(
    module_name: &str,
    agents: &[&IrAgent],
    imports: &[WasmHostImport],
) -> Result<String, WasmCodegenError> {
    let exports = agents
        .iter()
        .map(|agent| {
            let params = agent
                .params
                .iter()
                .map(|param| WasmParam {
                    name: param.name.clone(),
                    ty: param.ty.display_name(),
                    kind: wasm_boundary_kind(&param.ty),
                })
                .collect();
            WasmExport {
                name: agent.name.clone(),
                params,
                return_type: agent.return_ty.display_name(),
                return_kind: wasm_boundary_kind(&agent.return_ty),
            }
        })
        .collect();
    let imports = imports
        .iter()
        .map(|import| WasmImport {
            kind: import.kind.label(),
            source_name: import.source_name.clone(),
            import_module: "corvid:host",
            import_name: import.import_name.clone(),
            params: import
                .params
                .iter()
                .map(|(name, ty)| WasmParam {
                    name: name.clone(),
                    ty: ty.display_name(),
                    kind: wasm_boundary_kind(ty),
                })
                .collect(),
            return_type: import.return_ty.display_name(),
            return_kind: wasm_boundary_kind(&import.return_ty),
        })
        .collect();
    let manifest = WasmManifest {
        schema_version: 1,
        module_name: module_name.to_string(),
        target: "wasm32-unknown-unknown",
        exports,
        imports,
        host_capability_abi: "corvid:host scalar imports v1; string ABI v1",
        non_scope: vec![
            // Phase 20n-B v1 lifted the String parameter and return
            // boundary at the agent surface. Host imports stay
            // scalar in v1; the items below remain follow-up work.
            "string ABI on host imports",
            "struct ABI",
            "provenance handles",
            "streaming host callbacks",
        ],
    };
    serde_json::to_string_pretty(&manifest)
        .map_err(|err| WasmCodegenError::unsupported(format!("serialize wasm manifest: {err}")))
}
