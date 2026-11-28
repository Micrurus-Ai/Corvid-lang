//! A database's data key (encryption at rest K3).
//!
//! A `database` block declared `encryption: key(secret("NAME"))` seals its
//! `Secret` columns with the key the secret store holds under `NAME`. The
//! key is read once per runtime and cached, so every read and write uses
//! the same key and the secret store is not consulted per row. A missing
//! key, or text that is not 32 bytes of base64, is an error naming the
//! secret and never the key's text.

use corvid_crypto::DataKey;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The data keys a runtime has read, by secret name. Clones share them.
#[derive(Clone, Default)]
pub struct DataKeys {
    keys: Arc<Mutex<HashMap<String, Arc<DataKey>>>>,
}

impl std::fmt::Debug for DataKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.keys.lock().map(|keys| keys.len()).unwrap_or(0);
        write!(f, "DataKeys({count} read)")
    }
}

impl crate::Runtime {
    /// The data key of `database`, read from the secret store under
    /// `secret` the first time and cached after.
    pub fn data_key(&self, database: &str, secret: &str) -> Result<Arc<DataKey>, String> {
        let mut keys = self.data_keys.keys.lock().map_err(|_| "the data key cache is unusable".to_string())?;
        if let Some(key) = keys.get(secret) {
            return Ok(key.clone());
        }
        let read = self.secrets.read_env(secret).map_err(|e| e.to_string())?;
        let Some(text) = read.value else {
            return Err(format!(
                "database `{database}` declares `encryption: key(secret(\"{secret}\"))`, but `{secret}` is not set; set it to 32 random bytes in base64, such as `openssl rand -base64 32` prints"
            ));
        };
        let mut text = text;
        let key = DataKey::from_base64(&text);
        corvid_crypto::wipe_text(&mut text);
        let key = Arc::new(key.map_err(|problem| format!("database `{database}`'s data key `{secret}`: {problem}"))?);
        keys.insert(secret.to_string(), key.clone());
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use crate::Runtime;

    #[test]
    fn a_data_key_is_read_once_and_a_bad_one_is_refused_by_name() {
        let good = corvid_crypto::DataKey::generate_base64().unwrap();
        std::env::set_var("CORVID_TEST_DATA_KEY_GOOD", &good);
        std::env::set_var("CORVID_TEST_DATA_KEY_SHORT", "c2hvcnQ=");
        std::env::remove_var("CORVID_TEST_DATA_KEY_MISSING");
        let runtime = Runtime::builder().build();
        let first = runtime.data_key("billing", "CORVID_TEST_DATA_KEY_GOOD").unwrap();
        std::env::remove_var("CORVID_TEST_DATA_KEY_GOOD");
        let again = runtime.data_key("billing", "CORVID_TEST_DATA_KEY_GOOD").unwrap();
        assert_eq!(first.id(), again.id(), "the cached key is used");
        let short = runtime.data_key("billing", "CORVID_TEST_DATA_KEY_SHORT").unwrap_err();
        assert!(short.contains("CORVID_TEST_DATA_KEY_SHORT") && short.contains("32"), "{short}");
        let missing = runtime.data_key("billing", "CORVID_TEST_DATA_KEY_MISSING").unwrap_err();
        assert!(missing.contains("not set") && missing.contains("base64"), "{missing}");
        assert!(!short.contains("c2hvcnQ"), "the key's text is never in a message");
    }
}
