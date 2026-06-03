# Claim Inventory

This document defines the rules every public Corvid claim follows. It is the reason a reader can take Corvid's launch material at face value without auditing the source tree — because the source tree has already been audited, and the audit refuses to pass if any claim is aspirational, weakened, or unbacked.

**Audience.** Three readers should be able to use this document without external context: a downstream developer or organization deciding whether to depend on Corvid's stated guarantees in production, an external reviewer (security, compliance, or third-party auditor) confirming that the published claims are the actual claims the tooling enforces, and a Corvid maintainer authoring a new claim and reaching for the registry rules.

**Position.** This is the *rules* document. The *data* — every individual claim row, its evidence type, its status — lives in [`docs/meta/launch-claim-audit.md`](meta/launch-claim-audit.md). The auditor that ties the two together is the `corvid claim audit` subcommand, backed by the guarantee registry under [`crates/corvid-guarantees/`](../crates/corvid-guarantees/). The three pieces are interlocked: the rules below are what the registry encodes, the data table is what the rules apply to, and the subcommand is the verifier that refuses to pass when the data violates the rules.

## How the audit runs

The full audit:

```bash
corvid claim audit --json
```

emits a JSON report shaped like:

```json
{
  "inventory": "docs/meta/launch-claim-audit.md",
  "claim_count": 14,
  "finding_count": 0,
  "findings": []
}
```

`claim_count` is the row count of the inventory table. `finding_count` is the count of rule violations the audit detected. A release with `finding_count > 0` is blocked at the publisher — see [`docs/release-policy.md`](release-policy.md) for the channel-level rules and [`docs/maintainer-runbooks.md`](maintainer-runbooks.md) for the runbook the maintainer follows when a finding shows up.

## Inventory rules

Every claim in the inventory must satisfy these rules. The audit checks them mechanically against the table and the guarantee registry; aspirational text doesn't survive contact with the verifier.

1. **Runnable command, linked committed artifact, or explicit blocked/non-scope status.** A claim that asserts behaviour ("Corvid does X") must carry one of:
   - a backticked code command an auditor can run, exit-code-determined,
   - a `[link]`-style markdown reference to a committed file in the repo,
   - the literal status `blocked` or `non-scope` with a follow-up reference.

   A claim with no evidence column is a hard fail.

2. **Claims backed by `docs/reference/core-semantics.md` must match the guarantee registry.** `core-semantics.md` is auto-generated from `crates/corvid-guarantees/src/registry.rs` — a claim that quotes the semantics document is implicitly making a registry claim and inherits the registry's `runtime_checked` / `statically_checked` / `benchmarked` / `out_of_scope` status. Any drift between the inventory and the registry surfaces as a finding.

3. **Claims that depend on external beta feedback remain blocked until real issue evidence exists.** Until a downstream beta tester files real evidence of the behaviour, the claim cannot promote out of `blocked`. The status field is the contract — the audit refuses to flip a blocked claim to runnable without the linked beta-feedback artifact.

4. **Claims that use aspirational evidence wording fail the audit unless they are explicitly blocked or non-scope.** "We plan to," "is being developed," "will support," and equivalent phrasings trip the audit's text-pattern check. A claim can still use forward-looking language, but only when its status row records `blocked` or `non-scope` so the language is honest about the gap.

## What the inventory covers

The inventory includes:

- **README-facing claims** — anything stated on the project README and the launch page.
- **Docs-facing claims** — claims made in `docs/reference/inventions.md`, the developer production guide, the release policy, and the maintainer runbooks.
- **Release / deploy claims** — what `corvid release` and `corvid deploy` promise about their artifacts.
- **External beta status** — what the beta program promises participants, with explicit `blocked` rows until participant evidence lands.

Website and launch-page claims must be copied into the same table before public launch so `corvid claim audit` is checking one source of truth. A claim that lives only on the website and never lands in the inventory is the failure mode this rule exists to prevent — the website would assert something the audit never verified.

## How this is enforced

| Rule | Enforcement |
|---|---|
| Evidence column required per claim | `corvid claim audit` parses the inventory table and reports a finding when any row's evidence column is empty, missing, or unparseable. |
| Backticked-command evidence | The audit cannot execute arbitrary commands during audit (unsafe-by-default), but registry rows linked via `core-semantics.md` carry runtime-checked status that DOES run in CI; the linkage means the runnable-command claim is grounded in a test the build actually executes. |
| Linked-artifact evidence | The audit verifies the path resolves to a committed file at the linked-from commit. A broken link is a finding. |
| Registry-backed claims match the registry | The audit cross-references each claim row's tag against the guarantee registry under `crates/corvid-guarantees/`; mismatch in status (e.g. claim says `runtime_checked` but registry says `out_of_scope`) is a finding. |
| Blocked claims have real follow-up evidence | A row marked `blocked` must reference an issue / file / artifact that, when present, would unblock it. A `blocked` row with no follow-up reference is a finding. |
| Aspirational text outside blocked/non-scope rows | The audit's text-pattern check scans the claim's stated-behaviour column for forward-looking phrasings ("plans to," "will support," "is being developed") and reports a finding when those appear in a `runnable` / `runtime_checked` / `statically_checked` row. |
| Website / launch-page claims included | This rule is currently a maintainer commitment rather than a mechanically-checked rule. A Phase 43 follow-up wires a `corvid claim audit --include-website <url>` mode so the website is included in the audit set automatically. |

The closing claim, consistent with the release policy and maintainer runbooks: *neither the downstream developer reading a claim nor the auditor confirming the published process has to trust that the inventory is being kept honest — the tooling refuses to publish unless it actually is.*

## See also

- [`docs/release-policy.md`](release-policy.md) — channel-level rules that the audit's `finding_count > 0` result feeds into as a release blocker.
- [`docs/maintainer-runbooks.md`](maintainer-runbooks.md) — the runbook a maintainer follows when an audit finding lands.
- [`docs/meta/launch-claim-audit.md`](meta/launch-claim-audit.md) — the inventory data table this document defines the rules for.
- [`docs/reference/core-semantics.md`](reference/core-semantics.md) — the auto-generated semantics reference that registry-backed claims point at.
- [`docs/reference/inventions.md`](reference/inventions.md) — the invention catalog whose entries are required to be inventory-backed before they appear in launch material.
- [`crates/corvid-guarantees/`](../crates/corvid-guarantees/) — the registry implementation: `runtime_checked`, `statically_checked`, `benchmarked`, `out_of_scope` status taxonomy and the per-tag verifier code.
