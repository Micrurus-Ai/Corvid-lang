# Corvid Release Policy

This policy defines how Corvid ships nightly, beta, and stable releases. It is a maintainer contract: a release is not publishable unless the channel rules, SemVer rules, signed artifacts, migration notes, and claim review all pass.

**Audience.** Two readers should be able to use this document without external context: a Corvid maintainer cutting a release, and a downstream developer or organization deciding whether to depend on Corvid for production. The first uses it as a checklist; the second uses it as the public contract for what changing-and-not-changing means at each channel.

The channel sections specify release requirements. The [verification section](#verification) describes the checks currently available to maintainers.

## Build a candidate on GitHub

Run the **release** workflow on the desired branch and leave **Publish all candidate archives to GitHub Releases** unchecked. After all six builds succeed, download the platform packages and `corvid-archive-checksums` from the workflow run. Extract the artifacts into one directory and run `sha256sum --check --strict SHA256SUMS.txt` there. These hashes check the downloaded bytes. Installation and application tests establish whether a candidate works on its target platform.

Download `corvid-archive-sboms` from the same workflow run. It contains one CycloneDX 1.6 JSON document per archive and `SBOM-SHA256SUMS.txt`. Run `sha256sum --check --strict SBOM-SHA256SUMS.txt` in the extracted directory. Each document records the archive digest, the digest of each regular file, and package names and versions from the bundled Rust notices, Python runtime manifest and VS Code extension (including its bundled npm dependencies). The Rust list includes build dependencies. To verify the inventory against downloaded bytes, run `python install/archive_sbom.py ARCHIVE rebuilt.json` with Python 3.11 or later from the matching checkout and compare `rebuilt.json` byte-for-byte with the downloaded document.

Each platform archive carries the project `LICENSE` and a target-specific Rust dependency inventory at `lib/corvid-rust-notices.txt`. It lists the locked package versions, their declared licenses, and the notice files available for those versions. The PDF extraction dependency incorporates TeX glyph mappings from two tables. The inventory includes the LCDF package's copyright statement and COPYING file. Its README describes the license for source code. The second table identifies itself as an extension of LCDF version 2.33. The applicability of those terms to the generated mappings remains open for review before publication. [LCDF README](https://github.com/kohler/lcdf-typetools/blob/46fa0a9f2fe86fbd0eb11ac1fc83a1ad43a20615/README), [extended glyph table](https://github.com/michal-h21/htfgen/blob/0d9e796ebe9d6f8978f7c069d4922278e8716a75/glyphlist-extended.txt)

For the PDF notice review, the three pinned input tables reproduce all 4,700 name/code pairs in the locked `pdf-extract 0.10.0` package. Its generator takes the first applicable mapping in input order and adds `mapsto`. The htfgen input differs from the historical LCDF 2.33 file even after removing comments and blank lines. Review the htfgen modifications alongside the LCDF material before deciding whether to distribute the package. [Generator](https://docs.rs/crate/pdf-extract/0.10.0/source/src/glyphlist-export.py), [historical LCDF table](https://github.com/kohler/lcdf-typetools/blob/e767ccc0381036ffd9f6f15fe64e896cc7d8cddd/glyphlist.txt), [htfgen file history](https://github.com/michal-h21/htfgen/commits/0d9e796ebe9d6f8978f7c069d4922278e8716a75/glyphlist-extended.txt).

The Linux x64 and ARM64 archives target Debian 12 (Bookworm) with `libpython3.11`. The build runs on each native architecture. The workflow tests Linux packages in a fresh Debian 12 container. On each of the six platforms it passes the archive through the production installer, checks the language server, invokes a Python host tool and runs a starter. It then builds and extracts a web app, renames the source project, and requests the HTML, JSON and WebAssembly assets from the relocated server. Uninstall must preserve a user-created file.

For compiler verification after switching source snapshots, use a fresh checkout with an empty Cargo target directory. The workspace CI job caches downloaded dependencies and builds the compiler and tests from the checked-out source. A reused Linux build accepted an imported agent call inside a pure function. Building the same source from an empty target correctly rejected it.

## Channels

### Nightly

- Purpose: expose the latest merged work to early adopters and CI consumers.
- Version format: `0.0.0-nightly.YYYYMMDD+<short-sha>`.
- Cadence: at most once per UTC day from `main`.
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
- a comparison using `corvid upgrade check PATH --claims-current CURRENT --claims-target TARGET` reports a removed or downgraded guarantee,
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

- **Version and channel.** The local `corvid release build CHANNEL VERSION --out DIR` command parses SemVer and checks the channel before creating its output directory. Stable takes a plain version such as `1.0.0`, beta takes a numbered prerelease such as `1.0.0-beta.2`, and nightly accepts a prerelease starting with `nightly.`. Publication cadence is a maintainer responsibility.
- **Candidate identifiers.** The hosted workflow computes a nightly tag as `nightly-YYYY-MM-DD-<short-sha>` using UTC. Stable and beta tags must equal `v` followed by the CLI package version. For local staging, omitting `VERSION` selects `0.0.0-nightly.local` for nightly, `1.0.0-beta.1` for beta, or the running CLI's package version for stable. Supply the intended SemVer explicitly when producing a local manifest for a particular candidate.
- **Archive contents.** The hosted workflow checks version tags against the CLI package version, tests each candidate archive and computes its checksum. The local release command creates its own signed release record. Before publishing, verify that signatures and inventory cover the actual downloads.
- **Windows performance measurements.** A manual candidate build with `verify_performance` enabled retains `baseline.json` and `repeat.json` in the `application-profile-windows-x64` artifact. To identify the measured executable, verify the Windows x64 ZIP against `SHA256SUMS.txt`, extract it, and run `Get-FileHash -Algorithm SHA256 .\bin\corvid.exe` from the extracted package directory. Compare the hexadecimal digest with `cli_sha256` in both reports and the SHA-256 for `bin/corvid.exe` in the archive's CycloneDX inventory.
- **Local attestation.** `corvid release build` copies the running CLI. Its signed DSSE payload contains the exact bytes of `release-manifest.json`. Verify the Ed25519 signature with the expected release public key, then compare the copied binary's SHA-256 with `binary_sha256` in that manifest. This command does not generate a software bill of materials (SBOM). The hosted workflow generates the candidate SBOM from the finished archive.
- **Compatibility.** Run `corvid upgrade check PATH` and the application's tests with the candidate CLI. To compare embedded library guarantees, supply the two exported manifests using `--claims-current` and `--claims-target`. See the [migration guide](meta/upgrade-migrations.md#comparing-guarantees-across-library-versions).
- **Maintainer review.** Record signoff, review open advisories and beta feedback, and check that changelog entries, upgrade instructions and key rotation records apply to this candidate.

## See also

- [`docs/developer-production-guide.md`](developer-production-guide.md) — what downstream developers should expect from a stable Corvid release.
- [`docs/maintainer-runbooks.md`](maintainer-runbooks.md) — release checklist, security advisory process, CI gates, benchmarks, claim review, rollback.
- [`docs/operations/receipts-and-signing.md`](operations/receipts-and-signing.md) — DSSE attestation chain and key publication.
- [`docs/operations/production-checklist.md`](operations/production-checklist.md) — pre-merge checklist that gates whether a commit is releasable at all.
- [`docs/reference/inventions.md`](reference/inventions.md) — the catalog every launch claim cross-references.
