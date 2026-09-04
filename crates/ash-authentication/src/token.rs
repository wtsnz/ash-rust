use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AuthError, Result};

/// JWT token claims payload conforming to RFC 7519 standards.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject identifier (the authenticated User ID).
    pub sub: Uuid,
    /// Expiration time (seconds since Unix epoch).
    pub exp: u64,
    /// Issued-at time (seconds since Unix epoch).
    pub iat: u64,
    /// Unique token identifier (JTI) for revocation tracking.
    pub jti: String,
    /// Optional user role (e.g. "admin", "member").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Optional tenant string identifier for multi-tenant isolation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// Token revocation storage trait for invalidating active tokens.
pub trait TokenRevocationStore: Send + Sync {
    /// Mark a token ID (`jti`) as revoked until its natural expiration.
    fn revoke(&self, jti: &str, expires_at: u64) -> Result<()>;
    /// Check whether a token ID (`jti`) has been revoked.
    fn is_revoked(&self, jti: &str) -> Result<bool>;
}

/// In-memory token revocation store suitable for single-instance applications or testing.
#[derive(Clone, Debug, Default)]
pub struct MemoryRevocationStore {
    revoked: Arc<RwLock<HashSet<String>>>,
}

impl MemoryRevocationStore {
    pub fn new() -> Self {
        Self {
            revoked: Arc::new(RwLock::new(HashSet::new())),
        }
    }
}

impl TokenRevocationStore for MemoryRevocationStore {
    fn revoke(&self, jti: &str, _expires_at: u64) -> Result<()> {
        let mut set = self
            .revoked
            .write()
            .map_err(|e| AuthError::Crypto(e.to_string()))?;
        set.insert(jti.to_string());
        Ok(())
    }

    fn is_revoked(&self, jti: &str) -> Result<bool> {
        let set = self
            .revoked
            .read()
            .map_err(|e| AuthError::Crypto(e.to_string()))?;
        Ok(set.contains(jti))
    }
}

/// JWT Token Service responsible for signing and verifying tokens.
#[derive(Clone)]
pub struct JwtService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    token_lifetime: Duration,
    revocation_store: Option<Arc<dyn TokenRevocationStore>>,
}

impl JwtService {
    /// Create a new `JwtService` with a secret signing key.
    pub fn new(secret: impl AsRef<[u8]>) -> Self {
        let secret_bytes = secret.as_ref();
        Self {
            encoding_key: EncodingKey::from_secret(secret_bytes),
            decoding_key: DecodingKey::from_secret(secret_bytes),
            token_lifetime: Duration::from_secs(3600), // Default 1 hour
            revocation_store: Some(Arc::new(MemoryRevocationStore::new())),
        }
    }

    /// Set custom token lifetime duration.
    pub fn with_lifetime(mut self, lifetime: Duration) -> Self {
        self.token_lifetime = lifetime;
        self
    }

    /// Attach a custom token revocation store.
    pub fn with_revocation_store(mut self, store: Arc<dyn TokenRevocationStore>) -> Self {
        self.revocation_store = Some(store);
        self
    }

    /// Generate and sign a new JWT token for a given user ID, optional role, and optional tenant.
    pub fn sign_token(
        &self,
        user_id: Uuid,
        role: Option<String>,
        tenant: Option<String>,
    ) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AuthError::Crypto(e.to_string()))?
            .as_secs();

        let exp = now + self.token_lifetime.as_secs();
        let jti = Uuid::new_v4().to_string();

        let claims = Claims {
            sub: user_id,
            exp,
            iat: now,
            jti,
            role,
            tenant,
        };

        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| AuthError::InvalidToken(e.to_string()))
    }

    /// Verify and decode a JWT bearer token, checking signature, expiration, and revocation status.
    pub fn verify_token(&self, token: &str) -> Result<Claims> {
        let validation = Validation::default();

        let token_data = decode::<Claims>(token, &self.decoding_key, &validation).map_err(|e| {
            if e.kind() == &jsonwebtoken::errors::ErrorKind::ExpiredSignature {
                AuthError::TokenExpired
            } else {
                AuthError::InvalidToken(e.to_string())
            }
        })?;

        let claims = token_data.claims;

        if let Some(store) = &self.revocation_store
            && store.is_revoked(&claims.jti)?
        {
            return Err(AuthError::TokenRevoked);
        }

        Ok(claims)
    }

    /// Revoke an active token.
    pub fn revoke_token(&self, token: &str) -> Result<()> {
        let claims = self.verify_token(token)?;
        if let Some(store) = &self.revocation_store {
            store.revoke(&claims.jti, claims.exp)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_sign_and_verify() {
        let service = JwtService::new("my-super-secret-key-that-is-at-least-32-chars-long");
        let user_id = Uuid::new_v4();

        let token = service
            .sign_token(user_id, Some("admin".to_string()), None)
            .unwrap();

        let claims = service.verify_token(&token).unwrap();
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.role.as_deref(), Some("admin"));
        assert!(claims.tenant.is_none());
    }

    #[test]
    fn test_jwt_revocation() {
        let service = JwtService::new("my-super-secret-key-that-is-at-least-32-chars-long");
        let user_id = Uuid::new_v4();

        let token = service.sign_token(user_id, None, None).unwrap();
        assert!(service.verify_token(&token).is_ok());

        service.revoke_token(&token).unwrap();
        assert!(matches!(
            service.verify_token(&token),
            Err(AuthError::TokenRevoked)
        ));
    }
}
