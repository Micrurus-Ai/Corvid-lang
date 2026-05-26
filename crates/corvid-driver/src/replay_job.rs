//! Replay a recorded durable-queue job by `job_id`.
//!
//! Slice `35V2-P38-C-3` — thin wrapper over the existing Phase 21
//! replay surface ([`crate::run_replay_from_source_with_builder_async`])
//! that resolves the trace by `<trace_dir>/<job_id>.jsonl` instead of
//! requiring the operator to know the path. Trace path is deterministic
//! from `job_id` per the C-2 design (see
//! `docs/phases/phase-38-replay-quarantine.md`), so the queue's
//! `replay_key` field is NOT consulted here — operators pass the
//! `job_id` directly and we look the file up under the configured
//! trace dir.
//!
//! Mode is fixed to [`ReplayMode::Plain`] (byte-identical reproduction).
//! Differential / Mutation modes for job traces are out of scope per the
//! design doc's non-goals; the existing `corvid replay` CLI is the entry
//! point for those when the operator already knows the trace path.
//!
//! Sub-slices C-4 / C-5 install quarantine wrappers (LLM / HTTP / Store
//! / IO) on top of this entry — those slices touch the runtime adapter
//! layer rather than `replay_job_from_source` itself.
//!
//! [`ReplayMode::Plain`]: crate::ReplayMode::Plain

use anyhow::{anyhow, Context, Result};
use corvid_runtime::RuntimeBuilder;
use std::path::Path;

use crate::replay::{run_replay_from_source_with_builder_async, ReplayMode, ReplayOutcome};

/// Replay the recorded trace for durable-queue job `job_id` against
/// the compiled agent in `source_path`. Resolves the trace as
/// `<trace_dir>/<job_id>.jsonl` (matching the per-job emission path
/// from slice C-2). Returns a typed [`ReplayOutcome`] from the
/// underlying Phase 21 replay machinery — the `agent_name`,
/// `result_value`, and `result_error` round-trip back to the caller
/// for the CLI to print.
///
/// Errors with a helpful diagnostic when the trace file is missing
/// (the most common cause is that the original job was not
/// `@replayable` at run time — C-2 gates trace emission on the
/// `IrAgent.is_replayable` flag, so non-`@replayable` jobs leave no
/// file behind to replay).
pub async fn replay_job_from_source(
    source_path: &Path,
    job_id: &str,
    trace_dir: &Path,
    base_builder: RuntimeBuilder,
) -> Result<ReplayOutcome> {
    let trace_path = trace_dir.join(format!("{job_id}.jsonl"));
    if !trace_path.exists() {
        return Err(anyhow!(
            "no trace at `{}` for job `{job_id}`.\n\
             Possible reasons:\n\
             - the original job was not declared `@replayable`, so the\n  \
               executor did not emit a JSONL trace at run time;\n\
             - the trace directory was wiped or relocated between the\n  \
               original run and the replay (default is\n  \
               `target/trace/jobs/`, override with `--trace-dir`);\n\
             - the job id is wrong (check `corvid jobs inspect`).",
            trace_path.display()
        ));
    }
    run_replay_from_source_with_builder_async(
        &trace_path,
        source_path,
        ReplayMode::Plain,
        base_builder,
    )
    .await
    .with_context(|| format!("replay of job `{job_id}` failed"))
}
