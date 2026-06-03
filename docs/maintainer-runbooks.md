# Corvid Maintainer Runbooks

These runbooks define the minimum maintainer process for a production release. They are intentionally operational: every section maps to a command, artifact, or explicit decision record.

**Audience.** Three readers should be able to use this document without external context: a Corvid maintainer cutting or rolling back a release, a security responder triaging an incoming advisory, and an external auditor confirming that the published process is the one actually followed. The first runs the checklists; the second uses the advisory process; the third matches the runbook against the manifest fields produced by `corvid release`.

**Position.** Companion to [`docs/release-policy.md`](release-policy.md): the policy says *what* must be true at each channel, this document says *how* maintainers make it true. When the two disagree the policy wins — that means this file is wrong and needs a follow-up. Every section is paired with a machine-checked gate; the [How this is enforced](#how-this-is-enforced) section at the end names each one.

## Release Checklist

Before cutting nightly, beta, or stable:

- run `cargo test --workspace` or the documented CI equivalent,
- run `cargo run -q -p corvid-cli -- contract regen-doc docs/reference/core-semantics.md` and confirm no drift,
- run `corvid upgrade check . --json`,
- run `corvid claim audit` for launch-facing claims,
- run `corvid release <channel> <version> --out target/release/<channel>`,
- verify `SHA256SUMS.txt`,
- verify `release-attestation.dsse.json`,
- attach changelog, SBOM, checksums, release manifest, and reproducible-build notes.

Stable releases also require the beta-feedback closure report, migration guide, advisory-contact check, and final launch rehearsal.

## Security Advisory Process

1. Triage incoming reports within two business days.
2. Assign severity: critical, high, medium, low, or informational.
3. Mark release-blocking advisories in the release issue.
4. Patch privately when exploitability is credible.
5. Add regression tests before disclosure unless doing so would reveal an active exploit.
6. Publish advisory with affected versions, fixed versions, workaround, and verification command.
7. Rotate release keys if signing material may be affected.

Security contact and incident-response ownership must be present in the stable release manifest.

## Compatibility Policy

Compatibility follows [`docs/release-policy.md`](release-policy.md).

- Source syntax, stdlib APIs, ABI attestation, receipt verification, trace schemas, migration state, connector manifests, and stable CLI flags are public contracts.
- Breaking changes need an upgrade rule, migration note, changelog entry, and maintainer signoff.
- Patch releases may tighten security behavior when the documented contract already required it.
- Experimental features cannot appear in launch claims.

## CI Gates

The release branch must pass:

- parser/typechecker/unit suites,
- native parity and binary suites,
- byte-fuzz and source-bypass corpora,
- ABI verification tests,
- guarantee registry validation,
- docs/core-semantics drift check,
- reference app checks,
- deploy package and release artifact tests,
- upgrade migrator tests.

A skipped gate needs an issue link and explicit non-scope note in the release manifest.

## Benchmark Reproduction

Benchmark claims must include:

- exact command,
- repository commit,
- machine shape,
- input corpus,
- expected output hash or summary,
- comparison target and version.

For clone-to-production-shaped-deploy, maintainers reproduce `benches/comparisons/clone_to_deploy.md` and attach the generated report before using the benchmark in launch material.

## Claim Review

Every public claim must be classified as:

- runnable artifact,
- documented guarantee,
- benchmark result,
- explicit non-scope,
- removed.

Claim review uses the guarantee registry as source of truth. A launch claim is blocked if it says Corvid enforces behavior that the registry marks `out_of_scope`, or if it has no runnable command, test, or signed artifact.

## Rollback

Rollback requires:

- revoke or hide the broken release artifact,
- publish a rollback note,
- identify affected channel and version,
- keep checksums and attestations for forensic verification,
- open follow-up work for any failed release gate.

## How this is enforced

Every runbook above is paired with a CLI command, CI gate, or signed-artifact field that catches the violation before it ships. The pairing is what lets the auditor confirm the runbook is the actual process and not aspirational text.

| Runbook | Enforcement |
|---|---|
| Release checklist | `corvid release <channel> <version>` refuses to emit a stable manifest if any required artifact is absent; CI's release job runs `cargo test --workspace`, the docs/core-semantics drift check, `corvid upgrade check`, and `corvid claim audit` as preflights. |
| Security advisory severity + release-blocking flag | Release-blocking advisories are referenced by id in the release manifest's `blocking_advisories` field; the publisher refuses to release while any are open. |
| Compatibility policy | `corvid upgrade check` consults the guarantee registry under `crates/corvid-guarantees/` and refuses to advance the version when a stable contract has weakened without an upgrade rule. |
| CI gates passing on the release branch | CI's release job is configured to require green status on parser/typechecker/unit, native-parity, byte-fuzz/source-bypass corpora, ABI verification, guarantee registry validation, docs drift, reference-app checks, deploy-package + release-artifact tests, and upgrade-migrator tests. A `# skipped:` annotation must carry an issue link and a non-scope note that lands in the release manifest. |
| Benchmark reproduction | Benchmark claims are required to carry `command`, `commit`, `host`, `corpus`, `expected_output`, and `comparator` fields; `corvid claim audit` refuses to admit a benchmark claim missing any of these. |
| Claim review | `corvid claim audit` classifies every public claim against the guarantee registry; a launch claim that does not match a registry row with `runtime_checked` / `statically_checked` / `out_of_scope` / `benchmarked` status is rejected. |
| Rollback | Rollback notes are required artifacts in the post-publish manifest; the publisher's `revoke` flow rejects a rollback that does not name the affected channel + version and does not attach the failing-gate evidence. |

The closing claim mirrors the release policy: neither the maintainer running the runbook nor the auditor reading the manifest has to trust that the runbook is being followed — the tooling refuses to publish unless it actually was.

## See also

- [`docs/release-policy.md`](release-policy.md) — the public contract these runbooks operationalize.
- [`docs/developer-production-guide.md`](developer-production-guide.md) — what downstream developers see when one of these runbooks executes correctly.
- [`docs/operations/receipts-and-signing.md`](operations/receipts-and-signing.md) — DSSE attestation chain, key publication, key rotation mechanics.
- [`docs/operations/production-checklist.md`](operations/production-checklist.md) — pre-merge checklist that gates whether a commit even becomes a release candidate.
- [`docs/reference/inventions.md`](reference/inventions.md) — the catalog every claim audit cross-references.
- [`docs/operations/observability-conformance.md`](operations/observability-conformance.md) — the conformance suite a rollback investigation reaches for.
