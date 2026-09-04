use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ash_core::FieldMap;

use crate::error::Result;

/// Type alias for an asynchronous boxed future returned by sender methods.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Trait for dispatching authentication tokens (e.g. password reset, confirmation, magic link)
/// to end users without binding authentication to a specific transport (Email, SMS, Push, etc.).
///
/// Modeled after `AshAuthentication.Sender` in the Elixir Ash Framework.
pub trait AuthSender: Send + Sync + Debug + 'static {
    /// Send password reset instructions containing the reset token.
    fn send_password_reset<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>>;

    /// Send account confirmation instructions containing the confirmation token.
    fn send_confirmation<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        let _ = (user_fields, token);
        Box::pin(std::future::ready(Ok(())))
    }

    /// Send a passwordless magic sign-in link containing the token.
    fn send_magic_link<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        let _ = (user_fields, token);
        Box::pin(std::future::ready(Ok(())))
    }
}

impl<T: AuthSender + ?Sized> AuthSender for Arc<T> {
    fn send_password_reset<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        (**self).send_password_reset(user_fields, token)
    }

    fn send_confirmation<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        (**self).send_confirmation(user_fields, token)
    }

    fn send_magic_link<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        (**self).send_magic_link(user_fields, token)
    }
}

/// No-op authentication sender that silently discards all outgoing messages.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopSender;

impl AuthSender for NoopSender {
    fn send_password_reset<'a>(
        &'a self,
        _user_fields: &'a FieldMap,
        _token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Console sender that prints outgoing reset tokens and links to stdout.
///
/// Ideal for local development and rapid prototyping.
#[derive(Clone, Debug, Default)]
pub struct ConsoleSender {
    prefix: String,
}

impl ConsoleSender {
    /// Create a new console sender.
    pub fn new() -> Self {
        Self {
            prefix: "[ash-auth]".to_string(),
        }
    }

    /// Set a custom console prefix tag.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }
}

impl AuthSender for ConsoleSender {
    fn send_password_reset<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        let email = user_fields
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        println!(
            "{} [PASSWORD RESET] To: {} | Token: {}",
            self.prefix, email, token
        );
        Box::pin(std::future::ready(Ok(())))
    }

    fn send_confirmation<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        let email = user_fields
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        println!(
            "{} [CONFIRMATION] To: {} | Token: {}",
            self.prefix, email, token
        );
        Box::pin(std::future::ready(Ok(())))
    }

    fn send_magic_link<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        let email = user_fields
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        println!(
            "{} [MAGIC LINK] To: {} | Token: {}",
            self.prefix, email, token
        );
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Dynamic callback sender allowing custom closure-based dispatch.
pub struct CallbackSender<F> {
    callback: F,
}

impl<F> CallbackSender<F> {
    pub fn new(callback: F) -> Self {
        Self { callback }
    }
}

impl<F> Debug for CallbackSender<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackSender").finish()
    }
}

impl<F> AuthSender for CallbackSender<F>
where
    F: Fn(&str, &FieldMap, &str) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
{
    fn send_password_reset<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        (self.callback)("password_reset", user_fields, token)
    }

    fn send_confirmation<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        (self.callback)("confirmation", user_fields, token)
    }

    fn send_magic_link<'a>(
        &'a self,
        user_fields: &'a FieldMap,
        token: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        (self.callback)("magic_link", user_fields, token)
    }
}
