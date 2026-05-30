use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum BundleCommand {
    Verify {
        path: PathBuf,
        #[arg(long)]
        rebuild: bool,
    },
    Diff {
        old: PathBuf,
        new: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Audit {
        path: PathBuf,
        #[arg(long)]
        question: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Explain {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Report {
        path: PathBuf,
        #[arg(long, default_value = "soc2")]
        format: String,
        #[arg(long)]
        json: bool,
    },
    Query {
        path: PathBuf,
        #[arg(long, value_name = "DELTA_KEY")]
        delta: String,
        #[arg(long, value_name = "NAME")]
        predecessor: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Lineage {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// INTERNAL: subprocess helper used by `bundle verify --rebuild`
    /// to isolate the cdylib load from the parent corvid process.
    ///
    /// The parent process spawns this subcommand and reads a single
    /// JSON line from stdout describing the replay outcome:
    ///
    ///   {"agent":"...","result_json":"...","observation_present":bool}
    ///
    /// On glibc, loading a Rust-built cdylib in-process and then
    /// returning from the loading thread crashes in
    /// `__call_tls_dtors` because the cdylib's TLS destructors are
    /// registered in the calling thread's destructor list and
    /// remain "live" even when the library mapping is preserved via
    /// `RTLD_NODELETE`. The bundle_rebuild test corpus exercises
    /// exactly this scenario. Isolating the dlopen + call into a
    /// short-lived subprocess sidesteps the issue at the
    /// architecture layer rather than papering over it: the
    /// subprocess may still crash during its OWN teardown, but it
    /// has already printed the JSON result by then; the parent
    /// reads that line regardless of the subprocess's exit code.
    #[command(name = "__replay-trace", hide = true)]
    ReplayTrace {
        /// Path to the rebuilt cdylib (e.g. `target/release/main.so`).
        #[arg(long, value_name = "PATH")]
        library: PathBuf,
        /// Path to the trace JSONL the replay should consume.
        #[arg(long, value_name = "PATH")]
        trace: PathBuf,
    },
}
