use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ash_core::{Notification, Notifier};

use crate::email::Email;
use crate::mailer::Mailer;

type EmailRule = Arc<dyn Fn(&Notification) -> Option<Email> + Send + Sync + 'static>;

/// Asynchronous notification consumer that composes and delivers transactional emails
/// in response to committed Ash resource actions.
///
/// Implements [`ash_core::Notifier`].
#[derive(Clone)]
pub struct EmailNotifier {
    mailer: Arc<dyn Mailer>,
    rules: Vec<EmailRule>,
}

impl Debug for EmailNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailNotifier")
            .field("mailer", &self.mailer)
            .field("rules_count", &self.rules.len())
            .finish()
    }
}

impl EmailNotifier {
    /// Create a new `EmailNotifier` backed by the given mailer.
    pub fn new(mailer: impl Mailer) -> Self {
        Self {
            mailer: Arc::new(mailer),
            rules: Vec::new(),
        }
    }

    /// Create a new `EmailNotifier` backed by a shared `Arc<dyn Mailer>`.
    pub fn with_arc(mailer: Arc<dyn Mailer>) -> Self {
        Self {
            mailer,
            rules: Vec::new(),
        }
    }

    /// Register an email rule triggered whenever an action matching `action_name` commits.
    pub fn on<F>(mut self, action_name: &'static str, builder: F) -> Self
    where
        F: Fn(&Notification) -> Email + Send + Sync + 'static,
    {
        let rule: EmailRule = Arc::new(move |notif| {
            if notif.action == action_name {
                Some(builder(notif))
            } else {
                None
            }
        });

        self.rules.push(rule);
        self
    }

    /// Register an email rule scoped to both a specific `resource` name and `action` name.
    pub fn on_action<F>(mut self, resource_name: &'static str, action_name: &'static str, builder: F) -> Self
    where
        F: Fn(&Notification) -> Email + Send + Sync + 'static,
    {
        let rule: EmailRule = Arc::new(move |notif| {
            if notif.resource == resource_name && notif.action == action_name {
                Some(builder(notif))
            } else {
                None
            }
        });

        self.rules.push(rule);
        self
    }

    /// Register an arbitrary predicate and email builder rule.
    pub fn rule<F>(mut self, rule_fn: F) -> Self
    where
        F: Fn(&Notification) -> Option<Email> + Send + Sync + 'static,
    {
        let rule: EmailRule = Arc::new(rule_fn);
        self.rules.push(rule);
        self
    }
}

impl Notifier for EmailNotifier {
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = ash_core::Result<()>> + Send + 'a>> {
        let mailer = Arc::clone(&self.mailer);
        let emails: Vec<Email> = self
            .rules
            .iter()
            .filter_map(|r| r(notification))
            .collect();

        Box::pin(async move {
            for email in emails {
                mailer
                    .deliver(email)
                    .await
                    .map_err(|e| ash_core::Error::Invalid(format!("mailer delivery failed: {e}")))?;
            }
            Ok(())
        })
    }
}
