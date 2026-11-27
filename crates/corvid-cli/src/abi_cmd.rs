use anyhow::{anyhow, Context, Result};
use corvid_abi::read_embedded_section_from_library;
use corvid_driver::{build_catalog_descriptor_for_source, render_all_pretty};
use std::path::Path;

pub fn run_dump(library: &Path) -> Result<u8> {
    let section = read_embedded_section_from_library(library)
        .with_context(|| format!("read embedded descriptor from `{}`", library.display()))?;
    println!("{}", section.json);
    Ok(0)
}

pub fn run_hash(source: &Path) -> Result<u8> {
    let out = build_catalog_descriptor_for_source(source)?;
    if let Some(json) = out.descriptor_json {
        let hash = corvid_abi::hash_json_str(&json);
        println!("{}", encode_hex(&hash));
        Ok(0)
    } else {
        eprint!("{}", render_all_pretty(&out.diagnostics, source, &out.source));
        Ok(1)
    }
}

pub fn run_verify(library: &Path, expected_hash: &str) -> Result<u8> {
    let section = read_embedded_section_from_library(library)
        .with_context(|| format!("read embedded descriptor from `{}`", library.display()))?;
    let expected = decode_hex(expected_hash)?;
    Ok(if section.sha256 == expected { 0 } else { 2 })
}

fn decode_hex(hex: &str) -> Result<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err(anyhow!("expected 64 hex chars, got {}", hex.len()));
    }
    let bytes = corvid_hex::decode(hex).map_err(|problem| anyhow!(problem))?;
    Ok(bytes.try_into().expect("64 hex digits are 32 bytes"))
}

fn encode_hex(bytes: &[u8]) -> String {
    corvid_hex::encode(bytes)
}
