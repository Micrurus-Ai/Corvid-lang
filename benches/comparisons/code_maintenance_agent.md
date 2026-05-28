# Code Maintenance Agent — per-app side-by-side

## Headline

For a code-maintenance agent that ingests repo metadata, triages issues
with CI-aware risk labels, and gates five repository operations
(post-review-comment, create-patch-proposal, open-pull-request,
merge-pull-request, tag-release) behind typed approvals — where a
high-severity risk label must be grounded in a failing CI signal —
Corvid encodes the CI-grounding and the approval flow in the language,
enforced at compile time. The FastAPI + LangChain and Next.js baselines
reconstruct the same GitHub/CI orchestration + approval + audit as
runtime conventions.

## Reproduce

Corvid governance surface lives in
[`examples/backend/code_maintenance_agent/src/main.cor`](../../examples/backend/code_maintenance_agent/src/main.cor).

```bash
m=examples/backend/code_maintenance_agent/src/main.cor
grep -c 'trust: human_required' $m   # repository-write effects
grep -c 'dangerous uses' $m          # dangerous tool declarations
grep -c '^    approve ' $m           # compiler-enforced approve gates
grep -c 'agent permission_for_' $m   # typed permission per dangerous tool
```

`cargo test -p corvid-cli --test serve_smoke` proves the source serves
its `/schema` route.

## Side-by-side (sketch)

### Corvid

```corvid
effect merge_write:
    cost: $0.03
    trust: human_required
    data: code

tool merge_pull_request(req: MergePullRequestRequest)
    -> MergePullRequestReceipt dangerous uses merge_write

agent triage_issue_mock() -> CodeRiskLabel
    uses repo_read, ci_read, code_ai:
    issue = demo_issue()
    ci = demo_ci_signal()
    return CodeRiskLabel(issue, ci, "test_regression", "high", 0.87,
        "code:triage:issue-1")

agent execute_approved_merge_pull_request(req: MergePullRequestRequest)
    -> MergePullRequestReceipt uses merge_write:
    approve MergePullRequest(req)
    return merge_pull_request(req)
```

The compiler rejects any reachable comment/patch/PR/merge/tag lacking
its `approve` (`E0101`, pinned by the 5 `ungated_*.cor` fixtures). The
CI-grounding posture is structural: `code_triage_valid` requires the
risk label's `CiSignal` to be `failed` for a high-severity regression,
so a high-severity label without CI evidence fails its contract. The 3
cron jobs (CI signal scan, repo reindex, stale-issue sweep) carry only
read effects, so no scheduled path merges or tags.

### Python (FastAPI + LangChain + custom approval) — bounty-open

FastAPI routes; PyGithub for repo/PR ops; a CI client for signals;
LangChain for triage; a Pydantic approval model; SQLAlchemy approval +
audit tables; Celery for the scans. "No merge without approval" and
"high-severity label requires failing CI" are runtime conventions.
Submission lands under `runs/python/code_maintenance_agent/`.

### TypeScript (Next.js + Vercel AI SDK + Octokit + zod) — bounty-open

Next.js handlers; Octokit for repo/PR ops; zod approval contracts; the
CI-grounding + approval checks are TypeScript code. Submission lands
under `runs/typescript/code_maintenance_agent/`.

## Governance line count

| Implementation | Governance surface | Governance lines | Notes |
|---|---|---|---|
| Corvid | 4 human_required effects, 5 dangerous tools, 5 approve gates, 5 approval contracts (Admin/irreversible for merge/tag; Reviewer/reversible for comment/patch/PR), 5 typed permissions | ~84 | CI-grounding + role gradient are language-level |
| Python (FastAPI + LangChain + Celery) | same intent across Pydantic + middleware + PyGithub | bounty-open | grounding + reachability are runtime conventions |
| TypeScript (Next.js + Vercel AI SDK + Octokit) | same intent across zod + handlers | bounty-open | grounding + reachability are runtime conventions |

## What Corvid wins on

- **Reachability at typecheck.** No repository write executes without
  its `approve` boundary (`E0101`).
- **CI-grounding is structural.** A high-severity risk label must be
  grounded in a failing CI signal; an ungrounded label fails its
  contract rather than driving a write.
- **Role gradient is typed**: Admin + irreversible for mainline-changing
  merge/tag; Reviewer + reversible for comment/patch/PR proposals.
- **One source → HTTP service + worker + eval corpus.**

## What Corvid does not claim

- **GitHub/CI provider breadth** is not shipped day one.
- **Raw triage-model latency** is not the moat.
- **The `bounty-open` cells are not yet measured.**
