use anyhow::{bail, Result};
use std::{fs, io::ErrorKind, path::Path};

/// List browser files explicitly placed in a web project's public assets directory.
pub(crate) fn list(root: &Path) -> Result<Vec<(String, &'static str)>> {
    let directory = root.join("assets");
    match directory.symlink_metadata() {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if !metadata.is_dir() => {
            bail!("expected a directory at {}", directory.display())
        }
        Ok(_) => {}
    }
    let mut assets = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            bail!(
                "expected a regular asset file at {}",
                entry.path().display()
            );
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("asset names must be UTF-8"))?;
        if !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            || name.starts_with('.')
        {
            bail!("invalid public asset filename {name}");
        }
        let mime = match name.rsplit_once('.').map(|(_, extension)| extension) {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("ttf") => "font/ttf",
            Some("woff") => "font/woff",
            Some("woff2") => "font/woff2",
            Some("txt") => "text/plain; charset=utf-8",
            _ => bail!("unsupported public asset type for {name}"),
        };
        assets.push((name, mime));
    }
    Ok(assets)
}
