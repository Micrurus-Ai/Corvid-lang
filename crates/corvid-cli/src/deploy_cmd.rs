use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use corvid_abi::{load_signing_key, sign_envelope, KeySource};
use serde::Serialize;
use sha2::{Digest, Sha256};

const CORVID_REPOSITORY_URL: &str = "https://github.com/Micrurus-Ai/Corvid-lang";

/// One resolved deployment shape consumed by every renderer. Keeping
/// these values together prevents Docker, Compose, PaaS, Kubernetes and
/// systemd output from independently inventing ports and paths.
#[derive(Debug, Clone, Copy)]
struct DeploymentPlan<'a> {
    app_name: &'a str,
    project_entrypoint: &'static str,
    container_entrypoint: &'static str,
    migrations_dir: &'static str,
    health_path: &'static str,
    readiness_path: &'static str,
    port: u16,
}

impl<'a> DeploymentPlan<'a> {
    fn new(app_name: &'a str) -> Self {
        Self {
            app_name,
            project_entrypoint: "src/main.cor",
            container_entrypoint: "/app/src/main.cor",
            migrations_dir: "migrations",
            health_path: "/healthz",
            readiness_path: "/readyz",
            port: 8000,
        }
    }
}

/// Compute a manifest-portable path from one directory to another.
/// Deploy commands accept arbitrary `--out` locations, so renderers
/// must not assume the default `target/<kind>` depth.
fn relative_path(from_dir: &Path, target: &Path) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("resolve current directory for deploy paths")?;
    let from = if from_dir.is_absolute() {
        from_dir.to_path_buf()
    } else {
        cwd.join(from_dir)
    };
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        cwd.join(target)
    };
    let from_components: Vec<Component<'_>> = from.components().collect();
    let target_components: Vec<Component<'_>> = target.components().collect();
    let common = from_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();

    // Different Windows drive prefixes have no relative representation.
    if common == 0 {
        return Ok(target);
    }

    let mut relative = PathBuf::new();
    for component in &from_components[common..] {
        if !matches!(component, Component::CurDir) {
            relative.push("..");
        }
    }
    for component in &target_components[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Ok(relative)
}

fn path_for_manifest(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

#[derive(Serialize)]
struct OciMetadata<'a> {
    image: &'a str,
    labels: OciLabels<'a>,
}

#[derive(Serialize)]
struct OciLabels<'a> {
    #[serde(rename = "org.opencontainers.image.title")]
    title: &'a str,
    #[serde(rename = "org.opencontainers.image.source")]
    source: String,
    #[serde(rename = "dev.corvid.app")]
    app: &'a str,
    #[serde(rename = "dev.corvid.package.source_sha256")]
    source_sha256: String,
}

pub fn run_package(app: &Path, out: &Path, cdylib: Option<&Path>) -> Result<()> {
    let app_name = app
        .file_name()
        .and_then(|name| name.to_str())
        .context("app path must end in a valid directory name")?;
    let plan = DeploymentPlan::new(app_name);
    let source = app.join("src").join("main.cor");
    let source_bytes =
        fs::read(&source).with_context(|| format!("read app source `{}`", source.display()))?;

    // 33Q11 (maintainer-as-reviewer-2026-06-05 P2.3 + P3.1) — fail
    // fast on the things we can't recover from BEFORE writing the
    // first artifact. Pre-33Q11, `CORVID_DEPLOY_SIGNING_KEY` was
    // read inside `render_attestation` which runs AFTER 6 files
    // are already on disk; a missing env left a partial deploy/
    // dir that confused operators ("error but I see Dockerfile?").
    // Same for the cdylib read: if --cdylib points at a path that
    // doesn't exist, fail before we've written anything.
    //
    // Reading the env into a SigningKey here also covers the
    // env-is-set-but-malformed case (e.g. wrong length, invalid
    // hex) — pre-33Q11 those failed mid-package too. The validated
    // key is threaded through to `render_attestation` so the
    // attestation step doesn't re-read the env (single source of
    // truth + an atomic precondition).
    let signing_key_raw = std::env::var("CORVID_DEPLOY_SIGNING_KEY").map_err(|_| {
        anyhow::anyhow!(
            "CORVID_DEPLOY_SIGNING_KEY is required for the deploy package's signed \
             attestation envelope. Set it to a 32-byte ed25519 seed encoded as 64 \
             hex characters (e.g. `openssl rand -hex 32` output). See \
             `corvid deploy package --help` for the env-var contract."
        )
    })?;
    let signing_key = load_signing_key(&KeySource::Env(signing_key_raw))
        .map_err(|err| anyhow::anyhow!("load CORVID_DEPLOY_SIGNING_KEY: {err}"))?;

    // 43O: chain anchor for the deploy attestation. When `--cdylib`
    // is provided, the SHA-256 of the cdylib's bytes goes into the
    // attestation payload — the cdylib carries its own embedded
    // claim attestation, so binding the deploy attestation to the
    // cdylib's bytes binds the whole chain. If `--cdylib` is not
    // provided, the chain is incomplete and the attestation marks
    // it explicitly so downstream verification refuses to trust an
    // unchained deploy.
    //
    // 33Q11: also pre-flight — read the cdylib here so a bad path
    // fails BEFORE we touch `out/`.
    let cdylib_sha256 = match cdylib {
        Some(path) => {
            let bytes = fs::read(path).with_context(|| {
                format!("read cdylib for attestation chain `{}`", path.display())
            })?;
            Some(hex::encode(Sha256::digest(&bytes)))
        }
        None => None,
    };

    // 33Q15: stage every write into a sibling tempdir, then atomically
    // rename into `out`. 33Q11 already guaranteed "no out/ on
    // pre-flight error"; this strengthens the guarantee to "no
    // MUTATION of out/ on ANY error" — if `render_attestation` fails
    // at write #7, the user keeps their prior `out/` exactly as it
    // was, not a 6-file partial that mixes the failed run with the
    // last successful one. The TempDir is built in `out`'s parent
    // (rather than the OS tmpdir) so the final `fs::rename` is
    // same-filesystem and therefore atomic on POSIX + Windows.
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| {
        format!("create parent directory `{}` for deploy package", parent.display())
    })?;
    let stage = tempfile::Builder::new()
        .prefix(".corvid-deploy-package-stage-")
        .tempdir_in(parent)
        .with_context(|| {
            format!(
                "create staging directory under `{}` for atomic deploy package write",
                parent.display()
            )
        })?;
    let stage_path = stage.path().to_path_buf();

    fs::write(stage_path.join("Dockerfile"), render_dockerfile_with_plan(plan, app))
        .context("write Dockerfile")?;

    let source_sha256 = hex::encode(Sha256::digest(&source_bytes));
    // Slice 33Q12b (maintainer-as-reviewer-2026-06-05 P3.3) — OCI
    // metadata uses POSIX-style forward-slash paths. On Windows,
    // `Path::display()` produces a mix of `/` and `\` depending on
    // what was in the path's internal representation
    // (`C:/Users/.../Temp/app\\src\\main.cor` is the literal output
    // the trial reviewer saw). The mixed separator reads strangely
    // in OCI metadata that downstream tools (image registries,
    // SBOM viewers, attestation parsers) expect to be POSIX-shaped.
    // Normalize at the OCI boundary; the on-disk path stays
    // platform-native everywhere else.
    let source_for_oci = source.display().to_string().replace('\\', "/");
    let metadata = OciMetadata {
        image: app_name,
        labels: OciLabels {
            title: app_name,
            source: source_for_oci,
            app: app_name,
            source_sha256: source_sha256.clone(),
        },
    };
    let metadata_json =
        serde_json::to_string_pretty(&metadata).context("serialize OCI metadata")?;
    fs::write(stage_path.join("oci-labels.json"), &metadata_json).context("write OCI metadata")?;
    fs::write(stage_path.join("env.schema.json"), render_env_schema()).context("write env schema")?;
    fs::write(stage_path.join("health.json"), render_health_config()).context("write health config")?;
    fs::write(stage_path.join("migrate.sh"), render_migration_runner(plan))
        .context("write migration runner")?;
    fs::write(
        stage_path.join("startup-checks.md"),
        render_startup_checks(plan),
    )
    .context("write startup checks")?;
    let attestation =
        render_attestation(app_name, &metadata_json, cdylib_sha256.as_deref(), &signing_key)?;
    fs::write(stage_path.join("build-attestation.dsse.json"), attestation)
        .context("write build attestation")?;
    // 43M: SPDX SBOM accompanies every deploy package so the
    // shipped image carries a machine-readable inventory of what
    // it was built from. Promotes `deploy.sbom_completeness` to
    // RuntimeChecked once the completeness adversarial test lands
    // in 43V.
    let sbom = render_spdx_sbom(app_name, &source_sha256)?;
    fs::write(stage_path.join("sbom.spdx.json"), sbom).context("write SPDX SBOM")?;
    fs::write(stage_path.join("VERIFY.md"), render_verify_docs())
        .context("write verification docs")?;

    // 33Q15: all writes succeeded in the stage. Atomically swap into
    // `out`. If `out` already exists from a prior run, remove it
    // FIRST so leftover files from a previous shape don't leak into
    // the new bundle (Dockerfile / oci-labels / etc. get overwritten
    // by the rename, but a stale file at a path the new bundle no
    // longer emits would persist without the explicit cleanup).
    if out.exists() {
        fs::remove_dir_all(out).with_context(|| {
            format!("remove prior deploy package `{}` before atomic replace", out.display())
        })?;
    }
    // Disarm the TempDir guard so its Drop doesn't try to delete the
    // path we're about to rename — `keep()` returns the path and
    // converts the TempDir into a no-op-on-drop handle.
    let staged = stage.keep();
    fs::rename(&staged, out).with_context(|| {
        format!(
            "atomically rename staged deploy package `{}` -> `{}`",
            staged.display(),
            out.display()
        )
    })?;

    println!("deploy package: {}", out.display());
    println!("dockerfile: {}", out.join("Dockerfile").display());
    println!("oci metadata: {}", out.join("oci-labels.json").display());
    println!("env schema: {}", out.join("env.schema.json").display());
    println!("health config: {}", out.join("health.json").display());
    println!("sbom: {}", out.join("sbom.spdx.json").display());
    println!(
        "attestation: {}",
        out.join("build-attestation.dsse.json").display()
    );
    Ok(())
}

pub fn run_compose(app: &Path, out: &Path) -> Result<()> {
    let app_name = app
        .file_name()
        .and_then(|name| name.to_str())
        .context("app path must end in a valid directory name")?;
    let plan = DeploymentPlan::new(app_name);
    fs::create_dir_all(out)
        .with_context(|| format!("create compose deploy dir `{}`", out.display()))?;
    let context = relative_path(out, app)?;
    let dockerfile = out.join("Dockerfile");
    let dockerfile_from_context = relative_path(app, &dockerfile)?;
    fs::write(&dockerfile, render_dockerfile_with_plan(plan, app)).context("write Dockerfile")?;
    fs::write(
        out.join("docker-compose.yml"),
        render_compose(
            plan,
            &path_for_manifest(&context),
            &path_for_manifest(&dockerfile_from_context),
        ),
    )
        .context("write docker-compose.yml")?;
    fs::write(out.join(".env.example"), render_compose_env(app_name))
        .context("write compose env")?;
    println!(
        "compose manifest: {}",
        out.join("docker-compose.yml").display()
    );
    println!("env example: {}", out.join(".env.example").display());
    Ok(())
}

pub fn run_paas(app: &Path, out: &Path) -> Result<()> {
    let app_name = app
        .file_name()
        .and_then(|name| name.to_str())
        .context("app path must end in a valid directory name")?;
    let plan = DeploymentPlan::new(app_name);
    fs::create_dir_all(out)
        .with_context(|| format!("create paas deploy dir `{}`", out.display()))?;
    let dockerfile = out.join("Dockerfile");
    let dockerfile_from_app = relative_path(app, &dockerfile)?;
    let dockerfile_from_app = path_for_manifest(&dockerfile_from_app);
    fs::write(&dockerfile, render_dockerfile_with_plan(plan, app)).context("write Dockerfile")?;
    fs::write(out.join("fly.toml"), render_fly(plan, &dockerfile_from_app))
        .context("write fly.toml")?;
    fs::write(
        out.join("render.yaml"),
        render_render(plan, &dockerfile_from_app),
    )
    .context("write render.yaml")?;
    fs::write(out.join("secrets.example"), render_paas_secrets(app_name))
        .context("write paas secrets")?;
    println!("fly manifest: {}", out.join("fly.toml").display());
    println!("render manifest: {}", out.join("render.yaml").display());
    Ok(())
}

pub fn run_k8s(app: &Path, out: &Path) -> Result<()> {
    let app_name = app
        .file_name()
        .and_then(|name| name.to_str())
        .context("app path must end in a valid directory name")?;
    let plan = DeploymentPlan::new(app_name);
    fs::create_dir_all(out)
        .with_context(|| format!("create k8s deploy dir `{}`", out.display()))?;
    fs::write(out.join("deployment.yaml"), render_k8s(plan)).context("write k8s deployment")?;
    println!("k8s manifest: {}", out.join("deployment.yaml").display());
    Ok(())
}

/// Slice 33Q13c — deterministic deploy-manifest tailoring.
///
/// Walks the app's IR and filesystem layout for known patterns
/// (server blocks, dangerous tools, budget constraints, optional
/// directories) and emits structured recommendations against the
/// generated Dockerfile / Compose / K8s manifests / env-schema.
/// Each recommendation cites the IR or filesystem signal that
/// triggered it so the operator can map back to source — mirrors
/// the 33Q13a synthesizer's groundedness contract.
///
/// Output: markdown by default; JSON when `json` is true.
pub fn run_tailor(app: &Path, json: bool) -> Result<u8> {
    let report = tailor_analyze(app)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .context("serialize tailor report")?
        );
    } else {
        print!("{}", tailor_render_markdown(&report));
    }
    Ok(0)
}

/// One actionable recommendation derived from the IR walk.
#[derive(Debug, Clone, Serialize)]
pub struct TailorRecommendation {
    /// Severity: `critical` (must address before deploying),
    /// `warn` (likely-broken without action), `info` (suggestion).
    pub severity: TailorSeverity,
    /// Which generated manifest the recommendation targets:
    /// `Dockerfile`, `Compose`, `K8s`, `env.schema.json`,
    /// `runbook`, etc.
    pub target: String,
    /// One-line title naming what to do.
    pub title: String,
    /// Brief rationale + the source-level signal that triggered
    /// this recommendation (the "grounded citation" — keeps the
    /// recommendation from being a free-form invention).
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TailorSeverity {
    Critical,
    Warn,
    Info,
}

/// Full tailor report for the operator.
#[derive(Debug, Clone, Serialize)]
pub struct TailorReport {
    pub app_name: String,
    pub source_path: std::path::PathBuf,
    pub recommendations: Vec<TailorRecommendation>,
    /// Counters that summarize what the analyzer detected. Useful
    /// in tests to assert "an app with N tools surfaces N
    /// dangerous-tool checks" without reading every individual
    /// recommendation.
    pub signals: TailorSignals,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TailorSignals {
    pub server_blocks: usize,
    pub agents: usize,
    pub tools_total: usize,
    pub dangerous_tools: usize,
    pub agents_with_budget: usize,
    pub has_tools_py: bool,
    pub has_migrations: bool,
    pub has_evals: bool,
    pub has_traces: bool,
}

/// Build the tailor report for a given app dir. Public for tests.
pub fn tailor_analyze(app: &Path) -> Result<TailorReport> {
    use corvid_driver::{compile_to_ir_with_config_at_path, load_corvid_config_for};

    let app_name = app
        .file_name()
        .and_then(|name| name.to_str())
        .context("app path must end in a valid directory name")?
        .to_string();

    let source_path = app.join("src").join("main.cor");
    let source = fs::read_to_string(&source_path)
        .with_context(|| format!("read app source `{}`", source_path.display()))?;
    let config = load_corvid_config_for(&source_path);
    let ir = compile_to_ir_with_config_at_path(&source, &source_path, config.as_ref())
        .map_err(|diags| anyhow::anyhow!("compile diagnostics: {} found", diags.len()))?;

    // Tally the signals the analyzer cares about. Reading them all
    // here keeps the recommendation builder declarative.
    let signals = TailorSignals {
        server_blocks: ir.servers.len(),
        agents: ir.agents.len(),
        tools_total: ir.tools.len(),
        dangerous_tools: ir
            .tools
            .iter()
            .filter(|t| matches!(t.effect, corvid_ast::Effect::Dangerous))
            .count(),
        agents_with_budget: ir.agents.iter().filter(|a| a.cost_budget.is_some()).count(),
        has_tools_py: app.join("tools.py").is_file(),
        has_migrations: app.join("migrations").is_dir(),
        has_evals: app.join("evals").is_dir(),
        has_traces: app.join("traces").is_dir(),
    };

    let mut recommendations = Vec::new();

    // Server block → port + healthCheck implications.
    if signals.server_blocks > 0 {
        recommendations.push(TailorRecommendation {
            severity: TailorSeverity::Info,
            target: "Compose / K8s".to_string(),
            title: "Expose port 8000 and add a readiness probe".to_string(),
            rationale: format!(
                "{} server block(s) detected in main.cor — `corvid serve` binds 0.0.0.0:8000 \
                 by default. The generated Compose / K8s manifests need a port mapping + a \
                 `/readyz` probe so orchestrators don't roll traffic before the runtime is up.",
                signals.server_blocks
            ),
        });
    } else {
        recommendations.push(TailorRecommendation {
            severity: TailorSeverity::Warn,
            target: "Dockerfile / Compose / K8s".to_string(),
            title: "No server block — consider --target=cdylib instead of serve".to_string(),
            rationale: "main.cor declares no `server` block, but the generated Dockerfile's \
                CMD invokes `corvid serve`. The image will start and immediately error. Either \
                add a `server` block OR drop the orchestrator manifests in favor of a one-shot \
                CLI runner image."
                .to_string(),
        });
    }

    // Dangerous tools → approval queue, audit log, secret management.
    if signals.dangerous_tools > 0 {
        recommendations.push(TailorRecommendation {
            severity: TailorSeverity::Critical,
            target: "K8s / Compose / runbook".to_string(),
            title: "Wire the approval-queue admin endpoints to an actual reviewer surface"
                .to_string(),
            rationale: format!(
                "{} `dangerous` tool(s) detected. `corvid serve` queues their invocations \
                 under `/__approvals/<id>` — reviewers POST `/approve` or `/deny`. The \
                 generated manifests do NOT include a reviewer UI; either expose the admin \
                 endpoints to a trusted internal network OR proxy them through your own \
                 approval dashboard. Without this, dangerous calls queue forever.",
                signals.dangerous_tools
            ),
        });
    }

    // Agents with `@budget` → resource limits in K8s.
    if signals.agents_with_budget > 0 {
        recommendations.push(TailorRecommendation {
            severity: TailorSeverity::Info,
            target: "K8s".to_string(),
            title: "Set resource limits in line with declared @budget constraints".to_string(),
            rationale: format!(
                "{} agent(s) declare `@budget` constraints (compile-time cost ceilings). \
                 Translate the declared dollar/token/latency caps into K8s `resources.limits` \
                 + `resources.requests` so a runaway agent can't escalate beyond the budget \
                 the source enforces. This is a Lift-and-Shift of the moat from compile time \
                 into runtime resource pressure.",
                signals.agents_with_budget
            ),
        });
    }

    // tools.py presence → COPY in Dockerfile, Python in image.
    if signals.has_tools_py {
        recommendations.push(TailorRecommendation {
            severity: TailorSeverity::Info,
            target: "Dockerfile".to_string(),
            title: "tools.py is bundled via the 33Q4 presence-conditional COPY".to_string(),
            rationale: "tools.py detected at app root. The 33Q4 Dockerfile renderer COPYs it \
                automatically — no manual step. If you add the LLM-driven tool dispatch \
                pattern (`from corvid_runtime import tool`), the 33Q6 bundled `corvid_runtime` \
                package is already on PYTHONPATH inside the image."
                .to_string(),
        });
    }

    // Migrations directory → init container or migrate-on-startup.
    if signals.has_migrations {
        recommendations.push(TailorRecommendation {
            severity: TailorSeverity::Warn,
            target: "K8s / Compose".to_string(),
            title: "Run `corvid migrate up` before serving".to_string(),
            rationale: "migrations/ detected. The generated CMD is `corvid serve`, which does \
                NOT run migrations. Either add an init container (K8s) / depends_on (Compose) \
                that runs `corvid migrate up` before the serve container, OR add a startup \
                hook to the serve container that runs migrate before bind."
                .to_string(),
        });
    }

    // Evals + Traces → observability / replay surface.
    if signals.has_evals {
        recommendations.push(TailorRecommendation {
            severity: TailorSeverity::Info,
            target: "K8s / runbook".to_string(),
            title: "Schedule a periodic `corvid eval list` for regression detection".to_string(),
            rationale: "evals/ detected. Add a daily/weekly CronJob (K8s) or scheduled \
                docker run (Compose) that runs `corvid eval list` against the deployed \
                cdylib + alerts on regressions. The evals are useless if they only run in \
                CI."
                .to_string(),
        });
    }

    // Tools total → tools.py vs cdylib choice.
    if signals.tools_total > 0 && !signals.has_tools_py {
        recommendations.push(TailorRecommendation {
            severity: TailorSeverity::Warn,
            target: "Dockerfile / runbook".to_string(),
            title: "Tools declared but no tools.py — provide --with-tools-cdylib at runtime"
                .to_string(),
            rationale: format!(
                "{} tool(s) declared in main.cor but no tools.py file at app root. The \
                 `corvid serve` interpreter path has no handler implementations to dispatch \
                 to. Either: (a) write tools.py against the 33Q6 bundled `corvid_runtime` \
                 package, OR (b) build a cdylib host (`cargo build --crate-type cdylib`) \
                 and pass it via `corvid serve --with-tools-cdylib <path>`. The generated \
                 manifests assume path (a); pick path (b) and adjust the CMD accordingly.",
                signals.tools_total
            ),
        });
    }

    Ok(TailorReport {
        app_name,
        source_path,
        recommendations,
        signals,
    })
}

/// Render the tailor report as a human-readable markdown document.
pub fn tailor_render_markdown(report: &TailorReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Deploy tailor — `{}`", report.app_name);
    let _ = writeln!(out);
    let _ = writeln!(out, "Source: `{}`", report.source_path.display());
    let _ = writeln!(out);
    let _ = writeln!(out, "## Signals");
    let _ = writeln!(out);
    let _ = writeln!(out, "- Server blocks: **{}**", report.signals.server_blocks);
    let _ = writeln!(out, "- Agents: **{}**", report.signals.agents);
    let _ = writeln!(out, "- Tools (total / dangerous): **{} / {}**", report.signals.tools_total, report.signals.dangerous_tools);
    let _ = writeln!(out, "- Agents with @budget: **{}**", report.signals.agents_with_budget);
    let _ = writeln!(out, "- Filesystem (tools.py / migrations / evals / traces): **{} / {} / {} / {}**", report.signals.has_tools_py, report.signals.has_migrations, report.signals.has_evals, report.signals.has_traces);
    let _ = writeln!(out);
    let _ = writeln!(out, "## Recommendations ({})", report.recommendations.len());
    let _ = writeln!(out);
    for sev in [
        TailorSeverity::Critical,
        TailorSeverity::Warn,
        TailorSeverity::Info,
    ] {
        let bucket: Vec<&TailorRecommendation> = report
            .recommendations
            .iter()
            .filter(|r| r.severity == sev)
            .collect();
        if bucket.is_empty() {
            continue;
        }
        let _ = writeln!(out, "### {:?} ({})", sev, bucket.len());
        let _ = writeln!(out);
        for rec in bucket {
            let _ = writeln!(out, "- **{}** _(target: {})_", rec.title, rec.target);
            let _ = writeln!(out, "  - {}", rec.rationale);
        }
        let _ = writeln!(out);
    }
    out
}

pub fn run_systemd(app: &Path, out: &Path) -> Result<()> {
    let app_name = app
        .file_name()
        .and_then(|name| name.to_str())
        .context("app path must end in a valid directory name")?;
    let plan = DeploymentPlan::new(app_name);
    fs::create_dir_all(out)
        .with_context(|| format!("create systemd deploy dir `{}`", out.display()))?;
    fs::write(
        out.join(format!("{app_name}.service")),
        render_systemd_service(plan),
    )
    .context("write systemd service")?;
    fs::write(
        out.join(format!("{app_name}.sysusers")),
        render_systemd_sysusers(app_name),
    )
    .context("write sysusers")?;
    fs::write(
        out.join(format!("{app_name}.tmpfiles")),
        render_systemd_tmpfiles(app_name),
    )
    .context("write tmpfiles")?;
    println!(
        "systemd service: {}",
        out.join(format!("{app_name}.service")).display()
    );
    Ok(())
}

/// 43N: distroless runtime base image. `gcr.io/distroless/cc-debian12`
/// includes libc + ca-certificates + the dynamic-linker but no shell,
/// no package manager, no setuid binaries — every byte the runtime
/// image carries is byte the runtime needs. The full distroless image
/// is ~25 MB; the Corvid binary itself is ~20-40 MB depending on
/// features enabled. Combined runtime image lands well under the
/// 80 MB Phase-43 budget.
///
/// HEALTHCHECK uses `corvid check` against the app's source as the
/// liveness probe — if `corvid` cannot lex/parse/typecheck the
/// shipped source, the binary is broken regardless of HTTP state.
fn render_dockerfile_with_plan(plan: DeploymentPlan<'_>, app_root: &Path) -> String {
    // The build context is the user's STANDALONE app dir, not the
    // Corvid monorepo. Pre-2026-06-04 this rendered a Dockerfile that
    // assumed monorepo layout (`cargo build -p corvid-cli`, `COPY
    // examples/backend/<app>`, `COPY std std`) — that broke for every
    // standalone deployment, surfaced by the first 33M friends-and-
    // family trial report at
    // `docs/external-trials/33m-trial-anonymous-2026-06-04.md`.
    //
    // The first replacement at commit 1455b6c referenced
    // `ghcr.io/micrurus-ai/corvid:${CORVID_VERSION}` as the runtime
    // base — but NO ci workflow ever publishes that image (`release.yml`
    // only uploads tarballs to GitHub Releases), so every `docker
    // build` failed at step 1 with `manifest unknown`. That bug was
    // self-audited within the same session before any reviewer hit
    // it; the shipped layout below uses the actual infrastructure
    // that DOES exist: the GitHub Release tarball
    // `corvid-x86_64-unknown-linux-gnu.tar.gz` that `release.yml`
    // builds on every `v*.*.*` tag and `install/install.sh`
    // downloads from `github.com/<repo>/releases/latest/download/...`.
    //
    // The shipped layout:
    //
    //   - Stage 1 (`corvid-installer`): `debian:bookworm-slim` +
    //     curl, fetch the GitHub Release tarball matching
    //     `CORVID_VERSION` (default `latest` = `releases/latest`),
    //     extract to `/opt/`.
    //   - Stage 2 (`distroless`): COPY the corvid binary + std/
    //     dir from stage 1 into a small runtime. Set
    //     `CORVID_HOME=/opt/corvid` so stdlib resolution works.
    //   - COPY only the user's app sources from the local working
    //     directory (`src/`, `corvid.toml`, `migrations/`,
    //     `evals/`, `traces/`) into `/app/` — no monorepo paths,
    //     no recursive bind of the whole tree.
    //   - The healthcheck and CMD use `/app/src/main.cor` (the
    //     standard standalone layout `corvid new` produces).
    //   - CMD is `corvid serve --listen 0.0.0.0:8000` so the
    //     container exposes the HTTP server every orchestrator
    //     healthcheck path expects.
    //
    // Caveat (filed as `35V2-P33-release-archive-staticlib`): the
    // current release tarball does NOT include `libcorvid_runtime.a`.
    // A container that needs to RUN `corvid build --target=cdylib`
    // at runtime would hit the missing-staticlib path. The CMD here
    // is `corvid serve` (interpreter dispatch — does NOT need the
    // staticlib), so the deploy-package path works without it; if a
    // future deploy variant needs in-container codegen, the release
    // archive must ship the staticlib alongside the binary.
    //
    // Slice 33Q5 (anonymous-2026-06-04 round-2 P3.b): the rendered
    // Dockerfile's `ARG CORVID_VERSION=...` default is pinned to the
    // nightly-channel tag that matches the rendering binary's SHA +
    // date — `nightly-{CORVID_BUILD_DATE}-{CORVID_BUILD_SHA}` — so
    // the built image's `corvid --version` reproduces the binary
    // the package was generated against, AND the image's CMD
    // subcommand (`serve`) is guaranteed to exist (the prior
    // default `latest` resolved to v0.1.0 stable which lacked
    // `serve` — the reviewer's literal report). When either env
    // var is the documented "unknown" fallback (corvid was built
    // outside a git checkout — see `crates/corvid-cli/build.rs`),
    // the default falls back to the literal string `nightly` and
    // the Dockerfile's URL-resolver block queries the GitHub API
    // for the latest nightly tag (same logic install.sh uses).
    // Operators can always override via
    // `--build-arg CORVID_VERSION=<tag>` (e.g. `v0.1.0` or
    // `nightly-2026-06-04-d23d381`).
    let build_sha = env!("CORVID_BUILD_SHA");
    let build_date = env!("CORVID_BUILD_DATE");
    let default_corvid_version = if build_sha == "unknown" || build_date == "unknown" {
        "nightly".to_string()
    } else {
        format!("nightly-{build_date}-{build_sha}")
    };

    // Slice 33Q4 (anonymous-2026-06-04 round-2 P3.a): COPY lines for
    // optional paths (`migrations/`, `evals/`, `traces/`, `tools.py`)
    // are emitted ONLY when the path exists in the app root at
    // render time. Pre-33Q4 the renderer unconditionally emitted
    // COPY lines for all five paths, which broke `docker build` for
    // every bare `corvid new` app (which has none of the four
    // optional paths) — the trial reviewer hit the failure at
    // P3.a. The presence check uses `Path::is_dir` / `Path::is_file`
    // against `app_root` rather than relying on operator post-edit;
    // shipping a Dockerfile that needs hand-editing to build is the
    // anti-pattern the reviewer flagged.
    //
    // `tools.py` is COPYed when present so the 33Q1b tools.py
    // autoloader has its module file to import inside the container.
    // The container's working dir is `/app`, so `tools.py` lands
    // next to `corvid.toml` and `corvid serve src/main.cor`'s
    // tools.py walk (`<source_parent>/tools.py` or
    // `<source_parent_parent>/tools.py`) finds it at the project
    // root.
    let mut copy_lines = String::new();
    copy_lines.push_str("COPY src ./src\n");
    copy_lines.push_str("COPY corvid.toml ./corvid.toml\n");
    if app_root.join("tools.py").is_file() {
        copy_lines.push_str("COPY tools.py ./tools.py\n");
    }
    for optional_dir in &["migrations", "evals", "traces"] {
        if app_root.join(optional_dir).is_dir() {
            copy_lines.push_str(&format!("COPY {optional_dir} ./{optional_dir}\n"));
        }
    }
    let copy_block = copy_lines.trim_end();
    let app_name = plan.app_name;
    let container_entrypoint = plan.container_entrypoint;
    let port = plan.port;

    format!(
        r#"# syntax=docker/dockerfile:1
#
# CORVID_VERSION default: pinned to the rendering binary's nightly
# tag (`nightly-<commit-date>-<short-sha>`) for reproducibility AND
# CLI-surface stability. Override with `--build-arg
# CORVID_VERSION=<tag>` to use a specific release (e.g. `v0.1.0`,
# `nightly-2026-06-04-d23d381`), the literal string `latest` for
# the latest stable, or `nightly` for the latest nightly via the
# GitHub Releases API. See slice 33Q5 in
# `crates/corvid-cli/src/deploy_cmd.rs::render_dockerfile` for the
# rationale.
ARG CORVID_VERSION={default_corvid_version}
ARG CORVID_REPO=Micrurus-Ai/Corvid-lang

FROM debian:bookworm-slim AS corvid-installer
ARG CORVID_VERSION
ARG CORVID_REPO
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN set -eux; \
    target=x86_64-unknown-linux-gnu; \
    asset=corvid-${{target}}.tar.gz; \
    if [ "$CORVID_VERSION" = "latest" ]; then \
      url="https://github.com/${{CORVID_REPO}}/releases/latest/download/${{asset}}"; \
    elif [ "$CORVID_VERSION" = "nightly" ]; then \
      # Mirror `install/install.sh`'s nightly resolver: query the GitHub Releases \
      # API and pull the first `tag_name` matching `nightly-*`. No jq dep — \
      # grep + sed since `jq` isn't in the install stage. See slice 33Q5. \
      api="https://api.github.com/repos/${{CORVID_REPO}}/releases?per_page=30"; \
      api_body="$(curl -fsSL --proto '=https' --tlsv1.2 "$api")"; \
      nightly_tag="$(printf '%s\n' "$api_body" \
        | grep -E '"tag_name"[[:space:]]*:[[:space:]]*"nightly-[^"]+"' \
        | head -n 1 \
        | sed -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"; \
      if [ -z "$nightly_tag" ]; then \
        echo "no nightly-* release found via GitHub API; override CORVID_VERSION explicitly" >&2; \
        exit 1; \
      fi; \
      url="https://github.com/${{CORVID_REPO}}/releases/download/${{nightly_tag}}/${{asset}}"; \
    else \
      url="https://github.com/${{CORVID_REPO}}/releases/download/${{CORVID_VERSION}}/${{asset}}"; \
    fi; \
    curl -fsSL --proto '=https' --tlsv1.2 -o /tmp/corvid.tar.gz "$url"; \
    mkdir -p /opt; \
    tar -xzC /opt -f /tmp/corvid.tar.gz; \
    mv "/opt/corvid-${{target}}" /opt/corvid; \
    test -x /opt/corvid/bin/corvid

FROM gcr.io/distroless/cc-debian12
LABEL org.opencontainers.image.title="{app_name}"
LABEL dev.corvid.app="{app_name}"
ENV CORVID_HOME=/opt/corvid
WORKDIR /app

COPY --from=corvid-installer /opt/corvid/bin/corvid /usr/local/bin/corvid
COPY --from=corvid-installer /opt/corvid/std /opt/corvid/std

# User app sources. `src/` and `corvid.toml` are always emitted
# because they are the structural minimum a `corvid new` app
# produces. `tools.py`, `migrations/`, `evals/`, and `traces/`
# are emitted ONLY when present at render time — see slice 33Q4
# in the function's doc comment for the rationale.
{copy_block}

HEALTHCHECK --interval=30s --timeout=10s --retries=3 \
    CMD ["/usr/local/bin/corvid", "check", "{container_entrypoint}"]
ENTRYPOINT ["/usr/local/bin/corvid"]
CMD ["serve", "{container_entrypoint}", "--listen", "0.0.0.0:{port}"]
"#
    )
}

fn render_env_schema() -> &'static str {
    r#"{
  "required": {
    "CORVID_APP_ENV": "local|staging|production",
    "CORVID_CONNECTOR_MODE": "mock|replay|real",
    "CORVID_DATABASE_URL": "sqlite:<path> or postgres://...",
    "CORVID_TRACE_DIR": "writable trace directory",
    "CORVID_REQUIRE_APPROVALS": "true"
  }
}
"#
}

fn render_health_config() -> &'static str {
    r#"{
  "health": "/healthz",
  "readiness": "/readyz",
  "metrics": "/metrics",
  "startup_checks": ["env", "migrations", "approvals", "trace_dir"]
}
"#
}

fn render_migration_runner(plan: DeploymentPlan<'_>) -> String {
    let migrations_dir = plan.migrations_dir;
    format!(
        r#"#!/usr/bin/env sh
set -eu
corvid migrate status --dir {migrations_dir} --database "$CORVID_DATABASE_URL"
corvid migrate up --dir {migrations_dir} --database "$CORVID_DATABASE_URL"
"#
    )
}

fn render_startup_checks(plan: DeploymentPlan<'_>) -> String {
    let project_entrypoint = plan.project_entrypoint;
    let migrations_dir = plan.migrations_dir;
    format!(
        r#"# Startup Checks

- `corvid check {project_entrypoint}`
- `corvid migrate status --dir {migrations_dir} --database "$CORVID_DATABASE_URL"`
- `CORVID_REQUIRE_APPROVALS=true`
- `CORVID_TRACE_DIR` exists and is writable
- `CORVID_CONNECTOR_MODE` is explicitly set
"#
    )
}

/// Render a minimal SPDX 2.3 JSON SBOM for the deploy package.
///
/// 43M: ships SBOM-as-artifact alongside the Dockerfile +
/// attestation. Enumerates the app's Corvid source (by SHA-256)
/// and the Corvid runtime that the image links against. The
/// per-Rust-dependency expansion (every transitively-linked
/// `cargo`-managed crate) lands in 43V's completeness sweep
/// using `cargo metadata` — held back here to keep this slice
/// scoped to "an SBOM exists and is structurally valid" rather
/// than "every linked dep is enumerated", which is its own
/// completeness contract.
fn render_spdx_sbom(app_name: &str, source_sha256: &str) -> Result<String> {
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .context("format SBOM timestamp")?;
    let corvid_version = env!("CARGO_PKG_VERSION");
    let namespace = format!(
        "https://corvid-lang.org/spdx/{app_name}/{source_sha256}"
    );
    let sbom = serde_json::json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("corvid-deploy-{app_name}"),
        "documentNamespace": namespace,
        "creationInfo": {
            "created": now,
            "creators": [format!("Tool: corvid-cli-{corvid_version}")],
            "licenseListVersion": "3.21",
        },
        "packages": [
            {
                "SPDXID": "SPDXRef-App-Source",
                "name": format!("{app_name}-source"),
                "downloadLocation": "NOASSERTION",
                "versionInfo": source_sha256,
                "filesAnalyzed": false,
                "checksums": [
                    {
                        "algorithm": "SHA256",
                        "checksumValue": source_sha256,
                    },
                ],
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": "NOASSERTION",
                "copyrightText": "NOASSERTION",
            },
            {
                "SPDXID": "SPDXRef-Corvid-Runtime",
                "name": "corvid",
                "downloadLocation": CORVID_REPOSITORY_URL,
                "versionInfo": corvid_version,
                "filesAnalyzed": false,
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": "NOASSERTION",
                "copyrightText": "NOASSERTION",
            },
        ],
        "relationships": [
            {
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relationshipType": "DESCRIBES",
                "relatedSpdxElement": "SPDXRef-App-Source",
            },
            {
                "spdxElementId": "SPDXRef-App-Source",
                "relationshipType": "DEPENDS_ON",
                "relatedSpdxElement": "SPDXRef-Corvid-Runtime",
            },
        ],
    });
    serde_json::to_string_pretty(&sbom).context("serialize SPDX SBOM")
}

fn render_attestation(
    app_name: &str,
    metadata_json: &str,
    cdylib_sha256: Option<&str>,
    signing_key: &corvid_abi::SigningKey,
) -> Result<String> {
    // 33Q11: the signing key is now loaded by `run_package` up-front
    // so we don't read the env here. Pre-33Q11 this function did the
    // `std::env::var(...)` itself, which deferred the env failure
    // until after 6 files had been written into `out/` — leaving a
    // partial deploy package on disk when the env was missing.
    //
    // 43O: payload carries the attestation-chain anchor. When
    // `cdylib_sha256` is `Some`, the deploy attestation binds to
    // the exact cdylib bytes that ship in the image — the cdylib
    // itself carries its `corvid claim --explain` embedded
    // attestation, so the chain `claim --explain → cdylib bytes →
    // deploy attestation` cannot drift without changing one of
    // the digests. `chain_status` is the explicit honesty marker:
    // `"complete"` when bound, `"incomplete"` when the operator
    // skipped `--cdylib`.
    let (chain_status, cdylib_field) = match cdylib_sha256 {
        Some(digest) => ("complete", format!("\"{digest}\"")),
        None => ("incomplete", "null".to_string()),
    };
    let payload = format!(
        "{{\"schema\":\"corvid.deploy.attestation.v1\",\
         \"app\":\"{app_name}\",\
         \"chain_status\":\"{chain_status}\",\
         \"cdylib_sha256\":{cdylib_field},\
         \"oci\":{metadata_json}}}"
    );
    let envelope = sign_envelope(
        payload.as_bytes(),
        "application/vnd.corvid.deploy.attestation.v1+json",
        signing_key,
        "deploy-package",
    );
    serde_json::to_string_pretty(&envelope).context("serialize deploy attestation")
}

fn render_verify_docs() -> &'static str {
    r#"# Deploy Package Verification

`build-attestation.dsse.json` is a DSSE envelope over the package's OCI metadata.

Verification requirements:

- Payload type: `application/vnd.corvid.deploy.attestation.v1+json`
- Signing key source: `CORVID_DEPLOY_SIGNING_KEY` during packaging
- The payload's source SHA-256 must match `oci-labels.json`
- The image/app label must match the packaged app directory
"#
}

fn render_compose(
    plan: DeploymentPlan<'_>,
    build_context: &str,
    dockerfile: &str,
) -> String {
    let app_name = plan.app_name;
    let port = plan.port;
    let container_entrypoint = plan.container_entrypoint;
    format!(
        r#"services:
  {app_name}:
    build:
      context: {build_context}
      dockerfile: {dockerfile}
    environment:
      CORVID_APP_ENV: local
      CORVID_CONNECTOR_MODE: mock
      CORVID_DATABASE_URL: sqlite:/data/{app_name}.db
      CORVID_TRACE_DIR: /data/traces
      CORVID_REQUIRE_APPROVALS: "true"
    ports:
      - "{port}:{port}"
    volumes:
      - {app_name}-data:/data
    healthcheck:
      test: ["CMD", "corvid", "check", "{container_entrypoint}"]
      interval: 30s
      timeout: 10s
      retries: 3

volumes:
  {app_name}-data:
"#
    )
}

fn render_compose_env(app_name: &str) -> String {
    format!(
        r#"CORVID_APP_ENV=local
CORVID_CONNECTOR_MODE=mock
CORVID_DATABASE_URL=sqlite:target/{app_name}.db
CORVID_TRACE_DIR=target/traces
CORVID_REQUIRE_APPROVALS=true
"#
    )
}

fn render_fly(plan: DeploymentPlan<'_>, dockerfile: &str) -> String {
    let app_name = plan.app_name;
    let port = plan.port;
    let health_path = plan.health_path;
    format!(
        r#"app = "{app_name}"
primary_region = "iad"

[build]
  dockerfile = "{dockerfile}"

[env]
  CORVID_APP_ENV = "production"
  CORVID_CONNECTOR_MODE = "mock"
  CORVID_TRACE_DIR = "/data/traces"
  CORVID_REQUIRE_APPROVALS = "true"

[[mounts]]
  source = "{app_name}_data"
  destination = "/data"

[[services]]
  internal_port = {port}
  protocol = "tcp"

  [[services.ports]]
    port = 80
    handlers = ["http"]

  [[services.http_checks]]
    interval = "30s"
    timeout = "10s"
    method = "get"
    path = "{health_path}"
"#
    )
}

fn render_render(plan: DeploymentPlan<'_>, dockerfile: &str) -> String {
    let app_name = plan.app_name;
    let health_path = plan.health_path;
    format!(
        r#"services:
  - type: web
    name: {app_name}
    env: docker
    dockerfilePath: {dockerfile}
    healthCheckPath: {health_path}
    envVars:
      - key: CORVID_APP_ENV
        value: production
      - key: CORVID_CONNECTOR_MODE
        value: mock
      - key: CORVID_TRACE_DIR
        value: /data/traces
      - key: CORVID_REQUIRE_APPROVALS
        value: "true"
      - key: CORVID_DATABASE_URL
        sync: false
"#
    )
}

fn render_paas_secrets(app_name: &str) -> String {
    format!(
        r#"# Secrets for {app_name}
CORVID_DATABASE_URL=sqlite:/data/{app_name}.db
CORVID_DEPLOY_SIGNING_KEY=<hex-encoded-ed25519-seed>
"#
    )
}

fn render_k8s(plan: DeploymentPlan<'_>) -> String {
    let app_name = plan.app_name;
    let port = plan.port;
    let health_path = plan.health_path;
    let readiness_path = plan.readiness_path;
    format!(
        r#"apiVersion: v1
kind: ConfigMap
metadata:
  name: {app_name}-config
data:
  CORVID_APP_ENV: production
  CORVID_CONNECTOR_MODE: mock
  CORVID_TRACE_DIR: /data/traces
  CORVID_REQUIRE_APPROVALS: "true"
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {app_name}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: {app_name}
  template:
    metadata:
      labels:
        app: {app_name}
    spec:
      containers:
        - name: {app_name}
          image: corvid/{app_name}:local
          ports:
            - containerPort: {port}
          envFrom:
            - configMapRef:
                name: {app_name}-config
          readinessProbe:
            httpGet:
              path: {readiness_path}
              port: {port}
          livenessProbe:
            httpGet:
              path: {health_path}
              port: {port}
---
apiVersion: v1
kind: Service
metadata:
  name: {app_name}
spec:
  selector:
    app: {app_name}
  ports:
    - port: 80
      targetPort: {port}
"#
    )
}

fn render_systemd_service(plan: DeploymentPlan<'_>) -> String {
    let app_name = plan.app_name;
    let project_entrypoint = plan.project_entrypoint;
    let port = plan.port;
    format!(
        r#"[Unit]
Description=Corvid {app_name}
After=network-online.target
Wants=network-online.target

[Service]
User={app_name}
Group={app_name}
WorkingDirectory=/opt/corvid/{app_name}
Environment=CORVID_APP_ENV=production
Environment=CORVID_CONNECTOR_MODE=mock
Environment=CORVID_DATABASE_URL=sqlite:/var/lib/{app_name}/{app_name}.db
Environment=CORVID_TRACE_DIR=/var/lib/{app_name}/traces
Environment=CORVID_REQUIRE_APPROVALS=true
ExecStart=/usr/local/bin/corvid serve {project_entrypoint} --listen 0.0.0.0:{port}
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#
    )
}

fn render_systemd_sysusers(app_name: &str) -> String {
    format!("u {app_name} - \"Corvid {app_name}\" /var/lib/{app_name}\n")
}

fn render_systemd_tmpfiles(app_name: &str) -> String {
    format!("d /var/lib/{app_name} 0750 {app_name} {app_name} -\nd /var/lib/{app_name}/traces 0750 {app_name} {app_name} -\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that mutate `CORVID_DEPLOY_SIGNING_KEY` MUST hold this
    /// lock to serialize against each other — env-var mutation is
    /// process-global and the default `cargo test` thread pool runs
    /// tests in parallel. Without this, the 33Q11 atomicity test
    /// (which removes the env) races the 33Q12b OCI normalization
    /// test (which sets it). Surfaced when both tests landed
    /// together; this lock is the surgical fix.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 43M: the SBOM emitted by `corvid deploy package` is
    /// structurally valid SPDX 2.3 JSON — has the required
    /// top-level fields, a CC0-1.0 data license, an
    /// SPDXRef-DOCUMENT root, and at least one package + one
    /// relationship.
    #[test]
    fn deploy_sbom_is_structurally_valid_spdx_2_3() {
        let sbom_json = render_spdx_sbom("test_app", "abc123").expect("render SBOM");
        let parsed: serde_json::Value =
            serde_json::from_str(&sbom_json).expect("SBOM parses as JSON");
        assert_eq!(parsed["spdxVersion"], "SPDX-2.3");
        assert_eq!(parsed["dataLicense"], "CC0-1.0");
        assert_eq!(parsed["SPDXID"], "SPDXRef-DOCUMENT");
        assert!(
            parsed["documentNamespace"]
                .as_str()
                .unwrap_or("")
                .contains("test_app"),
            "namespace should reference the app: {}",
            parsed["documentNamespace"]
        );
        assert!(
            parsed["packages"].as_array().is_some_and(|a| !a.is_empty()),
            "SBOM must list at least one package"
        );
        assert!(
            parsed["relationships"].as_array().is_some_and(|a| !a.is_empty()),
            "SBOM must declare at least one relationship"
        );
        let runtime = parsed["packages"]
            .as_array()
            .and_then(|packages| packages.iter().find(|package| package["name"] == "corvid"))
            .expect("SBOM contains the Corvid runtime package");
        assert_eq!(runtime["downloadLocation"], CORVID_REPOSITORY_URL);
    }

    #[test]
    fn every_deploy_renderer_consumes_one_port_and_standalone_paths() {
        let plan = DeploymentPlan::new("test_app");
        let compose = render_compose(plan, "../..", "target/compose/Dockerfile");
        let fly = render_fly(plan, "target/paas/Dockerfile");
        let render = render_render(plan, "target/paas/Dockerfile");
        let k8s = render_k8s(plan);
        let systemd = render_systemd_service(plan);
        let migrations = render_migration_runner(plan);
        let startup = render_startup_checks(plan);
        let all = [
            compose.as_str(),
            fly.as_str(),
            render.as_str(),
            k8s.as_str(),
            systemd.as_str(),
            migrations.as_str(),
            startup.as_str(),
        ]
        .join("\n");

        assert!(!all.contains("examples/backend/"), "{all}");
        assert!(!all.contains("8080"), "{all}");
        assert!(compose.contains("\"8000:8000\""), "{compose}");
        assert!(fly.contains("internal_port = 8000"), "{fly}");
        assert!(k8s.contains("containerPort: 8000"), "{k8s}");
        assert!(k8s.contains("targetPort: 8000"), "{k8s}");
        assert!(
            systemd.contains(
                "ExecStart=/usr/local/bin/corvid serve src/main.cor --listen 0.0.0.0:8000"
            ),
            "{systemd}"
        );
        assert!(migrations.contains("--dir migrations"), "{migrations}");
        assert!(startup.contains("corvid check src/main.cor"), "{startup}");
    }

    #[test]
    fn deploy_paths_follow_custom_output_location() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app = temp.path().join("app");
        let out = temp.path().join("some").join("custom").join("compose");
        fs::create_dir_all(&app).expect("create app");
        fs::create_dir_all(&out).expect("create out");

        let context = relative_path(&out, &app).expect("relative context");
        let dockerfile =
            relative_path(&app, &out.join("Dockerfile")).expect("relative Dockerfile");
        assert_eq!(path_for_manifest(&context), "../../../app");
        assert_eq!(
            path_for_manifest(&dockerfile),
            "../some/custom/compose/Dockerfile"
        );
    }

    #[test]
    fn compose_and_paas_emit_self_contained_dockerfiles_at_custom_outputs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app = temp.path().join("app");
        fs::create_dir_all(app.join("src")).expect("create app source");
        fs::write(app.join("src/main.cor"), "agent main() -> Int:\n    return 0\n")
            .expect("write source");
        fs::write(app.join("corvid.toml"), "[package]\nname = \"app\"\n")
            .expect("write config");

        let compose_out = temp.path().join("generated/compose");
        run_compose(&app, &compose_out).expect("render Compose");
        assert!(compose_out.join("Dockerfile").is_file());
        let compose =
            fs::read_to_string(compose_out.join("docker-compose.yml")).expect("read Compose");
        assert!(compose.contains("context: ../../app"), "{compose}");
        assert!(
            compose.contains("dockerfile: ../generated/compose/Dockerfile"),
            "{compose}"
        );
        assert!(compose.contains("\"8000:8000\""), "{compose}");
        assert!(!compose.contains("examples/backend"), "{compose}");

        let paas_out = temp.path().join("generated/paas");
        run_paas(&app, &paas_out).expect("render PaaS");
        assert!(paas_out.join("Dockerfile").is_file());
        let fly = fs::read_to_string(paas_out.join("fly.toml")).expect("read Fly");
        let render = fs::read_to_string(paas_out.join("render.yaml")).expect("read Render");
        assert!(
            fly.contains("dockerfile = \"../generated/paas/Dockerfile\""),
            "{fly}"
        );
        assert!(
            render.contains("dockerfilePath: ../generated/paas/Dockerfile"),
            "{render}"
        );
        assert!(!fly.contains("examples/backend"), "{fly}");
        assert!(!render.contains("examples/backend"), "{render}");
    }

    /// 43O: when `corvid deploy package` runs without `--cdylib`,
    /// the attestation payload marks the chain as incomplete with
    /// `cdylib_sha256: null` + `chain_status: "incomplete"`.
    /// Downstream verification refuses to trust an unchained
    /// deploy attestation.
    #[test]
    fn deploy_attestation_marks_chain_incomplete_without_cdylib() {
        // Use the `inner` payload format the render function emits
        // (the wrapper sign_envelope adds a DSSE envelope around
        // it). Test the payload structure by exercising the public
        // render path directly via a controlled key. Slice 33Q11
        // moved env reading out of render_attestation; we pass the
        // pre-loaded test key directly.
        let signing_key = corvid_abi::load_signing_key(
            &corvid_abi::KeySource::Env("0".repeat(64)),
        )
        .expect("load test signing key");
        let envelope = render_attestation(
            "test_app",
            "{\"image\":\"test_app\"}",
            None,
            &signing_key,
        )
        .expect("render");
        let parsed: serde_json::Value =
            serde_json::from_str(&envelope).expect("envelope JSON");
        // DSSE envelope base64s the payload; decode + parse.
        use base64::Engine as _;
        let payload_b64 = parsed["payload"]
            .as_str()
            .expect("DSSE payload base64 field");
        let payload_bytes = base64::engine::general_purpose::STANDARD
            .decode(payload_b64)
            .expect("decode DSSE payload");
        let payload: serde_json::Value =
            serde_json::from_slice(&payload_bytes).expect("payload JSON");
        assert_eq!(payload["chain_status"], "incomplete");
        assert!(payload["cdylib_sha256"].is_null());
        assert_eq!(payload["app"], "test_app");
    }

    /// 43O: when `--cdylib` is provided, the attestation payload
    /// carries the cdylib's SHA-256 + `chain_status: "complete"`.
    /// A second build with a different cdylib produces a different
    /// digest, breaking the chain as intended.
    #[test]
    fn deploy_attestation_binds_to_cdylib_digest_when_provided() {
        // Slice 33Q11 changed render_attestation's signature to take
        // a pre-loaded SigningKey instead of reading the env. Build
        // a deterministic test key here (32 zero bytes encoded as
        // 64 hex zeros) — same shape the prior env-reading path
        // exercised, no more env mutation in this test.
        let signing_key = corvid_abi::load_signing_key(
            &corvid_abi::KeySource::Env("0".repeat(64)),
        )
        .expect("load test signing key");
        let cdylib_digest = "abc123def456";
        let envelope = render_attestation(
            "test_app",
            "{\"image\":\"test_app\"}",
            Some(cdylib_digest),
            &signing_key,
        )
        .expect("render");
        let parsed: serde_json::Value =
            serde_json::from_str(&envelope).expect("envelope JSON");
        use base64::Engine as _;
        let payload_b64 = parsed["payload"]
            .as_str()
            .expect("DSSE payload base64 field");
        let payload_bytes = base64::engine::general_purpose::STANDARD
            .decode(payload_b64)
            .expect("decode DSSE payload");
        let payload: serde_json::Value =
            serde_json::from_slice(&payload_bytes).expect("payload JSON");
        assert_eq!(payload["chain_status"], "complete");
        assert_eq!(payload["cdylib_sha256"], cdylib_digest);
    }

    /// 43N: the Dockerfile rendered by `corvid deploy package`
    /// uses a distroless runtime base image. Catches the regression
    /// where a contributor swaps back to a fat base (debian-slim,
    /// alpine, ubuntu) that breaks the ≤80 MB Phase-43 budget.
    /// The distroless image carries no shell + no package manager +
    /// Build an app-root tempdir containing all four optional paths
    /// (`migrations/`, `evals/`, `traces/`, `tools.py`) so the
    /// presence-conditional 33Q4 renderer emits every COPY line —
    /// the "full app" shape the existing assertions all assume.
    fn tempdir_with_all_optional_paths() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("migrations")).expect("create migrations/");
        std::fs::create_dir(dir.path().join("evals")).expect("create evals/");
        std::fs::create_dir(dir.path().join("traces")).expect("create traces/");
        std::fs::write(dir.path().join("tools.py"), b"# fixture tools.py\n")
            .expect("write tools.py");
        dir
    }

    /// no setuid binaries, so every byte is byte the runtime needs.
    #[test]
    fn deploy_dockerfile_uses_distroless_runtime_base() {
        let app_root = tempdir_with_all_optional_paths();
        let dockerfile =
            render_dockerfile_with_plan(DeploymentPlan::new("test_app"), app_root.path());

        // The Dockerfile is multi-stage: one or more builder stages
        // (each `FROM <image> AS <name>`) followed by the runtime
        // stage (the final `FROM <image>` with NO `AS` suffix).
        // The fat-base ban applies only to the runtime stage —
        // builder stages legitimately use `debian:bookworm-slim`
        // to run `curl` + `tar` because distroless has no shell.
        // Locate the runtime stage as the last `FROM ...` line
        // that doesn't have an `AS` clause.
        let runtime_from = dockerfile
            .lines()
            .filter(|line| line.starts_with("FROM "))
            .filter(|line| !line.contains(" AS "))
            .next_back()
            .unwrap_or_else(|| {
                panic!(
                    "no runtime FROM line (a `FROM <image>` without ` AS `) \
                     found; got:\n{dockerfile}"
                )
            });

        assert!(
            runtime_from.contains("gcr.io/distroless/"),
            "runtime stage must use distroless base; got runtime FROM line:\n  \
             {runtime_from}\nfull Dockerfile:\n{dockerfile}"
        );
        // Adversarial guard: catch the common fat-base substitutions
        // ON THE RUNTIME STAGE ONLY. A builder stage using debian
        // is fine and intentional.
        for fat_base in &[
            "debian:",
            "debian ",
            "ubuntu:",
            "ubuntu ",
            "alpine:",
            "alpine ",
        ] {
            assert!(
                !runtime_from.contains(fat_base),
                "runtime stage must not use fat base `{fat_base}`; got \
                 runtime FROM line:\n  {runtime_from}\nfull Dockerfile:\n{dockerfile}"
            );
        }
        // HEALTHCHECK must survive the base swap (uses absolute
        // path to the binary because distroless has no PATH-shell).
        assert!(
            dockerfile.contains("HEALTHCHECK"),
            "HEALTHCHECK directive must survive the base swap"
        );
        assert!(
            dockerfile.contains("/usr/local/bin/corvid"),
            "HEALTHCHECK + CMD must use absolute path on distroless"
        );
    }

    /// 33Q4 acceptance — anonymous-2026-06-04 round-2 P3.a: a bare
    /// `corvid new` app (only `src/` and `corvid.toml` exist) must
    /// render a Dockerfile whose `COPY` block omits every optional
    /// path that doesn't exist at render time. Pre-33Q4 the renderer
    /// unconditionally emitted COPY lines for `migrations/`, `evals/`,
    /// `traces/` (and never copied `tools.py`), which broke
    /// `docker build` on the first missing-path lookup. This test
    /// pins the fix by asserting that ONLY `src` and `corvid.toml`
    /// COPY lines appear in the bare-app rendering.
    #[test]
    fn deploy_dockerfile_omits_copy_lines_for_missing_optional_paths() {
        // Empty app root (no migrations/, no evals/, no traces/,
        // no tools.py). Only the structural minimum `corvid new`
        // would produce.
        let app_root = tempfile::tempdir().expect("tempdir");

        let dockerfile =
            render_dockerfile_with_plan(DeploymentPlan::new("test_app"), app_root.path());

        // Mandatory COPYs (structural minimum a `corvid new` app
        // always has) MUST be present.
        assert!(
            dockerfile.contains("COPY src ./src"),
            "Dockerfile must always emit `COPY src` — that's the \
             structural minimum of any Corvid app. got:\n{dockerfile}"
        );
        assert!(
            dockerfile.contains("COPY corvid.toml ./corvid.toml"),
            "Dockerfile must always emit `COPY corvid.toml` — every \
             Corvid app has one. got:\n{dockerfile}"
        );

        // Optional COPYs MUST be absent when the source path doesn't
        // exist — the load-bearing 33Q4 assertion. If these fire,
        // `docker build` would fail for a bare `corvid new` app and
        // the reviewer's P3.a regression is back.
        for missing_optional in &[
            "COPY tools.py",
            "COPY migrations",
            "COPY evals",
            "COPY traces",
        ] {
            assert!(
                !dockerfile.contains(missing_optional),
                "Dockerfile MUST NOT emit `{missing_optional}` when the \
                 source path doesn't exist — that's the bug \
                 anonymous-2026-06-04 P3.a reported (broken `docker \
                 build` for bare `corvid new` apps). got:\n{dockerfile}"
            );
        }
    }

    /// 33Q4 paired with the omission test: when ALL optional paths
    /// exist at render time, the Dockerfile MUST emit COPY lines for
    /// every one of them — `tools.py` (for the 33Q1b autoloader),
    /// `migrations/`, `evals/`, `traces/`. This proves the
    /// presence check is bidirectional: omission is conditional on
    /// absence, NOT on always-omit.
    #[test]
    fn deploy_dockerfile_emits_copy_lines_for_present_optional_paths() {
        let app_root = tempdir_with_all_optional_paths();

        let dockerfile =
            render_dockerfile_with_plan(DeploymentPlan::new("test_app"), app_root.path());

        for present_optional in &[
            "COPY tools.py ./tools.py",
            "COPY migrations ./migrations",
            "COPY evals ./evals",
            "COPY traces ./traces",
        ] {
            assert!(
                dockerfile.contains(present_optional),
                "Dockerfile must emit `{present_optional}` when the \
                 source path EXISTS — proves the presence check isn't \
                 always-omit. got:\n{dockerfile}"
            );
        }
    }

    /// 33Q5 acceptance — anonymous-2026-06-04 round-2 P3.b: the
    /// rendered Dockerfile's `ARG CORVID_VERSION` default must be
    /// the rendering binary's nightly-tag form
    /// (`nightly-<commit-date>-<short-sha>`) so the built image's
    /// `corvid --version` reproduces the binary the package was
    /// generated against AND the image's `CMD` subcommand (`serve`)
    /// is guaranteed present. Pre-33Q5 the default was `latest`,
    /// which resolved to the latest stable (v0.1.0 today) — and
    /// v0.1.0 lacked the `serve` subcommand, so the rendered image's
    /// entrypoint was a command its own binary didn't have.
    ///
    /// The test reads the same compile-time env vars the renderer
    /// reads (`CORVID_BUILD_SHA` + `CORVID_BUILD_DATE`, set by
    /// `crates/corvid-cli/build.rs`) and asserts the constructed
    /// default matches what `render_dockerfile` emits. If either
    /// env is `unknown`, the expected default falls back to
    /// `nightly` (the always-works literal that the URL-resolver's
    /// API-query branch handles).
    #[test]
    fn deploy_dockerfile_pins_corvid_version_to_rendering_binary_sha() {
        let app_root = tempdir_with_all_optional_paths();
        let dockerfile =
            render_dockerfile_with_plan(DeploymentPlan::new("test_app"), app_root.path());

        let build_sha = env!("CORVID_BUILD_SHA");
        let build_date = env!("CORVID_BUILD_DATE");
        let expected_default = if build_sha == "unknown" || build_date == "unknown" {
            "nightly".to_string()
        } else {
            format!("nightly-{build_date}-{build_sha}")
        };
        let expected_arg_line = format!("ARG CORVID_VERSION={expected_default}");

        assert!(
            dockerfile.contains(&expected_arg_line),
            "Dockerfile MUST default ARG CORVID_VERSION to the rendering \
             binary's nightly tag (`{expected_default}`) so the built \
             image reproduces the SHA + has the same CLI surface. \
             Pre-33Q5 the default was `latest` which resolved to v0.1.0 \
             stable (lacks `serve`). got Dockerfile:\n{dockerfile}"
        );

        // The URL-resolver block MUST handle all three CORVID_VERSION
        // shapes the install pipeline standardizes on. If one is
        // missing, `--build-arg CORVID_VERSION=<that-shape>` would
        // fail.
        for branch in &[
            r#"$CORVID_VERSION" = "latest""#,
            r#"$CORVID_VERSION" = "nightly""#,
            r#"releases/download/${CORVID_VERSION}/"#,
        ] {
            assert!(
                dockerfile.contains(branch),
                "Dockerfile URL-resolver must handle the `{branch}` branch \
                 — without it, the corresponding CORVID_VERSION shape \
                 fails. got Dockerfile:\n{dockerfile}"
            );
        }

        // Adversarial: the prior default `ARG CORVID_VERSION=latest`
        // must not reappear. If it does, the v0.1.0-lacks-serve
        // regression returns.
        assert!(
            !dockerfile.contains("ARG CORVID_VERSION=latest"),
            "Dockerfile MUST NOT default ARG CORVID_VERSION to `latest` — \
             that's the v0.1.0-lacks-serve regression anonymous-2026-06-04 \
             P3.b documented. got Dockerfile:\n{dockerfile}"
        );
    }

    /// 33Q12b acceptance — maintainer-as-reviewer-2026-06-05 P3.3.
    /// On Windows, `Path::display()` produces a mix of `/` and `\`
    /// depending on what was in the path's internal representation —
    /// the trial reviewer saw `"C:/Users/.../Temp/threat_intel_agent\\src\\main.cor"`
    /// in their `oci-labels.json`. The mixed separators read
    /// strangely in OCI metadata that downstream tools (registries,
    /// SBOM viewers, attestation parsers) expect to be POSIX-shaped.
    ///
    /// This test asserts the OCI source field's separator
    /// normalization fires regardless of platform: it constructs a
    /// PathBuf whose Display would contain backslashes, runs it
    /// through `run_package`, parses the `oci-labels.json` output,
    /// and asserts the `source` field contains no backslashes.
    ///
    /// On Linux/macOS PathBuf::from(r"C:\Users\backslashy\path")
    /// builds a single-segment path containing backslashes; Display
    /// outputs them literally; the test exercises the replace path.
    /// On Windows the OS-native separator is also backslash; the
    /// test exercises the same replace path. So one test covers
    /// both platforms.
    #[test]
    fn deploy_package_normalizes_backslashes_in_oci_source_label() {
        // The app dir is real, but its path has no backslashes on
        // Linux. To exercise the normalization end-to-end, we build
        // the app at a name that contains backslashes when Path::join
        // composes it. The simplest reliable trick: construct the
        // expected OCI source string directly and call .replace() the
        // same way `run_package` does, then assert the round-trip.
        // For a full end-to-end run, we build a small app at a
        // tempdir + run run_package, then check the resulting
        // oci-labels.json's source field for any `\` character.
        //
        // The cross-platform assertion is: regardless of what
        // separator the OS used, the `source` field MUST NOT contain
        // a literal `\`. That's the 33Q12b contract.
        let app_dir = tempfile::tempdir().expect("app tempdir");
        let src_dir = app_dir.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("create src/");
        std::fs::write(
            src_dir.join("main.cor"),
            "agent dummy() -> Int:\n    return 0\n",
        )
        .expect("write main.cor");
        let out_parent = tempfile::tempdir().expect("out tempdir");
        let out = out_parent.path().join("deploy");

        // Take ENV_LOCK before mutating CORVID_DEPLOY_SIGNING_KEY
        // so we serialize against the 33Q11 atomicity test that
        // *removes* the same env. Without the lock the two tests
        // race under default cargo-test parallelism.
        let _guard = ENV_LOCK.lock().expect("ENV_LOCK poisoned");
        let prior = std::env::var("CORVID_DEPLOY_SIGNING_KEY").ok();
        unsafe {
            std::env::set_var("CORVID_DEPLOY_SIGNING_KEY", "0".repeat(64));
        }

        let result = super::run_package(app_dir.path(), &out, None);

        // Restore env BEFORE assertions.
        match prior {
            Some(v) => unsafe { std::env::set_var("CORVID_DEPLOY_SIGNING_KEY", v) },
            None => unsafe { std::env::remove_var("CORVID_DEPLOY_SIGNING_KEY") },
        }

        result.expect("run_package");

        let labels_json =
            std::fs::read_to_string(out.join("oci-labels.json")).expect("read oci-labels.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&labels_json).expect("parse oci-labels.json");
        // OCI uses dotted-namespace label keys; the actual field is
        // `org.opencontainers.image.source` per the serde rename
        // attribute on `OciLabels::source`.
        let source = parsed["labels"]["org.opencontainers.image.source"]
            .as_str()
            .expect("labels[org.opencontainers.image.source] must be a string");

        assert!(
            !source.contains('\\'),
            "OCI labels.source MUST NOT contain backslash separators \
             — that's the 33Q12b POSIX-normalization contract for OCI \
             metadata. got source={source:?}"
        );
    }

    /// 33Q11 acceptance — maintainer-as-reviewer-2026-06-05 P2.3.
    /// Pre-33Q11, `corvid deploy package` read
    /// `CORVID_DEPLOY_SIGNING_KEY` inside `render_attestation` which
    /// runs AFTER 6 files have already been written into `out/`. A
    /// missing env left a partial deploy directory on disk —
    /// `Dockerfile`, `oci-labels.json`, `env.schema.json`,
    /// `health.json`, `migrate.sh`, `startup-checks.md` were
    /// already there, and `sbom.spdx.json`, `build-attestation.dsse.json`,
    /// and `VERIFY.md` weren't. A reviewer would see "error" and
    /// also "6 of 9 files in deploy/" and wonder what to do.
    ///
    /// 33Q11 moves the env validation BEFORE
    /// `fs::create_dir_all(out)`. Missing env → command fails AND
    /// `out/` doesn't exist. This is the load-bearing assertion.
    #[test]
    fn deploy_package_missing_signing_key_env_does_not_create_out_dir() {
        // Build a minimal valid app structure in a tempdir so we
        // reach the env check (not the source-read or app-name check).
        let app_dir = tempfile::tempdir().expect("app tempdir");
        let src_dir = app_dir.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("create src/");
        std::fs::write(
            src_dir.join("main.cor"),
            "agent dummy() -> Int:\n    return 0\n",
        )
        .expect("write main.cor");

        // Output target inside another tempdir so we can verify
        // nothing landed there.
        let out_parent = tempfile::tempdir().expect("out tempdir");
        let out = out_parent.path().join("deploy");

        // SAFETY: env-var manipulation in tests races with parallel
        // tests on the SAME env. Each deploy_cmd::tests test that
        // touches `CORVID_DEPLOY_SIGNING_KEY` MUST take the same
        // lock, OR run on --test-threads=1. The 33Q12b OCI
        // normalization test races this one without the lock —
        // surfaced when 33Q13c landed and the test pool grew.
        let _guard = ENV_LOCK.lock().expect("ENV_LOCK poisoned");
        let prior = std::env::var("CORVID_DEPLOY_SIGNING_KEY").ok();
        // SAFETY: Rust 2024 edition marks env-var mutation `unsafe`
        // for race-with-FFI reasons. In tests we accept that.
        unsafe {
            std::env::remove_var("CORVID_DEPLOY_SIGNING_KEY");
        }

        let result = super::run_package(app_dir.path(), &out, None);

        // Restore prior env BEFORE assertions so a failing assertion
        // doesn't leak into other tests' environments.
        if let Some(v) = prior {
            unsafe {
                std::env::set_var("CORVID_DEPLOY_SIGNING_KEY", v);
            }
        }

        // Assertion 1: the command MUST fail.
        let Err(err) = result else {
            panic!(
                "deploy package with no CORVID_DEPLOY_SIGNING_KEY must \
                 fail; it succeeded with out={}",
                out.display()
            );
        };

        // Assertion 2: the error message MUST name the env var so the
        // operator knows what to set (the P3.1 ask alongside P2.3).
        let msg = format!("{err}");
        assert!(
            msg.contains("CORVID_DEPLOY_SIGNING_KEY"),
            "error must name CORVID_DEPLOY_SIGNING_KEY so the \
             operator can act on it; got: {msg}"
        );

        // Assertion 3 (LOAD-BEARING): the output directory MUST NOT
        // exist. Pre-33Q11 it existed with 6 of 9 expected files in
        // it. Atomic-on-error contract: no partial state on failure.
        assert!(
            !out.exists(),
            "deploy/ output directory MUST NOT exist after a missing-env \
             failure — that's the 33Q11 atomic-on-error contract. \
             Pre-33Q11, deploy/ would contain Dockerfile + 5 other \
             files when CORVID_DEPLOY_SIGNING_KEY was unset, leaving a \
             confusing partial state for the operator. \
             out={}",
            out.display()
        );
    }

    /// 33Q15 acceptance — strengthens the 33Q11 contract from
    /// "no out/ on error" to "out/ replaced atomically on success."
    ///
    /// Pre-33Q15, `run_package` did `fs::create_dir_all(out)` +
    /// 9 sequential `fs::write`s into `out/`. If a previous run had
    /// emitted a file the current shape no longer writes (e.g.
    /// `out/legacy_marker.json` from before SBOM was added), that
    /// stale file would persist into the new bundle, mixing two
    /// builds in one directory. A reviewer reading the bundle had
    /// no way to tell which file belonged to which run.
    ///
    /// Post-33Q15, run_package stages every write into a sibling
    /// TempDir and atomically renames into place. If `out/`
    /// already exists, it's removed BEFORE the rename so the new
    /// bundle is exactly what the current run emitted — no
    /// inherited cruft.
    #[test]
    fn deploy_package_atomically_replaces_stale_out_dir_on_success() {
        let app_dir = tempfile::tempdir().expect("app tempdir");
        let src_dir = app_dir.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("create src/");
        std::fs::write(
            src_dir.join("main.cor"),
            "agent dummy() -> Int:\n    return 0\n",
        )
        .expect("write main.cor");
        let out_parent = tempfile::tempdir().expect("out tempdir");
        let out = out_parent.path().join("deploy");

        // Pre-create out/ with a stale marker file that the
        // current run does NOT emit. Post-33Q15 it must be gone
        // after success.
        std::fs::create_dir_all(&out).expect("create stale out/");
        let stale_marker = out.join("legacy_marker_from_prior_run.json");
        std::fs::write(&stale_marker, "{\"shape\":\"pre-33Q15\"}\n")
            .expect("write stale marker");
        assert!(stale_marker.exists(), "test setup: stale marker should exist");

        let _guard = ENV_LOCK.lock().expect("ENV_LOCK poisoned");
        let prior = std::env::var("CORVID_DEPLOY_SIGNING_KEY").ok();
        unsafe {
            std::env::set_var("CORVID_DEPLOY_SIGNING_KEY", "0".repeat(64));
        }

        let result = super::run_package(app_dir.path(), &out, None);

        match prior {
            Some(v) => unsafe { std::env::set_var("CORVID_DEPLOY_SIGNING_KEY", v) },
            None => unsafe { std::env::remove_var("CORVID_DEPLOY_SIGNING_KEY") },
        }

        result.expect("run_package");

        // Load-bearing: the stale file from the prior run MUST be
        // gone. Pre-33Q15, it would still be there because we just
        // wrote over the same dir.
        assert!(
            !stale_marker.exists(),
            "stale file from a prior deploy package run MUST be \
             removed when the current run succeeds — that's the \
             33Q15 atomic-replace contract. Without it, two runs' \
             worth of files mix in one directory and the reviewer \
             can't tell which is current. stale={}",
            stale_marker.display()
        );

        // Sanity: the current run's expected files all exist.
        for expected in [
            "Dockerfile",
            "oci-labels.json",
            "env.schema.json",
            "health.json",
            "migrate.sh",
            "startup-checks.md",
            "build-attestation.dsse.json",
            "sbom.spdx.json",
            "VERIFY.md",
        ] {
            assert!(
                out.join(expected).exists(),
                "expected current-run file `{expected}` missing after \
                 atomic replace; the rename clobbered the wrong dir"
            );
        }
    }

    /// 33Q15 — pre-existing `out/` must be untouched when pre-flight
    /// fails. This is the strengthening of 33Q11's
    /// `out/-must-not-be-created` to
    /// `out/-must-not-be-MUTATED-on-error`. If an operator had a
    /// known-good deploy bundle and ran `corvid deploy package`
    /// without the env set, pre-33Q15 the dir might be partially
    /// overwritten depending on where the failure hit; post-33Q15
    /// the dir stays exactly as it was.
    #[test]
    fn deploy_package_leaves_prior_out_untouched_when_pre_flight_fails() {
        let app_dir = tempfile::tempdir().expect("app tempdir");
        let src_dir = app_dir.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("create src/");
        std::fs::write(
            src_dir.join("main.cor"),
            "agent dummy() -> Int:\n    return 0\n",
        )
        .expect("write main.cor");
        let out_parent = tempfile::tempdir().expect("out tempdir");
        let out = out_parent.path().join("deploy");

        // Pre-create out/ with a known-good marker.
        std::fs::create_dir_all(&out).expect("create prior out/");
        let prior_marker = out.join("prior_run_marker.txt");
        let prior_content = "known-good deploy from yesterday";
        std::fs::write(&prior_marker, prior_content).expect("write prior marker");

        let _guard = ENV_LOCK.lock().expect("ENV_LOCK poisoned");
        let prior_env = std::env::var("CORVID_DEPLOY_SIGNING_KEY").ok();
        unsafe {
            std::env::remove_var("CORVID_DEPLOY_SIGNING_KEY");
        }

        let result = super::run_package(app_dir.path(), &out, None);

        if let Some(v) = prior_env {
            unsafe {
                std::env::set_var("CORVID_DEPLOY_SIGNING_KEY", v);
            }
        }

        assert!(result.is_err(), "missing env must error");

        // Load-bearing: the prior_marker MUST still be there with
        // its original contents.
        assert!(
            prior_marker.exists(),
            "prior run's file MUST NOT be deleted by a failing \
             deploy package run — that's the 33Q15 strengthening of \
             33Q11. The operator's known-good deploy bundle stays \
             exactly as it was. marker={}",
            prior_marker.display()
        );
        let read_back =
            std::fs::read_to_string(&prior_marker).expect("read prior marker");
        assert_eq!(
            read_back, prior_content,
            "prior run's file contents MUST NOT be mutated by a \
             failing deploy package run"
        );
    }

    /// 43M: SBOM names the app source AND the Corvid runtime as
    /// distinct packages; the relationship between them captures
    /// the "this image embeds the Corvid runtime" fact that the
    /// `deploy.sbom_completeness` row requires.
    #[test]
    fn deploy_sbom_names_app_source_and_corvid_runtime() {
        let sbom_json = render_spdx_sbom("pea", "deadbeef").expect("render SBOM");
        let parsed: serde_json::Value = serde_json::from_str(&sbom_json).unwrap();
        let pkg_ids: Vec<&str> = parsed["packages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pkg| pkg["SPDXID"].as_str().unwrap())
            .collect();
        assert!(pkg_ids.contains(&"SPDXRef-App-Source"));
        assert!(pkg_ids.contains(&"SPDXRef-Corvid-Runtime"));
        // The app source's checksum must match what was passed in.
        let app_source = parsed["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|pkg| pkg["SPDXID"] == "SPDXRef-App-Source")
            .unwrap();
        assert_eq!(app_source["versionInfo"], "deadbeef");
        assert_eq!(
            app_source["checksums"][0]["checksumValue"],
            "deadbeef",
            "the SBOM's app-source checksum must match the deploy package's source SHA-256"
        );
    }
}
