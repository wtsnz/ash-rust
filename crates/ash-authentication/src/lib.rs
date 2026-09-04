pub mod api_key;
pub mod change;
pub mod def;
pub mod error;
pub mod password;
pub mod strategy;
pub mod token;

#[cfg(feature = "axum")]
pub mod axum;

pub use api_key::ApiKeyService;
pub use change::HashPasswordChange;
pub use def::{ApiKeyStrategyDef, AuthenticationDef, PasswordStrategyDef, TokenStrategyDef};
pub use error::{AuthError, Result};
pub use password::PasswordService;
pub use strategy::AuthStrategy;
pub use token::{Claims, JwtService, MemoryRevocationStore, TokenRevocationStore};

#[cfg(feature = "axum")]
pub use axum::{AuthRejection, AuthTokenResponse, AuthUser, PasswordAuthRequest, auth_router};

#[cfg(not(feature = "axum"))]
#[derive(Clone, Debug)]
pub struct AuthUser(pub ash_core::Actor);

pub use ash_authentication_macros::authentication;
