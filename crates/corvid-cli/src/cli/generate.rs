use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum GenerateCommand {
    /// Generate a client SDK from the Application Contract. TypeScript
    /// is the fully-realized target (typed client + methods reusing the
    /// shipped `@corvid/client`; `--framework react` adds a hooks
    /// example over `@corvid/react`). Swift / Kotlin / Python emit
    /// typed models tracking the contract exactly plus a transport
    /// scaffold. Writes into `--out` (default `sdk/generated`).
    Sdk {
        /// Source file. Defaults to `src/main.cor` in a project.
        file: Option<PathBuf>,
        /// Target language: `ts` | `swift` | `kotlin` | `python`.
        #[arg(long, default_value = "ts")]
        language: String,
        /// For `ts`: also emit a React hooks usage example
        /// (`react` — over the `@corvid/react` package).
        #[arg(long, value_name = "FRAMEWORK")]
        framework: Option<String>,
        /// Output directory.
        #[arg(long, value_name = "DIR", default_value = "sdk/generated")]
        out: PathBuf,
    },
    /// Scaffold a runnable frontend starter project from the contract.
    /// `--framework react` emits a Vite + React + TypeScript app with
    /// the generated typed client, a configured `CorvidClient`, and an
    /// `App.tsx` wiring a form/stream per public agent — a STARTING
    /// POINT you own and modify, not a file that is re-overwritten.
    Frontend {
        /// Source file. Defaults to `src/main.cor` in a project.
        file: Option<PathBuf>,
        /// Framework to scaffold. Currently `react`.
        #[arg(long, default_value = "react")]
        framework: String,
        /// Output directory for the starter project.
        #[arg(long, value_name = "DIR", default_value = "frontend")]
        out: PathBuf,
    },
}
