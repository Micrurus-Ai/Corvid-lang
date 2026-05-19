use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ApprovalsCommand {
    /// List approvals for a tenant, optionally filtered by status.
    Queue {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        /// Filter by status: `pending`, `approved`, `denied`, `expired`.
        #[arg(long)]
        status: Option<String>,
    },
    /// Inspect a single approval — record + every audit event.
    Inspect {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        approval_id: String,
    },
    /// Approve a pending approval.
    Approve {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        role: String,
        #[arg(long)]
        reason: Option<String>,
        approval_id: String,
    },
    /// Deny a pending approval.
    Deny {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        role: String,
        #[arg(long)]
        reason: Option<String>,
        approval_id: String,
    },
    /// Expire a pending approval whose contract expiry has passed.
    Expire {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        reason: Option<String>,
        approval_id: String,
    },
    /// Add a comment to an approval — does not change status.
    Comment {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        comment: String,
        approval_id: String,
    },
    /// Delegate a pending approval to another actor.
    Delegate {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        role: String,
        #[arg(long = "to")]
        delegate_to: String,
        #[arg(long)]
        reason: Option<String>,
        approval_id: String,
    },
    /// Approve multiple pending approvals in one invocation.
    /// Per-approval failures (wrong role, wrong tenant, already
    /// resolved) are reported individually rather than aborting
    /// the whole batch. A batch whose ids span >1 `data_class`
    /// refuses outright unless `--require-data-class <CLASS>` is
    /// supplied — the adversarial-prevention default for the
    /// `batch-approval-drift-across-data-classes` threat.
    Batch {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        role: String,
        #[arg(long)]
        reason: Option<String>,
        /// Approval ids (repeatable).
        #[arg(long = "id", value_name = "ID")]
        ids: Vec<String>,
        /// Pin the batch to a single `data_class`. Approvals
        /// whose data_class doesn't match surface as per-id
        /// failures. Without this flag, a batch that spans >1
        /// data class refuses outright.
        #[arg(long, value_name = "CLASS")]
        require_data_class: Option<String>,
    },
    /// Export every approval (with full audit trail) for a tenant
    /// since the supplied timestamp. The output is the auditable
    /// transcript a compliance review consumes.
    Export {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        /// Lower bound timestamp in ms since epoch.
        #[arg(long)]
        since_ms: Option<u64>,
        /// Output file. If omitted, prints to stdout.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Render a typed reviewer summary of a single approval +
    /// every audit-event transition. Output's `sources` array
    /// names every audit-event id the explanation consulted so a
    /// reviewer can audit-trail every claim back to a queue row.
    /// Assistive AI helper — deterministic by construction.
    Explain {
        #[arg(long, value_name = "PATH", default_value = "target/approvals.db")]
        approvals_state: PathBuf,
        #[arg(long)]
        tenant: String,
        approval_id: String,
    },
}
