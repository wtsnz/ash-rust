use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;

use crate::error::{AuthError, Result};

/// Password hashing and verification service using Argon2id.
#[derive(Clone, Debug, Default)]
pub struct PasswordService {
    min_length: usize,
}

impl PasswordService {
    /// Create a new `PasswordService` with the default minimum password length of 8 characters.
    pub fn new() -> Self {
        Self { min_length: 8 }
    }

    /// Set minimum password length constraint.
    pub fn with_min_length(mut self, min: usize) -> Self {
        self.min_length = min;
        self
    }

    /// Minimum password length required.
    pub fn min_length(&self) -> usize {
        self.min_length
    }

    /// Validate password against length constraints.
    pub fn validate_password(&self, password: &str) -> Result<()> {
        if password.chars().count() < self.min_length {
            return Err(AuthError::WeakPassword(format!(
                "password must be at least {} characters",
                self.min_length
            )));
        }
        Ok(())
    }

    /// Hash a plaintext password into a PHC-formatted Argon2id string.
    pub fn hash_password(&self, password: &str) -> Result<String> {
        self.validate_password(password)?;

        let argon2 = Argon2::default();

        let password_hash = argon2
            .hash_password(password.as_bytes())
            .map_err(|e| AuthError::Crypto(e.to_string()))?
            .to_string();

        Ok(password_hash)
    }

    /// Verify a plaintext password against a stored PHC-formatted Argon2id hash.
    ///
    /// Runs in constant time to prevent side-channel timing attacks.
    pub fn verify_password(&self, password: &str, hashed_password: &str) -> Result<bool> {
        let parsed_hash = match PasswordHash::new(hashed_password) {
            Ok(h) => h,
            Err(e) => return Err(AuthError::Crypto(e.to_string())),
        };

        match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
            Err(e) => Err(AuthError::Crypto(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify_password() {
        let service = PasswordService::new();
        let pass = "super-secret-password-123";
        let hash = service.hash_password(pass).unwrap();

        assert!(hash.starts_with("$argon2id$"));
        assert!(service.verify_password(pass, &hash).unwrap());
        assert!(!service.verify_password("wrong-password", &hash).unwrap());
    }

    #[test]
    fn test_password_length_validation() {
        let service = PasswordService::new().with_min_length(10);
        assert!(service.hash_password("short").is_err());
        assert!(service.hash_password("long-enough-password").is_ok());
    }
}
