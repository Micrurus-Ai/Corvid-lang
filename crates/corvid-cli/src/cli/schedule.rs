//! Clap argument tree for `corvid schedule *` — the governed-cron
//! runner that fires language-level `schedule` declarations through
//! the durable-jobs queue.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum ScheduleCommand {
    /// Run the scheduler: register every `schedule` declaration in the
    /// source as a durable cron schedule, then fire due jobs through
    /// the durable-jobs worker pool so scheduled agents inherit
    /// tracing, retries, dead-letters, and replay.
    Run {
        /// Corvid source whose `schedule` declarations drive the runner.
        #[arg(long, value_name = "PATH")]
        source: PathBuf,
        /// SQLite state database for the durable queue.
        #[arg(long, default_value = "target/corvid-jobs.sqlite")]
        state: PathBuf,
        /// Worker threads executing fired jobs.
        #[arg(long, default_value_t = 1)]
        workers: usize,
        /// Lease TTL for fired jobs, in milliseconds.
        #[arg(long, default_value_t = 60_000)]
        lease_ttl_ms: u64,
        /// Scheduler tick interval — how often due fires are enqueued.
        #[arg(long, default_value_t = 500)]
        poll_ms: u64,
        /// Stop after this many milliseconds (0 = run until Ctrl-C).
        #[arg(long, default_value_t = 0)]
        max_runtime_ms: u64,
        /// Bound on catch-up fires per schedule per tick.
        #[arg(long, default_value_t = 16)]
        max_missed_per_schedule: usize,
    },
}
