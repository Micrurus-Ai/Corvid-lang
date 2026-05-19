use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ReviewQueueCommand {
    /// List human-review queue records from a JSONL input file.
    ///
    /// The runtime exposes review-queue records at
    /// `corvid_runtime::review_queue`; a backend operator captures
    /// the pending queue as JSONL (one `ReviewQueueRecord` per
    /// line) and this command renders it.
    ///
    /// `--rank=cost-of-being-wrong` sorts records by their
    /// `cost_of_being_wrong` field, descending, so the highest-cost
    /// pending review surfaces at the top — the policy named in
    /// the `review_queue.cost_of_being_wrong_ranking` registry row.
    List {
        /// Path to a JSONL file of `ReviewQueueRecord` objects. Pass
        /// `-` to read from stdin.
        #[arg(long, value_name = "PATH")]
        records: PathBuf,
        /// Optional ranking. Today only `cost-of-being-wrong` is
        /// supported; without `--rank`, records render in input order.
        #[arg(long, value_name = "POLICY")]
        rank: Option<String>,
        /// Only show records whose `status` matches (e.g. `pending`).
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
        /// Emit the (optionally ranked) list as JSON instead of a
        /// human-readable table.
        #[arg(long)]
        json: bool,
    },
}
