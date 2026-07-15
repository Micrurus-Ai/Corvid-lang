use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ContractCommand {
    /// Emit the Application Contract for a Corvid source file — the
    /// machine-readable description of its public HTTP + agent
    /// surface (routes, public agents/prompts, exchanged types with
    /// field refinements, and each callable's AI-native capabilities:
    /// streaming, grounding, approvals, confidence, cost, latency).
    /// Frontends and SDK generators consume this. Writes
    /// `target/contracts/app.corvid.json` unless `--out -` (stdout).
    App {
        /// Source file. Defaults to `src/main.cor` in a project.
        file: Option<std::path::PathBuf>,
        /// Output path, or `-` for stdout. Defaults to
        /// `target/contracts/app.corvid.json`.
        #[arg(long, value_name = "FILE")]
        out: Option<String>,
    },
    /// Emit a standard OpenAPI 3.1 document for a Corvid source file —
    /// routes, request/response schemas (with field-refinement
    /// constraints), and the session security scheme. Any OpenAPI
    /// tool (client generators, Swagger UI) consumes it. Writes
    /// `target/contracts/openapi.json` unless `--out -` (stdout).
    Openapi {
        /// Source file. Defaults to `src/main.cor` in a project.
        file: Option<std::path::PathBuf>,
        /// Output path, or `-` for stdout. Defaults to
        /// `target/contracts/openapi.json`.
        #[arg(long, value_name = "FILE")]
        out: Option<String>,
    },
    /// Print the canonical guarantee table.
    ///
    /// Default output is human-readable: one row per guarantee with
    /// id, kind, class (static / runtime-checked / out-of-scope),
    /// pipeline phase, and a one-line description. `--json` emits the
    /// full structured table including test references and (where
    /// applicable) the explicit `out_of_scope_reason` for non-defenses.
    /// The output is the single source of truth that `docs/reference/core-semantics.md`
    /// is generated from in slice 35-D and that `corvid claim --explain`
    /// reports against in slice 35-I.
    List {
        /// Emit machine-readable JSON instead of the human-readable table.
        #[arg(long)]
        json: bool,
        /// Filter by class. Accepts `static`, `runtime_checked`, or
        /// `out_of_scope`. Repeatable; unspecified shows everything.
        #[arg(long, value_name = "CLASS")]
        class: Option<String>,
        /// Filter by kind (e.g. `approval`, `effect_row`, `grounded`,
        /// `budget`, `confidence`, `replay`, `provenance_trace`,
        /// `abi_descriptor`, `abi_attestation`, `platform`).
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
    },
    /// Regenerate `docs/reference/core-semantics.md` from the canonical
    /// guarantee registry. Writes the rendered markdown to the given
    /// `OUTPUT` path (typically `docs/reference/core-semantics.md`); CI fails on
    /// drift between the committed file and the live render, so this
    /// command is the only sanctioned way to evolve the spec doc when
    /// the registry changes.
    RegenDoc {
        /// Output path, e.g. `docs/reference/core-semantics.md`.
        output: PathBuf,
    },
}
