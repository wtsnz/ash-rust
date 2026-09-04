use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AuthError, Result};

fn default_purpose() -> String {
    "access".to_string()
}

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
    /// Purpose of this token ("access", "refresh", "password_reset", etc.).
    #[serde(default = "default_purpose")]
    pub purpose: String,
    /// Optional user role (e.g. "admin", "member").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Optional tenant string identifier for multi-tenant isolation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// Token pair returned upon successful authentication or token refresh.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenPair {
    /// Short-lived bearer token used for accessing protected resources.
    pub access_token: String,
    /// Long-lived token used for rotating and issuing new token pairs.
    pub refresh_token: String,
    /// Access token validity duration in seconds.
    pub expires_in: u64,
    /// Authorization header scheme (always "Bearer").
    pub token_type: String,
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

/// JWT Token Service responsible for signing, verifying, rotating, and revoking tokens.
#[derive(Clone)]
pub struct JwtService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    token_lifetime: Duration,
    refresh_token_lifetime: Duration,
    reset_token_lifetime: Duration,
    confirmation_token_lifetime: Duration,
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
            refresh_token_lifetime: Duration::from_secs(86400 * 14), // Default 14 days
            reset_token_lifetime: Duration::from_secs(900), // Default 15 minutes
            confirmation_token_lifetime: Duration::from_secs(86400 * 3), // Default 3 days
            revocation_store: Some(Arc::new(MemoryRevocationStore::new())),
        }
    }

    /// Set custom access token lifetime duration.
    pub fn with_lifetime(mut self, lifetime: Duration) -> Self {
        self.token_lifetime = lifetime;
        self
    }

    /// Set custom refresh token lifetime duration.
    pub fn with_refresh_lifetime(mut self, lifetime: Duration) -> Self {
        self.refresh_token_lifetime = lifetime;
        self
    }

    /// Set custom password reset token lifetime duration.
    pub fn with_reset_lifetime(mut self, lifetime: Duration) -> Self {
        self.reset_token_lifetime = lifetime;
        self
    }

    /// Set custom email confirmation token lifetime duration.
    pub fn with_confirmation_lifetime(mut self, lifetime: Duration) -> Self {
        self.confirmation_token_lifetime = lifetime;
        self
    }

    /// Access token lifetime duration.
    pub fn token_lifetime(&self) -> Duration {
        self.token_lifetime
    }

    /// Refresh token lifetime duration.
    pub fn refresh_token_lifetime(&self) -> Duration {
        self.refresh_token_lifetime
    }

    /// Password reset token lifetime duration.
    pub fn reset_token_lifetime(&self) -> Duration {
        self.reset_token_lifetime
    }

    /// Confirmation token lifetime duration.
    pub fn confirmation_token_lifetime(&self) -> Duration {
        self.confirmation_token_lifetime
    }

    /// Attach a custom token revocation store.
    pub fn with_revocation_store(mut self, store: Arc<dyn TokenRevocationStore>) -> Self {
        self.revocation_store = Some(store);
        self
    }

    /// Get reference to revocation store if present.
    pub fn revocation_store(&self) -> Option<&Arc<dyn TokenRevocationStore>> {
        self.revocation_store.as_ref()
    }

    /// Generate and sign a new JWT access token (alias for [`sign_access_token`]).
    pub fn sign_token(
        &self,
        user_id: Uuid,
        role: Option<String>,
        tenant: Option<String>,
    ) -> Result<String> {
        self.sign_access_token(user_id, role, tenant)
    }

    /// Generate and sign a short-lived JWT access token (`purpose = "access"`).
    pub fn sign_access_token(
        &self,
        user_id: Uuid,
        role: Option<String>,
        tenant: Option<String>,
    ) -> Result<String> {
        self.sign_custom_token(user_id, "access", self.token_lifetime, role, tenant)
    }

    /// Generate and sign a long-lived JWT refresh token (`purpose = "refresh"`).
    pub fn sign_refresh_token(
        &self,
        user_id: Uuid,
        role: Option<String>,
        tenant: Option<String>,
    ) -> Result<String> {
        self.sign_custom_token(
            user_id,
            "refresh",
            self.refresh_token_lifetime,
            role,
            tenant,
        )
    }

    /// Generate and sign a short-lived password reset token (`purpose = "password_reset"`).
    pub fn sign_reset_token(
        &self,
        user_id: Uuid,
        tenant: Option<String>,
    ) -> Result<String> {
        self.sign_custom_token(
            user_id,
            "password_reset",
            self.reset_token_lifetime,
            None,
            tenant,
        )
    }

    /// Generate and sign an email confirmation token (`purpose = "email_confirmation"`).
    pub fn sign_confirmation_token(
        &self,
        user_id: Uuid,
        tenant: Option<String>,
    ) -> Result<String> {
        self.sign_custom_token(
            user_id,
            "email_confirmation",
            self.confirmation_token_lifetime,
            None,
            tenant,
        )
    }

    /// Generate an `(access_token, refresh_token)` pair for the given user.
    pub fn sign_token_pair(
        &self,
        user_id: Uuid,
        role: Option<String>,
        tenant: Option<String>,
    ) -> Result<TokenPair> {
        let access_token = self.sign_access_token(user_id, role.clone(), tenant.clone())?;
        let refresh_token = self.sign_refresh_token(user_id, role, tenant)?;

        Ok(TokenPair {
            access_token,
            refresh_token,
            expires_in: self.token_lifetime.as_secs(),
            token_type: "Bearer".to_string(),
        })
    }

    fn sign_custom_token(
        &self,
        user_id: Uuid,
        purpose: &str,
        lifetime: Duration,
        role: Option<String>,
        tenant: Option<String>,
    ) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AuthError::Crypto(e.to_string()))?
            .as_secs();

        let exp = now + lifetime.as_secs();
        let jti = Uuid::new_v4().to_string();

        let claims = Claims {
            sub: user_id,
            exp,
            iat: now,
            jti,
            purpose: purpose.to_string(),
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

    /// Verify a token and ensure its purpose matches the expected purpose (e.g. "access", "refresh", "password_reset").
    pub fn verify_token_with_purpose(&self, token: &str, expected_purpose: &str) -> Result<Claims> {
        let claims = self.verify_token(token)?;
        if claims.purpose != expected_purpose {
            return Err(AuthError::InvalidToken(format!(
                "invalid token purpose: expected '{}', got '{}'",
                expected_purpose, claims.purpose
            )));
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

    /// Revoke a token ID (`jti`) directly.
    pub fn revoke_jti(&self, jti: &str, expires_at: u64) -> Result<()> {
        if let Some(store) = &self.revocation_store {
            store.revoke(jti, expires_at)?;
        }
        Ok(())
    }

    /// Check whether a `jti` has been revoked.
    pub fn is_revoked(&self, jti: &str) -> Result<bool> {
        if let Some(store) = &self.revocation_store {
            store.is_revoked(jti)
        } else {
            Ok(false)
        }
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
        assert_eq!(claims.purpose, "access");
        assert!(claims.tenant.is_none());
    }

    #[test]
    fn test_jwt_token_pair_and_purpose() {
        let service = JwtService::new("my-super-secret-key-that-is-at-least-32-chars-long");
        let user_id = Uuid::new_v4();

        let pair = service
            .sign_token_pair(user_id, Some("member".to_string()), None)
            .unwrap();

        assert_eq!(pair.token_type, "Bearer");
        assert_eq!(pair.expires_in, 3600);

        let access_claims = service
            .verify_token_with_purpose(&pair.access_token, "access")
            .unwrap();
        assert_eq!(access_claims.sub, user_id);

        let refresh_claims = service
            .verify_token_with_purpose(&pair.refresh_token, "refresh")
            .unwrap();
        assert_eq!(refresh_claims.sub, user_id);

        // Attempting to verify refresh token as access token must fail
        assert!(
            service
                .verify_token_with_purpose(&pair.refresh_token, "access")
                .is_err()
        );
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
