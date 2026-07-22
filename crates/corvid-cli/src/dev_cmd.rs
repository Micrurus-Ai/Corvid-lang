//! `corvid dev` — serve the universal contract-driven console
//! (slice 51m).
//!
//! The console HTML is rendered from the app's Application Contract by
//! `corvid_abi::dev_console::emit_dev_console`. This command builds the
//! contract, then either writes the HTML (`--out`) or serves it over a
//! tiny blocking HTTP server that exposes three GET routes:
//!
//! - `/`                       the console page
//! - `/_corvid/contract.json`  the Application Contract
//! - `/_corvid/ai.json`        the AI-native metadata
//!
//! Agent execution from the console targets a backend base URL set in
//! the page (a running `corvid serve`), so this server stays a static
//! file server — no runtime, no async, nothing to go wrong in a demo.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

use anyhow::{Context, Result};

pub fn cmd_dev(file: Option<&Path>, listen: &str, out: Option<&Path>) -> Result<u8> {
    let Some(contract) = build_contract(file)? else {
        return Ok(1);
    };
    let ai = corvid_abi::corvid_ai::emit_corvid_ai(&contract);
    let html = corvid_abi::dev_console::emit_dev_console(&contract, &ai);

    if let Some(out) = out {
        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        std::fs::write(out, &html)
            .with_context(|| format!("writing console HTML to `{}`", out.display()))?;
        println!("wrote dev console: {} ({} bytes)", out.display(), html.len());
        return Ok(0);
    }

    let contract_json = serde_json::to_string_pretty(&contract).unwrap_or_default();
    let ai_json = serde_json::to_string_pretty(&ai).unwrap_or_default();

    let listener = TcpListener::bind(listen)
        .with_context(|| format!("binding dev console to `{listen}`"))?;
    let addr = listener.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| listen.to_string());
    println!("corvid dev console serving at http://{addr}");
    println!("  set the backend URL in the console to a running `corvid serve` to execute agents");

    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                if let Err(e) = handle(&mut s, &html, &contract_json, &ai_json) {
                    eprintln!("dev console connection error: {e}");
                }
            }
            Err(e) => eprintln!("dev console accept error: {e}"),
        }
    }
    Ok(0)
}

/// Serve one request. Reads the request line, ignores the rest of the
/// headers, and routes on the path.
fn handle(stream: &mut TcpStream, html: &str, contract_json: &str, ai_json: &str) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Drain remaining headers so the client is happy (no body handling —
    // this server answers only GETs).
    let mut header = String::new();
    loop {
        header.clear();
        let n = reader.read_line(&mut header)?;
        if n == 0 || header == "\r\n" || header == "\n" {
            break;
        }
    }

    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, content_type, body) = match path {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", html),
        "/_corvid/contract.json" => ("200 OK", "application/json", contract_json),
        "/_corvid/ai.json" => ("200 OK", "application/json", ai_json),
        _ => ("404 Not Found", "text/plain; charset=utf-8", "not found"),
    };

    write_response(stream, status, content_type, body)
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    // Best-effort read to end so some clients don't RST.
    let _ = stream.read(&mut [0u8; 0]);
    Ok(())
}

/// Build the Application Contract for `file` (default the project's
/// `src/main.cor`). Mirrors `contract_cmd::build_contract`.
fn build_contract(
    file: Option<&Path>,
) -> Result<Option<corvid_abi::app_contract::ApplicationContract>> {
    let source_path = match file {
        Some(f) => f.to_path_buf(),
        None => crate::project_source::resolve_project_source(None)
            .context("no source file given and no src/main.cor found")?,
    };
    let source = std::fs::read_to_string(&source_path)
        .with_context(|| format!("cannot read `{}`", source_path.display()))?;
    let config = corvid_driver::load_corvid_config_for(&source_path);
    let generated_at =
        std::env::var("CORVID_BUILD_DATE").unwrap_or_else(|_| "unknown".to_string());

    match corvid_driver::compile_to_application_contract_with_config(
        &source,
        &source_path.display().to_string(),
        &generated_at,
        config.as_ref(),
    ) {
        Ok(contract) => Ok(Some(contract)),
        Err(diags) => {
            eprint!(
                "{}",
                corvid_driver::render_all_pretty(&diags, &source_path, &source)
            );
            Ok(None)
        }
    }
}
