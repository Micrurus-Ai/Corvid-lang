//! `corvid claim --explain` — a quoteable, per-binary statement of
//! what a Corvid cdylib actually proves.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use corvid_abi::{
    descriptor_from_json, parse_embedded_attestation_bytes, read_embedded_section_from_library,
    verify_envelope, CORVID_ABI_ATTESTATION_PAYLOAD_TYPE, CORVID_ABI_ATTESTATION_SYMBOL,
    CORVID_ABI_DESCRIPTOR_SYMBOL,
};
use corvid_guarantees::{GuaranteeClass, GUARANTEE_REGISTRY};
use sha2::{Digest, Sha256};

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-
/// coverage sentinel. Names the registry id whose CLI surface
/// lives in `--explain-failures` mode below — every finding
/// gets a typed `kind` + a `suggested_fix` that back-references
/// the inventory line (Grounded<T> shape).
#[allow(dead_code)]
pub const GUARANTEE_ID_CLAIM_AUDIT_EXPLAIN_GROUNDED: &str =
    "claim.audit_explain_failures_grounded";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimFindingKind {
    /// Claim row has no runnable command, linked artifact, or
    /// explicit `blocked`/`non-scope` annotation.
    MissingEvidence,
    /// Claim row's evidence contains aspirational wording
    /// (`todo`, `planned`, `future`, `soon`, `will support`)
    /// without an explicit `blocked`/`non-scope` annotation.
    AspirationalWording,
}

impl ClaimFindingKind {
    fn legacy_reason(&self) -> &'static str {
        match self {
            Self::MissingEvidence => {
                "claim must have runnable command, linked artifact, or explicit blocked/non-scope status"
            }
            Self::AspirationalWording => {
                "evidence uses aspirational wording without blocked/non-scope status"
            }
        }
    }

    fn suggested_fix(&self, line: usize) -> String {
        match self {
            Self::MissingEvidence => format!(
                "inventory line {line}: wrap the evidence cell in backticks if it's a \
                 CLI command (e.g. ``corvid <subcommand> ...``), or use \
                 `[label](path)` markdown link syntax for a file/test reference, \
                 or annotate the cell as `blocked: <slice-id>` / `non-scope` \
                 if the claim is deliberately deferred"
            ),
            Self::AspirationalWording => format!(
                "inventory line {line}: remove aspirational words (todo / planned \
                 / future / soon / will support) and either point at the shipped \
                 evidence today, or explicitly mark the row `blocked: <slice-id>` \
                 if the claim is filed for a later slice"
            ),
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct ClaimAuditFinding {
    line: usize,
    claim: String,
    reason: String,
    /// Typed classification. Only populated when
    /// `--explain-failures` is set; defaults to None to keep
    /// the existing JSON shape stable for callers that don't
    /// opt in.
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<ClaimFindingKind>,
    /// Concrete remediation paired with a back-reference to
    /// the inventory line (Grounded<T> shape). Same gating as
    /// `kind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    suggested_fix: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct ClaimAuditReport {
    inventory: String,
    claim_count: usize,
    finding_count: usize,
    findings: Vec<ClaimAuditFinding>,
}

pub fn run_claim_audit(inventory: &Path, json: bool, explain_failures: bool) -> Result<u8> {
    let text = fs::read_to_string(inventory)
        .with_context(|| format!("read claim inventory `{}`", inventory.display()))?;
    let report = audit_claim_inventory(inventory, &text, explain_failures);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).context("serialize claim audit report")?
        );
    } else {
        println!("corvid claim audit");
        println!("inventory: {}", report.inventory);
        println!("claim_count: {}", report.claim_count);
        println!("finding_count: {}", report.finding_count);
        for finding in &report.findings {
            println!(
                "line {}: {} ({})",
                finding.line, finding.claim, finding.reason
            );
            if let Some(fix) = finding.suggested_fix.as_deref() {
                println!("  fix: {fix}");
            }
        }
    }
    Ok(if report.findings.is_empty() { 0 } else { 1 })
}

pub fn run_claim_explain(
    cdylib: &Path,
    explain: bool,
    key_path: Option<&Path>,
    source_path: Option<&Path>,
) -> Result<u8> {
    if !explain {
        bail!("`corvid claim` currently requires `--explain`");
    }
    if !cdylib.exists() {
        bail!("cdylib path `{}` does not exist", cdylib.display());
    }

    let descriptor_section = read_embedded_section_from_library(cdylib).with_context(|| {
        format!(
            "reading `{}` from `{}`",
            CORVID_ABI_DESCRIPTOR_SYMBOL,
            cdylib.display()
        )
    })?;
    let descriptor = descriptor_from_json(&descriptor_section.json)
        .context("embedded ABI descriptor JSON is malformed")?;
    descriptor
        .validate_supported_version()
        .map_err(|err| anyhow!("embedded ABI descriptor version is unsupported: {err:?}"))?;
    let descriptor_hash =
        corvid_abi_verify::hex_hash(&corvid_abi::hash_json_str(&descriptor_section.json));

    let signature = inspect_signature(cdylib, key_path)?;
    let source_agreement = inspect_source_agreement(cdylib, source_path);
    let mut exit_code = 0u8;
    if signature.failed_requested_verification() || source_agreement.failed_requested_verification()
    {
        exit_code = 1;
    }

    println!("Corvid cdylib claim explanation");
    println!("binary: {}", cdylib.display());
    println!("abi_descriptor:");
    println!("  version: {}", descriptor.corvid_abi_version);
    println!("  compiler_version: {}", descriptor.compiler_version);
    println!("  source_path: {}", descriptor.source_path);
    println!("  descriptor_sha256: {descriptor_hash}");
    println!(
        "  surface: {} agent(s), {} prompt(s), {} tool(s), {} type(s), {} store(s), {} approval site(s)",
        descriptor.agents.len(),
        descriptor.prompts.len(),
        descriptor.tools.len(),
        descriptor.types.len(),
        descriptor.stores.len(),
        descriptor.approval_sites.len()
    );
    println!("attestation:");
    for line in signature.lines() {
        println!("  {line}");
    }
    println!("source_descriptor_agreement:");
    for line in source_agreement.lines() {
        println!("  {line}");
    }
    println!("enforced_guarantees:");
    for guarantee in &descriptor.claim_guarantees {
        println!(
            "  - id: {}; class: {}; kind: {}; phase: {}",
            guarantee.id, guarantee.class, guarantee.kind, guarantee.phase
        );
    }
    println!("non_defenses:");
    for guarantee in GUARANTEE_REGISTRY
        .iter()
        .filter(|g| g.class == GuaranteeClass::OutOfScope)
    {
        println!(
            "  - id: {}; reason: {}",
            guarantee.id, guarantee.out_of_scope_reason
        );
    }

    Ok(exit_code)
}

fn audit_claim_inventory(
    path: &Path,
    text: &str,
    explain_failures: bool,
) -> ClaimAuditReport {
    let mut claim_count = 0usize;
    let mut findings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|')
            || trimmed.contains("| Claim |")
            || trimmed.contains("|---")
            || trimmed.matches('|').count() < 3
        {
            continue;
        }
        let cells = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if cells.len() < 2 {
            continue;
        }
        claim_count += 1;
        let claim = cells[0].to_string();
        let evidence = cells[1];
        let blocked = evidence.contains("blocked") || evidence.contains("non-scope");
        let runnable = evidence.contains('`') || evidence.contains('[');
        let line_no = index + 1;
        if !blocked && !runnable {
            findings.push(make_finding(
                line_no,
                claim.clone(),
                ClaimFindingKind::MissingEvidence,
                explain_failures,
            ));
        }
        if contains_aspirational_word(evidence) && !blocked {
            findings.push(make_finding(
                line_no,
                cells[0].to_string(),
                ClaimFindingKind::AspirationalWording,
                explain_failures,
            ));
        }
    }
    ClaimAuditReport {
        inventory: path.display().to_string(),
        claim_count,
        finding_count: findings.len(),
        findings,
    }
}

fn make_finding(
    line: usize,
    claim: String,
    kind: ClaimFindingKind,
    explain_failures: bool,
) -> ClaimAuditFinding {
    ClaimAuditFinding {
        line,
        claim,
        reason: kind.legacy_reason().to_string(),
        kind: explain_failures.then_some(kind),
        suggested_fix: explain_failures.then(|| kind.suggested_fix(line)),
    }
}

fn contains_aspirational_word(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ["todo", "planned", "future", "soon", "will support"]
        .iter()
        .any(|word| lower.contains(word))
}

#[derive(Debug)]
enum SignatureInspection {
    Verified {
        key_fingerprint: String,
        envelope_keyid: String,
        payload_bytes: usize,
    },
    PresentNotVerified {
        envelope_keyid: String,
    },
    AbsentNotRequested,
    AbsentRequested,
    VerificationFailed(String),
}

impl SignatureInspection {
    fn failed_requested_verification(&self) -> bool {
        matches!(self, Self::AbsentRequested | Self::VerificationFailed(_))
    }

    fn lines(&self) -> Vec<String> {
        match self {
            Self::Verified {
                key_fingerprint,
                envelope_keyid,
                payload_bytes,
            } => vec![
                "status: verified".to_string(),
                format!("signing_key_fingerprint: sha256:{key_fingerprint}"),
                format!("envelope_keyid: {envelope_keyid}"),
                format!("signed_descriptor_bytes: {payload_bytes}"),
            ],
            Self::PresentNotVerified { envelope_keyid } => vec![
                "status: present_not_verified".to_string(),
                format!("envelope_keyid: {envelope_keyid}"),
                "reason: pass `--key <pubkey>` to verify the signature".to_string(),
            ],
            Self::AbsentNotRequested => vec![
                "status: absent_not_verified".to_string(),
                "reason: cdylib does not export CORVID_ABI_ATTESTATION".to_string(),
            ],
            Self::AbsentRequested => vec![
                "status: failed".to_string(),
                "reason: cdylib does not export CORVID_ABI_ATTESTATION".to_string(),
            ],
            Self::VerificationFailed(reason) => {
                vec!["status: failed".to_string(), format!("reason: {reason}")]
            }
        }
    }
}

fn inspect_signature(cdylib: &Path, key_path: Option<&Path>) -> Result<SignatureInspection> {
    let bytes = match read_attestation_bytes(cdylib) {
        Ok(bytes) => bytes,
        Err(ReadAttestationError::Absent) if key_path.is_none() => {
            return Ok(SignatureInspection::AbsentNotRequested);
        }
        Err(ReadAttestationError::Absent) => return Ok(SignatureInspection::AbsentRequested),
        Err(ReadAttestationError::Other(err)) => return Err(err),
    };
    let parsed = parse_embedded_attestation_bytes(&bytes)
        .with_context(|| format!("embedded `{CORVID_ABI_ATTESTATION_SYMBOL}` is malformed"))?;
    let envelope: corvid_abi::DsseEnvelope = serde_json::from_str(&parsed.envelope_json)
        .context("embedded ABI attestation envelope JSON is malformed")?;
    let envelope_keyid = envelope
        .signatures
        .first()
        .map(|sig| sig.keyid.clone())
        .unwrap_or_else(|| "<none>".to_string());

    let Some(key_path) = key_path else {
        return Ok(SignatureInspection::PresentNotVerified { envelope_keyid });
    };

    let verifying_key = corvid_abi::load_verifying_key(key_path)
        .map_err(|err| anyhow!("loading verifying key `{}`: {err}", key_path.display()))?;
    match verify_envelope(
        parsed.envelope_json.as_bytes(),
        &[CORVID_ABI_ATTESTATION_PAYLOAD_TYPE],
        &verifying_key,
    ) {
        Ok(payload) => Ok(SignatureInspection::Verified {
            key_fingerprint: hex::encode(Sha256::digest(verifying_key.as_bytes())),
            envelope_keyid,
            payload_bytes: payload.len(),
        }),
        Err(err) => Ok(SignatureInspection::VerificationFailed(err.to_string())),
    }
}

#[derive(Debug)]
enum SourceAgreementInspection {
    Verified {
        source_hash: String,
        embedded_hash: String,
        bytes: usize,
    },
    NotRequested,
    Failed(String),
}

impl SourceAgreementInspection {
    fn failed_requested_verification(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    fn lines(&self) -> Vec<String> {
        match self {
            Self::Verified {
                source_hash,
                embedded_hash,
                bytes,
            } => vec![
                "status: verified".to_string(),
                format!("source_descriptor_sha256: {source_hash}"),
                format!("embedded_descriptor_sha256: {embedded_hash}"),
                format!("descriptor_bytes: {bytes}"),
            ],
            Self::NotRequested => vec![
                "status: not_verified".to_string(),
                "reason: pass `--source <file.cor>` to rebuild and compare the descriptor"
                    .to_string(),
            ],
            Self::Failed(reason) => vec!["status: failed".to_string(), format!("reason: {reason}")],
        }
    }
}

fn inspect_source_agreement(
    cdylib: &Path,
    source_path: Option<&Path>,
) -> SourceAgreementInspection {
    let Some(source_path) = source_path else {
        return SourceAgreementInspection::NotRequested;
    };
    match corvid_abi_verify::verify_source_matches_cdylib(source_path, cdylib) {
        Ok(report) => SourceAgreementInspection::Verified {
            source_hash: corvid_abi_verify::hex_hash(&report.source_json_hash),
            embedded_hash: corvid_abi_verify::hex_hash(&report.embedded_json_hash),
            bytes: report.embedded_json_len,
        },
        Err(err) => SourceAgreementInspection::Failed(err.to_string()),
    }
}

enum ReadAttestationError {
    Absent,
    Other(anyhow::Error),
}

fn read_attestation_bytes(cdylib: &Path) -> std::result::Result<Vec<u8>, ReadAttestationError> {
    let lib = unsafe { libloading::Library::new(cdylib) }.map_err(|err| {
        ReadAttestationError::Other(anyhow!("loading cdylib `{}`: {err}", cdylib.display()))
    })?;
    let header_ptr: libloading::Symbol<*const u8> =
        match unsafe { lib.get(CORVID_ABI_ATTESTATION_SYMBOL.as_bytes()) } {
            Ok(symbol) => symbol,
            Err(_) => return Err(ReadAttestationError::Absent),
        };
    let header = unsafe { std::slice::from_raw_parts(*header_ptr, 16) };
    let envelope_len = u64::from_le_bytes(header[8..16].try_into().expect("8-byte len"));
    let total = usize::try_from(envelope_len)
        .ok()
        .and_then(|len| len.checked_add(16))
        .ok_or_else(|| {
            ReadAttestationError::Other(anyhow!(
                "attestation envelope length {envelope_len} does not fit in memory"
            ))
        })?;
    let bytes = unsafe { std::slice::from_raw_parts(*header_ptr, total) }.to_vec();
    std::mem::forget(lib);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_inventory(rows: &[&str]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        // Header line + separator + data rows. The header line
        // contains "| Claim |" which `audit_claim_inventory` uses
        // to skip the table header.
        writeln!(file, "| Claim | Evidence |").unwrap();
        writeln!(file, "|---|---|").unwrap();
        for row in rows {
            writeln!(file, "{row}").unwrap();
        }
        file
    }

    /// 43V: positive — `corvid claim audit` produces an empty
    /// findings list when every claim has a runnable command, a
    /// linked artifact, or an explicit blocked / non-scope
    /// status. This is the contract `claim.audit_runnable_artifacts`
    /// promises.
    #[test]
    fn audit_passes_when_every_claim_resolves() {
        let inventory = write_inventory(&[
            "| Compile-time approval gate | `cargo test -p corvid-types --lib approval` |",
            "| Replay determinism | [Phase 21 corpus](../21-replay.md) |",
            "| WASM target shipped | blocked on browser e2e CI gap, see Phase 23 reopen |",
        ]);
        let text = std::fs::read_to_string(inventory.path()).unwrap();
        let report = audit_claim_inventory(inventory.path(), &text, false);
        assert_eq!(report.claim_count, 3, "all 3 rows counted");
        assert_eq!(
            report.findings.len(),
            0,
            "audit must be silent when every claim has evidence; got: {:?}",
            report.findings
        );
    }

    /// 43V: adversarial — a claim that lacks runnable evidence
    /// AND lacks an explicit blocked / non-scope status is
    /// flagged. Catches the "aspirational claim slipped in
    /// without backing" failure mode the `corvid claim audit`
    /// gate exists to prevent.
    #[test]
    fn audit_fails_when_a_claim_lacks_evidence() {
        let inventory = write_inventory(&[
            "| Future moat feature | will land in v2.0 |",
            "| Compile-time approval gate | `cargo test -p corvid-types approval` |",
        ]);
        let text = std::fs::read_to_string(inventory.path()).unwrap();
        let report = audit_claim_inventory(inventory.path(), &text, false);
        assert_eq!(report.claim_count, 2);
        assert!(
            !report.findings.is_empty(),
            "audit must flag the aspirational row"
        );
        let aspirational_flagged = report
            .findings
            .iter()
            .any(|f| f.claim == "Future moat feature");
        assert!(
            aspirational_flagged,
            "the 'Future moat feature' row must be in the findings: {:?}",
            report.findings
        );
        // The row with `cargo test` evidence must NOT be flagged.
        let backed_flagged = report
            .findings
            .iter()
            .any(|f| f.claim == "Compile-time approval gate");
        assert!(
            !backed_flagged,
            "the runnable-command-backed row must not be flagged: {:?}",
            report.findings
        );
        // Without `--explain-failures`, `kind` and `suggested_fix`
        // are absent — preserves the existing JSON shape for
        // callers that don't opt in.
        for finding in &report.findings {
            assert!(finding.kind.is_none(), "kind must be None without --explain-failures");
            assert!(
                finding.suggested_fix.is_none(),
                "suggested_fix must be None without --explain-failures"
            );
        }
    }

    /// Slice 35V2-P43-T-LR claim-audit-explain-failures
    /// (positive, MissingEvidence kind): a row that lacks
    /// runnable evidence + lacks a blocked annotation is
    /// classified as `MissingEvidence` AND paired with a
    /// suggested-fix string that back-references the inventory
    /// line — the Grounded<T> shape at the claim-audit layer.
    #[test]
    fn explain_failures_classifies_missing_evidence_with_line_grounded_fix() {
        let inventory = write_inventory(&[
            "| Vague claim | something something |",
        ]);
        let text = std::fs::read_to_string(inventory.path()).unwrap();
        let report = audit_claim_inventory(inventory.path(), &text, true);
        assert_eq!(report.findings.len(), 1);
        let finding = &report.findings[0];
        assert_eq!(finding.kind, Some(ClaimFindingKind::MissingEvidence));
        let fix = finding.suggested_fix.as_deref().unwrap();
        assert!(
            fix.contains(&format!("inventory line {}", finding.line)),
            "fix must back-reference the inventory line: {fix}"
        );
        assert!(
            fix.contains("backticks") || fix.contains("non-scope") || fix.contains("blocked"),
            "fix must name the concrete remediation: {fix}"
        );
    }

    /// Slice 35V2-P43-T-LR claim-audit-explain-failures
    /// (positive, AspirationalWording kind): a row whose
    /// evidence contains aspirational wording but no blocked
    /// annotation is classified as `AspirationalWording` and
    /// paired with a remediation naming the specific words to
    /// remove.
    #[test]
    fn explain_failures_classifies_aspirational_wording_with_typed_remediation() {
        let inventory = write_inventory(&[
            "| Future feature | will support oauth in v2 |",
        ]);
        let text = std::fs::read_to_string(inventory.path()).unwrap();
        let report = audit_claim_inventory(inventory.path(), &text, true);
        // This row triggers BOTH MissingEvidence (no backticks/
        // link) AND AspirationalWording — the audit fires both.
        let aspirational = report
            .findings
            .iter()
            .find(|f| f.kind == Some(ClaimFindingKind::AspirationalWording))
            .expect("expected an AspirationalWording finding");
        let fix = aspirational.suggested_fix.as_deref().unwrap();
        assert!(
            fix.contains("aspirational")
                || fix.contains("todo")
                || fix.contains("planned")
                || fix.contains("will support"),
            "fix must name the aspirational-words category: {fix}"
        );
        assert!(
            fix.contains(&format!("inventory line {}", aspirational.line)),
            "fix must back-reference the inventory line: {fix}"
        );
    }

    /// Slice 35V2-P43-T-LR claim-audit-explain-failures
    /// (adversarial, opt-in default): without
    /// `--explain-failures`, the `kind` + `suggested_fix`
    /// fields default to None and the JSON shape is the legacy
    /// `{line, claim, reason}`. This is the backward-compat
    /// invariant — pre-existing callers (CI scripts, prior
    /// audit tooling) read the same shape they read before.
    #[test]
    fn explain_failures_off_preserves_legacy_finding_shape() {
        let inventory = write_inventory(&[
            "| Vague claim | something soon |",
        ]);
        let text = std::fs::read_to_string(inventory.path()).unwrap();
        let report = audit_claim_inventory(inventory.path(), &text, false);
        assert!(!report.findings.is_empty());
        let json = serde_json::to_string(&report.findings[0]).unwrap();
        // Default shape carries `line, claim, reason` only.
        assert!(json.contains("\"line\""));
        assert!(json.contains("\"claim\""));
        assert!(json.contains("\"reason\""));
        // `kind` + `suggested_fix` are absent from the JSON
        // when None (#[serde(skip_serializing_if = "Option::is_none")]).
        assert!(!json.contains("\"kind\""), "kind must be absent: {json}");
        assert!(
            !json.contains("\"suggested_fix\""),
            "suggested_fix must be absent: {json}"
        );
    }

    /// Slice 35V2-P43-T-LR claim-audit-explain-failures
    /// (positive, no-findings case): `--explain-failures`
    /// against a clean inventory still produces zero findings.
    /// The narration layer never synthesises explanations for
    /// rows that aren't actually flagged — same grounding
    /// contract as the drift narrator (no narration without
    /// evidence).
    #[test]
    fn explain_failures_on_clean_inventory_yields_zero_findings() {
        let inventory = write_inventory(&[
            "| Approval gate | `cargo test approval` |",
        ]);
        let text = std::fs::read_to_string(inventory.path()).unwrap();
        let report = audit_claim_inventory(inventory.path(), &text, true);
        assert_eq!(report.finding_count, 0);
        assert!(report.findings.is_empty());
    }
}
