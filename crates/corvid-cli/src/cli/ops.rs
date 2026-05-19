use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum OpsCommand {
    /// Verify a signed `/__ops` snapshot envelope captured from
    /// a live Corvid backend. Operators typically pipe
    /// `curl http://prod/__ops > ops.json` then run
    /// `corvid ops show --envelope-file ops.json --pubkey
    /// deploy.pub` to confirm the binary at the URL is the one
    /// they expect.
    ///
    /// Verification fails closed on signature mismatch (wrong
    /// key, man-in-the-middle), payload tampering, or wrong
    /// payload-type (a signature valid over some other DSSE
    /// artifact cannot be replayed against the ops surface).
    /// On success, prints the parsed snapshot as pretty JSON so
    /// the operator can eyeball `build_id`, `request_count`,
    /// and `claim_manifest_ids`.
    Show {
        /// Path to the DSSE envelope JSON captured from the
        /// `/__ops` endpoint.
        #[arg(long, value_name = "FILE")]
        envelope_file: PathBuf,
        /// Path to the ed25519 verifying key (32-byte hex or
        /// raw 32 bytes). Same format as
        /// `corvid receipt verify-abi --pubkey`.
        #[arg(long, value_name = "FILE")]
        pubkey: PathBuf,
    },
}
