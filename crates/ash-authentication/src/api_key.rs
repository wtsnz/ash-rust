use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{AuthError, Result};

/// API Key generation, hashing, and verification service.
#[derive(Clone, Debug)]
pub struct ApiKeyService {
    prefix: String,
}

impl ApiKeyService {
    /// Create a new `ApiKeyService` with a key prefix (e.g., "ash_").
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }

    /// Generate a new random raw API key and its corresponding SHA-256 hash.
    ///
    /// Returns `(raw_api_key, api_key_hash)`.
    /// The `raw_api_key` should be shown to the user once and never stored directly.
    /// The `api_key_hash` is safely stored in the database.
    pub fn generate_api_key(&self) -> (String, String) {
        let raw_token = format!(
            "{}{}{}",
            self.prefix,
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let hash = self.hash_api_key(&raw_token);
        (raw_token, hash)
    }

    /// Hash a raw API key using SHA-256.
    pub fn hash_api_key(&self, raw_key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(raw_key.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }

    /// Verify a raw API key against a stored SHA-256 hash in constant time.
    pub fn verify_api_key(&self, raw_key: &str, stored_hash: &str) -> Result<bool> {
        let computed = self.hash_api_key(raw_key);
        // Constant-time comparison
        use subtle::ConstantTimeEq;
        if computed.as_bytes().ct_eq(stored_hash.as_bytes()).into() {
            Ok(true)
        } else {
            Err(AuthError::InvalidCredentials)
        }
    }
}

impl Default for ApiKeyService {
    fn default() -> Self {
        Self::new("ash_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_lifecycle() {
        let service = ApiKeyService::new("ash_test_");
        let (raw, hash) = service.generate_api_key();

        assert!(raw.starts_with("ash_test_"));
        assert!(service.verify_api_key(&raw, &hash).is_ok());
        assert!(service.verify_api_key("wrong_key", &hash).is_err());
    }
}
