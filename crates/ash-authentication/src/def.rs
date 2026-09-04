use std::any::Any;
use std::fmt::Debug;

use ash_core::ResourceExtension;

/// Configuration definition for password authentication strategy.
#[derive(Clone, Debug)]
pub struct PasswordStrategyDef {
    /// Action name for user registration (e.g., "register_with_password").
    pub register_action_name: &'static str,
    /// Action name for sign-in (e.g., "sign_in_with_password").
    pub sign_in_action_name: &'static str,
    /// Field containing user identity (e.g., "email" or "username").
    pub identity_field: &'static str,
    /// Field storing the hashed password (e.g., "hashed_password").
    pub hashed_password_field: &'static str,
    /// Minimum password length (default 8).
    pub min_password_length: usize,
    /// Whether password confirmation argument is required.
    pub require_confirmation: bool,
}

impl Default for PasswordStrategyDef {
    fn default() -> Self {
        Self {
            register_action_name: "register_with_password",
            sign_in_action_name: "sign_in_with_password",
            identity_field: "email",
            hashed_password_field: "hashed_password",
            min_password_length: 8,
            require_confirmation: true,
        }
    }
}

/// Configuration definition for token strategy.
#[derive(Clone, Debug)]
pub struct TokenStrategyDef {
    /// Lifetime of issued tokens in seconds.
    pub token_lifetime_secs: u64,
    /// Whether token revocation is enabled.
    pub track_revocations: bool,
}

impl Default for TokenStrategyDef {
    fn default() -> Self {
        Self {
            token_lifetime_secs: 3600,
            track_revocations: true,
        }
    }
}

/// Configuration definition for API Key strategy.
#[derive(Clone, Debug)]
pub struct ApiKeyStrategyDef {
    /// Name of the API key attribute (e.g., "api_key_hash").
    pub api_key_field: &'static str,
    /// Prefix for generated API keys (e.g., "ash_live_").
    pub key_prefix: &'static str,
}

impl Default for ApiKeyStrategyDef {
    fn default() -> Self {
        Self {
            api_key_field: "api_key_hash",
            key_prefix: "ash_",
        }
    }
}

/// Configuration definition for email/account confirmation strategy.
#[derive(Clone, Debug)]
pub struct ConfirmationStrategyDef {
    /// Attribute tracking confirmation state (e.g., "confirmed_at" or "confirmed").
    pub confirmed_field: &'static str,
    /// Whether confirmation is strictly required to sign in.
    pub prevent_unconfirmed_sign_in: bool,
    /// Confirmation token lifetime in seconds (default 86400 = 24 hours).
    pub token_lifetime_secs: u64,
}

impl Default for ConfirmationStrategyDef {
    fn default() -> Self {
        Self {
            confirmed_field: "confirmed_at",
            prevent_unconfirmed_sign_in: true,
            token_lifetime_secs: 86400,
        }
    }
}

/// Resource extension holding metadata for all enabled authentication strategies.
#[derive(Clone, Debug, Default)]
pub struct AuthenticationDef {
    pub password: Option<PasswordStrategyDef>,
    pub tokens: Option<TokenStrategyDef>,
    pub api_key: Option<ApiKeyStrategyDef>,
    pub confirmation: Option<ConfirmationStrategyDef>,
}

impl ResourceExtension for AuthenticationDef {
    fn name(&self) -> &'static str {
        "ash_authentication"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
