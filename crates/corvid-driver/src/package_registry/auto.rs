//! Development-only recovery for versioned imports missing from the project lock.
use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use corvid_syntax::{lex, parse_file};
use semver::Version;

use crate::modules::{build_module_resolution, ModuleLoadError};
use crate::package_lock::{load_or_empty_at, lock_path_for_project};
use crate::package_manifest::dependency;
use crate::package_version::{parse_package_spec, parse_version_requirement};

use super::{add::add_resolved, mutation_lock, AddPackageOutcome};

pub(crate) fn resolve_missing_for_run(path: &Path, source: &str) -> Result<bool> {
    let Ok(tokens) = lex(source) else {
        return Ok(false);
    };
    let (file, errors) = parse_file(&tokens);
    if !errors.is_empty() {
        return Ok(false);
    }
    let path = path.canonicalize()?;
    let Some(project) = path
        .ancestors()
        .skip(1)
        .find(|p| p.join("corvid.toml").is_file())
    else {
        return Ok(false);
    };
    let mut attempted = BTreeSet::new();
    loop {
        let (_, errors) = build_module_resolution(&file, &path);
        let missing: BTreeSet<_> = errors
            .into_iter()
            .filter_map(|error| match error {
                ModuleLoadError::PackageLockMissing { requested, .. }
                | ModuleLoadError::PackageNotLocked { requested, .. } => Some(requested),
                _ => None,
            })
            .collect();
        if missing.is_empty() {
            return Ok(!attempted.is_empty());
        }
        let mut guard = mutation_lock(project)?;
        let _writer = guard.write()?;
        for uri in missing {
            if !attempted.insert(uri.clone()) {
                bail!("could not resolve {uri} in the project lock");
            }
            let lock = load_or_empty_at(&lock_path_for_project(project)).map_err(|e| anyhow!(e))?;
            if lock.find(&uri).is_some() {
                continue; // Another run finished this import before we acquired the lock.
            }
            let (name, release) = uri
                .strip_prefix("corvid://")
                .and_then(|s| s.rsplit_once("/v"))
                .with_context(|| format!("expected a versioned package import, found {uri}"))?;
            let version = Version::parse(release).with_context(|| {
                format!("automatic resolution requires an exact version in {uri}")
            })?;
            if lock
                .package
                .iter()
                .any(|p| p.uri.starts_with(&format!("corvid://{name}/")))
            {
                bail!("{uri} conflicts with a locked version; update the dependency explicitly");
            }
            let declared = dependency(project, name)?;
            if let Some(dep) = &declared {
                if !parse_version_requirement(&dep.version)?.matches(&version) {
                    bail!(
                        "{uri} conflicts with the manifest requirement {}",
                        dep.version
                    );
                }
            }
            if crate::package_cache::offline() {
                bail!("cannot resolve a new import {uri} offline; restore the committed lockfile or run online once");
            }
            let registry = declared
                .as_ref()
                .and_then(|d| d.registry.clone())
                .or_else(|| std::env::var("CORVID_PACKAGE_REGISTRY").ok())
                .unwrap_or_else(|| "corvid:catalog".into());
            let registry = if registry == "corvid:catalog"
                || registry.contains("://")
                || Path::new(&registry).is_absolute()
            {
                registry
            } else {
                project.join(registry).to_string_lossy().into_owned()
            };
            let spec = parse_package_spec(&format!("{name}@={version}"))?;
            match add_resolved(&spec, project, Some(&registry), Some(&uri))? {
                AddPackageOutcome::Rejected { reason } => bail!("{uri}: {reason}"),
                AddPackageOutcome::Added { .. } => eprintln!("locked {uri}"),
            }
        }
    }
}
