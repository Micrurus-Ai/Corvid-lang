# Code Maintenance Agent Security Model

- Repository reads use mock metadata in demo mode; committed fixtures
  use patch/comment/tree fingerprints, not raw proprietary code.
- Risk triage is CI-aware: a `CodeRiskLabel` is grounded in a `CiSignal`
  (the triage contract requires `ci.status == "failed"` for a high-
  severity regression label). The agent labels risk from CI evidence, it
  does not invent it.
- Every dangerous code-maintenance operation is gated by a
  developer-authored, compiler-enforced approval contract. The developer
  owns the flow (role, cost ceiling, data class, irreversibility,
  expiry); Corvid enforces the `approve <Label>` boundary and never
  decides the flow:
  - `PostReviewComment` — Reviewer, code (reversible: delete comment).
  - `CreatePatchProposal` — Reviewer, code (reversible: a proposal, not a
    merge).
  - `OpenPullRequest` — Reviewer, code (reversible: close the PR).
  - `MergePullRequest` — Admin, code, irreversible (changes the mainline).
  - `TagRelease` — Admin, code, irreversible (publishes a release tag).
- Calling any dangerous tool without its prior `approve` boundary fails
  `corvid check` with `E0101`; the `adversarial/ungated_*.cor` fixtures
  plus `raw_patch_committed.json` are the named-threat corpus.
- The three durable cron jobs (`hourly_ci_signal_scan`,
  `nightly_repo_reindex`, `weekly_stale_issue_sweep`) carry only read
  effects — they scan CI, reindex repo trees, and flag stale issues.
  They can never open a PR, merge, or tag a release autonomously.
- No raw proprietary source is committed; the DB and fixtures hold
  fingerprints (patch, comment, tree, log), not code.
