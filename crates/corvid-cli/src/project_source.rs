//! Default source-file resolution for project-local commands.

use anyhow::{Context, Result};
use std::path::PathBuf;

pub(crate) fn resolve_project_source(file: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(file) = file {
        return Ok(file);
    }

    let cwd = std::env::current_dir().context("resolve current directory")?;
    let manifest = cwd.join("corvid.toml");
    let source = cwd.join("src").join("main.cor");
    if !manifest.is_file() {
        anyhow::bail!(
            "no project manifest at `{}`; pass a source file or run from a Corvid project directory",
            manifest.display()
        );
    }
    if !source.is_file() {
        anyhow::bail!(
            "missing project entry file `{}`; create it or pass a source file",
            source.display()
        );
    }
    Ok(source)
}
