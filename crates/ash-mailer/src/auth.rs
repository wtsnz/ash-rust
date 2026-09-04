use std::fmt::Debug;
use std::sync::Arc;

use ash_authentication::{AuthError, AuthSender, BoxFuture};
use ash_core::FieldMap;

use crate::email::Email;
use crate::mailer::Mailer;

type EmailFormatter = Arc<dyn Fn(&FieldMap, &str, Option<&str>) -> Email + Send + Sync>;

/// Authentication sender adapter that formats and delivers authentication emails
/// (password resets, account confirmations, magic links) through a configured [`Mailer`].
///
/// Implements [`ash_authentication::AuthSender`].
#[derive(Clone)]
pub struct AuthMailerSender {
    mailer: Arc<dyn Mailer>,
    from_address: String,
    base_url: Option<String>,
    password_reset_builder: Option<EmailFormatter>,
    confirmation_builder: Option<EmailFormatter>,
    magic_link_builder: Option<EmailFormatter>,
}

impl Debug for AuthMailerSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthMailerSender")
            .field("mailer", &self.mailer)
            .field("from_address", &self.from_address)
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl AuthMailerSender {
    /// Create a new `AuthMailerSender` with the specified mailer and sender address.
    pub fn new(mailer: impl Mailer, from_address: impl Into<String>) -> Self {
        Self {
            mailer: Arc::new(mailer),
            from_address: from_address.into(),
            base_url: None,
            password_reset_builder: None,
            confirmation_builder: None,
            magic_link_builder: None,
        }
    }

    /// Create an `AuthMailerSender` using a shared `Arc<dyn Mailer>`.
    pub fn with_arc(mailer: Arc<dyn Mailer>, from_address: impl Into<String>) -> Self {
        Self {
            mailer,
            from_address: from_address.into(),
            base_url: None,
            password_reset_builder: None,
            confirmation_builder: None,
            magic_link_builder: None,
        }
    }

    /// Set an optional application base URL (e.g. `"https://myapp.com"` or `"http://127.0.0.1:3000"`).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Customize the email template for password reset instructions.
    pub fn with_password_reset_template<F>(mut self, builder: F) -> Self
    where
        F: Fn(&FieldMap, &str, Option<&str>) -> Email + Send + Sync + 'static,
    {
        self.password_reset_builder = Some(Arc::new(builder));
        self
    }

    /// Customize the email template for account confirmation.
    pub fn with_confirmation_template<F>(mut self, builder: F) -> Self
    where
        F: Fn(&FieldMap, &str, Option<&str>) -> Email + Send + Sync + 'static,
    {
        self.confirmation_builder = Some(Arc::new(builder));
        self
    }

    /// Customize the email template for passwordless magic links.
    pub fn with_magic_link_template<F>(mut self, builder: F) -> Self
    where
        F: Fn(&FieldMap, &str, Option<&str>) -> Email + Send + Sync + 'static,
    {
        self.magic_link_builder = Some(Arc::new(builder));
        self
    }

    fn extract_recipient(user_fields: &FieldMap) -> ash_authentication::Result<String> {
        user_fields
            .get("email")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                AuthError::Crypto("user record has no valid 'email' attribute for delivery".into())
            })
    }
}

impl AuthSender for AuthMailerSender {
    fn send_password_reset<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, ash_authentication::Result<()>> {
        Box::pin(async move {
            let email = if let Some(ref builder) = self.password_reset_builder {
                builder(user_fields, token, self.base_url.as_deref())
            } else {
                let to = Self::extract_recipient(user_fields)?;
                let link = match &self.base_url {
                    Some(base) => format!("{base}/auth/reset-password?token={token}"),
                    None => format!("token: {token}"),
                };

                let text = format!(
                    "Hello,\n\nYou requested a password reset for your account.\n\nUse the following token or link:\n{link}\n\nIf you did not request this, please ignore this email.\n"
                );
                let html = format!(
                    "<p>Hello,</p><p>You requested a password reset for your account.</p><p><a href=\"{link}\">Click here to reset your password</a> or use token: <code>{token}</code></p><p>If you did not request this, please ignore this email.</p>"
                );

                Email::new()
                    .from(&self.from_address)
                    .to(to)
                    .subject("Password Reset Instructions")
                    .text(text)
                    .html(html)
            };

            self.mailer
                .deliver(email)
                .await
                .map_err(|e| AuthError::Crypto(e.to_string()))?;

            Ok(())
        })
    }

    fn send_confirmation<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, ash_authentication::Result<()>> {
        Box::pin(async move {
            let email = if let Some(ref builder) = self.confirmation_builder {
                builder(user_fields, token, self.base_url.as_deref())
            } else {
                let to = Self::extract_recipient(user_fields)?;
                let link = match &self.base_url {
                    Some(base) => format!("{base}/auth/confirm?token={token}"),
                    None => format!("token: {token}"),
                };

                let text = format!(
                    "Welcome!\n\nPlease confirm your email address by visiting:\n{link}\n"
                );
                let html = format!(
                    "<p>Welcome!</p><p><a href=\"{link}\">Click here to confirm your email address</a>.</p>"
                );

                Email::new()
                    .from(&self.from_address)
                    .to(to)
                    .subject("Please confirm your email address")
                    .text(text)
                    .html(html)
            };

            self.mailer
                .deliver(email)
                .await
                .map_err(|e| AuthError::Crypto(e.to_string()))?;

            Ok(())
        })
    }

    fn send_magic_link<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, ash_authentication::Result<()>> {
        Box::pin(async move {
            let email = if let Some(ref builder) = self.magic_link_builder {
                builder(user_fields, token, self.base_url.as_deref())
            } else {
                let to = Self::extract_recipient(user_fields)?;
                let link = match &self.base_url {
                    Some(base) => format!("{base}/auth/magic-sign-in?token={token}"),
                    None => format!("token: {token}"),
                };

                let text = format!(
                    "Here is your magic sign-in link:\n{link}\n"
                );
                let html = format!(
                    "<p><a href=\"{link}\">Click here to sign in to your account</a>.</p>"
                );

                Email::new()
                    .from(&self.from_address)
                    .to(to)
                    .subject("Your magic sign-in link")
                    .text(text)
                    .html(html)
            };

            self.mailer
                .deliver(email)
                .await
                .map_err(|e| AuthError::Crypto(e.to_string()))?;

            Ok(())
        })
    }
}
