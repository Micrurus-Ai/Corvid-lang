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
}
