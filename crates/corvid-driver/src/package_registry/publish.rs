//! Package publishing into a registry index.

use anyhow::{anyhow, Context, Result};
use semver::Version;

use crate::import_integrity::sha256_hex;
use crate::modules::summarize_module_source;
use crate::package_version::{normalize_version, validate_package_name};

use super::{
    sign_package, PublishPackageOptions, PublishPackageOutcome, RegistryIndex, RegistryPackage,
};

pub fn publish_package(options: PublishPackageOptions<'_>) -> Result<PublishPackageOutcome> {
    validate_package_name(options.name)?;
    let version = Version::parse(&normalize_version(options.version))
        .with_context(|| format!("invalid package version `{}`", options.version))?;
    std::fs::create_dir_all(options.out_dir)
        .with_context(|| format!("create registry output dir `{}`", options.out_dir.display()))?;
    let source = std::fs::read_to_string(options.source)
        .with_context(|| format!("read package source `{}`", options.source.display()))?;
    let summary = summarize_module_source(&source)
        .map_err(|message| anyhow!("package source failed semantic summary build: {message}"))?;
    let artifact_name = format!(
        "{}-{}.cor",
        options
            .name
            .trim_start_matches('@')
            .replace(['/', '\\'], "-"),
        version
    );
    let artifact = options.out_dir.join(&artifact_name);
    std::fs::write(&artifact, source.as_bytes())
        .with_context(|| format!("write package artifact `{}`", artifact.display()))?;
    let sha256 = sha256_hex(source.as_bytes());
    let uri = format!("corvid://{}/v{}", options.name, version);
    let url = format!(
        "{}/{}",
        options.url_base.trim_end_matches('/'),
        artifact_name
    );
    let mut package = RegistryPackage {
        name: options.name.to_string(),
        version: version.to_string(),
        uri: Some(uri.clone()),
        url,
        sha256: sha256.clone(),
        registry: None,
        signature: None,
        semantic_summary: Some(summary),
    };
    // Slice 33R4b: detached-sig signing returns (detached sig hex,
    // root-key fingerprint). Detached sig goes into the per-version
    // `signature`; fingerprint lives once at the index root and is
    // upsert-checked on re-publish so a single index can't end up
    // with multiple registry keys (that would be a real footgun).
    let (detached_sig_hex, fingerprint) =
        sign_package(&package, options.signing_seed_hex, options.key_id)?;
    package.signature = Some(detached_sig_hex);

    let index_path = options.out_dir.join("index.json");
    let mut index = match std::fs::read_to_string(&index_path) {
        Ok(source) => serde_json::from_str::<RegistryIndex>(&source)
            .with_context(|| format!("parse existing registry index `{}`", index_path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => RegistryIndex::default(),
        Err(err) => return Err(anyhow!("read `{}`: {err}", index_path.display())),
    };
    if let Some(existing_key) = &index.signing_key {
        if existing_key != &fingerprint {
            anyhow::bail!(
                "registry index `{}` is already signed by `{existing_key}`; \
                 publishing with a different signing key (`{fingerprint}`) is refused — \
                 either reuse the original key, or publish to a new index path",
                index_path.display()
            );
        }
    } else {
        index.signing_key = Some(fingerprint);
    }
    upsert_registry_package(&mut index, package);
    let index_source = serde_json::to_string_pretty(&index)
        .with_context(|| format!("serialize registry index `{}`", index_path.display()))?;
    std::fs::write(&index_path, index_source)
        .with_context(|| format!("write registry index `{}`", index_path.display()))?;

    Ok(PublishPackageOutcome {
        uri,
        index: index_path,
        artifact,
        sha256,
    })
}

fn upsert_registry_package(index: &mut RegistryIndex, package: RegistryPackage) {
    let name = package.name.clone();
    let version = package.version.clone();
    let entry = index.packages.entry(name).or_default();
    entry.versions.insert(version.clone(), package);
    // Slice 33R4b: `latest` tracks the highest published semver in
    // the entry. Recompute on every upsert; small fixed cost,
    // avoids drift.
    let latest = entry
        .versions
        .keys()
        .filter_map(|v| Version::parse(&normalize_version(v)).ok().map(|sv| (sv, v.clone())))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, raw)| raw);
    entry.latest = latest;
}
