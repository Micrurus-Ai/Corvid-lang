//! Deploy-manifest shape guard — slice `35V2-P42-E-LR-app-deploy-smoke-ci`.
//!
//! Companion to `serve_smoke.rs`: that test proves `corvid serve` works;
//! this one proves the deploy manifests actually invoke it (and with the
//! full in-container source path, so std imports resolve). It guards
//! against regressing the E0-serve-3 reconciliation — e.g. a stray
//! `corvid run --target=server` or a relative `src/main.cor` creeping
//! back in. Pure file/TOML parsing, no Docker, runs in the normal
//! `cargo test` job.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const APPS: [&str; 5] = [
    "personal_executive_agent",
    "personal_knowledge_agent",
    "finance_operations_agent",
    "customer_support_agent",
    "code_maintenance_agent",
];

#[test]
fn deploy_manifests_invoke_corvid_serve_with_full_path() {
    for app in APPS {
        let deploy = repo_root().join("examples").join("backend").join(app).join("deploy");
        let full_path = format!("examples/backend/{app}/src/main.cor");

        // fly.toml: the `api` process must run `corvid serve <full path>`.
        let fly_text = fs::read_to_string(deploy.join("fly.toml"))
            .unwrap_or_else(|e| panic!("{app}: read fly.toml: {e}"));
        let fly: toml::Value = toml::from_str(&fly_text)
            .unwrap_or_else(|e| panic!("{app}: parse fly.toml: {e}"));
        let api = fly
            .get("processes")
            .and_then(|p| p.get("api"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{app}: fly.toml missing [processes] api"));
        assert!(
            api.contains("corvid serve") && api.contains(&full_path),
            "{app}: fly api process should run `corvid serve {full_path}`, got `{api}`"
        );
        assert!(
            !api.contains("--target=server"),
            "{app}: fly api process still references the non-existent --target=server"
        );

        // Dockerfile CMD + docker-compose command: corvid serve, full path.
        let dockerfile = fs::read_to_string(deploy.join("Dockerfile"))
            .unwrap_or_else(|e| panic!("{app}: read Dockerfile: {e}"));
        assert!(
            dockerfile.contains("\"serve\"") && dockerfile.contains(&full_path),
            "{app}: Dockerfile CMD should `corvid serve {full_path}`"
        );

        let compose = fs::read_to_string(deploy.join("docker-compose.yml"))
            .unwrap_or_else(|e| panic!("{app}: read docker-compose.yml: {e}"));
        assert!(
            compose.contains("- serve") && compose.contains(&full_path),
            "{app}: docker-compose command should `corvid serve {full_path}`"
        );

        // k8s api deployment: serve, full path, no stale --target=server.
        let k8s_api = fs::read_to_string(deploy.join("k8s").join("deployment-api.yaml"))
            .unwrap_or_else(|e| panic!("{app}: read k8s deployment-api.yaml: {e}"));
        assert!(
            k8s_api.contains("- serve") && k8s_api.contains(&full_path),
            "{app}: k8s api command should `corvid serve {full_path}`"
        );
        assert!(
            !k8s_api.contains("--target=server"),
            "{app}: k8s api command still references --target=server"
        );

        // The whole deploy/ tree must be free of the bogus serve command.
        for entry in ["fly.toml", "docker-compose.yml", "Dockerfile"] {
            let text = fs::read_to_string(deploy.join(entry)).unwrap();
            assert!(
                !text.contains("run --target=server"),
                "{app}: {entry} still contains `run --target=server`"
            );
        }
    }
}

/// Slice 35V2-P42-F-LR: every reference app has a per-app comparison
/// file under benches/comparisons/ with the required skeleton sections.
#[test]
fn each_reference_app_has_a_benchmark_comparison_file() {
    use std::fs;
    for app in APPS {
        let path = repo_root()
            .join("benches")
            .join("comparisons")
            .join(format!("{app}.md"));
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{app}: read {}: {e}", path.display()));
        for section in [
            "## Headline",
            "## Reproduce",
            "## Governance line count",
            "## What Corvid wins on",
            "## What Corvid does not claim",
        ] {
            assert!(
                text.contains(section),
                "{app}: comparison file missing `{section}`"
            );
        }
        // Honesty rule: baseline cells are explicitly bounty-open, not
        // fabricated numbers.
        assert!(
            text.contains("bounty-open"),
            "{app}: comparison file must mark unmeasured baselines `bounty-open`"
        );
    }
}

/// v1.0 launch criterion `ROADMAP.md` L47 — every reference app
/// deploys via the Phase 43 packaging path (`corvid deploy package`).
///
/// For each of the 5 apps, run `corvid deploy package <app> --out <dir>`
/// and assert the 9 artifacts the packager promises ALL land on disk:
/// Dockerfile, oci-labels.json (OCI metadata), env.schema.json,
/// health.json, migrate.sh, startup-checks.md,
/// build-attestation.dsse.json (43O signed-attestation chain anchor),
/// sbom.spdx.json (43M SPDX SBOM), VERIFY.md.
///
/// The packager output isn't actually deployed in this test — running
/// the resulting image would require Docker daemon access which the
/// cargo-test sandbox doesn't have. This gate proves "the packaging
/// path produces a complete, well-formed deploy artifact for every
/// reference app" — the on-disk artifact is what an operator runs
/// `docker build` against, and a missing artifact would surface at
/// build time. The smoke-deploy CI (`app-deploy-smoke.yml`) already
/// proves the resulting binary serves correctly via `corvid serve`;
/// this gate closes the packager-side of the deploy story.
#[test]
fn every_reference_app_produces_a_complete_deploy_package() {
    use std::process::Command;

    let corvid = PathBuf::from(env!("CARGO_BIN_EXE_corvid"));

    for app in APPS {
        let app_dir = repo_root().join("examples").join("backend").join(app);
        let out_dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{app}: tempdir: {e}"));

        // Same dev signing key as `reference_apps.rs::deploy_package_smoke`
        // — the deploy attestation needs a key to sign with; this 32-byte
        // hex seed is the documented test key the existing tests use.
        let status = Command::new(&corvid)
            .arg("deploy")
            .arg("package")
            .arg(&app_dir)
            .arg("--out")
            .arg(out_dir.path())
            .env(
                "CORVID_DEPLOY_SIGNING_KEY",
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            )
            .current_dir(repo_root())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .status()
            .unwrap_or_else(|e| panic!("{app}: spawn corvid deploy package: {e}"));
        assert!(
            status.success(),
            "{app}: `corvid deploy package` exited non-zero ({:?})",
            status.code()
        );

        // Every artifact the packager promises (see
        // `crates/corvid-cli/src/deploy_cmd.rs::run_package`).
        let required: &[&str] = &[
            "Dockerfile",
            "oci-labels.json",
            "env.schema.json",
            "health.json",
            "migrate.sh",
            "startup-checks.md",
            "build-attestation.dsse.json",
            "sbom.spdx.json",
            "VERIFY.md",
        ];
        for artifact in required {
            let path = out_dir.path().join(artifact);
            let meta = fs::metadata(&path).unwrap_or_else(|e| {
                panic!(
                    "{app}: required deploy artifact `{artifact}` missing at `{}`: {e}",
                    path.display()
                )
            });
            assert!(
                meta.len() > 0,
                "{app}: deploy artifact `{artifact}` is empty"
            );
        }

        // Spot-check the structured artifacts parse cleanly.
        let oci_text = fs::read_to_string(out_dir.path().join("oci-labels.json"))
            .unwrap_or_else(|e| panic!("{app}: read oci-labels.json: {e}"));
        let _: serde_json::Value = serde_json::from_str(&oci_text)
            .unwrap_or_else(|e| panic!("{app}: oci-labels.json is not valid JSON: {e}"));

        let sbom_text = fs::read_to_string(out_dir.path().join("sbom.spdx.json"))
            .unwrap_or_else(|e| panic!("{app}: read sbom.spdx.json: {e}"));
        let _: serde_json::Value = serde_json::from_str(&sbom_text)
            .unwrap_or_else(|e| panic!("{app}: sbom.spdx.json is not valid JSON: {e}"));

        let att_text =
            fs::read_to_string(out_dir.path().join("build-attestation.dsse.json"))
                .unwrap_or_else(|e| panic!("{app}: read build-attestation.dsse.json: {e}"));
        let _: serde_json::Value = serde_json::from_str(&att_text).unwrap_or_else(|e| {
            panic!(
                "{app}: build-attestation.dsse.json is not valid JSON: {e}"
            )
        });
    }
}
