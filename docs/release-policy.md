# Corvid Release Policy

This policy defines how Corvid ships nightly, beta, and stable releases. It is a maintainer contract: a release is not publishable unless the channel rules, SemVer rules, signed artifacts, migration notes, and claim review all pass.

**Audience.** Two readers should be able to use this document without external context: a Corvid maintainer cutting a release, and a downstream developer or organization deciding whether to depend on Corvid for production. The first uses it as a checklist; the second uses it as the public contract for what changing-and-not-changing means at each channel.

The channel requirements below describe the maintainer's release policy. The
[release workflow](../.github/workflows/release.yml) builds the downloadable
archives. The [verification section](#verification) describes the checks in the
workflow and the additional checks maintainers perform separately.

For compiler verification after switching source snapshots, use a fresh checkout with an empty Cargo target directory. A reused Linux build accepted an imported agent call inside a pure function. Building the same source from an empty target correctly rejected it.

## Channels

### Nightly

- Purpose: expose the latest merged work to early adopters and CI consumers.
- The hosted prerelease tag is `nightly-YYYY-MM-DD-<short-sha>`. For a local manifest, use a SemVer nightly prerelease such as `1.0.0-nightly.local`.
- Pushes to `main` and manual workflow runs can build a nightly. Different commits on the same day have distinct tags.
- Stability: no source or binary compatibility promise beyond the generated upgrade report.
- Required artifacts: signed binary archive, `SHA256SUMS.txt`, release manifest, changelog fragment, and claim-audit report.
- Promotion rule: a nightly can become a beta candidate only when all Phase 43 launch gates are green or explicitly non-scope.

### Beta

- Purpose: validate the release candidate with real backend applications.
- Version format: `MAJOR.MINOR.PATCH-beta.N`.
- Cadence: weekly or on critical fix, never more than one beta per commit.
- Stability: source compatibility is expected inside one beta train; breaking changes require a new beta number and a migration note.
- Required artifacts: everything nightly ships, plus upgrade check output from the previous beta and stable baseline.
- Promotion rule: a beta can become stable only after beta feedback is closed as code, docs, tests, or explicit non-scope.

### Stable

- Purpose: production use by external Corvid developers.
- Version format: `MAJOR.MINOR.PATCH`.
- Cadence: explicit maintainer cut, not automatic.
- Stability: SemVer applies to the language surface, stdlib public surface, ABI attestation format, receipt verification behavior, deploy package shape, and connector manifest schema.
- Required artifacts: signed binaries, checksums, SBOM, release manifest, changelog, reproducible-build notes, migration guide, advisory contact, and final claim audit.
- Promotion rule: the stable tag is blocked if any launch claim lacks a runnable command, test, or explicit non-scope reason.

## SemVer Scope

SemVer covers these public contracts:

- Corvid source syntax accepted by the parser.
- Typechecker diagnostics that are documented as stable guarantees.
- Standard library modules documented in `docs/reference/stdlib/README.md`.
- ABI descriptor and ABI attestation schema consumed by `corvid receipt verify-abi`.
- Trace, receipt, migration-state, and connector-manifest schemas.
- CLI commands documented for production flows: `build`, `migrate`, `deploy`, `release`, `upgrade`, `ops`, and `claim`.

Patch releases may add diagnostics, tighten security checks, and fix runtime behavior when the documented contract already required that behavior. Minor releases may add syntax, stdlib APIs, connector fields, and deployment targets. Major releases may remove or change public contracts only with an upgrade tool and migration guide.

## Stability Classes

- `stable`: supported for all patch and minor releases in the same major line.
- `beta`: may change during beta, but every change must be represented in the upgrade report.
- `experimental`: available only behind an explicit flag or preview namespace; never used in launch claims.
- `internal`: not a public contract and not covered by SemVer.

Every public feature must declare one of these classes in its docs or generated manifest before a stable release.

## Breaking Change Rules

A change is breaking when it removes syntax, changes a public type, weakens a guarantee, changes a serialized schema without migration tooling, changes a stable CLI flag, or changes deploy artifact layout in a way that invalidates existing automation.

Breaking changes require:

- a migration note,
- an upgrade check rule,
- at least one accepted fixture and one rejected fixture when source syntax changes,
- a changelog entry,
- maintainer approval in the release checklist.

## Release Blockers

A release is blocked when any of these are true:

- signed artifact generation fails,
- checksums or SBOM are missing,
- `corvid claim audit` reports an aspirational launch claim,
- a comparison using `corvid upgrade check PATH --claims-current OLD.json --claims-target NEW.json` reports a removed or weakened guarantee,
- generated docs drift from the guarantee registry,
- migration execution or drift detection fails,
- a security advisory marked release-blocking is open,
- beta feedback required for stable remains untriaged.

## Key Rotation

Release keys are rotated by publishing a signed key-rotation note in the previous key and the new key. A stable release must name the release key id in the release manifest. Revoked keys may verify historical artifacts but may not sign new releases.

## Maintainer Signoff

Stable release signoff requires:

- one maintainer responsible for release mechanics,
- one maintainer responsible for security/advisory review,
- one maintainer responsible for claim audit,
- one maintainer responsible for compatibility and migration review.

The same person may hold more than one role, but each role must be recorded in the release manifest.

## Verification

- **Version and channels.** The release workflow derives the nightly tag from
  the date and commit. It accepts `vMAJOR.MINOR.PATCH` for stable releases and
  `vMAJOR.MINOR.PATCH-beta.N` for beta prereleases. Both tags must match the CLI
  Cargo package version after removing the leading `v`. The archive check supplies the candidate download to the
  production installer in a temporary directory. It checks the installed
  version, calls Python, runs the starter project, then builds and serves a
  web application. Uninstall must remove the package and preserve a user file. The publish
  job waits for all platform archives. Manual workflow runs retain the verified
  archives as workflow artifacts by default. Publishing a manual run requires
  selecting the publish option.
- **Project migration.** Run `corvid upgrade check PATH` to preview known source
  and standard-library migrations. Review the report before using
  `corvid upgrade apply PATH`. To compare guarantees between two applications,
  supply their current and target claim manifests to the check command. Build
  and test the application under the new CLI to find incompatibilities outside
  those rules.
- **Release records.** `corvid release build CHANNEL VERSION --out DIR` creates
  local release artifacts. It validates the channel, version, and release signing
  key before writing to the output directory. The hosted archive workflow also produces a
  `SHA256SUMS.txt` artifact from the tested platform archives on build-only runs.
  When publishing, it checks the downloaded archives against that file and
  includes it with the downloads. To verify one archive locally, put it and
  `SHA256SUMS.txt` in an empty directory and run `sha256sum --check --ignore-missing SHA256SUMS.txt`.
  Each platform archive includes `lib/corvid-rust-notices.txt`. It records
  the resolved Rust dependencies of the CLI, language server and native runtime,
  their declared licenses, and notice text found in their Cargo packages.
  When a Cargo package omits the upstream license file, the inventory can
  include a checked-in copy from its exact source revision. Each such entry
  links to that revision. Entries marked “No separate notice file found”
  show the package authors when available. For MIT packages the inventory
  includes the standard SPDX template. These entries still need attribution review.
  The VSIX carries the editor dependencies' notices inside the extension.
  Before a stable cut, maintainers must separately verify that the required
  manifest, signatures, notices, SBOM and checksums cover the actual downloads.
  The signed descriptor for an application cdylib attests that library.
- **Review.** Maintainers record compatibility changes, migration instructions,
  advisory review, beta feedback and signoff for the candidate. Archive build
  success establishes that the exercised installation and application worked.
  Stable promotion also requires the review and channel criteria above.

## See also

- [`docs/developer-production-guide.md`](developer-production-guide.md) — what downstream developers should expect from a stable Corvid release.
- [`docs/maintainer-runbooks.md`](maintainer-runbooks.md) — release checklist, security advisory process, CI gates, benchmarks, claim review, rollback.
- [`docs/operations/receipts-and-signing.md`](operations/receipts-and-signing.md) — DSSE attestation chain and key publication.
- [`docs/operations/production-checklist.md`](operations/production-checklist.md) — pre-merge checklist that gates whether a commit is releasable at all.
- [`docs/reference/inventions.md`](reference/inventions.md) — the catalog every launch claim cross-references.
