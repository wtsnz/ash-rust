use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use crate::email::Email;
use crate::error::Result;

/// Type alias for an asynchronous boxed future returned by mailer delivery operations.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Pluggable mail transport delivery trait.
///
/// Modeled after Swoosh Mailer in the Phoenix / Ash Elixir ecosystem.
pub trait Mailer: Send + Sync + Debug + 'static {
    /// Deliver an email message asynchronously.
    fn deliver<'a>(&'a self, email: Email) -> BoxFuture<'a, Result<()>>;
}

impl<T: Mailer + ?Sized> Mailer for Arc<T> {
    fn deliver<'a>(&'a self, email: Email) -> BoxFuture<'a, Result<()>> {
        (**self).deliver(email)
    }
}

/// In-memory mailer suitable for unit tests and local mock verification.
///
/// Stores all delivered emails in an internal synchronized collection,
/// matching Phoenix's `Swoosh.Adapters.Test`.
#[derive(Clone, Debug, Default)]
pub struct MemoryMailer {
    delivered: Arc<RwLock<Vec<Email>>>,
}

impl MemoryMailer {
    /// Create a new empty in-memory mailer.
    pub fn new() -> Self {
        Self {
            delivered: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Retrieve a snapshot of all emails delivered so far.
    pub fn delivered_emails(&self) -> Vec<Email> {
        self.delivered.read().unwrap().clone()
    }

    /// Count how many emails have been delivered.
    pub fn delivered_count(&self) -> usize {
        self.delivered.read().unwrap().len()
    }

    /// Retrieve the most recently delivered email.
    pub fn last_delivered(&self) -> Option<Email> {
        self.delivered.read().unwrap().last().cloned()
    }

    /// Filter delivered emails sent to a specific recipient address.
    pub fn find_to(&self, recipient: &str) -> Vec<Email> {
        self.delivered
            .read()
            .unwrap()
            .iter()
            .filter(|e| e.to.iter().any(|t| t == recipient))
            .cloned()
            .collect()
    }

    /// Check if at least one email was delivered to the given recipient.
    pub fn has_delivered_to(&self, recipient: &str) -> bool {
        self.delivered
            .read()
            .unwrap()
            .iter()
            .any(|e| e.to.iter().any(|t| t == recipient))
    }

    /// Clear all delivered emails from memory.
    pub fn clear(&self) {
        self.delivered.write().unwrap().clear();
    }
}

impl Mailer for MemoryMailer {
    fn deliver<'a>(&'a self, email: Email) -> BoxFuture<'a, Result<()>> {
        if let Err(e) = email.validate() {
            return Box::pin(std::future::ready(Err(e)));
        }
        self.delivered.write().unwrap().push(email);
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Console mailer that prints outgoing emails with clean ASCII borders to standard output.
///
/// Ideal for development and debugging without needing an external SMTP server.
#[derive(Clone, Debug)]
pub struct ConsoleMailer {
    tag: String,
}

impl Default for ConsoleMailer {
    fn default() -> Self {
        Self {
            tag: "[ash-mailer]".to_string(),
        }
    }
}

impl ConsoleMailer {
    /// Create a new console mailer with default `[ash-mailer]` tag.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a custom console prefix tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = tag.into();
        self
    }
}

impl Mailer for ConsoleMailer {
    fn deliver<'a>(&'a self, email: Email) -> BoxFuture<'a, Result<()>> {
        if let Err(e) = email.validate() {
            return Box::pin(std::future::ready(Err(e)));
        }

        println!("┌────────────────────────────────────────────────────────┐");
        println!("│ {} OUTGOING EMAIL", self.tag);
        println!("│ From:    {}", email.from.as_deref().unwrap_or(""));
        println!("│ To:      {}", email.to.join(", "));
        if !email.cc.is_empty() {
            println!("│ Cc:      {}", email.cc.join(", "));
        }
        println!("│ Subject: {}", email.subject.as_deref().unwrap_or(""));
        println!("├────────────────────────────────────────────────────────┤");
        if let Some(text) = &email.text {
            for line in text.lines() {
                println!("│ {line}");
            }
        } else if let Some(html) = &email.html {
            for line in html.lines() {
                println!("│ {line}");
            }
        }
        println!("└────────────────────────────────────────────────────────┘");

        Box::pin(std::future::ready(Ok(())))
    }
}

/// No-op mailer that silently accepts and discards messages.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopMailer;

impl Mailer for NoopMailer {
    fn deliver<'a>(&'a self, email: Email) -> BoxFuture<'a, Result<()>> {
        let _ = email;
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Closure-based mailer for testing custom delivery logic or errors.
pub struct CallbackMailer<F> {
    func: F,
}

impl<F> CallbackMailer<F> {
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F> Debug for CallbackMailer<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackMailer").finish()
    }
}

impl<F> Mailer for CallbackMailer<F>
where
    F: Fn(Email) -> Result<()> + Send + Sync + 'static,
{
    fn deliver<'a>(&'a self, email: Email) -> BoxFuture<'a, Result<()>> {
        match (self.func)(email) {
            Ok(()) => Box::pin(std::future::ready(Ok(()))),
            Err(e) => Box::pin(std::future::ready(Err(e))),
        }
    }
}
