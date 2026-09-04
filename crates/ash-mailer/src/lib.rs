pub mod email;
pub mod error;
pub mod mailer;
pub mod notifier;

#[cfg(feature = "auth")]
pub mod auth;

pub use email::Email;
pub use error::{MailerError, Result};
pub use mailer::{BoxFuture, CallbackMailer, ConsoleMailer, Mailer, MemoryMailer, NoopMailer};
pub use notifier::EmailNotifier;

#[cfg(feature = "auth")]
pub use auth::AuthMailerSender;

/// Extension trait for [`ash_core::Context`] allowing fluent attachment of an [`EmailNotifier`].
pub trait ContextMailerExt<D> {
    /// Attach an [`EmailNotifier`] to the context using the provided mailer and builder rules.
    fn with_email_notifier(self, notifier: EmailNotifier) -> Self;
}

impl<D> ContextMailerExt<D> for ash_core::Context<D> {
    fn with_email_notifier(self, notifier: EmailNotifier) -> Self {
        self.with_notifier(std::sync::Arc::new(notifier))
    }
}
