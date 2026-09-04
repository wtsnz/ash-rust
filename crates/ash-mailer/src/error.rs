use std::fmt;

/// Errors arising from email composition, template rendering, and delivery operations.
#[derive(Debug)]
pub enum MailerError {
    /// Attempted to send an email without at least one recipient.
    MissingRecipient,
    /// Attempted to send an email without a sender `From` address.
    MissingSender,
    /// Missing or invalid subject header.
    MissingSubject,
    /// Delivery failed via the underlying transport.
    DeliveryFailed(String),
    /// Underlying Ash Core error.
    Core(ash_core::Error),
    /// Authentication error when integrated with ash-authentication.
    #[cfg(feature = "auth")]
    Auth(ash_authentication::AuthError),
}

impl fmt::Display for MailerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRecipient => write!(f, "email has no recipient (To, Cc, or Bcc)"),
            Self::MissingSender => write!(f, "email has no sender (From) address"),
            Self::MissingSubject => write!(f, "email has no subject"),
            Self::DeliveryFailed(msg) => write!(f, "email delivery failed: {msg}"),
            Self::Core(err) => write!(f, "ash core error: {err}"),
            #[cfg(feature = "auth")]
            Self::Auth(err) => write!(f, "auth error: {err}"),
        }
    }
}

impl std::error::Error for MailerError {}

impl From<ash_core::Error> for MailerError {
    fn from(err: ash_core::Error) -> Self {
        Self::Core(err)
    }
}

#[cfg(feature = "auth")]
impl From<ash_authentication::AuthError> for MailerError {
    fn from(err: ash_authentication::AuthError) -> Self {
        Self::Auth(err)
    }
}

#[cfg(feature = "auth")]
impl From<MailerError> for ash_authentication::AuthError {
    fn from(err: MailerError) -> Self {
        ash_authentication::AuthError::Crypto(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, MailerError>;
