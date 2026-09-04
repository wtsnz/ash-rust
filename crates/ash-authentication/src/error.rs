use std::fmt;

/// Errors arising from authentication and token operations.
#[derive(Debug)]
pub enum AuthError {
    /// Invalid username/email or password credentials.
    InvalidCredentials,
    /// Targeted user account was not found.
    UserNotFound,
    /// Password does not meet complexity/length requirements.
    WeakPassword(String),
    /// The supplied password confirmation does not match the password.
    PasswordConfirmationMismatch,
    /// Token verification failed or token is malformed.
    InvalidToken(String),
    /// Token has expired.
    TokenExpired,
    /// Token has been revoked.
    TokenRevoked,
    /// Missing `Authorization: Bearer <token>` header or parameter.
    MissingToken,
    /// Cryptographic or hashing failure.
    Crypto(String),
    /// Underlying Ash Core error.
    Core(ash_core::Error),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredentials => write!(f, "invalid credentials"),
            Self::UserNotFound => write!(f, "user not found"),
            Self::WeakPassword(msg) => write!(f, "weak password: {msg}"),
            Self::PasswordConfirmationMismatch => write!(f, "password confirmation does not match"),
            Self::InvalidToken(msg) => write!(f, "invalid token: {msg}"),
            Self::TokenExpired => write!(f, "token has expired"),
            Self::TokenRevoked => write!(f, "token has been revoked"),
            Self::MissingToken => write!(f, "missing authentication token"),
            Self::Crypto(msg) => write!(f, "cryptographic error: {msg}"),
            Self::Core(err) => write!(f, "ash core error: {err}"),
        }
    }
}

impl std::error::Error for AuthError {}

impl From<ash_core::Error> for AuthError {
    fn from(err: ash_core::Error) -> Self {
        Self::Core(err)
    }
}

impl From<AuthError> for ash_core::Error {
    fn from(err: AuthError) -> Self {
        ash_core::Error::Authentication(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AuthError>;
