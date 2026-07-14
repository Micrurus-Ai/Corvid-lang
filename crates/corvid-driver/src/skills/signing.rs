//! Skill signing: a DSSE envelope over the skill's content manifest.
//!
//! The signed payload is a canonical JSON manifest — name, version,
//! and the sha256 of every file in the skill (skill.toml + sources).
//! Verification therefore proves BOTH publisher identity (ed25519
//! signature) and content integrity (any post-signing edit changes a
//! file hash and fails verification). Registry-free by design: the
//! publisher distributes their verifying key out of band and the
//! consumer passes it to `corvid add skill --publisher-key`.

use corvid_abi::{load_signing_key, sign_envelope, verify_envelope, DsseEnvelope, KeySource};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// The DSSE payload type for skill content manifests.
pub const SKILL_MANIFEST_PAYLOAD_TYPE: &str = "application/vnd.corvid.skill-manifest+json";

/// File name of the signature envelope inside a skill directory.
pub const SKILL_SIG_FILE: &str = "skill.sig";

/// Canonical content manifest: BTreeMap keeps the JSON key order
/// deterministic so the signed bytes are reproducible.
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SkillContentManifest {
    pub name: String,
    pub version: String,
    /// Relative path (forward slashes) → hex sha256.
    pub files: BTreeMap<String, String>,
}

/// Compute the content manifest over every file in the skill dir
/// EXCEPT the signature itself (which cannot sign itself).
pub fn content_manifest(skill_dir: &Path) -> anyhow::Result<SkillContentManifest> {
    let manifest = super::load_manifest(skill_dir)?;
    let mut files = BTreeMap::new();
    let mut stack = vec![skill_dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(skill_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if rel == SKILL_SIG_FILE || rel == super::pin::SKILL_PIN_FILE {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            files.insert(rel, hex::encode(Sha256::digest(&bytes)));
        }
    }
    Ok(SkillContentManifest {
        name: manifest.skill.name,
        version: manifest.skill.version,
        files,
    })
}

/// Hex sha256 over the canonical manifest JSON — the skill's content
/// hash, used for lock pinning and update diffing.
pub fn content_hash(manifest: &SkillContentManifest) -> anyhow::Result<String> {
    let canonical = serde_json::to_vec(manifest)?;
    Ok(hex::encode(Sha256::digest(&canonical)))
}

/// Sign a skill directory: write `skill.sig` (DSSE envelope over the
/// content manifest). Returns (key id, full hex verifying key) —
/// the verifying key is what the publisher distributes.
pub fn sign_skill(
    skill_dir: &Path,
    key_source: &KeySource,
) -> anyhow::Result<(String, String)> {
    let manifest = content_manifest(skill_dir)?;
    let payload = serde_json::to_vec(&manifest)?;
    let key = load_signing_key(key_source).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let verifying_hex = hex::encode(key.verifying_key().to_bytes());
    let key_id = verifying_hex[..16].to_string();
    let envelope = sign_envelope(&payload, SKILL_MANIFEST_PAYLOAD_TYPE, &key, &key_id);
    std::fs::write(
        skill_dir.join(SKILL_SIG_FILE),
        serde_json::to_vec_pretty(&envelope)?,
    )?;
    Ok((key_id, verifying_hex))
}

/// The verification outcome `corvid add skill` renders on the label.
#[derive(Debug, PartialEq, Eq)]
pub enum SignatureStatus {
    /// No `skill.sig` in the directory.
    Unsigned,
    /// `skill.sig` exists but no `--publisher-key` was supplied, so
    /// the signature cannot be checked.
    PresentUnverified,
    /// Signature valid AND every file hash matches the signed
    /// manifest. Carries the signing key id.
    Verified { key_id: String },
}

/// Verify a skill directory's signature (if any) against a verifying
/// key (if given). Tampering — a signature that does not verify, or
/// file hashes that no longer match the signed manifest — is an Err,
/// never a status: a broken signature must refuse, not warn.
pub fn verify_skill_signature(
    skill_dir: &Path,
    publisher_key: Option<&Path>,
) -> anyhow::Result<SignatureStatus> {
    let sig_path = skill_dir.join(SKILL_SIG_FILE);
    if !sig_path.exists() {
        return Ok(SignatureStatus::Unsigned);
    }
    let Some(key_path) = publisher_key else {
        return Ok(SignatureStatus::PresentUnverified);
    };
    let key = corvid_abi::load_verifying_key(key_path)
        .map_err(|e| anyhow::anyhow!("cannot load publisher key: {e:?}"))?;
    let envelope_json = std::fs::read(&sig_path)?;
    let payload = verify_envelope(&envelope_json, &[SKILL_MANIFEST_PAYLOAD_TYPE], &key)
        .map_err(|e| anyhow::anyhow!("skill signature does not verify: {e:?}"))?;
    let signed: SkillContentManifest = serde_json::from_slice(&payload)?;
    let actual = content_manifest(skill_dir)?;
    if signed != actual {
        anyhow::bail!(
            "skill content does not match its signature — the files were modified after \
             signing (signed {} file hashes, computed {})",
            signed.files.len(),
            actual.files.len()
        );
    }
    let envelope: DsseEnvelope = serde_json::from_slice(&envelope_json)?;
    let key_id = envelope
        .signatures
        .first()
        .map(|s| s.keyid.clone())
        .unwrap_or_default();
    Ok(SignatureStatus::Verified { key_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_skill(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("skill.toml"),
            "[skill]\nname = \"signed-demo\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("main.cor"),
            "public agent hello() -> String:\n    return \"hi\"\n",
        )
        .unwrap();
    }

    fn keypair(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        // 32-byte hex seed; the verifying key file carries the hex
        // public key (load_verifying_key accepts 64 hex chars).
        let seed = "7f".repeat(32);
        let sk_path = dir.join("sk.hex");
        std::fs::write(&sk_path, &seed).unwrap();
        let key = load_signing_key(&KeySource::Path(sk_path.clone())).unwrap();
        let vk_path = dir.join("vk.hex");
        std::fs::write(&vk_path, hex::encode(key.verifying_key().to_bytes())).unwrap();
        (sk_path, vk_path)
    }

    #[test]
    fn sign_verify_roundtrip_and_tamper_refusal() {
        let tmp = tempfile::tempdir().unwrap();
        let skill = tmp.path().join("skill");
        demo_skill(&skill);
        let (sk, vk) = keypair(tmp.path());

        assert_eq!(
            verify_skill_signature(&skill, None).unwrap(),
            SignatureStatus::Unsigned
        );

        let (key_id, _verifying_hex) = sign_skill(&skill, &KeySource::Path(sk)).unwrap();
        assert_eq!(
            verify_skill_signature(&skill, None).unwrap(),
            SignatureStatus::PresentUnverified
        );
        match verify_skill_signature(&skill, Some(&vk)).unwrap() {
            SignatureStatus::Verified { key_id: got } => assert_eq!(got, key_id),
            other => panic!("expected Verified; got {other:?}"),
        }

        // Tamper: edit a source file after signing.
        std::fs::write(
            skill.join("main.cor"),
            "public agent hello() -> String:\n    return \"tampered\"\n",
        )
        .unwrap();
        let err = verify_skill_signature(&skill, Some(&vk))
            .expect_err("tampered content must refuse verification");
        assert!(
            err.to_string().contains("modified after"),
            "got: {err}"
        );
    }
}
