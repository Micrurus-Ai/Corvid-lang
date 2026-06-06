use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum DeployCommand {
    /// Emit a deploy package containing Dockerfile and OCI metadata.
    ///
    /// REQUIRED env var:
    ///   CORVID_DEPLOY_SIGNING_KEY  32-byte ed25519 seed, encoded as
    ///                              64 hex characters (e.g.
    ///                              `openssl rand -hex 32`). Signs
    ///                              the build attestation. Missing or
    ///                              malformed -> command fails BEFORE
    ///                              any files are written (slice
    ///                              33Q11's atomic-on-error contract).
    Package {
        /// App directory, e.g. examples/backend/personal_executive_agent.
        app: PathBuf,
        /// Output directory for generated artifacts.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Path to the signed cdylib this deploy package will host.
        /// When provided, the build attestation's payload includes
        /// the cdylib's SHA-256 so the chain from `corvid claim
        /// --explain <cdylib>` to `corvid deploy package` cannot
        /// drift. Without `--cdylib`, the attestation marks the
        /// chain as incomplete and operators must record the cdylib
        /// digest manually.
        #[arg(long, value_name = "PATH")]
        cdylib: Option<PathBuf>,
    },
    /// Emit Docker Compose deployment artifacts.
    Compose {
        /// App directory, e.g. examples/backend/personal_executive_agent.
        app: PathBuf,
        /// Output directory for generated artifacts.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Emit Fly.io and Render-style single-service deployment artifacts.
    Paas {
        /// App directory, e.g. examples/backend/personal_executive_agent.
        app: PathBuf,
        /// Output directory for generated artifacts.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Emit Kubernetes manifests.
    K8s {
        /// App directory, e.g. examples/backend/personal_executive_agent.
        app: PathBuf,
        /// Output directory for generated artifacts.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Emit systemd service, sysusers, and tmpfiles artifacts.
    Systemd {
        /// App directory, e.g. examples/backend/personal_executive_agent.
        app: PathBuf,
        /// Output directory for generated artifacts.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Analyze an app's IR + filesystem and emit deploy-manifest
    /// tailoring recommendations — slice 33Q13c (the second of three
    /// remaining AI helpers under
    /// `35V2-P43-T-LR-phase-43-ai-helpers`).
    ///
    /// v1.0 ships a deterministic Rust analyzer that walks the IR
    /// for known patterns (server blocks, dangerous tools, budget
    /// constraints, etc.) and emits a structured list of
    /// recommendations against the generated Dockerfile / Compose /
    /// K8s / env-schema artifacts. Each recommendation cites the IR
    /// pattern it derived from (the source-level entity that
    /// triggered it) so the operator can map every suggestion back
    /// to their source. A post-v1.0 follow-up (33Q13d) adds an
    /// LLM-driven refinement layer that proposes free-form
    /// adjustments on top of the deterministic grounded base.
    Tailor {
        /// App directory to analyze (the IR + filesystem layout).
        app: PathBuf,
        /// Emit the recommendations as JSON instead of the default
        /// markdown rendering. Useful for downstream tooling that
        /// wants to consume the structured shape directly.
        #[arg(long)]
        json: bool,
    },
}
