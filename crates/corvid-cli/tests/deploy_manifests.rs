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
