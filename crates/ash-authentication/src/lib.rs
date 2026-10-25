pub mod api_key;
pub mod change;
pub mod def;
pub mod error;
mod identity;
pub use identity::BoundIdentity;
pub mod password;
pub mod sender;
pub mod strategy;
pub mod token;
pub mod token_store;

#[cfg(feature = "axum")]
pub mod axum;

pub use api_key::ApiKeyService;
pub use change::HashPasswordChange;
pub use def::{
    ApiKeyStrategyDef, AuthenticationDef, ConfirmationStrategyDef, PasswordStrategyDef,
    TokenStrategyDef,
};
pub use error::{AuthError, Result};
pub use password::PasswordService;
pub use sender::{AuthSender, BoxFuture, CallbackSender, ConsoleSender, NoopSender};
pub use strategy::AuthStrategy;
pub use token::{Claims, JwtService, TokenPair};
pub use token_store::{AshToken, DatabaseTokenStore};

#[cfg(feature = "axum")]
pub use axum::{
    AuthRejection, AuthTokenResponse, AuthUser, ChangePasswordRequest, PasswordAuthRequest,
    RefreshTokenRequest, RequestAuthenticator, RequestPasswordResetRequest, ResetPasswordRequest,
    RevokeTokenRequest, auth_router,
};

#[cfg(not(feature = "axum"))]
#[derive(Clone, Debug)]
pub struct AuthUser(pub BoundIdentity);

pub use ash_authentication_macros::authentication;
