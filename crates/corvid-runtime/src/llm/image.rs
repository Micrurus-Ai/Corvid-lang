use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::AsyncReadExt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptImage {
    pub media_type: String,
    pub data_base64: String,
}

impl PromptImage {
    const MAX_BYTES: usize = 8 * 1024 * 1024;

    fn media_type(bytes: &[u8]) -> Result<&'static str, String> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Ok("image/png")
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Ok("image/jpeg")
        } else {
            Err("expected a PNG or JPEG image signature".into())
        }
    }

    pub(crate) async fn read(path: &Path, root: &Path) -> Result<Self, String> {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(|e| format!("cannot resolve image root: {e}"))?;
        let path = tokio::fs::canonicalize(path)
            .await
            .map_err(|e| format!("cannot resolve image path: {e}"))?;
        if !path.starts_with(root) {
            return Err("image is outside the configured I/O root".into());
        }
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| format!("cannot open image: {e}"))?;
        let mut bytes = Vec::new();
        file.take(Self::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| format!("cannot read image: {e}"))?;
        if bytes.len() > Self::MAX_BYTES {
            return Err("image exceeds 8 MiB".into());
        }
        Ok(Self {
            media_type: Self::media_type(&bytes)?.into(),
            data_base64: STANDARD.encode(bytes),
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.data_base64.len() > Self::MAX_BYTES.div_ceil(3) * 4 {
            return Err("image exceeds 8 MiB".into());
        }
        let bytes = STANDARD
            .decode(&self.data_base64)
            .map_err(|_| "image data must be base64")?;
        if bytes.len() > Self::MAX_BYTES {
            return Err("image exceeds 8 MiB".into());
        }
        if self.media_type != Self::media_type(&bytes)? {
            return Err("image media type and signature differ".into());
        }
        Ok(())
    }
}
