//! Package add/install resolution.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use semver::Version;

use crate::import_integrity::sha256_hex;
use crate::modules::summarize_module_source;
use crate::package_lock::{
    load_or_empty_at, lock_path_for_project, remove_packages_by_name, upsert_package,
    write_package_lock, LockedPackage,
};
use crate::package_manifest::upsert_dependency;
use crate::package_policy::{load_package_policy, package_policy_violation};
use crate::package_version::{normalize_version, parse_package_spec, PackageSpec};

use super::{
    fetch_bytes, load_registry_index, verify_package_signature, AddPackageOutcome, RegistryIndex,
    RegistryPackage,
};

pub fn add_package(
    spec: &str,
    project_dir: &Path,
    registry: Option<&str>,
) -> Result<AddPackageOutcome> {
    let spec = parse_package_spec(spec)?;
    let mut guard = super::mutation_lock(project_dir)?;
    let _writer = guard.write()?;
    add_resolved(&spec, project_dir, registry, None)
}

pub(super) fn add_resolved(
    spec: &PackageSpec,
    project_dir: &Path,
    registry: Option<&str>,
    expected_uri: Option<&str>,
) -> Result<AddPackageOutcome> {
    let registry_location =
        match resolve_registry_location(registry, std::env::var("CORVID_PACKAGE_REGISTRY").ok()) {
            Ok(location) => location,
            Err(reason) => return Ok(AddPackageOutcome::Rejected { reason }),
        };
    let index = load_registry_index(&registry_location)?;
    let Some(selected) = select_package(&index, spec)? else {
        return Ok(AddPackageOutcome::Rejected {
            reason: format!(
                "registry `{registry_location}` has no package `{}` matching the requested version",
                spec.name
            ),
        });
    };

    let uri = selected
        .uri
        .clone()
        .unwrap_or_else(|| format!("corvid://{}/v{}", selected.name, selected.version));
    if selected.name != spec.name || expected_uri.is_some_and(|expected| expected != uri) {
        anyhow::bail!("registry returned an incompatible package identity: {uri}");
    }
    let url = if selected.url.starts_with("https://") || selected.url.starts_with("http://") {
        selected.url.clone()
    } else if registry_location.starts_with("https://") || registry_location.starts_with("http://")
    {
        return Ok(AddPackageOutcome::Rejected {
            reason: "a remote registry must supply an HTTP(S) artifact URL".into(),
        });
    } else if selected.url.starts_with("file://") {
        selected.url.clone()
    } else {
        let index_path = Path::new(&registry_location);
        let base = if index_path.is_dir() {
            index_path
        } else {
            index_path.parent().unwrap_or(Path::new("."))
        };
        let path = base.join(&selected.url).canonicalize()?;
        url::Url::from_file_path(&path)
            .map_err(|_| anyhow!("invalid artifact path {}", path.display()))?
            .to_string()
    };
    let bytes = fetch_bytes(&url)
        .with_context(|| format!("failed to fetch package source `{}`", selected.url))?;
    let actual = sha256_hex(&bytes);
    if !actual.eq_ignore_ascii_case(&selected.sha256) {
        return Ok(AddPackageOutcome::Rejected {
            reason: format!(
                "package `{}`@{} failed registry hash verification: expected sha256:{}, actual sha256:{actual}",
                selected.name, selected.version, selected.sha256
            ),
        });
    }
    let source = String::from_utf8(bytes).with_context(|| {
        format!(
            "package `{}`@{} is not valid UTF-8",
            selected.name, selected.version
        )
    })?;
    let summary = summarize_module_source(&source).map_err(|message| {
        anyhow!(
            "package `{}`@{} failed semantic summary build: {message}",
            selected.name,
            selected.version
        )
    })?;
    let policy = load_package_policy(project_dir)?;
    if let Some(reason) = package_policy_violation(&summary, &policy, selected.signature.as_deref())
    {
        return Ok(AddPackageOutcome::Rejected { reason });
    }
    // Slice 33R4b: verify the per-version detached signature against
    // the index's root signing key. Both must be present for a sig
    // check to fire; missing either is treated as "no signature
    // declared," gated separately by the package-policy check above.
    if let (Some(signature), Some(root_key)) = (&selected.signature, &index.signing_key) {
        if let Err(message) = verify_package_signature(selected, &summary, signature, root_key) {
            return Ok(AddPackageOutcome::Rejected { reason: message });
        }
    }

    crate::package_cache::store(
        &crate::package_cache::artifact(project_dir, &actual),
        source.as_bytes(),
    )?;
    let lockfile = lock_path_for_project(project_dir);
    let mut lock = load_or_empty_at(&lockfile).map_err(|message| anyhow!(message))?;
    remove_packages_by_name(&mut lock, &spec.name);
    upsert_package(
        &mut lock,
        LockedPackage {
            uri: uri.clone(),
            url,
            sha256: selected.sha256.to_ascii_lowercase(),
            registry: selected
                .registry
                .clone()
                .or_else(|| Some(registry_location.clone())),
            signature: selected.signature.clone(),
            semantic_summary: Some(summary.clone()),
        },
    );
    if expected_uri.is_none() || crate::package_manifest::dependency(project_dir, &spec.name)?.is_none() {
        upsert_dependency(
            project_dir,
            &spec.name,
            &spec.raw_requirement,
            Some(registry_location.as_str()),
        )?;
    }
    write_package_lock(&lockfile, &lock).map_err(|message| anyhow!(message))?;

    Ok(AddPackageOutcome::Added {
        uri,
        version: selected.version.clone(),
        lockfile,
        exports: summary.exports.len(),
    })
}

pub(super) fn resolve_registry_location(
    registry: Option<&str>,
    env_registry: Option<String>,
) -> std::result::Result<String, String> {
    registry
        .map(str::to_string)
        .or(env_registry)
        .ok_or_else(|| {
            "no hosted Corvid package registry runs yet; pass --registry <index.json|dir|url> \
             or set CORVID_PACKAGE_REGISTRY to a local or self-hosted registry index"
                .to_string()
        })
}

pub(super) fn select_package<'a>(
    index: &'a RegistryIndex,
    spec: &PackageSpec,
) -> Result<Option<&'a RegistryPackage>> {
    // Slice 33R4b: registry index is now nested under
    // `packages.{name}.versions.{version}`. Walk the package-name
    // bucket directly instead of scanning a flat array — O(1) name
    // lookup + linear over versions of the matching package only.
    let Some(entry) = index.packages.get(&spec.name) else {
        return Ok(None);
    };
    let mut candidates = Vec::new();
    for package in entry.versions.values() {
        let version = Version::parse(&normalize_version(&package.version)).with_context(|| {
            format!(
                "registry package `{}` has invalid version `{}`",
                package.name, package.version
            )
        })?;
        if spec.requirement.matches(&version) {
            candidates.push((version, package));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(candidates.pop().map(|(_, package)| package))
}
