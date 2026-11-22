use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Pack source-backed HTTP routes and ahead-of-time compiled browser handlers.
pub(super) fn build(entry: &Path) -> Result<u8> {
    let entry = entry.canonicalize()?;
    if !entry.ends_with("src/main.cor") {
        bail!("web bundles use a project's src/main.cor entrypoint");
    }
    let root = entry.parent().unwrap().parent().unwrap();
    let stage = tempfile::tempdir()?;
    let mut files = Vec::new();
    collect(root, Path::new("src"), &["cor", "py"], &mut files)?;
    if root.join("migrations").exists() {
        collect(root, Path::new("migrations"), &["sql"], &mut files)?;
    }
    for name in ["corvid.toml", "Corvid.lock", "tools.py"] {
        if root.join(name).is_file() {
            files.push(name.into());
        }
    }
    let locked = corvid_driver::locked_artifact_paths(root)?;
    for relative in &locked {
        if root.join(relative).try_exists()? {
            files.push(relative.clone());
        }
    }
    for relative in &files {
        let path = root.join(relative);
        if !path.canonicalize()?.starts_with(root) {
            bail!(
                "bundle input {} is outside the application",
                relative.display()
            );
        }
        let to = stage.path().join(relative);
        fs::create_dir_all(to.parent().unwrap())?;
        fs::copy(path, to)?;
    }

    let main = stage.path().join("src/main.cor");
    let source = fs::read_to_string(&main)?;
    let config = corvid_driver::load_corvid_config_for(&main);
    let ir = corvid_driver::compile_to_ir_with_config_at_path(&source, &main, config.as_ref())
        .map_err(|errors| {
            anyhow::anyhow!(
                "{}",
                corvid_driver::render_all_pretty(&errors, &entry, &source)
            )
        })?;
    if ir.servers.is_empty() {
        bail!("a web bundle needs a server declaration in src/main.cor");
    }
    let events = stage.path().join("src/events.cor");
    if events.is_file() {
        let build = corvid_driver::build_wasm_to_disk(&events)?;
        let (Some(js), Some(wasm)) = (build.js_loader_path, build.wasm_path) else {
            eprint!(
                "{}",
                corvid_driver::render_all_pretty(
                    &build.diagnostics,
                    &root.join("src/events.cor"),
                    &build.source
                )
            );
            return Ok(1);
        };
        for asset in [js, wasm] {
            files.push(asset.strip_prefix(stage.path())?.to_owned());
        }
    }
    // Compilation can populate the staged cache when a locked package is first fetched.
    for relative in locked {
        let path = stage.path().join(&relative);
        if path.try_exists()? {
            let digest = hex::encode(Sha256::digest(fs::read(&path)?));
            if relative.file_stem().and_then(|s| s.to_str()) != Some(digest.as_str()) {
                bail!("locked package digest mismatch for {}", relative.display());
            }
            files.push(relative);
        }
    }
    files.sort();
    files.dedup();
    let target = root.join("target");
    fs::create_dir_all(&target)?;
    let output = target.join("web.tar");
    let mut temp = tempfile::NamedTempFile::new_in(&target)?;
    {
        let mut archive = tar::Builder::new(temp.as_file_mut());
        for relative in files {
            let file = fs::File::open(stage.path().join(&relative))?;
            let mut header = tar::Header::new_gnu();
            header.set_size(file.metadata()?.len());
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, relative, file)?;
        }
        archive.finish()?;
    }
    temp.persist(&output)
        .context("publish completed local web archive")?;
    println!("built: {} -> {}", entry.display(), output.display());
    Ok(0)
}

fn collect(root: &Path, dir: &Path, extensions: &[&str], files: &mut Vec<PathBuf>) -> Result<()> {
    let path = root.join(dir);
    if path.symlink_metadata()?.file_type().is_symlink() {
        bail!("cannot package linked directory {}", dir.display());
    }
    for item in fs::read_dir(path)? {
        let item = item?;
        let relative = dir.join(item.file_name());
        let kind = item.file_type()?;
        if kind.is_symlink() {
            bail!("cannot package linked input {}", relative.display());
        }
        if kind.is_dir() {
            collect(root, &relative, extensions, files)?;
        } else if kind.is_file()
            && relative
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.contains(&e))
        {
            files.push(relative);
        }
    }
    Ok(())
}
